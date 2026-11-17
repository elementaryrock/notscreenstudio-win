//! Shared models, conventions and contracts.
//!
//! Port of upstream `NSSCore`. See `PLAN.md` §5 Stage 1 and Appendix A.
//!
//! # Conventions (inherited verbatim from upstream `Basics.swift`)
//!
//! * All times are [`MediaTime`]: seconds (`f64`) from the first screen frame.
//! * All positions are normalised `0...1` in SOURCE space, origin **top-left**,
//!   x to the right, y downward.
//! * Motion is a pure function of time: no wall-clock state anywhere.
//!
//! This crate is platform-neutral by rule. It must never depend on `windows`,
//! `wgpu` or `tauri`.
//!
//! # Compatibility
//!
//! The `serde` representation of every persisted type matches Swift's `Codable` output, so
//! `project.json` and `cursor.json` are interchangeable with the macOS build (PLAN.md §7.3)
//! and the committed Mac fixtures can be used as golden data.

#![forbid(unsafe_code)]

pub mod background_presets;
pub mod basics;
pub mod cursor;
pub mod frame_state;
pub mod project;
pub mod protocols;
pub mod recording;
pub mod serde_util;
pub mod style;
pub mod timeline;

pub use basics::{
    MediaTime, NormalizedPoint, NormalizedSize, PixelSize, Point2, RGBAColor, Rect, Size2,
    Timestamp,
};
pub use cursor::{
    ClickEvent, ClickKind, CursorSample, CursorShape, CursorShapeChange, CursorTrack, MouseButton,
};
pub use frame_state::{CameraState, CursorState, FrameState, RippleState};
pub use project::{
    ExportPreset, ExportProgress, Project, ProjectReference, ProjectSummary, VideoCodec,
};
pub use protocols::{
    InMemoryProjectLibrary, InputLogger, MotionModel, NssError, ProjectLibrary, ProjectStore,
    RecordingSession, Result,
};
pub use recording::{CaptureSource, RecordingAssets, RecordingOptions, RecordingState};
pub use style::{
    AspectRatio, Background, BackgroundBlur, CameraBorder, CameraBubble, CameraBubbleGeometry,
    CameraShape, CursorStyle, ExportResolution, GradientStop, MeshPoint, Shadow, Style,
};
pub use timeline::{Timeline, TrimRange, ZoomBlock, ZoomFocus};
