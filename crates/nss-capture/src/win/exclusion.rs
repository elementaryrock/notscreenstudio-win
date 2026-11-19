//! Per-HWND capture exclusion (`WDA_EXCLUDEFROMCAPTURE`, ADR 0007).
//!
//! On Windows, `Windows.Graphics.Capture` does not take an exclusion list on the capture session;
//! instead each window our app creates opts out via `SetWindowDisplayAffinity`.

#![allow(unsafe_code)]

use nss_core::NssError;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowDisplayAffinity, SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE, WDA_NONE,
    WINDOW_DISPLAY_AFFINITY,
};

/// Marks `hwnd` invisible to `Windows.Graphics.Capture` (Windows 10 2004+).
///
/// Only windows owned by the current process can have their affinity set by Win32.
pub fn exclude_window_from_capture(hwnd: u64) -> Result<(), NssError> {
    if hwnd == 0 {
        return Ok(());
    }
    let handle = HWND(hwnd as usize as *mut std::ffi::c_void);
    // SAFETY: `SetWindowDisplayAffinity` validates the HWND and returns an error for invalid or
    // foreign handles rather than invoking UB.
    unsafe {
        SetWindowDisplayAffinity(handle, WDA_EXCLUDEFROMCAPTURE).map_err(|e| {
            NssError::Io(format!(
                "Could not exclude window 0x{hwnd:x} from capture: {e}"
            ))
        })
    }
}

/// Clears capture exclusion on `hwnd`, making it capturable again.
pub fn allow_window_in_capture(hwnd: u64) -> Result<(), NssError> {
    if hwnd == 0 {
        return Ok(());
    }
    let handle = HWND(hwnd as usize as *mut std::ffi::c_void);
    // SAFETY: `SetWindowDisplayAffinity` validates the HWND.
    unsafe {
        SetWindowDisplayAffinity(handle, WDA_NONE).map_err(|e| {
            NssError::Io(format!(
                "Could not restore capture visibility for window 0x{hwnd:x}: {e}"
            ))
        })
    }
}

/// Applies `WDA_EXCLUDEFROMCAPTURE` to every handle in `hwnds`, returning how many succeeded.
/// Failures on individual handles (e.g. a window that closed between enumeration and exclusion)
/// are ignored so one stale HWND never breaks the others.
#[must_use]
pub fn exclude_all(hwnds: &[u64]) -> usize {
    let mut applied = 0usize;
    for &h in hwnds {
        if exclude_window_from_capture(h).is_ok() {
            applied += 1;
        }
    }
    applied
}

/// Returns whether `hwnd` currently has `WDA_EXCLUDEFROMCAPTURE` set.
#[must_use]
pub fn is_excluded_from_capture(hwnd: u64) -> bool {
    if hwnd == 0 {
        return false;
    }
    let handle = HWND(hwnd as usize as *mut std::ffi::c_void);
    let mut affinity: u32 = 0;
    // SAFETY: `affinity` is a valid out-pointer to `u32`.
    let ok = unsafe { GetWindowDisplayAffinity(handle, &mut affinity).is_ok() };
    ok && WINDOW_DISPLAY_AFFINITY(affinity) == WDA_EXCLUDEFROMCAPTURE
}
