//! 120 Hz cursor and click logging with cursor-shape detection.
//!
//! Replaces upstream NSSInput. See `PLAN.md` §5 Stage 6 and Appendix A.

#![forbid(unsafe_code)]

pub mod geometry;
pub mod hook;
pub mod recorder;
pub mod schedule;
pub mod shapes;
