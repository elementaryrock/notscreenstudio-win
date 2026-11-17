//! Pure, deterministic motion model: spring camera, cursor smoothing, ripples, zoom planning.
//!
//! Port of upstream `NSSMotion`. See `PLAN.md` §5 Stage 2 and Appendix A.
//!
//! The contract (inherited verbatim): [`SpringMotionModel::frame_state`] is a pure function of
//! time. Everything stateful — springs, dead zone, idle fade, press squish — is simulated once
//! at construction on a fixed 240 Hz grid, so preview, scrubbing and export all read the same
//! numbers and therefore render identical frames.
//!
//! Two invariants worth knowing before touching this crate (both pinned by tests):
//!
//! * The camera viewport never leaves the source frame — see [`viewport`] and
//!   `ViewportPropertyTests` upstream.
//! * Zoom segments never overlap and honour the 0.3 s floor — see [`zoom_planner`].

#![forbid(unsafe_code)]

pub mod linear_motion_model;
pub mod motion_tuning;
pub mod spring;
pub mod spring_motion_model;
pub mod viewport;
pub mod zoom_planner;

#[cfg(test)]
mod test_support;

pub use linear_motion_model::LinearMotionModel;
pub use motion_tuning::MotionTuning;
pub use spring::{Spring, SpringStepper};
pub use spring_motion_model::SpringMotionModel;
pub use viewport::SourceViewport;
pub use zoom_planner::{ZoomOrigin, ZoomPlanner, ZoomSegment};

/// Build the shipped motion model from a project's motion-relevant inputs.
///
/// One place so the preview and the exporter cannot drift apart: both call this.
#[must_use]
pub fn factory(
    track: &nss_core::CursorTrack,
    timeline: &nss_core::Timeline,
    style: nss_core::CursorStyle,
    duration: nss_core::MediaTime,
    source_aspect: f64,
) -> SpringMotionModel {
    SpringMotionModel::new(
        track,
        timeline,
        style,
        duration,
        source_aspect,
        MotionTuning::standard(),
    )
}
