//! 60 fps MP4 exporter, sequential at-or-before frame decoder, and audio mixer (replaces `NSSExport`).
//!
//! Pure Rust + FFmpeg subprocesses (`#![forbid(unsafe_code)]`). Preview and export share the same
//! `nss-motion` model, `nss-render` compositor, and [`AssetFrameSource`] at-or-before selection
//! rule so exported MP4 frames match the editor preview frame-for-frame.

#![forbid(unsafe_code)]

pub mod audio_pump;
pub mod exporter;
pub mod ffmpeg;
pub mod frame_source;

pub use audio_pump::{AudioMixPlan, OUTPUT_AAC_BITRATE, OUTPUT_CHANNELS, OUTPUT_SAMPLE_RATE};
pub use exporter::{
    target_bitrate, CancelToken, ExportError, ExportFrameSource, FrameSourceFactory, MotionFactory,
    Mp4Exporter,
};
pub use ffmpeg::{
    FfmpegCapabilities, FfmpegPaths, MediaProbe, PacketInfo, FFMPEG_DOWNLOAD_URL,
    TESTED_FFMPEG_VERSION,
};
pub use frame_source::{select_at_or_before, AssetFrameSource, DecodeErrorReporting};
