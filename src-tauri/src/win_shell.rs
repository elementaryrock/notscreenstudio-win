//! Confined Win32 FFI helpers for the Tauri shell (ADR 0012).
//!
//! Owns:
//! - `SHGetKnownFolderPath(FOLDERID_Videos)` so redirected OneDrive Videos folders are respected.
//! - `SHOpenFolderAndSelectItems` ("Show in Explorer").
//! - `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)` + `WS_EX_TOOLWINDOW` for HUD overlays.
//! - `RegisterHotKey` on a dedicated Win32 message-pump thread for global record/pause shortcuts.

#![allow(unsafe_code)]

use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};
use std::thread::{self, JoinHandle};

#[cfg(windows)]
use windows::core::{PCWSTR, PWSTR};
#[cfg(windows)]
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
#[cfg(windows)]
use windows::Win32::System::Com::{
    CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_APARTMENTTHREADED,
};
#[cfg(windows)]
use windows::Win32::System::Threading::GetCurrentThreadId;
#[cfg(windows)]
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT,
    MOD_SHIFT, MOD_WIN,
};
#[cfg(windows)]
use windows::Win32::UI::Shell::{
    FOLDERID_Videos, ILCreateFromPathW, ILFree, SHGetKnownFolderPath, SHOpenFolderAndSelectItems,
    KF_FLAG_DEFAULT,
};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    GetMessageW, GetWindowLongPtrW, PostThreadMessageW, SetWindowDisplayAffinity,
    SetWindowLongPtrW, GWL_EXSTYLE, MSG, WDA_EXCLUDEFROMCAPTURE, WM_HOTKEY, WM_QUIT,
    WS_EX_TOOLWINDOW,
};

/// Resolves the user's real Windows Videos folder via `SHGetKnownFolderPath(FOLDERID_Videos)`,
/// properly handling folders redirected into OneDrive or a secondary drive.
#[must_use]
pub fn known_videos_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        // SAFETY: `SHGetKnownFolderPath` allocates a wide string freed with `CoTaskMemFree`.
        unsafe {
            let raw: PWSTR = SHGetKnownFolderPath(&FOLDERID_Videos, KF_FLAG_DEFAULT, None).ok()?;
            let s = raw.to_string().ok();
            CoTaskMemFree(Some(raw.0.cast()));
            s.map(PathBuf::from)
        }
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Opens Windows File Explorer with `path` selected.
pub fn reveal_in_explorer(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        let wide: Vec<u16> = path
            .as_os_str()
            .to_string_lossy()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        // SAFETY: STA COM initialized/uninitialized locally; `ILCreateFromPathW` result freed via `ILFree`.
        unsafe {
            let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let should_uninit = hr.is_ok();
            let pidl = ILCreateFromPathW(PCWSTR(wide.as_ptr()));
            let result = if !pidl.is_null() {
                let res = SHOpenFolderAndSelectItems(pidl, None, 0);
                ILFree(Some(pidl));
                res.map_err(|e| format!("Failed to open Explorer: {e}"))
            } else {
                Err(format!("Path does not exist: {}", path.display()))
            };
            if should_uninit {
                CoUninitialize();
            }
            result
        }
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Ok(())
    }
}

/// Applies `WDA_EXCLUDEFROMCAPTURE` to a window handle so it never appears in recordings (ADR 0007).
pub fn exclude_window_from_capture(hwnd: u64) -> Result<(), String> {
    #[cfg(windows)]
    {
        if hwnd == 0 {
            return Ok(());
        }
        let handle = HWND(hwnd as usize as *mut std::ffi::c_void);
        // SAFETY: `SetWindowDisplayAffinity` validates `handle`.
        unsafe {
            SetWindowDisplayAffinity(handle, WDA_EXCLUDEFROMCAPTURE)
                .map_err(|e| format!("WDA_EXCLUDEFROMCAPTURE failed: {e}"))
        }
    }
    #[cfg(not(windows))]
    {
        let _ = hwnd;
        Ok(())
    }
}

/// Marks a HUD overlay window with `WS_EX_TOOLWINDOW` so it stays out of Alt+Tab and the taskbar.
pub fn mark_tool_window(hwnd: u64) {
    #[cfg(windows)]
    {
        if hwnd == 0 {
            return;
        }
        let handle = HWND(hwnd as usize as *mut std::ffi::c_void);
        // SAFETY: standard extended window style bit-OR on an owned HWND.
        unsafe {
            let current = GetWindowLongPtrW(handle, GWL_EXSTYLE);
            let _ = SetWindowLongPtrW(handle, GWL_EXSTYLE, current | (WS_EX_TOOLWINDOW.0 as isize));
        }
    }
    #[cfg(not(windows))]
    {
        let _ = hwnd;
    }
}

/// Parsed global hotkey chord (modifiers + Win32 virtual-key code).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HotKeyChord {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub win: bool,
    pub vk: u32,
}

impl HotKeyChord {
    /// Parses strings like `"Ctrl+Shift+R"` or `"Ctrl+Alt+P"`.
    #[must_use]
    pub fn parse(spec: &str) -> Option<Self> {
        let mut chord = Self {
            ctrl: false,
            shift: false,
            alt: false,
            win: false,
            vk: 0,
        };
        for raw in spec.split('+') {
            let part = raw.trim().to_ascii_uppercase();
            match part.as_str() {
                "CTRL" | "CONTROL" => chord.ctrl = true,
                "SHIFT" => chord.shift = true,
                "ALT" => chord.alt = true,
                "WIN" | "SUPER" | "META" => chord.win = true,
                s if s.len() == 1 => {
                    let b = s.as_bytes()[0];
                    if b.is_ascii_alphanumeric() {
                        chord.vk = u32::from(b);
                    }
                }
                s if s.starts_with('F') => {
                    if let Ok(n) = s[1..].parse::<u32>() {
                        if (1..=12).contains(&n) {
                            chord.vk = 0x6F + n; // VK_F1 = 0x70
                        }
                    }
                }
                _ => return None,
            }
        }
        (chord.vk != 0 && (chord.ctrl || chord.alt || chord.win)).then_some(chord)
    }

    #[cfg(windows)]
    fn win32_modifiers(self) -> HOT_KEY_MODIFIERS {
        let mut m = MOD_NOREPEAT;
        if self.ctrl {
            m |= MOD_CONTROL;
        }
        if self.shift {
            m |= MOD_SHIFT;
        }
        if self.alt {
            m |= MOD_ALT;
        }
        if self.win {
            m |= MOD_WIN;
        }
        m
    }
}

/// Which global shortcut fired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotKeyAction {
    ToggleRecord,
    TogglePause,
}

/// Registration status for both global shortcuts (surfaces conflicts cleanly in Settings).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotKeyRegistrationStatus {
    pub record_registered: bool,
    pub record_conflict: Option<String>,
    pub pause_registered: bool,
    pub pause_conflict: Option<String>,
}

/// Owns the Win32 `RegisterHotKey` message-pump thread. Dropping unregisters cleanly.
pub struct GlobalHotKeyManager {
    #[cfg(windows)]
    thread_id: u32,
    #[cfg(windows)]
    join: Option<JoinHandle<()>>,
    pub status: HotKeyRegistrationStatus,
}

impl GlobalHotKeyManager {
    pub fn start<F>(record_spec: &str, pause_spec: &str, on_action: F) -> Self
    where
        F: Fn(HotKeyAction) + Send + Sync + 'static,
    {
        #[cfg(windows)]
        {
            const ID_RECORD: i32 = 1;
            const ID_PAUSE: i32 = 2;

            let record_chord = HotKeyChord::parse(record_spec);
            let pause_chord = HotKeyChord::parse(pause_spec);
            let record_label = record_spec.to_owned();
            let pause_label = pause_spec.to_owned();
            let on_action = Arc::new(on_action);
            let (tx, rx) = mpsc::sync_channel::<(u32, HotKeyRegistrationStatus)>(1);

            let join = thread::Builder::new()
                .name("nss-global-hotkey".into())
                .spawn(move || {
                    // SAFETY: `RegisterHotKey` / `UnregisterHotKey` and `GetMessageW` run on this
                    // dedicated thread; `WM_QUIT` triggers clean teardown.
                    unsafe {
                        let tid = GetCurrentThreadId();
                        let mut status = HotKeyRegistrationStatus::default();
                        if let Some(ch) = record_chord {
                            match RegisterHotKey(None, ID_RECORD, ch.win32_modifiers(), ch.vk) {
                                Ok(()) => status.record_registered = true,
                                Err(_) => {
                                    status.record_conflict = Some(format!(
                                        "{record_label} is already in use by another app."
                                    ));
                                }
                            }
                        }
                        if let Some(ch) = pause_chord {
                            match RegisterHotKey(None, ID_PAUSE, ch.win32_modifiers(), ch.vk) {
                                Ok(()) => status.pause_registered = true,
                                Err(_) => {
                                    status.pause_conflict = Some(format!(
                                        "{pause_label} is already in use by another app."
                                    ));
                                }
                            }
                        }
                        let _ = tx.send((tid, status.clone()));

                        let mut msg = MSG::default();
                        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                            if msg.message == WM_HOTKEY {
                                match msg.wParam.0 as i32 {
                                    ID_RECORD => on_action(HotKeyAction::ToggleRecord),
                                    ID_PAUSE => on_action(HotKeyAction::TogglePause),
                                    _ => {}
                                }
                            }
                        }
                        if status.record_registered {
                            let _ = UnregisterHotKey(None, ID_RECORD);
                        }
                        if status.pause_registered {
                            let _ = UnregisterHotKey(None, ID_PAUSE);
                        }
                    }
                })
                .ok();

            if let (Some(join), Ok((thread_id, status))) = (join, rx.recv()) {
                return Self {
                    thread_id,
                    join: Some(join),
                    status,
                };
            }
            Self {
                thread_id: 0,
                join: None,
                status: HotKeyRegistrationStatus::default(),
            }
        }
        #[cfg(not(windows))]
        {
            let _ = (record_spec, pause_spec, on_action);
            Self {
                status: HotKeyRegistrationStatus::default(),
            }
        }
    }

    pub fn stop(&mut self) {
        #[cfg(windows)]
        if let Some(join) = self.join.take() {
            if self.thread_id != 0 {
                // SAFETY: `WM_QUIT` cleanly terminates our hotkey pump loop.
                unsafe {
                    let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
                }
            }
            let _ = join.join();
        }
    }
}

impl Drop for GlobalHotKeyManager {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_standard_and_function_key_hotkeys() {
        assert_eq!(
            HotKeyChord::parse("Ctrl+Shift+R"),
            Some(HotKeyChord {
                ctrl: true,
                shift: true,
                alt: false,
                win: false,
                vk: b'R' as u32,
            })
        );
        assert_eq!(HotKeyChord::parse("Ctrl+Alt+F9").map(|c| c.vk), Some(0x78));
        // Bare letter without modifier is rejected so typing cannot trigger a global hotkey.
        assert_eq!(HotKeyChord::parse("R"), None);
        assert_eq!(HotKeyChord::parse(""), None);
    }
}
