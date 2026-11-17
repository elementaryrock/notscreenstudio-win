//! The viewport guarantee, challenged with adversarial input.
//!
//! Port of upstream `Tests/NSSMotionTests/ViewportPropertyTests.swift`. These are the tests that
//! justify the strongest claim in the plan — "the screen content never shows a gap or the
//! background at its edges" — for ANY tuning, aspect and recording, including bouncy springs that
//! overshoot and clicks on (or beyond) the corners.

use nss_core::{
    CameraState, ClickEvent, ClickKind, CursorSample, CursorStyle, CursorTrack, MediaTime,
    MotionModel, MouseButton, NormalizedPoint, Timeline, ZoomBlock, ZoomFocus,
};
use nss_motion::viewport::{clamped_to_source, CameraViewport};
use nss_motion::{factory, MotionTuning, Spring, SpringMotionModel};

/// Deterministic generator so a failing case can be replayed from its seed.
struct SplitMix64(u64);

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`.
    fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + self.unit() * (hi - lo)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    fn boolean(&mut self) -> bool {
        self.next_u64() % 2 == 0
    }

    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.below(items.len())]
    }
}

/// 120 Hz samples from a position function over `[0, duration]`.
fn make_samples(
    duration: f64,
    rate: f64,
    mut f: impl FnMut(f64) -> NormalizedPoint,
) -> Vec<CursorSample> {
    let n = (duration * rate) as usize;
    (0..=n)
        .map(|i| {
            let t = i as f64 / rate;
            let p = f(t);
            CursorSample::new(t, p.x, p.y)
        })
        .collect()
}

fn click(t: MediaTime, kind: ClickKind, x: f64, y: f64) -> ClickEvent {
    ClickEvent::new(t, x, y, MouseButton::Left, kind)
}

/// Mirror of `RenderLayout` (which lives in `nss-render`), so this crate can check the region the
/// renderer actually samples for each output aspect. Returns the source coordinates of the four
/// corners of the content rect.
fn rendered_source_corners(
    camera: CameraState,
    output: (f64, f64),
    source_aspect: f64,
    padding: f64,
) -> [NormalizedPoint; 4] {
    let (w, h) = output;
    let pad = padding.max(0.0) * h;
    let avail_w = (w - 2.0 * pad).max(1.0);
    let avail_h = (h - 2.0 * pad).max(1.0);
    let (cw, ch) = if source_aspect > avail_w / avail_h {
        (avail_w, avail_w / source_aspect)
    } else {
        (avail_h * source_aspect, avail_h)
    };
    let (rx, ry) = ((w - cw) / 2.0, (h - ch) / 2.0);
    [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)].map(|(fx, fy)| {
        let px = rx + fx * cw;
        let py = ry + fy * ch;
        let ux = (px - rx) / cw;
        let uy = (py - ry) / ch;
        NormalizedPoint::new(
            camera.center.x + (ux - 0.5) / camera.scale,
            camera.center.y + (uy - 0.5) / camera.scale,
        )
    })
}

const OUTPUT_SIZES: [(&str, f64, f64); 3] = [
    ("16:9", 1920.0, 1080.0),
    ("1:1", 1080.0, 1080.0),
    ("9:16", 1080.0, 1920.0),
];

const SOURCE_ASPECTS: [f64; 7] = [
    16.0 / 9.0,
    16.0 / 10.0,
    4.0 / 3.0,
    1.0,
    9.0 / 16.0,
    21.0 / 9.0,
    32.0 / 9.0,
];

/// Asserts the viewport contract for one frame. Returns false on the first violation.
fn check_viewport(c: CameraState, source_aspect: f64, padding: f64, context: &str) -> bool {
    let v = c.source_viewport();
    let ok = c.scale.is_finite()
        && c.scale >= 1.0
        && c.center.x.is_finite()
        && c.center.y.is_finite()
        && v.is_inside_source();
    assert!(
        ok,
        "viewport {v:?} scale {} left the source: {context}",
        c.scale
    );
    if !ok {
        return false;
    }
    for (name, w, h) in OUTPUT_SIZES {
        for p in rendered_source_corners(c, (w, h), source_aspect, padding) {
            // The mirror goes through pixel space, so allow rounding error only.
            let inside = (-1e-9..=1.0 + 1e-9).contains(&p.x) && (-1e-9..=1.0 + 1e-9).contains(&p.y);
            assert!(
                inside,
                "{name} output samples source {p:?} outside 0...1: {context}"
            );
            if !inside {
                return false;
            }
        }
    }
    true
}

/// Random but adversarial tuning: underdamped springs (overshoot), extreme scales and dead zones.
fn random_tuning(r: &mut SplitMix64) -> MotionTuning {
    MotionTuning {
        simulation_rate: r.pick(&[60.0, 120.0, 240.0, 333.0]),
        zoom_spring: Spring::new(r.range(0.05, 1.5), r.range(0.1, 2.0)),
        pan_spring: Spring::new(r.range(0.05, 1.5), r.range(0.1, 2.0)),
        max_zoom_scale: r.range(1.0, 12.0),
        auto_zoom_scale: r.range(0.5, 14.0),
        auto_zoom_lead_in: r.range(0.0, 1.5),
        auto_zoom_hold: r.range(0.1, 3.0),
        cluster_max_distance: r.range(0.01, 1.0),
        cluster_pan_gap: r.range(0.0, 3.0),
        min_zoom_out_gap: r.range(0.0, 2.0),
        dead_zone: r.range(0.0, 1.5),
        pre_aim_max_scale: r.range(1.0, 3.0),
        ..MotionTuning::default()
    }
}

/// A position that is often exactly on a corner or edge, sometimes slightly outside (window capture).
fn random_point(r: &mut SplitMix64) -> NormalizedPoint {
    let coord = |r: &mut SplitMix64| -> f64 {
        match r.below(6) {
            0 => 0.0,
            1 => 1.0,
            2 => r.range(-0.2, 0.0),
            3 => r.range(1.0, 1.2),
            _ => r.range(0.0, 1.0),
        }
    };
    let x = coord(r);
    let y = coord(r);
    NormalizedPoint::new(x, y)
}

#[test]
fn random_clicks_tunings_and_aspects_never_show_edges() {
    // Mirrors upstream ViewportPropertyTests.randomClicksTuningsAndAspectsNeverShowEdges (24 trials).
    for trial in 0..24usize {
        let seed = 1000 + trial as u64;
        let mut r = SplitMix64::new(seed);
        let tuning = random_tuning(&mut r);
        let source_aspect = SOURCE_ASPECTS[trial % SOURCE_ASPECTS.len()];
        let duration = r.range(4.0, 16.0);

        // Cursor: random walk with jumps to corners and edges.
        let mut p = random_point(&mut r);
        let samples = make_samples(duration, 120.0, |_| {
            if r.below(60) == 0 {
                p = random_point(&mut r);
            }
            p.x += r.range(-0.01, 0.01);
            p.y += r.range(-0.01, 0.01);
            p
        });

        // Clicks: bursts (pans inside one segment), isolated clicks, drags, many at the corners.
        let mut clicks: Vec<ClickEvent> = Vec::new();
        let mut t = r.range(0.0, 1.0);
        while t < duration {
            let at = random_point(&mut r);
            clicks.push(click(t, ClickKind::Down, at.x, at.y));
            let hold = if r.boolean() { 0.05 } else { r.range(0.0, 1.5) };
            clicks.push(click(t + hold, ClickKind::Up, at.x, at.y));
            t += if r.boolean() {
                r.range(0.1, 0.8)
            } else {
                r.range(0.8, 4.0)
            };
        }

        // Manual blocks with focus anywhere, including outside the source, and extreme scales.
        let mut blocks: Vec<ZoomBlock> = Vec::new();
        for _ in 0..r.below(4) {
            let s = r.range(0.0, duration);
            let focus = if r.boolean() {
                ZoomFocus::Auto {}
            } else {
                ZoomFocus::Manual(random_point(&mut r))
            };
            blocks.push(ZoomBlock {
                focus,
                ..ZoomBlock {
                    scale: r.range(0.5, 20.0),
                    ..ZoomBlock::new(s, s + r.range(0.2, 4.0))
                }
            });
        }

        let style = CursorStyle {
            smoothing: r.range(0.0, 1.0),
            hide_when_idle: r.boolean(),
            ..CursorStyle::default()
        };
        let model = SpringMotionModel::new(
            &CursorTrack {
                samples,
                clicks,
                ..CursorTrack::default()
            },
            &Timeline {
                zoom_blocks: blocks,
                auto_zoom_enabled: trial % 5 != 0,
                ..Timeline::default()
            },
            style,
            duration,
            source_aspect,
            tuning,
        );

        // Every simulated step, plus random times in between (interpolation) and outside the range.
        let mut times: Vec<f64> = Vec::new();
        let mut tt = -0.5;
        while tt <= duration + 0.5 {
            times.push(tt);
            tt += 1.0 / 240.0;
        }
        for _ in 0..2000 {
            times.push(r.range(0.0, duration));
        }
        for time in times {
            let c = model.frame_state(time).camera;
            if !check_viewport(c, source_aspect, 0.06, &format!("seed {seed} t {time}")) {
                return;
            }
        }
    }
}

#[test]
fn corner_clicks_with_overshooting_springs_stay_inside() {
    // The camera overshoots both while zooming and while panning, and must still stop exactly at
    // the source edges.
    let tuning = MotionTuning {
        zoom_spring: Spring::new(0.6, 0.25),
        pan_spring: Spring::new(0.4, 0.2),
        auto_zoom_scale: 3.0,
        ..MotionTuning::default()
    };

    let corners = [
        NormalizedPoint::new(0.0, 0.0),
        NormalizedPoint::new(1.0, 0.0),
        NormalizedPoint::new(1.0, 1.0),
        NormalizedPoint::new(0.0, 1.0),
    ];
    let mut clicks: Vec<ClickEvent> = Vec::new();
    for (i, c) in corners.iter().enumerate() {
        clicks.push(click(1.0 + i as f64 * 0.6, ClickKind::Down, c.x, c.y));
    }
    for (i, c) in corners.iter().enumerate() {
        clicks.push(click(8.0 + i as f64 * 4.0, ClickKind::Down, c.x, c.y));
    }
    let samples = make_samples(26.0, 120.0, |t| {
        clicks
            .iter()
            .rfind(|c| c.t <= t)
            .map_or(NormalizedPoint::CENTER, |c| c.point())
    });

    for aspect in SOURCE_ASPECTS {
        let m = SpringMotionModel::new(
            &CursorTrack {
                samples: samples.clone(),
                clicks: clicks.clone(),
                ..CursorTrack::default()
            },
            &Timeline::default(),
            CursorStyle::default(),
            26.0,
            aspect,
            tuning,
        );
        let mut peak = 1.0f64;
        let mut touched_edge = false;
        let mut t = 0.0;
        while t <= 26.0 {
            let c = m.frame_state(t).camera;
            if !check_viewport(c, aspect, 0.06, &format!("aspect {aspect} t {t}")) {
                return;
            }
            peak = peak.max(c.scale);
            let v = c.source_viewport();
            if c.scale > 2.0
                && (v.min_x == 0.0 || v.max_x == 1.0)
                && (v.min_y == 0.0 || v.max_y == 1.0)
            {
                touched_edge = true;
            }
            t += 1.0 / 480.0;
        }
        assert!(
            peak > 3.3,
            "the zoom spring should overshoot 3x (peak {peak})"
        );
        assert!(
            touched_edge,
            "corner clicks should pin the viewport to the corners"
        );
    }
}

#[test]
fn settled_corner_zoom_is_pinned_to_the_corner() {
    let m = factory(
        &CursorTrack {
            clicks: vec![click(1.0, ClickKind::Down, 1.0, 1.0)],
            ..CursorTrack::default()
        },
        &Timeline::default(),
        CursorStyle::default(),
        6.0,
        16.0 / 9.0,
    );
    let c = m.frame_state(2.5).camera;
    assert!((c.scale - 2.0).abs() < 1e-3);
    let v = c.source_viewport();
    assert_eq!((v.max_x, v.max_y), (1.0, 1.0));
    assert!((v.min_x - (1.0 - 1.0 / c.scale)).abs() < 1e-12);
}

#[test]
fn non_finite_clicks_and_blocks_cannot_poison_the_camera() {
    let clicks = [
        click(1.0, ClickKind::Down, f64::NAN, 0.5),
        click(1.2, ClickKind::Down, 0.5, f64::INFINITY),
        click(1.4, ClickKind::Down, 0.9, 0.9),
    ];
    let blocks = [ZoomBlock {
        focus: ZoomFocus::Manual(NormalizedPoint::new(f64::NAN, f64::NEG_INFINITY)),
        ..ZoomBlock {
            scale: f64::INFINITY,
            ..ZoomBlock::new(4.0, 6.0)
        }
    }];
    let m = SpringMotionModel::new(
        &CursorTrack {
            clicks: clicks.to_vec(),
            ..CursorTrack::default()
        },
        &Timeline {
            zoom_blocks: blocks.to_vec(),
            ..Timeline::default()
        },
        CursorStyle::default(),
        8.0,
        16.0 / 9.0,
        MotionTuning::standard(),
    );
    let mut t = 0.0;
    while t <= 8.0 {
        check_viewport(m.frame_state(t).camera, 16.0 / 9.0, 0.06, &format!("t {t}"));
        t += 1.0 / 120.0;
    }
}

#[test]
fn clamp_is_idempotent() {
    // Re-clamping a clamped camera must not move it: the renderer relies on that when it
    // recomputes the layout for a different aspect ratio mid-frame.
    let mut r = SplitMix64::new(99);
    for _ in 0..5000 {
        let s = r.range(0.0, 30.0);
        let p = random_point(&mut r);
        let once = clamped_to_source(s, p);
        let twice = clamped_to_source(once.scale, once.center);
        assert_eq!(once, twice);
    }
}
