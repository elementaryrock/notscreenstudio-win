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
use windows::core::{BOOL, PCWSTR, PWSTR};
#[cfg(windows)]
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, HWND, LPARAM, RPC_E_CHANGED_MODE,
    WPARAM,
};
#[cfg(windows)]
use windows::Win32::System::Com::{
    CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
};
#[cfg(windows)]
use windows::Win32::System::Threading::{CreateMutexW, GetCurrentProcessId, GetCurrentThreadId};
#[cfg(windows)]
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
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
    EnumWindows, GetMessageW, GetWindowLongPtrW, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, IsIconic, MessageBoxW, PeekMessageW, PostThreadMessageW,
    SetForegroundWindow, SetWindowDisplayAffinity, SetWindowLongPtrW, ShowWindow, GWL_EXSTYLE,
    MB_ICONERROR, MB_OK, MSG, PM_NOREMOVE, SW_RESTORE, SW_SHOW, WDA_EXCLUDEFROMCAPTURE, WM_HOTKEY,
    WM_QUIT, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
};

/// Ensures the process runs under Per-Monitor DPI Awareness v2 before any monitor enumeration or
/// window placement occurs, so Win32 monitor bounds always report true physical pixels.
pub fn enable_per_monitor_dpi_awareness() {
    #[cfg(windows)]
    {
        // SAFETY: process-wide DPI awareness API taking a documented pseudo-handle constant;
        // fails harmlessly with E_ACCESSDENIED if already configured by manifest/runtime.
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }
    }
}

/// Process-lifetime Win32 named mutex guard ensuring only one instance of the app holds the
/// shared WebView2 profile and global hotkeys at a time.
pub struct SingleInstanceGuard {
    #[cfg(windows)]
    handle: HANDLE,
}

impl SingleInstanceGuard {
    /// Attempts to claim the named mutex. Returns `None` if another instance already holds it.
    #[must_use]
    pub fn acquire(mutex_name: &str) -> Option<Self> {
        #[cfg(windows)]
        {
            let wide: Vec<u16> = mutex_name
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            // SAFETY: `wide` is null-terminated; returned handle is checked and closed on Drop.
            unsafe {
                match CreateMutexW(None, false, PCWSTR(wide.as_ptr())) {
                    Ok(handle) => {
                        if GetLastError() == ERROR_ALREADY_EXISTS {
                            let _ = CloseHandle(handle);
                            return None;
                        }
                        Some(Self { handle })
                    }
                    // Fail open if mutex creation is unexpectedly denied so startup is never blocked.
                    Err(_) => Some(Self {
                        handle: HANDLE::default(),
                    }),
                }
            }
        }
        #[cfg(not(windows))]
        {
            let _ = mutex_name;
            Some(Self {})
        }
    }
}

impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        #[cfg(windows)]
        if !self.handle.0.is_null() {
            // SAFETY: handle was acquired by `CreateMutexW` in `SingleInstanceGuard::acquire`.
            unsafe {
                let _ = CloseHandle(self.handle);
            }
        }
    }
}

/// Restores and focuses an existing top-level window with exact title `expected_title` belonging
/// to another process (including windows hidden to the system tray).
pub fn focus_existing_main_window(expected_title: &str) -> bool {
    #[cfg(windows)]
    {
        struct SearchCtx<'a> {
            expected_title: &'a str,
            own_pid: u32,
            found: Option<HWND>,
        }

        unsafe extern "system" fn callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
            let ctx = &mut *(lparam.0 as *mut SearchCtx<'_>);
            let mut pid: u32 = 0;
            let _ = GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid == 0 || pid == ctx.own_pid {
                return BOOL(1);
            }
            let len = GetWindowTextLengthW(hwnd);
            if len <= 0 {
                return BOOL(1);
            }
            let mut buf = vec![0u16; (len as usize) + 1];
            let copied = GetWindowTextW(hwnd, &mut buf);
            if copied <= 0 {
                return BOOL(1);
            }
            let title = String::from_utf16_lossy(&buf[..copied as usize]);
            if title == ctx.expected_title {
                ctx.found = Some(hwnd);
                return BOOL(0);
            }
            BOOL(1)
        }

        // SAFETY: `ctx` lives for the entire synchronous `EnumWindows` call; `ShowWindow` and
        // `SetForegroundWindow` operate on the matched HWND.
        unsafe {
            let mut ctx = SearchCtx {
                expected_title,
                own_pid: GetCurrentProcessId(),
                found: None,
            };
            let _ = EnumWindows(Some(callback), LPARAM((&raw mut ctx) as isize));
            if let Some(hwnd) = ctx.found {
                let _ = ShowWindow(hwnd, SW_SHOW);
                if IsIconic(hwnd).as_bool() {
                    let _ = ShowWindow(hwnd, SW_RESTORE);
                }
                let _ = SetForegroundWindow(hwnd);
                return true;
            }
        }
        false
    }
    #[cfg(not(windows))]
    {
        let _ = expected_title;
        false
    }
}

/// Displays a native Windows error dialog for fatal startup errors when running under
/// `windows_subsystem = "windows"` (where stderr is not attached to a console).
pub fn show_error_dialog(title: &str, message: &str) {
    #[cfg(windows)]
    {
        let title_w: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
        let msg_w: Vec<u16> = message.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: both buffers are valid null-terminated UTF-16 strings for the call's duration.
        unsafe {
            let _ = MessageBoxW(
                None,
                PCWSTR(msg_w.as_ptr()),
                PCWSTR(title_w.as_ptr()),
                MB_OK | MB_ICONERROR,
            );
        }
    }
    #[cfg(not(windows))]
    {
        eprintln!("{title}: {message}");
    }
}

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
        let normalized = path.to_string_lossy().replace('/', "\\");
        let cleaned = normalized
            .strip_prefix(r"\\?\UNC\")
            .map(|u| format!(r"\\{u}"))
            .or_else(|| normalized.strip_prefix(r"\\?\").map(str::to_owned))
            .unwrap_or(normalized);
        let wide: Vec<u16> = cleaned.encode_utf16().chain(std::iter::once(0)).collect();
        let display = path.display().to_string();

        let run_sta = move || -> Result<(), String> {
            // SAFETY: STA COM initialized/uninitialized locally; `ILCreateFromPathW` freed via `ILFree`.
            unsafe {
                let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
                let should_uninit = hr.is_ok();
                let pidl = ILCreateFromPathW(PCWSTR(wide.as_ptr()));
                let result = if !pidl.is_null() {
                    let res = SHOpenFolderAndSelectItems(pidl, None, 0);
                    ILFree(Some(pidl));
                    res.map_err(|e| format!("Failed to open Explorer: {e}"))
                } else {
                    Err(format!("Path does not exist: {display}"))
                };
                if should_uninit {
                    CoUninitialize();
                }
                result
            }
        };

        // SAFETY: probe COM apartment mode; hop to a dedicated STA thread if current thread is MTA.
        let probe =
            unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
        if probe == RPC_E_CHANGED_MODE {
            return thread::spawn(run_sta)
                .join()
                .unwrap_or_else(|_| Err("Explorer helper panicked.".into()));
        }
        if probe.is_ok() {
            // SAFETY: balanced with `probe` above.
            unsafe { CoUninitialize() };
        }
        run_sta()
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

/// Marks a HUD overlay window with `WS_EX_TOOLWINDOW` (and clears `WS_EX_APPWINDOW`) so it stays
/// out of Alt+Tab and the taskbar.
pub fn mark_tool_window(hwnd: u64) {
    #[cfg(windows)]
    {
        if hwnd == 0 {
            return;
        }
        let handle = HWND(hwnd as usize as *mut std::ffi::c_void);
        // SAFETY: standard extended window style bit manipulation on an owned HWND.
        unsafe {
            let current = GetWindowLongPtrW(handle, GWL_EXSTYLE);
            let next = (current | (WS_EX_TOOLWINDOW.0 as isize)) & !(WS_EX_APPWINDOW.0 as isize);
            let _ = SetWindowLongPtrW(handle, GWL_EXSTYLE, next);
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
                        } else if !record_label.trim().is_empty() {
                            status.record_conflict = Some(format!(
                                "Invalid shortcut \"{record_label}\". Include Ctrl, Alt, or Win plus a key."
                            ));
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
                        } else if !pause_label.trim().is_empty() {
                            status.pause_conflict = Some(format!(
                                "Invalid shortcut \"{pause_label}\". Include Ctrl, Alt, or Win plus a key."
                            ));
                        }
                        // Ensure the Win32 message queue exists before publishing `tid` so an
                        // immediate `PostThreadMessageW(WM_QUIT)` on stop/restart is never lost.
                        let mut msg = MSG::default();
                        let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
                        let _ = tx.send((tid, status.clone()));

                        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
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

    #[test]
    fn single_instance_guard_blocks_duplicate_until_dropped() {
        let name = format!("Local\\nss-test-single-instance-{}", std::process::id());
        let first = SingleInstanceGuard::acquire(&name);
        assert!(first.is_some());
        #[cfg(windows)]
        {
            assert!(SingleInstanceGuard::acquire(&name).is_none());
        }
        drop(first);
        let after_drop = SingleInstanceGuard::acquire(&name);
        assert!(after_drop.is_some());
    }
}
