//! Turns platform errors into `NssError`s whose message can be shown to the user as-is.
//!
//! Port of upstream `NSSCapture/CaptureErrors.swift`. Upstream switched on `SCStreamErrorDomain` and
//! `AVFoundationErrorDomain` codes; here the codes come from HRESULTs (`E_ACCESSDENIED`, the
//! `MF_E_*` device family, `HRESULT_FROM_WIN32(ERROR_DISK_FULL)` and friends) and from WGC's
//! `GraphicsCaptureSession` failures. The wording is kept, because these strings are what the UI
//! shows and what the ADR 0004 floor promised for Win10 2004: no permission dialog that cannot
//! actually exist, and no "open System Settings" that Windows does not have.

use nss_core::NssError;

/// Where an error came from, so the message can be specific.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureContext {
    Screen,
    Window,
    Microphone,
    Camera,
    Writer,
}

impl CaptureContext {
    /// The noun used in "X was disconnected." / "X permission is required."
    #[must_use]
    pub fn device_name(self) -> &'static str {
        match self {
            Self::Microphone => "The microphone",
            Self::Camera => "The camera",
            Self::Window => "The window",
            Self::Screen => "The display",
            Self::Writer => "The recording file",
        }
    }
}

/// Windows error codes this module recognises, in the HRESULT form Win32/WinRT report them.
///
/// Kept as plain constants so the mapping is testable without a device or a display.
pub mod codes {
    /// `E_ACCESSDENIED`. A privacy setting or a policy refused the device.
    pub const E_ACCESSDENIED: u32 = 0x8007_0005;
    /// `E_ABORT`.
    pub const E_ABORT: u32 = 0x8000_4004;
    /// `E_FAIL`, the catch-all WGC returns when the session cannot start.
    pub const E_FAIL: u32 = 0x8000_4005;
    /// `E_UNEXPECTED`.
    pub const E_UNEXPECTED: u32 = 0x8000_FFFF;
    /// `HRESULT_FROM_WIN32(ERROR_DISK_FULL)`.
    pub const ERROR_DISK_FULL: u32 = 0x8007_0070;
    /// `HRESULT_FROM_WIN32(ERROR_HANDLE_DISK_FULL)`.
    pub const ERROR_HANDLE_DISK_FULL: u32 = 0x8007_0027;
    /// `HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND)`.
    pub const ERROR_FILE_NOT_FOUND: u32 = 0x8007_0002;
    /// `HRESULT_FROM_WIN32(ERROR_NOT_FOUND)`.
    pub const ERROR_NOT_FOUND: u32 = 0x8007_0490;
    /// `HRESULT_FROM_WIN32(ERROR_INVALID_HANDLE)` — the window or display went away.
    pub const ERROR_INVALID_HANDLE: u32 = 0x8007_0006;
    /// `HRESULT_FROM_WIN32(ERROR_DEVICE_NOT_CONNECTED)`.
    pub const ERROR_DEVICE_NOT_CONNECTED: u32 = 0x8007_048F;
    /// `HRESULT_FROM_WIN32(ERROR_DEVICE_IN_USE)` (2404 = 0x964).
    pub const ERROR_DEVICE_IN_USE: u32 = 0x8007_0964;
    /// `HRESULT_FROM_WIN32(ERROR_NOT_ENOUGH_MEMORY)`.
    pub const ERROR_NOT_ENOUGH_MEMORY: u32 = 0x8007_0008;
    /// `MF_E_ATTRIBUTENOTFOUND`.
    pub const MF_E_ATTRIBUTENOTFOUND: u32 = 0xC00D_36AE;
    /// `MF_E_INVALIDMEDIATYPE`.
    pub const MF_E_INVALIDMEDIATYPE: u32 = 0xC00D_36BC;
    /// `MF_E_HW_MFT_FAILED_START_STREAMING`.
    pub const MF_E_HW_MFT_FAILED_START_STREAMING: u32 = 0xC00D_3704;
    /// `AUDCLNT_E_DEVICE_INVALIDATED`.
    pub const AUDCLNT_E_DEVICE_INVALIDATED: u32 = 0x8889_0004;
    /// `AUDCLNT_E_DEVICE_IN_USE`.
    pub const AUDCLNT_E_DEVICE_IN_USE: u32 = 0x8889_000A;
    /// `AUDCLNT_E_UNSUPPORTED_FORMAT`.
    pub const AUDCLNT_E_UNSUPPORTED_FORMAT: u32 = 0x8889_0008;
}

/// Turns a classification code into `NssError`.
///
/// `code` is the raw HRESULT (`0x8004_0005` style) and `detail` the platform's own message, used only
/// for the fallback so an unexpected failure is still diagnosable.
#[must_use]
pub fn map_code(code: u32, detail: &str, context: CaptureContext) -> NssError {
    use codes::*;
    match code {
        E_ACCESSDENIED => NssError::PermissionDenied(format!(
            "{} permission is required. Turn it on in Settings > Privacy & security > {}.",
            context.device_name(),
            privacy_page(context)
        )),
        ERROR_DISK_FULL | ERROR_HANDLE_DISK_FULL => {
            NssError::Io("The disk is full. Free up space and record again.".into())
        }
        ERROR_FILE_NOT_FOUND | ERROR_NOT_FOUND | ERROR_INVALID_HANDLE => {
            if context == CaptureContext::Window {
                NssError::Io("The captured window was closed.".into())
            } else {
                NssError::Unsupported("The selected capture source is no longer available.".into())
            }
        }
        ERROR_DEVICE_NOT_CONNECTED
        | AUDCLNT_E_DEVICE_INVALIDATED
        | MF_E_HW_MFT_FAILED_START_STREAMING => {
            NssError::Io(format!("{} was disconnected.", context.device_name()))
        }
        ERROR_DEVICE_IN_USE | AUDCLNT_E_DEVICE_IN_USE => NssError::Io(format!(
            "{} is in use by another app.",
            context.device_name()
        )),
        AUDCLNT_E_UNSUPPORTED_FORMAT | MF_E_INVALIDMEDIATYPE | MF_E_ATTRIBUTENOTFOUND => {
            NssError::Unsupported(format!(
                "{} cannot record in the requested format.",
                context.device_name()
            ))
        }
        ERROR_NOT_ENOUGH_MEMORY => NssError::Io(
            "There is not enough memory to record. Close some apps and try again.".into(),
        ),
        E_ABORT => NssError::Io("Recording was stopped.".into()),
        E_FAIL | E_UNEXPECTED => NssError::Io(format!(
            "{} capture failed: {detail}",
            context.device_name()
        )),
        _ => {
            if detail.is_empty() {
                NssError::Io(format!(
                    "{} capture failed (0x{code:08X}).",
                    context.device_name()
                ))
            } else {
                NssError::Io(format!(
                    "{} capture failed: {detail}",
                    context.device_name()
                ))
            }
        }
    }
}

/// The Windows Settings page that controls `context`, for the permission message.
#[must_use]
pub fn privacy_page(context: CaptureContext) -> &'static str {
    match context {
        CaptureContext::Microphone => "Microphone",
        CaptureContext::Camera => "Camera",
        _ => "Screen capture",
    }
}

/// Passes an `NssError` through and classifies anything else, so callers can `?` both kinds.
///
/// The Windows layer wraps API failures in [`Hresult`], which is where the code to classify comes from;
/// any other error falls back to its own message.
#[must_use]
pub fn map_error(error: &(dyn std::error::Error + 'static), context: CaptureContext) -> NssError {
    if let Some(e) = error.downcast_ref::<NssError>() {
        return e.clone();
    }
    let code = error.downcast_ref::<Hresult>().map_or(0, |h| h.code);
    map_code(code, &error.to_string(), context)
}

/// A Windows API failure: the raw HRESULT plus the message the API or the docs gave for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hresult {
    pub code: u32,
    pub message: String,
}

impl Hresult {
    #[must_use]
    pub fn new(code: u32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for Hresult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (0x{:08X})", self.message, self.code)
    }
}

impl std::error::Error for Hresult {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disk_full_is_reported_as_disk_full() {
        let e = map_code(codes::ERROR_DISK_FULL, "", CaptureContext::Writer);
        assert!(matches!(&e, NssError::Io(m) if m.contains("disk is full")));
    }

    #[test]
    fn a_closed_window_is_an_io_error_not_a_permission_one() {
        let e = map_code(codes::ERROR_INVALID_HANDLE, "", CaptureContext::Window);
        assert!(matches!(&e, NssError::Io(m) if m.contains("window was closed")));
        let d = map_code(codes::ERROR_INVALID_HANDLE, "", CaptureContext::Screen);
        assert!(matches!(&d, NssError::Unsupported(m) if m.contains("no longer available")));
    }

    #[test]
    fn a_refused_device_points_at_the_right_settings_page() {
        let cam = map_code(codes::E_ACCESSDENIED, "", CaptureContext::Camera);
        assert!(matches!(&cam, NssError::PermissionDenied(m) if m.contains("Camera")));
        let mic = map_code(codes::E_ACCESSDENIED, "", CaptureContext::Microphone);
        assert!(matches!(&mic, NssError::PermissionDenied(m) if m.contains("Microphone")));
    }

    #[test]
    fn a_disconnected_device_names_itself() {
        let e = map_code(
            codes::ERROR_DEVICE_NOT_CONNECTED,
            "",
            CaptureContext::Camera,
        );
        assert!(matches!(&e, NssError::Io(m) if m == "The camera was disconnected."));
        let wasapi = map_code(
            codes::AUDCLNT_E_DEVICE_INVALIDATED,
            "",
            CaptureContext::Microphone,
        );
        assert!(matches!(&wasapi, NssError::Io(m) if m.contains("microphone was disconnected")));
    }

    #[test]
    fn an_unsupported_format_is_unsupported() {
        let e = map_code(
            codes::AUDCLNT_E_UNSUPPORTED_FORMAT,
            "",
            CaptureContext::Microphone,
        );
        assert!(matches!(&e, NssError::Unsupported(_)));
    }

    #[test]
    fn an_unknown_code_keeps_the_platform_detail() {
        let e = map_code(0x8000_1234, "something specific", CaptureContext::Screen);
        assert!(matches!(&e, NssError::Io(m) if m.contains("something specific")));
        let bare = map_code(0x8000_1234, "", CaptureContext::Screen);
        assert!(matches!(&bare, NssError::Io(m) if m.contains("0x80001234")));
    }

    #[test]
    fn an_nss_error_passes_through_unchanged() {
        let original = NssError::Unsupported("nope".into());
        assert_eq!(map_error(&original, CaptureContext::Screen), original);
    }

    #[test]
    fn an_hresult_is_classified_by_its_code() {
        let e = Hresult::new(codes::ERROR_NOT_ENOUGH_MEMORY, "out of memory");
        assert!(matches!(
            map_error(&e, CaptureContext::Camera),
            NssError::Io(m) if m.contains("not enough memory")
        ));
        assert_eq!(e.to_string(), "out of memory (0x80070008)");
    }
}
