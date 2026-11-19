//! Windows privacy consent checks (microphone, camera) and borderless WGC access.
//!
//! Windows has no macOS TCC gate for screen recording itself, so screen recording is always
//! allowed unless the user's policy blocks WGC. Microphone and camera desktop-app consent live in
//! `CapabilityAccessManager\ConsentStore\{microphone,webcam}` in the registry.

#![allow(unsafe_code)]

use std::sync::mpsc;
use std::time::Duration;

use windows::core::PCWSTR;
use windows::Graphics::Capture::{GraphicsCaptureAccess, GraphicsCaptureAccessKind};
use windows::Security::Authorization::AppCapabilityAccess::AppCapabilityAccessStatus;
use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::System::Registry::{
    RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE,
    KEY_READ, REG_SZ,
};
use windows::Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED};

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
/// Runs on a dedicated MTA worker thread with a bounded timeout so calling it can never deadlock
/// an STA UI thread or hang indefinitely when Windows suppresses the prompt. Returns `true` only
/// if Windows explicitly granted borderless capture; returns `false` cleanly on Windows 10,
/// unpackaged builds without the capability, timeout, or refusal. Never panics.
#[must_use]
pub fn request_borderless_access() -> bool {
    let (tx, rx) = mpsc::sync_channel::<bool>(1);
    let spawned = std::thread::Builder::new()
        .name("nss-borderless-access".into())
        .spawn(move || {
            crate::win::wgc::ensure_process_mta();
            // SAFETY: balanced WinRT MTA init on this dedicated thread.
            let should_uninit = match unsafe { RoInitialize(RO_INIT_MULTITHREADED) } {
                Ok(()) => true,
                Err(e) if e.code() == RPC_E_CHANGED_MODE => false,
                Err(_) => false,
            };
            let allowed =
                GraphicsCaptureAccess::RequestAccessAsync(GraphicsCaptureAccessKind::Borderless)
                    .ok()
                    .and_then(|op| op.join().ok())
                    .is_some_and(|status| status == AppCapabilityAccessStatus::Allowed);
            if should_uninit {
                // SAFETY: paired with successful `RoInitialize` above.
                unsafe { RoUninitialize() };
            }
            let _ = tx.send(allowed);
        });
    if spawned.is_err() {
        return false;
    }
    rx.recv_timeout(Duration::from_secs(2)).unwrap_or(false)
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
