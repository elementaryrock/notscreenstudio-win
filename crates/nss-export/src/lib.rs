//! 60 fps MP4 export: frame pump, FFmpeg encoder pipe, asset frame source, audio pump.
//!
//! Replaces upstream NSSExport. See `PLAN.md` §5 Stage 7 and Appendix A.

#![forbid(unsafe_code)]

pub mod audio_pump;
pub mod exporter;
pub mod frame_source;
