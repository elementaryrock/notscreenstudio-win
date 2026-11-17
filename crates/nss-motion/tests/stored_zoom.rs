//! Round 3 zoom behaviour: auto-zooms stored as blocks, follow vs fixed focus, per-zoom ease.
//!
//! Port of upstream `Tests/NSSMotionTests/StoredZoomTests.swift` (the cases not already covered
//! inside the crate's unit tests).

use nss_core::{
    ClickEvent, ClickKind, CursorSample, CursorStyle, CursorTrack, MediaTime, MotionModel,
    MouseButton, NormalizedPoint, Timeline, ZoomBlock, ZoomFocus,
};
use nss_motion::{MotionTuning, SpringMotionModel, ZoomOrigin, ZoomPlanner};

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

fn wander() -> Vec<CursorSample> {
    make_samples(20.0, 120.0, |t| {
        NormalizedPoint::new(0.3 + 0.2 * t.sin(), 0.4 + 0.1 * (t * 0.7).cos())
    })
}

fn clicks() -> Vec<ClickEvent> {
    vec![
        click(2.0, ClickKind::Down, 0.3, 0.3),
        click(2.6, ClickKind::Down, 0.32, 0.31),
        click(8.0, ClickKind::Down, 0.8, 0.7),
        click(14.0, ClickKind::Down, 0.2, 0.8),
    ]
}

fn build(
    samples: Vec<CursorSample>,
    clicks: Vec<ClickEvent>,
    timeline: Timeline,
    duration: MediaTime,
) -> SpringMotionModel {
    SpringMotionModel::new(
        &CursorTrack {
            samples,
            clicks,
            ..CursorTrack::default()
        },
        &timeline,
        CursorStyle::default(),
        duration,
        16.0 / 9.0,
        MotionTuning::standard(),
    )
}

#[test]
fn auto_blocks_match_the_on_the_fly_segments() {
    let tuning = MotionTuning::standard();
    let track = CursorTrack {
        samples: wander(),
        clicks: clicks(),
        ..CursorTrack::default()
    };
    let segs = ZoomPlanner::segments(&track, &Timeline::default(), 20.0, 16.0 / 9.0, &tuning);
    let blocks = ZoomPlanner::auto_zoom_blocks(&track, 20.0, 16.0 / 9.0, &[], &tuning);

    assert_eq!(blocks.len(), segs.len());
    assert!(!blocks.is_empty());
    for (b, s) in blocks.iter().zip(segs.iter()) {
        assert_eq!((b.start, b.end, b.scale), (s.start, s.end, s.scale));
        assert!(b.is_auto);
        assert_eq!(b.focus, ZoomFocus::Auto {});
        assert_eq!(b.ease_in, tuning.zoom_spring.response);
        assert_eq!(b.ease_out, b.ease_in);
    }
    for w in blocks.windows(2) {
        assert!(w[0].end <= w[1].start, "stored blocks must not overlap");
    }
}

#[test]
fn follow_block_aims_at_the_lead_in_click_then_follows() {
    let tuning = MotionTuning::standard();
    let track = CursorTrack {
        samples: wander(),
        clicks: vec![click(5.3, ClickKind::Down, 0.7, 0.6)],
        ..CursorTrack::default()
    };
    let block = ZoomBlock {
        focus: ZoomFocus::Auto {},
        ..ZoomBlock::new(5.0, 9.0)
    };
    let segs = ZoomPlanner::segments(
        &track,
        &Timeline {
            zoom_blocks: vec![block.clone()],
            auto_zoom_enabled: false,
            ..Timeline::default()
        },
        20.0,
        16.0 / 9.0,
        &tuning,
    );
    assert_eq!(segs.len(), 1);
    assert_eq!(segs[0].follow_from, Some(5.3));
    assert_eq!(segs[0].initial_focus, NormalizedPoint::new(0.7, 0.6));

    // A click later in the block does not move the start of following.
    let late = CursorTrack {
        samples: wander(),
        clicks: vec![click(7.0, ClickKind::Down, 0.7, 0.6)],
        ..CursorTrack::default()
    };
    let s2 = ZoomPlanner::segments(
        &late,
        &Timeline {
            zoom_blocks: vec![block],
            auto_zoom_enabled: false,
            ..Timeline::default()
        },
        20.0,
        16.0 / 9.0,
        &tuning,
    );
    assert_eq!(s2[0].follow_from, Some(5.0));
}

#[test]
fn follow_cursor_pans_but_a_fixed_point_holds() {
    // The cursor sweeps left to right while zoomed in.
    let sweep = make_samples(10.0, 120.0, |t| {
        NormalizedPoint::new((0.1 + 0.1 * t).min(0.95), 0.5)
    });
    let p = NormalizedPoint::new(0.3, 0.4);
    let follow = build(
        sweep.clone(),
        Vec::new(),
        Timeline {
            zoom_blocks: vec![ZoomBlock {
                focus: ZoomFocus::Auto {},
                ..ZoomBlock::new(1.0, 9.0)
            }],
            auto_zoom_enabled: false,
            ..Timeline::default()
        },
        10.0,
    );
    let fixed = build(
        sweep,
        Vec::new(),
        Timeline {
            zoom_blocks: vec![ZoomBlock {
                focus: ZoomFocus::Manual(p),
                ..ZoomBlock::new(1.0, 9.0)
            }],
            auto_zoom_enabled: false,
            ..Timeline::default()
        },
        10.0,
    );

    let f3 = follow.frame_state(3.0).camera.center.x;
    let f8 = follow.frame_state(8.0).camera.center.x;
    assert!(f8 - f3 > 0.2, "following should pan: {f3} -> {f8}");

    let a = fixed.frame_state(4.0).camera;
    let b = fixed.frame_state(8.0).camera;
    assert!((a.center.x - p.x).abs() < 1e-3 && (a.center.y - p.y).abs() < 1e-3);
    assert!((b.center.x - p.x).abs() < 1e-3);
    assert!((a.scale - 2.0).abs() < 1e-2);
}

#[test]
fn manual_scale_is_clamped_by_the_tuning() {
    // A corrupt or over-ambitious stored scale must not zoom past max_zoom_scale.
    let m = build(
        Vec::new(),
        Vec::new(),
        Timeline {
            zoom_blocks: vec![ZoomBlock {
                focus: ZoomFocus::Manual(NormalizedPoint::CENTER),
                ..ZoomBlock {
                    scale: 100.0,
                    ..ZoomBlock::new(1.0, 5.0)
                }
            }],
            auto_zoom_enabled: false,
            ..Timeline::default()
        },
        8.0,
    );
    assert_eq!(
        m.zoom_segments[0].scale,
        MotionTuning::standard().max_zoom_scale
    );
    assert!(
        (m.frame_state(4.0).camera.scale - MotionTuning::standard().max_zoom_scale).abs() < 1e-2
    );
}

#[test]
fn segment_origins_identify_manual_blocks() {
    let block = ZoomBlock {
        focus: ZoomFocus::Manual(NormalizedPoint::CENTER),
        ..ZoomBlock::new(1.0, 3.0)
    };
    let id = block.id;
    let m = build(
        Vec::new(),
        vec![click(6.0, ClickKind::Down, 0.5, 0.5)],
        Timeline {
            zoom_blocks: vec![block],
            ..Timeline::default()
        },
        12.0,
    );
    assert!(m
        .zoom_segments
        .iter()
        .any(|s| s.origin == ZoomOrigin::Manual(id)));
    assert!(m.zoom_segments.iter().any(|s| s.origin == ZoomOrigin::Auto));
}
