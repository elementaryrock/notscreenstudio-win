//! Windows capture: Graphics Capture (screen), WASAPI (mic), Media Foundation (camera).
//!
//! Replaces upstream NSSCapture. See `PLAN.md` §5 Stage 5 and Appendix A.

#![forbid(unsafe_code)]

pub mod catalog;
pub mod device;
pub mod gate;
pub mod pause;
pub mod recorder;
pub mod writer;
