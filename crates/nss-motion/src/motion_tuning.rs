//! Every tuning constant of the motion system, in one place.
//!
//! Port of upstream `NSSMotion/MotionTuning.swift`. [`MotionTuning::default`] is the shipped
//! configuration; the defaults are load-bearing because the recorded behaviour (how long a
//! zoom holds, how far clicks cluster) is defined by them.

use crate::spring::Spring;

/// Times are seconds, distances are normalised source units unless noted.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionTuning {
    // Simulation
    /// Fixed simulation rate (Hz) the springs are integrated at during the build.
    pub simulation_rate: f64,
    /// Hard cap on the simulated table (36 bytes per frame, so 1M frames ≈ 36 MB ≈ 69 min at
    /// 240 Hz). Longer ranges lower the effective rate instead: the springs are analytic, so
    /// any step is stable. Guards against bogus durations or absolute timestamps.
    pub max_simulation_frames: usize,
    /// Durations (and derived track spans) beyond this are clamped.
    pub max_duration: f64,

    // Camera springs
    /// Spring on log(scale). Critically damped: zooms never overshoot.
    pub zoom_spring: Spring,
    /// Spring on the camera focus point (panning).
    pub pan_spring: Spring,

    // Auto-zoom
    /// Magnification of auto-zoom segments.
    pub auto_zoom_scale: f64,
    /// Upper bound applied to every zoom (auto and manual).
    pub max_zoom_scale: f64,
    /// The zoom starts this long before the first click of a segment.
    pub auto_zoom_lead_in: f64,
    /// After the last click (and after the cursor last moved), stay zoomed this long before
    /// easing out.
    pub auto_zoom_hold: f64,
    /// Cursor movement after the last click keeps a segment zoomed, but at most this long after it.
    pub auto_zoom_max_hold: f64,
    /// Clicks further apart in time than this always start a new segment. Measured from the
    /// previous click's release, so a drag keeps the zoom held until the button is released.
    pub cluster_max_gap: f64,
    /// Clicks further apart than this (in source-height units, aspect-corrected) start a new
    /// segment, unless they are closer in time than `cluster_pan_gap` (then the camera pans
    /// instead). For such a spatial split the earlier segment's hold is shortened (down to
    /// `auto_zoom_min_hold`) to leave `min_zoom_out_gap` for a zoom out and back in; if there is
    /// no room, the camera pans. With the defaults: far clicks 1...2 s apart pan, 2...3 s apart
    /// zoom out and back in.
    pub cluster_max_distance: f64,
    /// Far-apart clicks closer in time than this stay in one segment (pan instead of zoom out/in).
    pub cluster_pan_gap: f64,
    /// Segments separated by less than this are merged, so the camera does not briefly zoom out.
    pub min_zoom_out_gap: f64,
    /// Shortest hold after the last release when a segment is shortened for a spatial split.
    pub auto_zoom_min_hold: f64,
    /// Between segments the focus is only re-aimed at the next one once the zoom has eased out
    /// below this scale, so the camera does not swing sideways while zooming out.
    pub pre_aim_max_scale: f64,
    /// Segments (or remnants after manual blocks cut them) shorter than this are dropped.
    pub min_segment_duration: f64,
    /// Dead zone for cursor following, as a fraction of the viewport half-extent per axis. The
    /// camera only pans once the cursor leaves the central `dead_zone` part of the viewport.
    pub dead_zone: f64,

    // Cursor
    /// Cursor spring response when `CursorStyle::smoothing` is just above 0 ...
    pub cursor_min_response: f64,
    /// ... and when it is 1. `smoothing == 0` means the raw cursor (no spring).
    pub cursor_max_response: f64,
    pub cursor_damping_ratio: f64,
    /// Movement between consecutive samples above this counts as activity (idle hide, holds).
    pub activity_epsilon: f64,
    /// With `hideWhenIdle`, the cursor starts fading after this long without movement or clicks.
    pub idle_hide_delay: f64,
    pub idle_fade_out_duration: f64,
    pub idle_fade_in_duration: f64,

    // Ripples
    /// A click ripple runs from progress 0 to 1 over this duration after each mouse-down.
    pub ripple_duration: f64,

    // Cursor shape
    /// Shape changes that last less than this are ignored, so a cursor that flickers between
    /// shapes (for example while crossing a one-pixel border) keeps drawing the surrounding shape.
    pub min_shape_duration: f64,

    // Press squish
    /// `CursorState::press_scale` while a mouse button is held down.
    pub press_scale: f64,
    /// Spring toward `press_scale` on mouse-down: quick, critically damped (no bounce).
    pub press_down_spring: Spring,
    /// Spring back to 1 on mouse-up: underdamped for a small overshoot (about 2% above 1).
    pub press_release_spring: Spring,
    /// A press counts as held for at least this long, so a click with no visible hold still squishes.
    pub press_min_hold: f64,
}

impl Default for MotionTuning {
    fn default() -> Self {
        Self {
            simulation_rate: 240.0,
            max_simulation_frames: 1_000_000,
            max_duration: 24.0 * 60.0 * 60.0,

            zoom_spring: Spring::new(0.75, 1.0),
            pan_spring: Spring::new(0.55, 1.0),

            auto_zoom_scale: 2.0,
            max_zoom_scale: 6.0,
            auto_zoom_lead_in: 0.5,
            auto_zoom_hold: 1.5,
            auto_zoom_max_hold: 5.0,
            cluster_max_gap: 3.0,
            cluster_max_distance: 0.35,
            cluster_pan_gap: 1.0,
            min_zoom_out_gap: 1.0,
            auto_zoom_min_hold: 0.5,
            pre_aim_max_scale: 1.02,
            min_segment_duration: 0.3,
            dead_zone: 0.5,

            cursor_min_response: 0.04,
            cursor_max_response: 0.3,
            cursor_damping_ratio: 1.0,
            activity_epsilon: 1e-4,
            idle_hide_delay: 2.0,
            idle_fade_out_duration: 0.35,
            idle_fade_in_duration: 0.12,

            ripple_duration: 0.45,

            min_shape_duration: 0.05,

            press_scale: 0.82,
            press_down_spring: Spring::new(0.1, 1.0),
            press_release_spring: Spring::new(0.35, 0.55),
            press_min_hold: 0.08,
        }
    }
}

impl MotionTuning {
    /// The shipped configuration.
    #[must_use]
    pub fn standard() -> Self {
        Self::default()
    }

    /// Cursor spring for a `CursorStyle::smoothing` value, or `None` for the raw cursor.
    #[must_use]
    pub fn cursor_spring(&self, smoothing: f64) -> Option<Spring> {
        let s = smoothing.clamp(0.0, 1.0);
        if s <= 0.0 {
            return None;
        }
        Some(Spring::new(
            self.cursor_min_response + s * (self.cursor_max_response - self.cursor_min_response),
            self.cursor_damping_ratio,
        ))
    }

    /// The simulation rate actually used, after guarding against a non-finite or non-positive
    /// value and lowering the rate when `span` seconds would exceed `max_simulation_frames`.
    #[must_use]
    pub fn effective_rate(&self, span: f64) -> f64 {
        let mut rate = if self.simulation_rate.is_finite() && self.simulation_rate > 0.0 {
            self.simulation_rate
        } else {
            240.0
        };
        let max_steps = max_steps(self) as f64;
        if span > 0.0 && span * rate > max_steps {
            rate = max_steps / span;
        }
        rate
    }

    /// `max_duration`, guarded (non-finite or non-positive becomes 24 h, as upstream).
    #[must_use]
    pub fn effective_max_duration(&self) -> f64 {
        if self.max_duration.is_finite() && self.max_duration > 0.0 {
            self.max_duration
        } else {
            24.0 * 60.0 * 60.0
        }
    }

    /// `press_scale`, guarded into `0.1...=1` (a corrupt value must not invert the squish).
    #[must_use]
    pub fn effective_press_scale(&self) -> f64 {
        if self.press_scale.is_finite() {
            self.press_scale.clamp(0.1, 1.0)
        } else {
            1.0
        }
    }
}

/// `max_simulation_frames` as a usable step count (never below 1).
pub(crate) fn max_steps(tuning: &MotionTuning) -> usize {
    tuning.max_simulation_frames.max(2) - 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_upstream() {
        let t = MotionTuning::default();
        assert_eq!(t.simulation_rate, 240.0);
        assert_eq!(t.zoom_spring, Spring::new(0.75, 1.0));
        assert_eq!(t.pan_spring, Spring::new(0.55, 1.0));
        assert_eq!(t.auto_zoom_hold, 1.5);
        assert_eq!(t.min_segment_duration, 0.3, "the 0.3 s zoom floor");
        assert_eq!(t.ripple_duration, 0.45);
        assert_eq!(t.press_scale, 0.82);
        assert_eq!(t.press_release_spring, Spring::new(0.35, 0.55));
        assert_eq!(t, MotionTuning::standard());
    }

    #[test]
    fn cursor_spring_spans_the_smoothing_range() {
        let t = MotionTuning::default();
        assert_eq!(t.cursor_spring(0.0), None);
        assert_eq!(t.cursor_spring(-1.0), None);
        assert_eq!(t.cursor_spring(1.0), Some(Spring::new(0.3, 1.0)));
        assert_eq!(t.cursor_spring(0.5), Some(Spring::new(0.17, 1.0)));
        assert_eq!(t.cursor_spring(9.0), t.cursor_spring(1.0), "clamped");
    }

    #[test]
    fn guard_rails_replace_bogus_values() {
        let mut t = MotionTuning {
            simulation_rate: f64::NAN,
            ..MotionTuning::default()
        };
        assert_eq!(t.effective_rate(10.0), 240.0);
        t.max_duration = -1.0;
        assert_eq!(t.effective_max_duration(), 86_400.0);
        t.press_scale = 4.0;
        assert_eq!(t.effective_press_scale(), 1.0);
        t.press_scale = 0.0;
        assert_eq!(t.effective_press_scale(), 0.1);
    }

    #[test]
    fn rate_drops_when_the_span_exceeds_the_frame_cap() {
        let t = MotionTuning {
            max_simulation_frames: 1000,
            ..MotionTuning::default()
        };
        // 100 s at 240 Hz would be 24000 steps; the cap allows 999.
        let rate = t.effective_rate(100.0);
        assert!((rate - 9.99).abs() < 1e-9, "rate {rate}");
        assert_eq!(
            t.effective_rate(1.0),
            240.0,
            "short spans keep the full rate"
        );
    }
}
