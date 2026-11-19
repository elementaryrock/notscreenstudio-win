//! 120 Hz cursor and click logging with cursor-shape detection (replaces upstream `NSSInput`).
//!
//! Pure sampling, geometry normalization, and cursor-shape fingerprinting live in hardware-free
//! modules ([`geometry`], [`schedule`], [`shapes`], [`hook`], [`recorder`]); all Win32 FFI
//! (`GetCursorPos`, `WH_MOUSE_LL`, `GetCursorInfo`/`GetIconInfoExW`, `timeBeginPeriod`) is
//! confined to [`win`] (ADR 0012).
//!
//! All timestamps use `nss_core::host_now()`, the exact same monotonic clock as
//! `RecordingAssets::start_host_time`, so `stop(start_host_time)` rebases every sample, click,
//! and shape change onto `t = 0` at the first delivered screen frame.

#![deny(unsafe_code)]

pub mod geometry;
pub mod hook;
pub mod recorder;
pub mod schedule;
pub mod shapes;

#[cfg(windows)]
pub mod win;

pub use geometry::{CoordinateMath, ScreenGeometryProvider, SystemScreenGeometry};
pub use hook::{map_win32_mouse_message, ClickCallback, MouseHookHandle, RawMouseClick};
pub use recorder::{
    ClockProvider, CursorRecorder, NoopInputLogger, PositionProvider, SystemInputLogger,
    SystemInputLoggerConfig,
};
pub use schedule::{SamplingSchedule, SamplingStats};
pub use shapes::{
    bitmap_hash, CursorFingerprint, CursorShapeIdentifier, CursorShapeRecorder, CursorShapeSampler,
    ShapeReference, SystemCursorShapeSampler,
};
