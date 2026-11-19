//! Screen, microphone and camera capture for Windows (replaces upstream `NSSCapture`).
//!
//! The crate is split the way the upstream module is split: the rules live in plain value types that
//! can be tested anywhere ([`capture_math`], [`pause_clock`], [`frame_gate`], [`capture_filter`],
//! [`capture_errors`], [`state_hub`], [`permissions`]), and the platform layer under [`win`] does
//! the WinRT/Win32 calls (ADR 0012). Nothing in here knows about Tauri, and nothing makes a network
//! request.
//!
//! What the recorders guarantee, carried over from upstream unchanged (Appendix B of `PLAN.md`):
//!
//! - t = 0 is the **first fully delivered screen frame after the countdown**, not the moment capture
//!   started: [`frame_gate::FrameGate`] holds frames during the countdown and stamps the held one with
//!   the instant it opens.
//! - Paused time is not in the recording. Samples inside a pause are dropped and everything later moves
//!   earlier, so the file has no gap ([`pause_clock`]).
//! - Our own windows never appear in a display or area capture (ADR 0007): the
//!   [`capture_filter::FilterPlan`] names them and the recorder hides each one with
//!   `WDA_EXCLUDEFROMCAPTURE`.
//! - The hardware cursor is **not** captured (`SetIsCursorCaptureEnabled(false)`); it is redrawn from
//!   the input log at 120 Hz (ADR 0008), which is why the recorder never sees a cursor in its frames.

#![deny(unsafe_code)]

pub mod capture_errors;
pub mod capture_filter;
pub mod capture_math;
pub mod catalog;
pub mod device;
pub mod frame_gate;
pub mod pause_clock;
pub mod permissions;
pub mod recorder;
pub mod state_hub;
pub mod win;
pub mod writer;

pub use capture_errors::{map_code, map_error, CaptureContext, Hresult};
pub use capture_filter::{ContentSnapshot, ExclusionPlan, FilterPlan, Target};
pub use capture_math::{
    even_pixels, host_now, CaptureFilePlan, SharedOrigin, TimeOrigin, FRAME_RATE,
};
pub use catalog::{CaptureDisplayInfo, CaptureSourceCatalog, CaptureSourceList, CaptureWindowInfo};
pub use device::{
    aac_bitrate, CameraRecorder, CaptureDeviceInfo, CaptureDevices, DeviceKind, MicRecorder,
};
pub use frame_gate::FrameGate;
pub use pause_clock::{PauseClock, PauseInterval, PauseTimeline, PausedFrameHold};
pub use permissions::{
    BorderlessAccessGate, CapturePermissions, PermissionSnapshot, PermissionStatus,
};
pub use recorder::{
    NoopRecordingSession, OwnWindowsProvider, ScreenCaptureRecordingSession, ScreenStreamPlan,
    WarmableRecordingSession,
};
pub use state_hub::{CaptureEvent, Component, EventHub, Listener, StateHub};
pub use writer::{
    find_ffmpeg, AudioWriterConfig, PcmAudioWriter, RawVideoWriter, VideoWriterConfig,
};
