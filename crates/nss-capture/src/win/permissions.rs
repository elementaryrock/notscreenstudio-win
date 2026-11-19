//! Windows privacy consent checks (microphone, camera) and borderless WGC access.
//!
//! Windows has no macOS TCC gate for screen recording itself, so screen recording is always
//! allowed unless the user's policy blocks WGC. Microphone and camera desktop-app consent live in
//! `CapabilityAccessManager\ConsentStore\{microphone,webcam}` in the registry.

#![allow(unsafe_code)]

use windows::core::PCWSTR;
use windows::Graphics::Capture::{GraphicsCaptureAccess, GraphicsCaptureAccessKind};
use windows::Security::Authorization::AppCapabilityAccess::AppCapabilityAccessStatus;
use windows::Win32::System::Registry::{
    RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE,
    KEY_READ, REG_SZ,
};

use crate::permissions::PermissionStatus;

const MIC_KEY: &str =
    "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\CapabilityAccessManager\\ConsentStore\\microphone";
const CAM_KEY: &str =
    "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\CapabilityAccessManager\\ConsentStore\\webcam";

/// Reads Windows privacy settings for desktop-app microphone access.
#[must_use]
pub fn microphone_status() -> PermissionStatus {
    consent_store_status(MIC_KEY)
}

/// Reads Windows privacy settings for desktop-app camera access.
#[must_use]
pub fn camera_status() -> PermissionStatus {
    consent_store_status(CAM_KEY)
}

/// Requests `GraphicsCaptureAccessKind::Borderless` once (ADR 0007).
///
/// Returns `true` only if Windows explicitly granted borderless capture; returns `false` cleanly
/// on Windows 10, unpackaged builds without the capability, or when refused. Never panics.
#[must_use]
pub fn request_borderless_access() -> bool {
    let Ok(op) = GraphicsCaptureAccess::RequestAccessAsync(GraphicsCaptureAccessKind::Borderless)
    else {
        return false;
    };
    matches!(op.join(), Ok(AppCapabilityAccessStatus::Allowed))
}

fn consent_store_status(subkey: &str) -> PermissionStatus {
    // Check both the top-level device toggle and the `NonPackaged` (Win32 desktop apps) toggle.
    let non_packaged = format!("{subkey}\\NonPackaged");
    for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        for path in [subkey, non_packaged.as_str()] {
            if let Some(val) = read_reg_string(root, path, "Value") {
                if val.eq_ignore_ascii_case("Deny") {
                    return PermissionStatus::Denied;
                }
            }
        }
    }
    PermissionStatus::Granted
}

fn read_reg_string(root: HKEY, subkey: &str, value_name: &str) -> Option<String> {
    let subkey_w: Vec<u16> = subkey.encode_utf16().chain(std::iter::once(0)).collect();
    let value_w: Vec<u16> = value_name
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut hkey = HKEY::default();
    // SAFETY: `subkey_w` is NUL-terminated; `hkey` is closed before returning.
    unsafe {
        if RegOpenKeyExW(
            root,
            PCWSTR(subkey_w.as_ptr()),
            Some(0),
            KEY_READ,
            &mut hkey,
        )
        .is_err()
        {
            return None;
        }
        let mut reg_type = REG_SZ;
        let mut buf = [0u16; 128];
        let mut byte_len = std::mem::size_of_val(&buf) as u32;
        let res = RegQueryValueExW(
            hkey,
            PCWSTR(value_w.as_ptr()),
            None,
            Some(&mut reg_type),
            Some(buf.as_mut_ptr().cast()),
            Some(&mut byte_len),
        );
        let _ = RegCloseKey(hkey);
        if res.is_err() || reg_type != REG_SZ {
            return None;
        }
        let u16_len = (byte_len as usize) / 2;
        let slice = &buf[..u16_len.min(buf.len())];
        let trimmed = slice
            .iter()
            .position(|&c| c == 0)
            .map_or(slice, |i| &slice[..i]);
        Some(String::from_utf16_lossy(trimmed))
    }
}
