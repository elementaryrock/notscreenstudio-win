//! Pure, deterministic motion model: spring camera, cursor smoothing, ripples, zoom planning.
//!
//! Replaces upstream NSSMotion. See `PLAN.md` §5 Stage 2 and Appendix A.

#![forbid(unsafe_code)]

pub mod linear_motion_model;
pub mod motion_tuning;
pub mod spring;
pub mod spring_motion_model;
pub mod viewport;
pub mod zoom_planner;
