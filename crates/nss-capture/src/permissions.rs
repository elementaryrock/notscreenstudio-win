//! Capture permissions and borderless-access tracking.
//!
//! Port of upstream `NSSCapture/CapturePermissions.swift`, `ScreenRecordingAccess.swift`, and
//! `ScreenRecordingGate.swift`, adapted for Windows:
//!
//! - Windows has no macOS TCC screen-recording permission that blocks enumeration or requires an
//!   app relaunch, so screen capture itself is `Granted`.
//! - Microphone and camera desktop-app toggles live in Windows Settings > Privacy & security and
//!   are checked before opening the devices.
//! - Borderless capture (hiding the Windows yellow capture border, ADR 0007) is requested at most
//!   once per session/process and degrades silently if unavailable or declined.

use std::sync::atomic::{AtomicBool, Ordering};

use nss_core::{NssError, RecordingOptions};

/// Authorization state for a single capture capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionStatus {
    Granted,
    Denied,
    NotDetermined,
}

/// Snapshot of every permission the Settings / Permissions panel renders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PermissionSnapshot {
    pub screen_recording: PermissionStatus,
    pub microphone: PermissionStatus,
    pub camera: PermissionStatus,
    pub borderless_capture: PermissionStatus,
}

/// Tracks one-shot user-prompted access requests (port of upstream `ScreenRecordingGate`).
#[derive(Debug)]
pub struct BorderlessAccessGate {
    asked: AtomicBool,
    granted: AtomicBool,
}

impl Default for BorderlessAccessGate {
    fn default() -> Self {
        Self::new()
    }
}

impl BorderlessAccessGate {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            asked: AtomicBool::new(false),
            granted: AtomicBool::new(false),
        }
    }

    #[must_use]
    pub fn has_asked(&self) -> bool {
        self.asked.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn is_granted(&self) -> bool {
        self.granted.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn status(&self) -> PermissionStatus {
        if self.is_granted() {
            PermissionStatus::Granted
        } else if self.has_asked() {
            PermissionStatus::Denied
        } else {
            PermissionStatus::NotDetermined
        }
    }

    /// Runs `requester` at most once and caches the result.
    pub fn request_once<F: FnOnce() -> bool>(&self, requester: F) -> bool {
        if !self.asked.swap(true, Ordering::SeqCst) {
            let ok = requester();
            self.granted.store(ok, Ordering::SeqCst);
            ok
        } else {
            self.is_granted()
        }
    }
}

static GLOBAL_BORDERLESS_GATE: BorderlessAccessGate = BorderlessAccessGate::new();

/// Check and request capture permissions on Windows.
pub struct CapturePermissions;

impl CapturePermissions {
    pub const SETTINGS_HINT_MIC: &'static str =
        "Microphone access is turned off. Enable it in Windows Settings > Privacy & security > Microphone.";
    pub const SETTINGS_HINT_CAMERA: &'static str =
        "Camera access is turned off. Enable it in Windows Settings > Privacy & security > Camera.";
    pub const BORDERLESS_NOTE: &'static str =
        "Windows shows a yellow border while recording — Windows 11 can turn this off.";

    /// Screen capture does not require a macOS-style TCC grant on Windows.
    #[must_use]
    pub fn screen_recording_status() -> PermissionStatus {
        PermissionStatus::Granted
    }

    #[must_use]
    pub fn microphone_status() -> PermissionStatus {
        #[cfg(windows)]
        {
            crate::win::permissions::microphone_status()
        }
        #[cfg(not(windows))]
        {
            PermissionStatus::Granted
        }
    }

    #[must_use]
    pub fn camera_status() -> PermissionStatus {
        #[cfg(windows)]
        {
            crate::win::permissions::camera_status()
        }
        #[cfg(not(windows))]
        {
            PermissionStatus::Granted
        }
    }

    #[must_use]
    pub fn borderless_status() -> PermissionStatus {
        GLOBAL_BORDERLESS_GATE.status()
    }

    /// Requests borderless capture access at most once per process (ADR 0007).
    pub fn request_borderless_once() -> bool {
        GLOBAL_BORDERLESS_GATE.request_once(|| {
            #[cfg(windows)]
            {
                crate::win::permissions::request_borderless_access()
            }
            #[cfg(not(windows))]
            {
                false
            }
        })
    }

    #[must_use]
    pub fn snapshot() -> PermissionSnapshot {
        PermissionSnapshot {
            screen_recording: Self::screen_recording_status(),
            microphone: Self::microphone_status(),
            camera: Self::camera_status(),
            borderless_capture: Self::borderless_status(),
        }
    }

    /// Validates that every capability requested by `options` is permitted, returning an
    /// actionable `NssError::PermissionDenied` naming the first blocked capability.
    pub fn ensure(options: &RecordingOptions) -> Result<(), NssError> {
        Self::ensure_with(options, Self::microphone_status(), Self::camera_status())
    }

    /// Pure, hardware-free core of [`Self::ensure`], unit-tested without touching the registry.
    pub fn ensure_with(
        options: &RecordingOptions,
        mic: PermissionStatus,
        camera: PermissionStatus,
    ) -> Result<(), NssError> {
        if options.mic_device_id.is_some() && mic == PermissionStatus::Denied {
            return Err(NssError::PermissionDenied(Self::SETTINGS_HINT_MIC.into()));
        }
        if options.camera_device_id.is_some() && camera == PermissionStatus::Denied {
            return Err(NssError::PermissionDenied(
                Self::SETTINGS_HINT_CAMERA.into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nss_core::CaptureSource;

    #[test]
    fn screen_only_options_ignore_denied_mic_and_camera() {
        let opts = RecordingOptions::new(CaptureSource::Display { id: 1 });
        assert!(CapturePermissions::ensure_with(
            &opts,
            PermissionStatus::Denied,
            PermissionStatus::Denied,
        )
        .is_ok());
    }

    #[test]
    fn requesting_denied_mic_returns_permission_denied() {
        let mut opts = RecordingOptions::new(CaptureSource::Display { id: 1 });
        opts.mic_device_id = Some("mic-1".into());
        let err = CapturePermissions::ensure_with(
            &opts,
            PermissionStatus::Denied,
            PermissionStatus::Granted,
        )
        .unwrap_err();
        assert!(matches!(err, NssError::PermissionDenied(m) if m.contains("Microphone")));
    }

    #[test]
    fn requesting_denied_camera_returns_permission_denied() {
        let mut opts = RecordingOptions::new(CaptureSource::Display { id: 1 });
        opts.camera_device_id = Some("cam-1".into());
        let err = CapturePermissions::ensure_with(
            &opts,
            PermissionStatus::Granted,
            PermissionStatus::Denied,
        )
        .unwrap_err();
        assert!(matches!(err, NssError::PermissionDenied(m) if m.contains("Camera")));
    }

    #[test]
    fn borderless_gate_prompts_at_most_once() {
        let gate = BorderlessAccessGate::new();
        assert_eq!(gate.status(), PermissionStatus::NotDetermined);
        let mut calls = 0;
        assert!(gate.request_once(|| {
            calls += 1;
            true
        }));
        assert!(gate.request_once(|| {
            calls += 1;
            false
        }));
        assert_eq!(calls, 1);
        assert_eq!(gate.status(), PermissionStatus::Granted);
    }

    #[test]
    fn borderless_gate_remembers_denial() {
        let gate = BorderlessAccessGate::new();
        assert!(!gate.request_once(|| false));
        assert_eq!(gate.status(), PermissionStatus::Denied);
        assert!(gate.has_asked());
    }
}
