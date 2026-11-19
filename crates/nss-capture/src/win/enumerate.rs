//! Win32 display and window enumeration.
//!
//! Confined `unsafe` boundary (ADR 0012): copies raw Win32 structs from `EnumDisplayMonitors`,
//! `GetDpiForMonitor`, `EnumWindows`, `DwmGetWindowAttribute` and `QueryFullProcessImageNameW`
//! into the plain value types in [`super`].

#![allow(unsafe_code)]

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::Path;

use nss_core::Rect;
use windows::core::BOOL;
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{
    DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS,
};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW,
};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowLongPtrW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, IsIconic, IsWindowVisible, GWL_EXSTYLE, MONITORINFOF_PRIMARY,
    WS_EX_TOOLWINDOW,
};

use super::{Monitor, WindowInfo};

/// A monitor paired with its raw Win32 `HMONITOR` value so WGC can call `CreateForMonitor`
/// without relying on handle truncation.
#[derive(Debug, Clone, PartialEq)]
pub struct EnumeratedMonitor {
    pub monitor: Monitor,
    pub hmonitor: isize,
}

/// Enumerates every active display monitor on the virtual desktop.
#[must_use]
pub fn enumerate_monitors() -> Vec<EnumeratedMonitor> {
    let mut results: Vec<EnumeratedMonitor> = Vec::new();

    unsafe extern "system" fn callback(
        hmonitor: HMONITOR,
        _hdc: HDC,
        _rect: *mut RECT,
        lparam: LPARAM,
    ) -> BOOL {
        let out = &mut *(lparam.0 as *mut Vec<EnumeratedMonitor>);
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if !GetMonitorInfoW(hmonitor, (&raw mut info.monitorInfo)).as_bool() {
            return BOOL(1);
        }

        let rc = info.monitorInfo.rcMonitor;
        let work = info.monitorInfo.rcWork;
        let mut dpi_x: u32 = 96;
        let mut dpi_y: u32 = 96;
        if GetDpiForMonitor(hmonitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y).is_err()
            || dpi_x == 0
        {
            dpi_x = 96;
        }

        let name_len = info
            .szDevice
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(info.szDevice.len());
        let device_name = OsString::from_wide(&info.szDevice[..name_len])
            .to_string_lossy()
            .into_owned();

        let hmon_raw = hmonitor.0 as isize;
        let id = (hmon_raw as usize & 0xFFFF_FFFF) as u32;
        let is_primary = (info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY) != 0;

        out.push(EnumeratedMonitor {
            monitor: Monitor {
                id: if id == 0 { (out.len() + 1) as u32 } else { id },
                rect: rect_from_win32(rc),
                work_rect: rect_from_win32(work),
                dpi: dpi_x,
                is_primary,
                device_name,
            },
            hmonitor: hmon_raw,
        });
        BOOL(1)
    }

    // SAFETY: `results` outlives the synchronous `EnumDisplayMonitors` call and is only mutated
    // on the current thread inside `callback`.
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(callback),
            LPARAM((&raw mut results) as isize),
        );
    }

    results
}

/// Resolves a `Monitor.id` back to its live `HMONITOR` handle.
#[must_use]
pub fn find_hmonitor(display_id: u32) -> Option<EnumeratedMonitor> {
    enumerate_monitors()
        .into_iter()
        .find(|m| m.monitor.id == display_id)
}

/// Enumerates every top-level window on the desktop along with DWM bounds, cloaking state,
/// tool-window flag, owning PID and executable file name.
#[must_use]
pub fn enumerate_windows() -> Vec<WindowInfo> {
    let mut results: Vec<WindowInfo> = Vec::new();

    unsafe extern "system" fn callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let out = &mut *(lparam.0 as *mut Vec<WindowInfo>);

        let visible = IsWindowVisible(hwnd).as_bool() && !IsIconic(hwnd).as_bool();
        let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let is_tool = (ex_style & WS_EX_TOOLWINDOW.0) != 0;

        let mut cloaked: u32 = 0;
        let is_cloaked = DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            (&raw mut cloaked).cast(),
            std::mem::size_of::<u32>() as u32,
        )
        .is_ok()
            && cloaked != 0;

        let mut rect = RECT::default();
        let got_dwm = DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            (&raw mut rect).cast(),
            std::mem::size_of::<RECT>() as u32,
        )
        .is_ok();
        if !got_dwm && GetWindowRect(hwnd, &mut rect).is_err() {
            return BOOL(1);
        }

        let width = f64::from(rect.right - rect.left).max(0.0);
        let height = f64::from(rect.bottom - rect.top).max(0.0);

        let mut pid: u32 = 0;
        let _ = GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let app_name = if pid != 0 {
            process_image_name(pid)
        } else {
            None
        };

        let title = window_title(hwnd);
        let handle = hwnd.0 as usize as u64;

        out.push(WindowInfo {
            handle,
            process_id: pid,
            app_name,
            title,
            is_on_screen: visible,
            is_cloaked,
            is_tool,
            width,
            height,
        });
        BOOL(1)
    }

    // SAFETY: `results` outlives the synchronous `EnumWindows` call and is only mutated on this
    // thread inside `callback`.
    unsafe {
        let _ = EnumWindows(Some(callback), LPARAM((&raw mut results) as isize));
    }

    results
}

/// Returns the DWM extended frame bounds (falling back to `GetWindowRect`) for a single HWND.
#[must_use]
pub fn window_bounds(hwnd_raw: u64) -> Option<Rect> {
    let hwnd = HWND(hwnd_raw as usize as *mut std::ffi::c_void);
    let mut rect = RECT::default();
    // SAFETY: `rect` is a valid out-pointer of size `size_of::<RECT>()`.
    unsafe {
        let ok = DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            (&raw mut rect).cast(),
            std::mem::size_of::<RECT>() as u32,
        )
        .is_ok()
            || GetWindowRect(hwnd, &mut rect).is_ok();
        ok.then(|| rect_from_win32(rect))
    }
}

/// Returns the executable file name (e.g. `notscreenstudio.exe`) for a process ID, or `None` if
/// the process cannot be opened (elevated / protected system process).
#[must_use]
pub fn process_image_name(pid: u32) -> Option<String> {
    // SAFETY: handle is checked and closed before returning.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 512];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
        .is_ok();
        let _ = CloseHandle(handle);
        if !ok || len == 0 {
            return None;
        }
        let full = OsString::from_wide(&buf[..len as usize])
            .to_string_lossy()
            .into_owned();
        Path::new(&full)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
    }
}

/// Returns the current process's executable file name, used to identify our own windows.
#[must_use]
pub fn current_app_name() -> Option<String> {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
}

fn window_title(hwnd: HWND) -> String {
    // SAFETY: `GetWindowTextLengthW` and `GetWindowTextW` operate on a stack/heap buffer whose
    // capacity is passed explicitly.
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; (len as usize) + 1];
        let copied = GetWindowTextW(hwnd, &mut buf);
        if copied <= 0 {
            return String::new();
        }
        OsString::from_wide(&buf[..copied as usize])
            .to_string_lossy()
            .into_owned()
    }
}

fn rect_from_win32(r: RECT) -> Rect {
    Rect::new(
        f64::from(r.left),
        f64::from(r.top),
        f64::from(r.right - r.left).max(0.0),
        f64::from(r.bottom - r.top).max(0.0),
    )
}
