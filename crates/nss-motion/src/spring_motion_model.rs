//! The real [`MotionModel`]: spring camera with auto-zoom, spring-smoothed cursor, idle fade,
//! click ripples, cursor shapes (with short flickers removed) and the press squish.
//!
//! Port of upstream `NSSMotion/SpringMotionModel.swift`.
//!
//! **Determinism:** everything stateful (springs, dead zone, idle fade) is simulated ONCE at
//! construction with a fixed step ([`MotionTuning::simulation_rate`], 240 Hz) from the trim
//! start, where all springs start settled, to the trim end. [`frame_state`](SpringMotionModel::frame_state)
//! then reads that table: a direct index on the uniform grid plus linear interpolation, O(1), no
//! allocation except the ripple array when a ripple is visible. The same `t` gives the same
//! `FrameState`, in any order, from any thread — which is why preview, scrubbing and export
//! match exactly.

use nss_core::{
    ClickEvent, ClickKind, CursorSample, CursorShape, CursorShapeChange, CursorState, CursorStyle,
    CursorTrack, FrameState, MediaTime, MotionModel, MouseButton, NormalizedPoint, RippleState,
    Timeline,
};

use crate::motion_tuning::{max_steps, MotionTuning};
use crate::spring::SpringStepper;
use crate::viewport::{clamped_to_source, CameraViewport, SourceViewport};
use crate::zoom_planner::{ZoomOrigin, ZoomPlanner, ZoomSegment};

/// Compact per-step state: 36 bytes, so 10 minutes at 240 Hz is about 5.2 MB.
#[derive(Debug, Clone, Copy, PartialEq)]
struct SimFrame {
    log_scale: f32,
    focus_x: f32,
    focus_y: f32,
    cursor_x: f32,
    cursor_y: f32,
    cursor_vx: f32,
    cursor_vy: f32,
    opacity: f32,
    press_scale: f32,
}

pub struct SpringMotionModel {
    pub tuning: MotionTuning,
    pub cursor_style: CursorStyle,
    pub source_aspect: f64,
    /// Resolved zoom segments (auto + manual), sorted and non-overlapping. Useful for the timeline UI.
    pub zoom_segments: Vec<ZoomSegment>,
    /// First and last simulated source time (the trim range).
    pub start_time: MediaTime,
    pub end_time: MediaTime,

    track: CursorTrack,
    rate: f64,
    frames: Vec<SimFrame>,
    raw_cursor: bool,
    ripple_times: Vec<MediaTime>,
    ripple_points: Vec<NormalizedPoint>,
    /// Shape changes with flickers shorter than `tuning.min_shape_duration` removed.
    shape_track: CursorTrack,
}

impl SpringMotionModel {
    /// - `duration`: recording duration; if not positive, derived from the last sample/click.
    /// - `source_aspect`: width / height of the recorded source (for example
    ///   `sourcePixelSize.aspect()`).
    #[must_use]
    pub fn new(
        track: &CursorTrack,
        timeline: &Timeline,
        cursor_style: CursorStyle,
        duration: MediaTime,
        source_aspect: f64,
        tuning: MotionTuning,
    ) -> Self {
        let mut samples: Vec<CursorSample> = track
            .samples
            .iter()
            .copied()
            .filter(|s| s.t.is_finite() && s.x.is_finite() && s.y.is_finite())
            .collect();
        if !is_sorted(&samples) {
            samples.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));
        }
        let clicks: Vec<ClickEvent> = track
            .clicks
            .iter()
            .copied()
            .filter(|c| c.t.is_finite() && c.x.is_finite() && c.y.is_finite())
            .collect();
        let track = CursorTrack {
            samples,
            clicks,
            sample_rate: track.sample_rate,
            shapes: track.shapes.clone(),
        };

        let mut duration = if duration.is_finite() { duration } else { 0.0 };
        if duration <= 0.0 {
            let last_sample = track.samples.last().map_or(0.0, |s| s.t);
            let last_click = track
                .clicks
                .iter()
                .map(|c| c.t)
                .filter(|t| t.is_finite())
                .fold(0.0f64, f64::max);
            duration = last_sample.max(last_click);
        }
        // Bound the simulated range: a bogus duration or absolute timestamps must not allocate
        // without limit.
        duration = duration.clamp(0.0, tuning.effective_max_duration());

        let trim_end = timeline.trim.resolved_end(duration);
        let trim_start = if timeline.trim.start.is_finite() {
            timeline.trim.start
        } else {
            0.0
        };
        let start = trim_start.clamp(0.0, duration);
        let end = if trim_end.is_finite() {
            trim_end.max(start)
        } else {
            duration.max(start)
        };
        let rate = tuning.effective_rate(end - start);

        let zoom_segments =
            ZoomPlanner::segments(&track, timeline, duration, source_aspect, &tuning);

        let mut downs: Vec<ClickEvent> = track
            .clicks
            .iter()
            .copied()
            .filter(|c| c.kind == ClickKind::Down && c.t.is_finite())
            .collect();
        downs.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));
        let ripple_times: Vec<MediaTime> = downs.iter().map(|c| c.t).collect();
        let ripple_points: Vec<NormalizedPoint> = downs.iter().map(|c| c.point()).collect();

        let shape_track = CursorTrack {
            shapes: Self::stable_shapes(&track.shapes, tuning.min_shape_duration),
            ..CursorTrack::default()
        };
        let frames = Self::simulate(
            &track,
            &zoom_segments,
            cursor_style,
            start,
            end,
            rate,
            &tuning,
        );

        Self {
            tuning,
            cursor_style,
            source_aspect,
            zoom_segments,
            start_time: start,
            end_time: end,
            raw_cursor: tuning.cursor_spring(cursor_style.smoothing).is_none(),
            track,
            rate,
            frames,
            ripple_times,
            ripple_points,
            shape_track,
        }
    }

    /// Build with the standard tuning and a 16:9 assumption.
    #[must_use]
    pub fn with_default_aspect(
        track: &CursorTrack,
        timeline: &Timeline,
        cursor_style: CursorStyle,
        duration: MediaTime,
    ) -> Self {
        Self::new(
            track,
            timeline,
            cursor_style,
            duration,
            16.0 / 9.0,
            MotionTuning::standard(),
        )
    }

    /// The visible source region at `t`.
    #[must_use]
    pub fn viewport(&self, t: MediaTime) -> SourceViewport {
        self.frame_state(t).camera.source_viewport()
    }

    /// `t` clamped to the simulated (trim) range, like every part of the frame state.
    #[must_use]
    pub fn clamped_time(&self, t: MediaTime) -> MediaTime {
        if t.is_finite() {
            t.clamp(self.start_time, self.end_time)
        } else {
            self.start_time
        }
    }

    /// Number of simulated frames (bounded by [`MotionTuning::max_simulation_frames`]).
    #[must_use]
    pub fn simulated_frame_count(&self) -> usize {
        self.frames.len()
    }

    /// Ripples for every mouse-down in `(t - ripple_duration, t]`. `t` is already clamped.
    fn ripples(&self, t: MediaTime) -> Vec<RippleState> {
        if !self.cursor_style.click_ripples || self.ripple_times.is_empty() || !t.is_finite() {
            return Vec::new();
        }
        let d = self.tuning.ripple_duration.max(1e-6);
        // First click with time > t - d.
        let (mut lo, mut hi) = (0usize, self.ripple_times.len());
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.ripple_times[mid] <= t - d {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        if lo >= self.ripple_times.len() || self.ripple_times[lo] > t {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut k = lo;
        while k < self.ripple_times.len() && self.ripple_times[k] <= t {
            out.push(RippleState::new(
                self.ripple_points[k],
                ((t - self.ripple_times[k]) / d).clamp(0.0, 1.0),
            ));
            k += 1;
        }
        out
    }

    /// Removes shape changes that are replaced within `min_duration` (a flicker): the shape before
    /// the flicker simply continues. Repeats are merged too.
    #[must_use]
    pub fn stable_shapes(
        changes: &[CursorShapeChange],
        min_duration: f64,
    ) -> Vec<CursorShapeChange> {
        let mut sorted: Vec<CursorShapeChange> = changes
            .iter()
            .copied()
            .filter(|c| c.t.is_finite())
            .collect();
        // Stable by time, keeping input order for equal times.
        sorted.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));

        let min_duration = if min_duration.is_finite() {
            min_duration.max(0.0)
        } else {
            0.0
        };
        let mut out: Vec<CursorShapeChange> = Vec::with_capacity(sorted.len());
        for (i, c) in sorted.iter().enumerate() {
            let next = if i + 1 < sorted.len() {
                sorted[i + 1].t
            } else {
                f64::INFINITY
            };
            if next - c.t < min_duration {
                continue;
            }
            // Before the first change the arrow is implied, so a leading arrow is redundant too.
            if out.last().map_or(CursorShape::Arrow, |o| o.shape) == c.shape {
                continue;
            }
            out.push(*c);
        }
        out
    }

    fn simulate(
        track: &CursorTrack,
        segments: &[ZoomSegment],
        cursor_style: CursorStyle,
        start: MediaTime,
        end: MediaTime,
        rate: f64,
        tuning: &MotionTuning,
    ) -> Vec<SimFrame> {
        let dt = 1.0 / rate;
        let cap = max_steps(tuning);
        let span = ((end - start) * rate).ceil();
        let steps = if span.is_finite() {
            (span.min(cap as f64) as usize).max(1)
        } else {
            cap
        };
        let mut frames: Vec<SimFrame> = Vec::with_capacity(steps + 1);

        // Zoom springs: the tuning's spring between blocks with no ease set, otherwise one spring
        // per segment edge (ease-in from `start`, ease-out from `end`) with the segment's ease as
        // response.
        let zoom_step = tuning.zoom_spring.stepper(dt);
        let mut zoom_steps: Vec<SpringStepper> = Vec::with_capacity(segments.len() * 2);
        for seg in segments {
            zoom_steps.push(
                crate::spring::Spring::new(seg.ease_in, tuning.zoom_spring.damping_ratio)
                    .stepper(dt),
            );
            zoom_steps.push(
                crate::spring::Spring::new(seg.ease_out, tuning.zoom_spring.damping_ratio)
                    .stepper(dt),
            );
        }
        let pan_step = tuning.pan_spring.stepper(dt);
        let cursor_step = tuning
            .cursor_spring(cursor_style.smoothing)
            .map(|s| s.stepper(dt));

        let mut cursor = RawCursor::new(&track.samples);
        let mut planner = CameraPlanner::new(segments, *tuning);
        let mut activity = ActivityTracker::new(track, *tuning, cursor_style.hide_when_idle);
        let mut presses = PressTracker::new(&track.clicks, tuning.press_min_hold);
        let press_down_step = tuning.press_down_spring.stepper(dt);
        let press_release_step = tuning.press_release_spring.stepper(dt);
        let pressed_scale = tuning.effective_press_scale();

        // Settled initial state at the trim start.
        let raw0 = cursor.position(start);
        let target0 = planner.target(start, raw0, 1.0);
        let mut log_scale = target0.scale.ln();
        let mut log_scale_v = 0.0;
        let (mut fx, mut fx_v) = (target0.focus.x, 0.0);
        let (mut fy, mut fy_v) = (target0.focus.y, 0.0);
        let (mut cx, mut cx_v) = (raw0.map_or(0.5, |p| p.x), 0.0);
        let (mut cy, mut cy_v) = (raw0.map_or(0.5, |p| p.y), 0.0);
        let mut opacity = activity.settled_opacity(start);
        let mut press = if presses.is_pressed(start) {
            pressed_scale
        } else {
            1.0
        };
        let mut press_v = 0.0;

        macro_rules! record {
            () => {
                frames.push(SimFrame {
                    log_scale: log_scale as f32,
                    focus_x: fx as f32,
                    focus_y: fy as f32,
                    cursor_x: cx as f32,
                    cursor_y: cy as f32,
                    cursor_vx: cx_v as f32,
                    cursor_vy: cy_v as f32,
                    opacity: opacity as f32,
                    press_scale: press as f32,
                })
            };
        }

        record!();
        let mut prev_raw = raw0;
        for k in 1..=steps {
            let t = start + (k as f64) * dt;
            let raw = cursor.position(t);
            let target = planner.target(t, raw, log_scale.exp());
            let zi = planner.zoom_spring_index;
            let stepper = if zi >= 0 && (zi as usize) < zoom_steps.len() {
                &zoom_steps[zi as usize]
            } else {
                &zoom_step
            };
            stepper.step(&mut log_scale, &mut log_scale_v, target.scale.ln());
            pan_step.step(&mut fx, &mut fx_v, target.focus.x);
            pan_step.step(&mut fy, &mut fy_v, target.focus.y);
            if let Some(raw) = raw {
                if let Some(step) = cursor_step {
                    step.step(&mut cx, &mut cx_v, raw.x);
                    step.step(&mut cy, &mut cy_v, raw.y);
                } else {
                    // Raw cursor velocity: slope of the raw track (finite difference over one step).
                    if let Some(p) = prev_raw {
                        cx_v = (raw.x - p.x) * rate;
                        cy_v = (raw.y - p.y) * rate;
                    } else {
                        cx_v = 0.0;
                        cy_v = 0.0;
                    }
                    cx = raw.x;
                    cy = raw.y;
                }
            }
            opacity = activity.opacity(opacity, t, dt);
            if presses.is_pressed(t) {
                press_down_step.step(&mut press, &mut press_v, pressed_scale);
            } else {
                press_release_step.step(&mut press, &mut press_v, 1.0);
            }
            prev_raw = raw;
            record!();
        }
        frames
    }
}

impl MotionModel for SpringMotionModel {
    fn frame_state(&self, t: MediaTime) -> FrameState {
        let tc = self.clamped_time(t);
        let u = (tc - self.start_time) * self.rate;
        let mut i = u.floor() as isize;
        if i >= self.frames.len() as isize - 1 {
            i = self.frames.len() as isize - 2;
        }
        if i < 0 {
            i = 0;
        }
        let i = i as usize;
        let f = (u - i as f64).clamp(0.0, 1.0) as f32;
        let (a, b) = (self.frames[i], self.frames[i + 1]);
        let mix = |x: f32, y: f32| -> f64 {
            // Interpolate in f32 (as upstream) then widen, so the table stays compact.
            f64::from(x + (y - x) * f)
        };

        // Camera: interpolated spring state, then clamp the viewport inside the source. The clamp
        // runs after the springs, so overshoot can never move the viewport outside.
        let camera = clamped_to_source(
            mix(a.log_scale, b.log_scale).exp(),
            NormalizedPoint::new(mix(a.focus_x, b.focus_x), mix(a.focus_y, b.focus_y)),
        );

        // Cursor.
        let has_cursor = !self.track.samples.is_empty();
        let mut position =
            NormalizedPoint::new(mix(a.cursor_x, b.cursor_x), mix(a.cursor_y, b.cursor_y));
        if self.raw_cursor {
            if let Some(raw) = self.track.raw_position(tc) {
                position = raw;
            }
        }
        let velocity =
            NormalizedPoint::new(mix(a.cursor_vx, b.cursor_vx), mix(a.cursor_vy, b.cursor_vy));
        let opacity = if has_cursor {
            mix(a.opacity, b.opacity).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let cursor = CursorState {
            position: if has_cursor {
                position
            } else {
                NormalizedPoint::CENTER
            },
            velocity: if has_cursor {
                velocity
            } else {
                NormalizedPoint::ZERO
            },
            visible: has_cursor && opacity > 0.0,
            ripples: self.ripples(tc),
            opacity,
            shape: self.shape_track.shape(tc),
            press_scale: if has_cursor {
                mix(a.press_scale, b.press_scale)
            } else {
                1.0
            },
        };
        FrameState::with(t, camera, cursor)
    }

    fn duration(&self) -> MediaTime {
        self.end_time
    }
}

// MARK: - Simulation helpers (sequential cursors over sorted input; used only during the build)

/// Linear interpolation of the raw cursor with a forward-only index (O(1) amortized per step).
struct RawCursor<'a> {
    samples: &'a [CursorSample],
    i: usize,
}

impl<'a> RawCursor<'a> {
    fn new(samples: &'a [CursorSample]) -> Self {
        Self { samples, i: 0 }
    }

    fn position(&mut self, t: MediaTime) -> Option<NormalizedPoint> {
        let first = *self.samples.first()?;
        let last = *self.samples.last()?;
        if t <= first.t {
            return Some(first.point());
        }
        if t >= last.t {
            return Some(last.point());
        }
        while self.i + 1 < self.samples.len() && self.samples[self.i + 1].t <= t {
            self.i += 1;
        }
        let a = self.samples[self.i];
        let b = *self.samples.get(self.i + 1)?;
        let span = b.t - a.t;
        let f = if span > 0.0 { (t - a.t) / span } else { 0.0 };
        Some(NormalizedPoint::new(
            a.x + (b.x - a.x) * f,
            a.y + (b.y - a.y) * f,
        ))
    }
}

/// The camera target (scale + focus) over time, including the dead-zone follow state.
struct CameraPlanner {
    segments: Vec<ZoomSegment>,
    tuning: MotionTuning,
    index: usize,
    active: Option<usize>,
    focus: NormalizedPoint,
    /// Which zoom spring the last `target` call asks for: `2 * i` = ease-in of segment i,
    /// `2 * i + 1` = ease-out of segment i, -1 = the default spring (before the first segment).
    zoom_spring_index: isize,
}

impl CameraPlanner {
    fn new(segments: &[ZoomSegment], tuning: MotionTuning) -> Self {
        Self {
            segments: segments.to_vec(),
            tuning,
            index: 0,
            active: None,
            focus: NormalizedPoint::CENTER,
            zoom_spring_index: -1,
        }
    }

    fn clamp_focus(p: NormalizedPoint, scale: f64) -> NormalizedPoint {
        clamped_to_source(scale, p).center
    }

    /// - `current_scale`: the camera's current (spring) scale, used to delay re-aiming between
    ///   segments until the zoom-out has finished.
    fn target(
        &mut self,
        t: MediaTime,
        cursor: Option<NormalizedPoint>,
        current_scale: f64,
    ) -> CameraTarget {
        while self.index < self.segments.len() && self.segments[self.index].end <= t {
            self.index += 1;
        }
        self.zoom_spring_index = if self.index > 0 {
            2 * (self.index as isize - 1) + 1
        } else {
            -1
        };
        let Some(seg) = self.segments.get(self.index).cloned() else {
            self.active = None;
            return CameraTarget {
                scale: 1.0,
                focus: self.focus,
            };
        };
        if seg.start > t {
            // Between segments: zoom out around the current focus. Once (nearly) fully zoomed out,
            // quietly pre-aim at the next segment so its zoom-in heads straight for it; invisible at
            // scale ~1, where the center is clamped to the middle.
            self.active = None;
            if current_scale <= self.tuning.pre_aim_max_scale {
                self.focus = Self::clamp_focus(seg.initial_focus, seg.scale);
            }
            return CameraTarget {
                scale: 1.0,
                focus: self.focus,
            };
        }
        self.zoom_spring_index = 2 * self.index as isize;
        if self.active != Some(self.index) {
            self.active = Some(self.index);
            self.focus = Self::clamp_focus(seg.initial_focus, seg.scale);
        }
        if let (Some(from), Some(p)) = (seg.follow_from, cursor) {
            if t >= from {
                // Dead zone: pan only when the cursor leaves the central part of the viewport.
                let dz = self.tuning.dead_zone.max(0.0) * 0.5 / seg.scale;
                let mut f = self.focus;
                if p.x > f.x + dz {
                    f.x = p.x - dz;
                } else if p.x < f.x - dz {
                    f.x = p.x + dz;
                }
                if p.y > f.y + dz {
                    f.y = p.y - dz;
                } else if p.y < f.y - dz {
                    f.y = p.y + dz;
                }
                self.focus = Self::clamp_focus(f, seg.scale);
            }
        }
        CameraTarget {
            scale: seg.scale,
            focus: self.focus,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct CameraTarget {
    scale: f64,
    focus: NormalizedPoint,
}

/// When a mouse button is held. Each mouse-down is paired with the next mouse-up of the same
/// button and held for at least `min_hold` (a down without an up is held for `min_hold`).
/// Overlapping holds (several buttons) are merged. Queried with non-decreasing times.
struct PressTracker {
    /// Merged press intervals, in time order.
    pub intervals: Vec<(MediaTime, MediaTime)>,
    i: usize,
}

impl PressTracker {
    fn new(clicks: &[ClickEvent], min_hold: f64) -> Self {
        let hold = if min_hold.is_finite() {
            min_hold.max(0.0)
        } else {
            0.0
        };
        let mut events: Vec<ClickEvent> =
            clicks.iter().copied().filter(|c| c.t.is_finite()).collect();
        events.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));

        let mut raw: Vec<(MediaTime, MediaTime)> = Vec::new();
        let mut open: Vec<(MouseButton, usize)> = Vec::new();
        for e in events {
            match e.kind {
                ClickKind::Down => {
                    open.retain(|(b, _)| *b != e.button);
                    open.push((e.button, raw.len()));
                    raw.push((e.t, e.t + hold));
                }
                ClickKind::Up => {
                    if let Some(pos) = open.iter().position(|(b, _)| *b == e.button) {
                        let (_, k) = open.remove(pos);
                        raw[k].1 = raw[k].1.max(e.t);
                    }
                }
            }
        }
        raw.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        let mut intervals: Vec<(MediaTime, MediaTime)> = Vec::new();
        for r in raw {
            match intervals.last_mut() {
                Some(last) if r.0 <= last.1 => last.1 = last.1.max(r.1),
                _ => intervals.push(r),
            }
        }
        Self { intervals, i: 0 }
    }

    fn is_pressed(&mut self, t: MediaTime) -> bool {
        while self.i < self.intervals.len() && self.intervals[self.i].1 <= t {
            self.i += 1;
        }
        self.i < self.intervals.len() && self.intervals[self.i].0 <= t
    }
}

/// Idle detection and the cursor fade (only when `hideWhenIdle`).
struct ActivityTracker {
    samples: Vec<CursorSample>,
    clicks: Vec<MediaTime>,
    tuning: MotionTuning,
    enabled: bool,
    si: usize,
    ci: usize,
    last_activity: MediaTime,
}

impl ActivityTracker {
    fn new(track: &CursorTrack, tuning: MotionTuning, hide_when_idle: bool) -> Self {
        let mut clicks: Vec<MediaTime> = track
            .clicks
            .iter()
            .filter(|c| c.t.is_finite())
            .map(|c| c.t)
            .collect();
        clicks.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        Self {
            samples: track.samples.clone(),
            clicks,
            tuning,
            enabled: hide_when_idle,
            si: 0,
            ci: 0,
            last_activity: 0.0,
        }
    }

    /// Advance to `t` and return the time of the last movement or click at or before `t`.
    fn advance(&mut self, t: MediaTime) -> MediaTime {
        while self.si < self.samples.len() && self.samples[self.si].t <= t {
            if self.si > 0 {
                let a = self.samples[self.si - 1];
                let b = self.samples[self.si];
                if (b.x - a.x).abs() + (b.y - a.y).abs() > self.tuning.activity_epsilon {
                    self.last_activity = self.last_activity.max(b.t);
                }
            }
            self.si += 1;
        }
        while self.ci < self.clicks.len() && self.clicks[self.ci] <= t {
            self.last_activity = self.last_activity.max(self.clicks[self.ci]);
            self.ci += 1;
        }
        self.last_activity
    }

    fn target_opacity(&self, t: MediaTime) -> f64 {
        if t - self.last_activity >= self.tuning.idle_hide_delay {
            0.0
        } else {
            1.0
        }
    }

    fn settled_opacity(&mut self, t: MediaTime) -> f64 {
        if !self.enabled {
            return 1.0;
        }
        self.advance(t);
        self.target_opacity(t)
    }

    fn opacity(&mut self, current: f64, t: MediaTime, dt: f64) -> f64 {
        if !self.enabled {
            return 1.0;
        }
        self.advance(t);
        let target = self.target_opacity(t);
        if target > current {
            target.min(current + dt / self.tuning.idle_fade_in_duration.max(1e-6))
        } else {
            target.max(current - dt / self.tuning.idle_fade_out_duration.max(1e-6))
        }
    }
}

fn is_sorted(samples: &[CursorSample]) -> bool {
    samples.windows(2).all(|w| w[0].t <= w[1].t)
}

/// True when the origin is an auto-zoom segment (helper for UI code).
#[must_use]
pub fn is_auto_segment(seg: &ZoomSegment) -> bool {
    matches!(seg.origin, ZoomOrigin::Auto)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{click, make_samples, model};
    use nss_core::{TrimRange, ZoomBlock, ZoomFocus};
    use std::f64::consts::PI;

    fn wander() -> Vec<CursorSample> {
        make_samples(20.0, 120.0, |t| {
            NormalizedPoint::new(0.5 + 0.4 * (t * 0.9).sin(), 0.5 + 0.35 * (t * 1.3).cos())
        })
    }

    #[test]
    fn deterministic_forward_backward_and_across_builds() {
        let clicks = [
            click(2.0, ClickKind::Down, 0.2, 0.3),
            click(2.6, ClickKind::Down, 0.25, 0.35),
            click(9.0, ClickKind::Down, 0.8, 0.8),
        ];
        let m1 = model(
            wander(),
            &clicks,
            Timeline::default(),
            CursorStyle::default(),
            20.0,
        );
        let m2 = model(
            wander(),
            &clicks,
            Timeline::default(),
            CursorStyle::default(),
            20.0,
        );
        let times: Vec<f64> = (0..=1200).map(|i| i as f64 / 60.0).collect();
        let forward: Vec<FrameState> = times.iter().map(|t| m1.frame_state(*t)).collect();
        let backward: Vec<FrameState> = times
            .iter()
            .rev()
            .map(|t| m1.frame_state(*t))
            .rev()
            .collect();
        assert_eq!(forward, backward);
        for k in 0..500 {
            let t = (k as f64) * 0.04 % 20.0;
            assert_eq!(m1.frame_state(t), m2.frame_state(t));
        }
    }

    #[test]
    fn viewport_stays_inside_source() {
        // Mirrors upstream SpringMotionModelTests.viewportStaysInsideSource.
        let corners = make_samples(12.0, 120.0, |t| {
            let k = (t / 2.0) as usize % 4;
            [
                NormalizedPoint::new(0.0, 0.0),
                NormalizedPoint::new(1.0, 0.0),
                NormalizedPoint::new(1.0, 1.0),
                NormalizedPoint::new(0.0, 1.0),
            ][k]
        });
        let clicks = [
            click(1.0, ClickKind::Down, 0.0, 0.0),
            click(3.0, ClickKind::Down, 1.0, 0.0),
            click(5.0, ClickKind::Down, 1.0, 1.0),
            click(7.0, ClickKind::Down, 0.0, 1.0),
            click(9.5, ClickKind::Down, 0.01, 0.99),
        ];
        let timeline = Timeline {
            zoom_blocks: vec![ZoomBlock {
                focus: ZoomFocus::Manual(NormalizedPoint::new(1.0, 1.0)),
                ..ZoomBlock {
                    scale: 4.0,
                    ..ZoomBlock::new(10.5, 12.0)
                }
            }],
            ..Timeline::default()
        };
        let m = model(corners, &clicks, timeline, CursorStyle::default(), 12.0);
        let mut max_scale = 1.0f64;
        let mut t = 0.0;
        while t <= 12.0 {
            let c = m.frame_state(t).camera;
            let h = 0.5 / c.scale;
            assert!(c.scale >= 1.0);
            assert!(c.center.x >= h - 1e-9 && c.center.x <= 1.0 - h + 1e-9);
            assert!(c.center.y >= h - 1e-9 && c.center.y <= 1.0 - h + 1e-9);
            max_scale = max_scale.max(c.scale);
            t += 1.0 / 240.0;
        }
        assert!(max_scale > 3.5, "max scale {max_scale}");
    }

    #[test]
    fn clustered_clicks_make_one_segment_with_lead_in_and_hold() {
        let t = MotionTuning::standard();
        let clicks = [
            click(2.0, ClickKind::Down, 0.3, 0.3),
            click(2.5, ClickKind::Down, 0.32, 0.31),
            click(3.0, ClickKind::Down, 0.31, 0.29),
        ];
        let m = model(
            Vec::new(),
            &clicks,
            Timeline::default(),
            CursorStyle::default(),
            20.0,
        );
        assert_eq!(m.zoom_segments.len(), 1);
        let seg = &m.zoom_segments[0];
        assert!((seg.start - (2.0 - t.auto_zoom_lead_in)).abs() < 1e-9);
        assert!((seg.end - (3.0 + t.auto_zoom_hold)).abs() < 1e-9);
        assert_eq!(seg.origin, ZoomOrigin::Auto);
        assert_eq!(seg.scale, t.auto_zoom_scale);

        assert_eq!(m.frame_state(0.5).camera.scale, 1.0);
        assert!(
            m.frame_state(2.0).camera.scale > 1.6,
            "mostly in by the first click"
        );
        let held = m.frame_state(4.0).camera;
        assert!((held.scale - 2.0).abs() < 0.02);
        assert!((held.center.x - 0.3).abs() < 0.02 && (held.center.y - 0.3).abs() < 0.02);
        assert!(
            (m.frame_state(9.0).camera.scale - 1.0).abs() < 1e-3,
            "eased out after idle"
        );
    }

    #[test]
    fn distant_in_time_clicks_make_separate_segments() {
        let clicks = [
            click(2.0, ClickKind::Down, 0.3, 0.3),
            click(10.0, ClickKind::Down, 0.3, 0.3),
        ];
        let m = model(
            Vec::new(),
            &clicks,
            Timeline::default(),
            CursorStyle::default(),
            20.0,
        );
        assert_eq!(m.zoom_segments.len(), 2);
        assert!((m.frame_state(7.0).camera.scale - 1.0).abs() < 1e-2);
    }

    #[test]
    fn far_apart_clicks_split_but_quick_ones_pan() {
        let quick = model(
            Vec::new(),
            &[
                click(2.0, ClickKind::Down, 0.1, 0.1),
                click(2.5, ClickKind::Down, 0.9, 0.9),
            ],
            Timeline::default(),
            CursorStyle::default(),
            20.0,
        );
        assert_eq!(quick.zoom_segments.len(), 1);

        let tuning = MotionTuning {
            auto_zoom_hold: 0.5,
            auto_zoom_lead_in: 0.3,
            ..MotionTuning::default()
        };
        let split = SpringMotionModel::new(
            &CursorTrack {
                clicks: vec![
                    click(2.0, ClickKind::Down, 0.1, 0.1),
                    click(4.9, ClickKind::Down, 0.9, 0.9),
                ],
                ..CursorTrack::default()
            },
            &Timeline::default(),
            CursorStyle::default(),
            20.0,
            16.0 / 9.0,
            tuning,
        );
        assert_eq!(split.zoom_segments.len(), 2);

        let near = SpringMotionModel::new(
            &CursorTrack {
                clicks: vec![
                    click(2.0, ClickKind::Down, 0.4, 0.4),
                    click(4.9, ClickKind::Down, 0.42, 0.41),
                ],
                ..CursorTrack::default()
            },
            &Timeline::default(),
            CursorStyle::default(),
            20.0,
            16.0 / 9.0,
            tuning,
        );
        assert_eq!(near.zoom_segments.len(), 1);
    }

    #[test]
    fn hold_extends_while_cursor_moves() {
        let t = MotionTuning::standard();
        let samples = make_samples(12.0, 120.0, |tt| {
            NormalizedPoint::new(0.3 + 0.05 * ((tt.min(5.0)) * 3.0).sin(), 0.3)
        });
        let m = model(
            samples,
            &[click(2.0, ClickKind::Down, 0.3, 0.3)],
            Timeline::default(),
            CursorStyle::default(),
            20.0,
        );
        assert_eq!(m.zoom_segments.len(), 1);
        let end = m.zoom_segments[0].end;
        assert!(end > 5.0 + t.auto_zoom_hold - 0.05, "end {end}");
        assert!(end <= 2.0 + t.auto_zoom_max_hold + 1e-9, "end {end}");
    }

    #[test]
    fn auto_zoom_disabled() {
        let timeline = Timeline {
            auto_zoom_enabled: false,
            ..Timeline::default()
        };
        let m = model(
            Vec::new(),
            &[click(2.0, ClickKind::Down, 0.3, 0.3)],
            timeline,
            CursorStyle::default(),
            20.0,
        );
        assert!(m.zoom_segments.is_empty());
        let mut t = 0.0;
        while t <= 20.0 {
            assert_eq!(m.frame_state(t).camera, nss_core::CameraState::IDENTITY);
            t += 0.25;
        }
    }

    #[test]
    fn manual_block_overrides_auto() {
        let block = ZoomBlock {
            focus: ZoomFocus::Manual(NormalizedPoint::new(0.75, 0.7)),
            ..ZoomBlock {
                scale: 3.0,
                ..ZoomBlock::new(3.0, 8.0)
            }
        };
        let id = block.id;
        let timeline = Timeline {
            zoom_blocks: vec![block],
            ..Timeline::default()
        };
        let clicks = [
            click(2.0, ClickKind::Down, 0.2, 0.2),
            click(3.5, ClickKind::Down, 0.25, 0.2),
        ];
        let m = model(wander(), &clicks, timeline, CursorStyle::default(), 20.0);
        assert_eq!(m.zoom_segments[0].origin, ZoomOrigin::Auto);
        assert_eq!(m.zoom_segments[1].origin, ZoomOrigin::Manual(id));
        assert_eq!(m.zoom_segments[0].end, 3.0);
        for s in m
            .zoom_segments
            .iter()
            .filter(|s| s.origin == ZoomOrigin::Auto)
        {
            assert!(
                s.end <= 3.0 || s.start >= 8.0,
                "auto segment overlaps the manual block"
            );
        }
        let c = m.frame_state(7.5).camera;
        assert!((c.scale - 3.0).abs() < 0.02);
        assert!((c.center.x - 0.75).abs() < 0.01 && (c.center.y - 0.7).abs() < 0.01);

        // A manual scale-1 block suppresses auto zoom entirely.
        let off = ZoomBlock::new(0.0, 20.0);
        let m2 = model(
            Vec::new(),
            &[click(2.0, ClickKind::Down, 0.2, 0.2)],
            Timeline {
                zoom_blocks: vec![ZoomBlock { scale: 1.0, ..off }],
                ..Timeline::default()
            },
            CursorStyle::default(),
            20.0,
        );
        assert_eq!(m2.frame_state(3.0).camera.scale, 1.0);
    }

    #[test]
    fn manual_auto_focus_follows_cursor_with_dead_zone() {
        let samples = make_samples(10.0, 120.0, |t| {
            if t < 2.5 {
                NormalizedPoint::new(0.4, 0.5)
            } else if t < 4.0 {
                NormalizedPoint::new(0.43, 0.52)
            } else {
                NormalizedPoint::new(0.8, 0.5)
            }
        });
        let block = ZoomBlock {
            focus: ZoomFocus::Auto {},
            ..ZoomBlock {
                scale: 2.0,
                ..ZoomBlock::new(1.0, 10.0)
            }
        };
        let timeline = Timeline {
            zoom_blocks: vec![block],
            auto_zoom_enabled: false,
            ..Timeline::default()
        };
        let m = model(samples, &[], timeline, CursorStyle::default(), 10.0);
        let a = m.frame_state(3.9).camera;
        // Small move (0.03) inside the dead zone: the camera did not pan.
        assert!((a.center.x - 0.4).abs() < 1e-3 && (a.center.y - 0.5).abs() < 1e-3);
        let b = m.frame_state(9.0).camera;
        // Big move: pans so the cursor sits at the dead-zone edge (0.8 - 0.125), clamped.
        assert!((b.center.x - 0.675).abs() < 0.01, "center {}", b.center.x);
    }

    #[test]
    fn ripples_timing() {
        let d = MotionTuning::standard().ripple_duration;
        let clicks = [
            click(1.0, ClickKind::Down, 0.4, 0.6),
            ClickEvent::new(1.1, 0.4, 0.6, MouseButton::Left, ClickKind::Up),
        ];
        let m = model(
            Vec::new(),
            &clicks,
            Timeline::default(),
            CursorStyle::default(),
            20.0,
        );
        assert!(m.frame_state(0.99).cursor.ripples.is_empty());
        let r0 = m.frame_state(1.0).cursor.ripples;
        assert_eq!(r0.len(), 1);
        assert_eq!(r0[0].progress, 0.0);
        assert_eq!(r0[0].position, NormalizedPoint::new(0.4, 0.6));
        let rh = m.frame_state(1.0 + d / 2.0).cursor.ripples;
        assert_eq!(rh.len(), 1);
        assert!((rh[0].progress - 0.5).abs() < 1e-9);
        assert!(m.frame_state(1.0 + d).cursor.ripples.is_empty());

        let two = model(
            Vec::new(),
            &[
                click(1.0, ClickKind::Down, 0.1, 0.1),
                click(1.1, ClickKind::Down, 0.2, 0.2),
            ],
            Timeline::default(),
            CursorStyle::default(),
            20.0,
        );
        assert_eq!(two.frame_state(1.2).cursor.ripples.len(), 2);

        let off = model(
            Vec::new(),
            &[click(1.0, ClickKind::Down, 0.4, 0.6)],
            Timeline::default(),
            CursorStyle {
                click_ripples: false,
                ..CursorStyle::default()
            },
            20.0,
        );
        assert!(off.frame_state(1.1).cursor.ripples.is_empty());
    }

    #[test]
    fn smooth_cursor_follows_and_reports_velocity() {
        let samples = make_samples(6.0, 120.0, |t| {
            NormalizedPoint::new(0.2 + 0.2 * t.min(3.0), 0.5)
        });
        let m = model(
            samples.clone(),
            &[],
            Timeline::default(),
            CursorStyle::default(),
            6.0,
        );
        let moving = m.frame_state(2.0).cursor;
        assert!(
            (moving.velocity.x - 0.2).abs() < 0.01,
            "v {}",
            moving.velocity.x
        );
        assert!(moving.velocity.y.abs() < 1e-6);
        assert!(
            moving.position.x < 0.6 && moving.position.x > 0.55,
            "p {}",
            moving.position.x
        );
        let settled = m.frame_state(5.0).cursor;
        assert!((settled.position.x - 0.8).abs() < 1e-4);
        assert!(settled.velocity.x.abs() < 1e-3);
        assert!(settled.visible && settled.opacity == 1.0);

        let raw = model(
            samples,
            &[],
            Timeline::default(),
            CursorStyle {
                smoothing: 0.0,
                ..CursorStyle::default()
            },
            6.0,
        )
        .frame_state(2.0)
        .cursor;
        assert!((raw.position.x - 0.6).abs() < 1e-9);
        assert!((raw.velocity.x - 0.2).abs() < 1e-6);
    }

    #[test]
    fn no_samples_hides_the_cursor() {
        let s = model(
            Vec::new(),
            &[click(1.0, ClickKind::Down, 0.5, 0.5)],
            Timeline::default(),
            CursorStyle::default(),
            20.0,
        );
        assert!(!s.frame_state(2.0).cursor.visible);
    }

    #[test]
    fn hide_when_idle_fades() {
        let t = MotionTuning::standard();
        let samples = make_samples(10.0, 120.0, |tt| {
            NormalizedPoint::new(
                if !(1.0..=7.0).contains(&tt) {
                    0.2 + 0.05 * tt
                } else {
                    0.25
                },
                0.5,
            )
        });
        let m = model(
            samples.clone(),
            &[],
            Timeline::default(),
            CursorStyle {
                hide_when_idle: true,
                ..CursorStyle::default()
            },
            10.0,
        );
        assert_eq!(m.frame_state(1.0).cursor.opacity, 1.0);
        let fading = m
            .frame_state(1.0 + t.idle_hide_delay + t.idle_fade_out_duration / 2.0)
            .cursor;
        assert!(
            fading.opacity > 0.2 && fading.opacity < 0.8,
            "opacity {}",
            fading.opacity
        );
        let hidden = m.frame_state(6.5).cursor;
        assert_eq!(hidden.opacity, 0.0);
        assert!(!hidden.visible);
        let back = m.frame_state(7.0 + t.idle_fade_in_duration + 0.05).cursor;
        assert_eq!(back.opacity, 1.0);
        assert!(back.visible);

        let always = model(
            samples,
            &[],
            Timeline::default(),
            CursorStyle::default(),
            10.0,
        )
        .frame_state(6.5)
        .cursor;
        assert_eq!(always.opacity, 1.0);
        assert!(always.visible);
    }

    #[test]
    fn trim_start_is_settled() {
        let clicks = [
            click(4.0, ClickKind::Down, 0.3, 0.3),
            click(4.3, ClickKind::Down, 0.3, 0.3),
            click(5.0, ClickKind::Down, 0.3, 0.3),
        ];
        let timeline = Timeline {
            trim: TrimRange::new(4.5, Some(15.0)),
            ..Timeline::default()
        };
        let samples = wander();
        let m = model(
            samples.clone(),
            &clicks,
            timeline,
            CursorStyle::default(),
            20.0,
        );
        let s = m.frame_state(4.5);
        assert_eq!(
            s.cursor.ripples.len(),
            1,
            "the 4.3 s click is still rippling"
        );
        assert!((s.camera.scale - 2.0).abs() < 1e-6 || (s.camera.scale - 2.0).abs() < 1e-3);
        let raw = CursorTrack {
            samples,
            ..CursorTrack::default()
        }
        .raw_position(4.5)
        .unwrap();
        assert!((s.cursor.position.x - raw.x).abs() < 1e-6);
        assert!((s.cursor.position.y - raw.y).abs() < 1e-6);
        // Times outside the trim clamp to its edges.
        let before = m.frame_state(1.0);
        assert_eq!(before.camera, s.camera);
        assert_eq!(before.cursor, s.cursor);
        assert_eq!(before.time, 1.0);
    }

    #[test]
    fn nested_manual_block_keeps_the_outer_remainder() {
        let outer = ZoomBlock {
            focus: ZoomFocus::Manual(NormalizedPoint::new(0.3, 0.3)),
            ..ZoomBlock::new(0.0, 10.0)
        };
        let inner = ZoomBlock {
            focus: ZoomFocus::Manual(NormalizedPoint::new(0.6, 0.6)),
            ..ZoomBlock {
                scale: 3.0,
                ..ZoomBlock::new(2.0, 3.0)
            }
        };
        let (a, b) = (outer.id, inner.id);
        let timeline = Timeline {
            zoom_blocks: vec![inner.clone(), outer],
            ..Timeline::default()
        };
        let m = model(
            Vec::new(),
            &[click(6.0, ClickKind::Down, 0.9, 0.9)],
            timeline,
            CursorStyle::default(),
            20.0,
        );
        let segs = &m.zoom_segments;
        assert_eq!(
            segs.iter().map(|s| s.origin).collect::<Vec<_>>(),
            vec![
                ZoomOrigin::Manual(a),
                ZoomOrigin::Manual(b),
                ZoomOrigin::Manual(a)
            ]
        );
        assert_eq!(
            segs.iter().map(|s| s.start).collect::<Vec<_>>(),
            vec![0.0, 2.0, 3.0]
        );
        assert_eq!(
            segs.iter().map(|s| s.end).collect::<Vec<_>>(),
            vec![2.0, 3.0, 10.0]
        );
        let c = m.frame_state(8.0).camera;
        assert!((c.scale - 2.0).abs() < 0.01);
        assert!((c.center.x - 0.3).abs() < 0.01 && (c.center.y - 0.3).abs() < 0.01);

        // Follow-mode remainders are re-anchored on the cursor where they resume.
        let samples = make_samples(10.0, 120.0, |t| {
            NormalizedPoint::new(if t < 3.0 { 0.2 } else { 0.7 }, 0.5)
        });
        let follow = ZoomBlock {
            focus: ZoomFocus::Auto {},
            ..ZoomBlock::new(0.0, 10.0)
        };
        let timeline = Timeline {
            zoom_blocks: vec![follow, inner],
            auto_zoom_enabled: false,
            ..Timeline::default()
        };
        let m2 = model(samples, &[], timeline, CursorStyle::default(), 10.0);
        assert_eq!(m2.zoom_segments.len(), 3);
        assert!((m2.zoom_segments[2].initial_focus.x - 0.7).abs() < 1e-9);
        assert_eq!(m2.zoom_segments[2].follow_from, Some(0.0));
    }

    #[test]
    fn huge_finite_durations_and_timestamps_are_bounded() {
        let tuning = MotionTuning {
            max_simulation_frames: 10_000,
            ..MotionTuning::default()
        };
        let m = SpringMotionModel::new(
            &CursorTrack {
                clicks: vec![click(5.0, ClickKind::Down, 0.3, 0.3)],
                ..CursorTrack::default()
            },
            &Timeline::default(),
            CursorStyle::default(),
            1e15,
            16.0 / 9.0,
            tuning,
        );
        assert!(m.simulated_frame_count() <= 10_000);
        assert_eq!(m.end_time, tuning.max_duration);
        let _ = m.frame_state(1e15);
        let _ = m.frame_state(6.0);

        // Absolute timestamps (seconds since 1970) by mistake, duration derived from them.
        let absolute = SpringMotionModel::new(
            &CursorTrack {
                samples: vec![CursorSample::new(1.7e9, 0.5, 0.5)],
                clicks: vec![click(1.7e9 + 1.0, ClickKind::Down, 0.4, 0.4)],
                ..CursorTrack::default()
            },
            &Timeline::default(),
            CursorStyle::default(),
            0.0,
            16.0 / 9.0,
            tuning,
        );
        assert!(absolute.simulated_frame_count() <= 10_000);
        assert!(absolute.frame_state(1.7e9).camera.scale >= 1.0);

        let absurd = SpringMotionModel::new(
            &CursorTrack {
                clicks: vec![click(1e300, ClickKind::Down, 0.4, 0.4)],
                ..CursorTrack::default()
            },
            &Timeline {
                trim: TrimRange::new(1e300, Some(f64::MAX)),
                ..Timeline::default()
            },
            CursorStyle::default(),
            f64::MAX,
            16.0 / 9.0,
            tuning,
        );
        assert!(absurd.simulated_frame_count() <= 10_000);

        // Default cap: the table stays at or below max_simulation_frames (about 36 MB).
        assert!(
            std::mem::size_of::<SimFrame>() * MotionTuning::standard().max_simulation_frames
                <= 36_000_000
        );
    }

    #[test]
    fn zoom_out_does_not_swing_toward_the_next_segment() {
        let clicks = [
            click(2.0, ClickKind::Down, 0.2, 0.2),
            click(10.0, ClickKind::Down, 0.8, 0.8),
        ];
        let m = model(
            Vec::new(),
            &clicks,
            Timeline::default(),
            CursorStyle::default(),
            20.0,
        );
        assert_eq!(m.zoom_segments.len(), 2);
        let end1 = m.zoom_segments[0].end;
        let mut t = end1;
        while t < m.zoom_segments[1].start {
            let c = m.frame_state(t).camera;
            if c.scale > 1.05 {
                assert!(
                    c.center.x <= (0.2f64).max(0.5 / c.scale) + 1e-4,
                    "x {}",
                    c.center.x
                );
                assert!(
                    c.center.y <= (0.2f64).max(0.5 / c.scale) + 1e-4,
                    "y {}",
                    c.center.y
                );
            }
            t += 1.0 / 120.0;
        }
        let c2 = m.frame_state(11.0).camera;
        assert!((c2.center.x - 0.75).abs() < 0.02, "x {}", c2.center.x);
    }

    #[test]
    fn drag_holds_zoom_until_release() {
        let t = MotionTuning::standard();
        let clicks = [
            click(2.0, ClickKind::Down, 0.3, 0.3),
            ClickEvent::new(9.0, 0.35, 0.3, MouseButton::Left, ClickKind::Up),
        ];
        let m = model(
            Vec::new(),
            &clicks,
            Timeline::default(),
            CursorStyle::default(),
            20.0,
        );
        assert_eq!(m.zoom_segments.len(), 1);
        assert!((m.zoom_segments[0].end - (9.0 + t.auto_zoom_hold)).abs() < 1e-9);
        assert!((m.frame_state(8.0).camera.scale - 2.0).abs() < 0.01);

        let quick = model(
            Vec::new(),
            &[
                click(2.0, ClickKind::Down, 0.3, 0.3),
                ClickEvent::new(2.1, 0.3, 0.3, MouseButton::Left, ClickKind::Up),
            ],
            Timeline::default(),
            CursorStyle::default(),
            20.0,
        );
        assert!((quick.zoom_segments[0].end - (2.1 + t.auto_zoom_hold)).abs() < 1e-9);

        // Mismatched buttons do not pair.
        let other = model(
            Vec::new(),
            &[
                click(2.0, ClickKind::Down, 0.3, 0.3),
                ClickEvent::new(9.0, 0.3, 0.3, MouseButton::Right, ClickKind::Up),
            ],
            Timeline::default(),
            CursorStyle::default(),
            20.0,
        );
        assert!((other.zoom_segments[0].end - (2.0 + t.auto_zoom_hold)).abs() < 1e-9);
    }

    #[test]
    fn spatial_split_is_effective_with_standard_tuning() {
        let t = MotionTuning::standard();
        let far = model(
            Vec::new(),
            &[
                click(2.0, ClickKind::Down, 0.1, 0.1),
                click(4.5, ClickKind::Down, 0.9, 0.9),
            ],
            Timeline::default(),
            CursorStyle::default(),
            20.0,
        );
        assert_eq!(far.zoom_segments.len(), 2);
        assert!(far.zoom_segments[1].start - far.zoom_segments[0].end >= t.min_zoom_out_gap - 1e-9);
        assert!(far.zoom_segments[0].end >= 2.0 + t.auto_zoom_min_hold - 1e-9);

        let near = model(
            Vec::new(),
            &[
                click(2.0, ClickKind::Down, 0.3, 0.3),
                click(4.5, ClickKind::Down, 0.35, 0.3),
            ],
            Timeline::default(),
            CursorStyle::default(),
            20.0,
        );
        assert_eq!(near.zoom_segments.len(), 1);

        let tight = model(
            Vec::new(),
            &[
                click(2.0, ClickKind::Down, 0.1, 0.1),
                click(3.5, ClickKind::Down, 0.9, 0.9),
            ],
            Timeline::default(),
            CursorStyle::default(),
            20.0,
        );
        assert_eq!(tight.zoom_segments.len(), 1);

        let fast = model(
            Vec::new(),
            &[
                click(2.0, ClickKind::Down, 0.1, 0.1),
                click(2.8, ClickKind::Down, 0.9, 0.9),
            ],
            Timeline::default(),
            CursorStyle::default(),
            20.0,
        );
        assert_eq!(fast.zoom_segments.len(), 1);
    }

    #[test]
    fn degenerate_inputs_do_not_panic() {
        let empty = SpringMotionModel::new(
            &CursorTrack::default(),
            &Timeline::default(),
            CursorStyle::default(),
            0.0,
            16.0 / 9.0,
            MotionTuning::standard(),
        );
        assert_eq!(
            empty.frame_state(0.0).camera,
            nss_core::CameraState::IDENTITY
        );
        assert_eq!(
            empty.frame_state(f64::NAN).camera,
            nss_core::CameraState::IDENTITY
        );

        let weird = SpringMotionModel::new(
            &CursorTrack {
                samples: vec![
                    CursorSample::new(1.0, 0.5, 0.5),
                    CursorSample::new(0.5, f64::NAN, 0.2),
                ],
                clicks: vec![click(f64::INFINITY, ClickKind::Down, 0.5, 0.5)],
                ..CursorTrack::default()
            },
            &Timeline {
                trim: TrimRange::new(30.0, Some(10.0)),
                zoom_blocks: vec![ZoomBlock::new(5.0, 2.0)],
                ..Timeline::default()
            },
            CursorStyle::default(),
            20.0,
            16.0 / 9.0,
            MotionTuning::standard(),
        );
        let _ = weird.frame_state(3.0);
    }

    #[test]
    fn stable_shapes_drop_flickers() {
        let raw = [
            CursorShapeChange::new(0.0, CursorShape::Arrow),
            CursorShapeChange::new(1.0, CursorShape::IBeam),
            CursorShapeChange::new(1.03, CursorShape::PointingHand),
            CursorShapeChange::new(2.0, CursorShape::Arrow),
            CursorShapeChange::new(2.01, CursorShape::IBeam),
            CursorShapeChange::new(2.02, CursorShape::Arrow),
            CursorShapeChange::new(4.0, CursorShape::Crosshair),
        ];
        let stable = SpringMotionModel::stable_shapes(&raw, 0.05);
        assert_eq!(
            stable,
            vec![
                CursorShapeChange::new(1.03, CursorShape::PointingHand),
                CursorShapeChange::new(2.02, CursorShape::Arrow),
                CursorShapeChange::new(4.0, CursorShape::Crosshair),
            ]
        );
    }

    #[test]
    fn unsorted_and_non_finite_shapes_are_handled() {
        let raw = [
            CursorShapeChange::new(3.0, CursorShape::IBeam),
            CursorShapeChange::new(f64::NAN, CursorShape::Crosshair),
            CursorShapeChange::new(1.0, CursorShape::PointingHand),
            CursorShapeChange::new(f64::INFINITY, CursorShape::Arrow),
        ];
        assert_eq!(
            SpringMotionModel::stable_shapes(&raw, 0.05),
            vec![
                CursorShapeChange::new(1.0, CursorShape::PointingHand),
                CursorShapeChange::new(3.0, CursorShape::IBeam),
            ]
        );
        assert!(SpringMotionModel::stable_shapes(&[], 0.05).is_empty());
    }

    #[test]
    fn frame_state_reports_the_stable_shape() {
        let track = CursorTrack {
            samples: make_samples(6.0, 120.0, |_| NormalizedPoint::new(0.4, 0.4)),
            shapes: vec![
                CursorShapeChange::new(1.0, CursorShape::IBeam),
                CursorShapeChange::new(1.02, CursorShape::PointingHand),
                CursorShapeChange::new(3.0, CursorShape::Unknown),
                CursorShapeChange::new(5.0, CursorShape::Arrow),
            ],
            ..CursorTrack::default()
        };
        let m = SpringMotionModel::new(
            &track,
            &Timeline::default(),
            CursorStyle::default(),
            6.0,
            16.0 / 9.0,
            MotionTuning::standard(),
        );
        assert_eq!(m.frame_state(0.5).cursor.shape, CursorShape::Arrow);
        assert_eq!(
            m.frame_state(1.01).cursor.shape,
            CursorShape::Arrow,
            "the 20 ms I-beam never shows"
        );
        assert_eq!(m.frame_state(1.02).cursor.shape, CursorShape::PointingHand);
        assert_eq!(m.frame_state(2.99).cursor.shape, CursorShape::PointingHand);
        assert_eq!(m.frame_state(3.5).cursor.shape, CursorShape::Unknown);
        assert_eq!(m.frame_state(5.5).cursor.shape, CursorShape::Arrow);
    }

    #[test]
    fn press_squishes_then_springs_back() {
        let tuning = MotionTuning::standard();
        let still = make_samples(10.0, 120.0, |_| NormalizedPoint::new(0.5, 0.5));
        let clicks = [
            ClickEvent::new(2.0, 0.5, 0.5, MouseButton::Left, ClickKind::Down),
            ClickEvent::new(2.6, 0.5, 0.5, MouseButton::Left, ClickKind::Up),
        ];
        let m = SpringMotionModel::new(
            &CursorTrack {
                samples: still,
                clicks: clicks.to_vec(),
                ..CursorTrack::default()
            },
            &Timeline {
                auto_zoom_enabled: false,
                ..Timeline::default()
            },
            CursorStyle::default(),
            10.0,
            16.0 / 9.0,
            tuning,
        );
        let mut t = 0.0;
        while t < 1.999 {
            assert_eq!(m.frame_state(t).cursor.press_scale, 1.0);
            t += 1.0 / 240.0;
        }
        assert!(m.frame_state(2.1).cursor.press_scale < tuning.press_scale + 0.02);
        let mut prev = 1.0;
        t = 2.0;
        while t < 2.59 {
            let s = m.frame_state(t).cursor.press_scale;
            assert!(s <= prev + 1e-6, "must be monotone: {s} after {prev}");
            assert!(s >= tuning.press_scale - 1e-6);
            prev = s;
            t += 1.0 / 240.0;
        }
        assert!((m.frame_state(2.55).cursor.press_scale - tuning.press_scale).abs() < 1e-3);

        let mut peak: f64 = 0.0;
        t = 2.6;
        while t <= 4.0 {
            peak = peak.max(m.frame_state(t).cursor.press_scale);
            t += 1.0 / 240.0;
        }
        assert!(peak > 1.005 && peak < 1.05, "peak {peak}");
        assert!((m.frame_state(4.5).cursor.press_scale - 1.0).abs() < 1e-3);
    }

    #[test]
    fn a_click_without_a_visible_hold_still_squishes() {
        let still = make_samples(10.0, 120.0, |_| NormalizedPoint::new(0.5, 0.5));
        for up in [5.0, 5.005, 5.03] {
            let clicks = vec![
                ClickEvent::new(5.0, 0.5, 0.5, MouseButton::Left, ClickKind::Down),
                ClickEvent::new(up, 0.5, 0.5, MouseButton::Left, ClickKind::Up),
            ];
            let m = SpringMotionModel::new(
                &CursorTrack {
                    samples: still.clone(),
                    clicks,
                    ..CursorTrack::default()
                },
                &Timeline {
                    auto_zoom_enabled: false,
                    ..Timeline::default()
                },
                CursorStyle::default(),
                10.0,
                16.0 / 9.0,
                MotionTuning::standard(),
            );
            let mut dip = 1.0f64;
            let mut t = 5.0;
            while t <= 5.4 {
                dip = dip.min(m.frame_state(t).cursor.press_scale);
                t += 1.0 / 240.0;
            }
            assert!(
                dip < 0.88,
                "released after {} s only dipped to {dip}",
                up - 5.0
            );
            assert!((m.frame_state(7.0).cursor.press_scale - 1.0).abs() < 1e-3);
        }
    }

    #[test]
    fn missing_mouse_up_and_overlapping_buttons_are_handled() {
        let still = make_samples(10.0, 120.0, |_| NormalizedPoint::new(0.5, 0.5));
        let build = |clicks: Vec<ClickEvent>, trim_start: f64| {
            SpringMotionModel::new(
                &CursorTrack {
                    samples: still.clone(),
                    clicks,
                    ..CursorTrack::default()
                },
                &Timeline {
                    trim: TrimRange::new(trim_start, None),
                    auto_zoom_enabled: false,
                    ..Timeline::default()
                },
                CursorStyle::default(),
                10.0,
                16.0 / 9.0,
                MotionTuning::standard(),
            )
        };

        let lost = build(
            vec![ClickEvent::new(
                1.0,
                0.5,
                0.5,
                MouseButton::Left,
                ClickKind::Down,
            )],
            0.0,
        );
        let mut dip = 1.0f64;
        let mut t = 1.0;
        while t <= 1.4 {
            dip = dip.min(lost.frame_state(t).cursor.press_scale);
            t += 1.0 / 240.0;
        }
        assert!(dip < 0.88);
        assert!((lost.frame_state(3.0).cursor.press_scale - 1.0).abs() < 1e-3);

        // Left held 1...3 s, right pressed and released inside it: stays squished until the left is up.
        let both = build(
            vec![
                ClickEvent::new(1.0, 0.5, 0.5, MouseButton::Left, ClickKind::Down),
                ClickEvent::new(1.5, 0.5, 0.5, MouseButton::Right, ClickKind::Down),
                ClickEvent::new(1.7, 0.5, 0.5, MouseButton::Right, ClickKind::Up),
                ClickEvent::new(3.0, 0.5, 0.5, MouseButton::Left, ClickKind::Up),
            ],
            0.0,
        );
        let tuning = MotionTuning::standard();
        let mut t = 1.3;
        while t <= 3.0 {
            assert!((both.frame_state(t).cursor.press_scale - tuning.press_scale).abs() < 0.01);
            t += 1.0 / 240.0;
        }

        // A trim that starts inside a press is settled squished.
        let inside = build(
            vec![
                ClickEvent::new(1.0, 0.5, 0.5, MouseButton::Left, ClickKind::Down),
                ClickEvent::new(4.0, 0.5, 0.5, MouseButton::Left, ClickKind::Up),
            ],
            2.0,
        );
        assert!((inside.frame_state(2.0).cursor.press_scale - tuning.press_scale).abs() < 1e-6);
    }

    #[test]
    fn press_tracker_merges_and_extends_short_clicks() {
        let clicks = vec![
            ClickEvent::new(1.0, 0.5, 0.5, MouseButton::Left, ClickKind::Down),
            ClickEvent::new(1.01, 0.5, 0.5, MouseButton::Left, ClickKind::Up),
            ClickEvent::new(1.05, 0.5, 0.5, MouseButton::Left, ClickKind::Down),
            ClickEvent::new(1.5, 0.5, 0.5, MouseButton::Left, ClickKind::Up),
        ];
        let mut p = PressTracker::new(&clicks, 0.08);
        assert_eq!(p.intervals.len(), 1);
        assert_eq!(p.intervals[0], (1.0, 1.5));
        let states: Vec<bool> = [0.9, 1.0, 1.03, 1.5]
            .iter()
            .map(|t| p.is_pressed(*t))
            .collect();
        assert_eq!(states, vec![false, true, true, false]);
    }

    #[test]
    fn cursor_velocity_signs_follow_the_track() {
        // Sanity check on the raw-velocity path used when smoothing is off.
        let samples = make_samples(4.0, 120.0, |t| NormalizedPoint::new(0.2, 0.2 + 0.1 * t));
        let m = model(
            samples,
            &[],
            Timeline::default(),
            CursorStyle {
                smoothing: 0.0,
                ..CursorStyle::default()
            },
            4.0,
        );
        let c = m.frame_state(2.0).cursor;
        assert!((c.velocity.y - 0.1).abs() < 1e-6, "vy {}", c.velocity.y);
        assert!(c.velocity.x.abs() < 1e-9);
    }

    #[test]
    fn easing_in_and_out_use_the_segment_spring() {
        let base = ZoomBlock {
            focus: ZoomFocus::Manual(NormalizedPoint::CENTER),
            ..ZoomBlock::new(2.0, 8.0)
        };
        let scale_at = |block: &ZoomBlock, t: f64| -> f64 {
            let m = SpringMotionModel::new(
                &CursorTrack::default(),
                &Timeline {
                    zoom_blocks: vec![block.clone()],
                    auto_zoom_enabled: false,
                    ..Timeline::default()
                },
                CursorStyle::default(),
                12.0,
                16.0 / 9.0,
                MotionTuning::standard(),
            );
            m.frame_state(t).camera.scale
        };

        let fast = ZoomBlock {
            ease_in: 0.3,
            ..base.clone()
        };
        let slow = ZoomBlock {
            ease_in: 1.5,
            ..base.clone()
        };
        assert!(scale_at(&fast, 2.4) > scale_at(&slow, 2.4) + 0.3);
        assert!((scale_at(&fast, 2.4) - 2.0).abs() < 0.1);
        assert!((scale_at(&slow, 2.0 + 1.5 * 1.3) - 2.0).abs() < 0.05);
        assert!(
            (scale_at(&slow, 1.99) - 1.0).abs() < 1e-6,
            "nothing before the block starts"
        );

        let exit = ZoomBlock::new(1.0, 5.0);
        let fast_out = ZoomBlock {
            ease_out: 0.3,
            ..exit.clone()
        };
        let slow_out = ZoomBlock {
            ease_out: 1.5,
            ..exit
        };
        assert!((scale_at(&fast_out, 4.99) - scale_at(&slow_out, 4.99)).abs() < 1e-6);
        assert!(scale_at(&slow_out, 5.4) > scale_at(&fast_out, 5.4) + 0.3);
    }

    #[test]
    fn ease_is_a_spring_not_a_linear_tween() {
        let block = ZoomBlock {
            focus: ZoomFocus::Manual(NormalizedPoint::CENTER),
            ease_in: 1.0,
            ease_out: 1.0,
            ..ZoomBlock::new(1.0, 6.0)
        };
        let m = SpringMotionModel::new(
            &CursorTrack::default(),
            &Timeline {
                zoom_blocks: vec![block],
                auto_zoom_enabled: false,
                ..Timeline::default()
            },
            CursorStyle::default(),
            10.0,
            16.0 / 9.0,
            MotionTuning::standard(),
        );
        let mut prev = 1.0;
        let mut steps: Vec<f64> = Vec::new();
        let mut t = 1.0;
        while t <= 3.0 {
            let s = m.frame_state(t).camera.scale;
            assert!(s >= prev - 1e-9 && s <= 2.0 + 1e-9, "scale {s}");
            steps.push(s - prev);
            prev = s;
            t += 1.0 / 240.0;
        }
        let sum = |a: usize, b: usize| steps[a..=b].iter().sum::<f64>();
        let (early, mid, late) = (sum(1, 10), sum(60, 69), sum(400, 409));
        assert!(mid > early * 2.0, "mid {mid} vs early {early}");
        assert!(mid > late * 2.0, "mid {mid} vs late {late}");
    }

    #[test]
    fn stored_blocks_move_like_round_two_auto_zoom() {
        let clicks = [
            click(2.0, ClickKind::Down, 0.3, 0.3),
            click(2.6, ClickKind::Down, 0.32, 0.31),
            click(8.0, ClickKind::Down, 0.8, 0.7),
            click(14.0, ClickKind::Down, 0.2, 0.8),
        ];
        let samples = wander();
        let legacy = model(
            samples.clone(),
            &clicks,
            Timeline::default(),
            CursorStyle::default(),
            20.0,
        );
        let track = CursorTrack {
            samples: samples.clone(),
            clicks: clicks.to_vec(),
            ..CursorTrack::default()
        };
        let blocks =
            ZoomPlanner::auto_zoom_blocks(&track, 20.0, 16.0 / 9.0, &[], &MotionTuning::standard());
        let stored = model(
            samples,
            &clicks,
            Timeline {
                zoom_blocks: blocks,
                auto_zoom_enabled: false,
                ..Timeline::default()
            },
            CursorStyle::default(),
            20.0,
        );
        let (mut max_scale, mut max_center) = (0.0f64, 0.0f64);
        let mut t = 0.0;
        while t <= 20.0 {
            let a = legacy.frame_state(t).camera;
            let b = stored.frame_state(t).camera;
            max_scale = max_scale.max((a.scale - b.scale).abs());
            max_center = max_center
                .max((a.center.x - b.center.x).abs())
                .max((a.center.y - b.center.y).abs());
            t += 1.0 / 60.0;
        }
        assert!(max_scale < 1e-4, "max scale diff {max_scale}");
        assert!(max_center < 1e-4, "max center diff {max_center}");
    }

    #[test]
    fn pi_constant_is_available_to_fixtures() {
        // Keeps the fixture-style imports honest (upstream fixtures use sin/cos of time).
        assert!((PI * 2.0 - std::f64::consts::TAU).abs() < 1e-12);
    }
}
