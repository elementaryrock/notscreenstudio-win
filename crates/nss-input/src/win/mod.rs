//! Confined Win32 FFI adapters for `nss-input` (ADR 0012).
//!
//! Provides cursor position (`GetCursorPos`), monitor/window bounds for source normalization,
//! multimedia timer resolution (`timeBeginPeriod(1)` for accurate 120 Hz sampling), the
//! `WH_MOUSE_LL` click hook (`hook`), and `GetCursorInfo`/`GetIconInfoExW` cursor-shape
//! fingerprinting (`shapes`).

#![allow(unsafe_code)]

use nss_core::{Point2, Rect};
use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
};
use windows::Win32::Media::{timeBeginPeriod, timeEndPeriod};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetCursorPos, GetWindowRect, MONITORINFOF_PRIMARY,
};

pub mod hook;
pub mod shapes;

/// Current pointer location in virtual-desktop pixels (top-left primary origin, Y down).
#[must_use]
pub fn cursor_pos() -> Option<Point2> {
    let mut pt = POINT::default();
    // SAFETY: `pt` is a valid out-pointer to `POINT`.
    unsafe { GetCursorPos(&mut pt).ok()? };
    Some(Point2::new(f64::from(pt.x), f64::from(pt.y)))
}

/// Returns the virtual-desktop rect and DPI scale factor for `display_id` (lower 32 bits of
/// `HMONITOR`, matching `nss-capture`'s `Monitor.id`).
#[must_use]
pub fn display_rect_and_scale(display_id: u32) -> Option<(Rect, f64)> {
    struct Search {
        target_id: u32,
        found: Option<(Rect, f64)>,
    }

    unsafe extern "system" fn cb(
        hmonitor: HMONITOR,
        _hdc: HDC,
        _rect: *mut RECT,
        lparam: LPARAM,
    ) -> BOOL {
        let state = &mut *(lparam.0 as *mut Search);
        let id = (hmonitor.0 as usize & 0xFFFF_FFFF) as u32;
        if id != state.target_id {
            return BOOL(1);
        }
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(hmonitor, &mut info).as_bool() {
            let r = info.rcMonitor;
            let mut dpi_x: u32 = 96;
            let mut dpi_y: u32 = 96;
            if GetDpiForMonitor(hmonitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y).is_err()
                || dpi_x == 0
            {
                dpi_x = 96;
            }
            let rect = Rect::new(
                f64::from(r.left),
                f64::from(r.top),
                f64::from(r.right - r.left).max(0.0),
                f64::from(r.bottom - r.top).max(0.0),
            );
            state.found = Some((rect, f64::from(dpi_x) / 96.0));
        }
        BOOL(0)
    }

    let mut search = Search {
        target_id: display_id,
        found: None,
    };
    // SAFETY: `search` outlives the synchronous `EnumDisplayMonitors` call.
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(cb), LPARAM((&raw mut search) as isize));
    }
    search.found
}

/// Returns the primary display's `id` (`HMONITOR` lower 32 bits), if a desktop session exists.
#[must_use]
pub fn primary_display_id() -> Option<u32> {
    unsafe extern "system" fn cb(
        hmonitor: HMONITOR,
        _hdc: HDC,
        _rect: *mut RECT,
        lparam: LPARAM,
    ) -> BOOL {
        let out = &mut *(lparam.0 as *mut Option<u32>);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(hmonitor, &mut info).as_bool()
            && (info.dwFlags & MONITORINFOF_PRIMARY) != 0
        {
            *out = Some((hmonitor.0 as usize & 0xFFFF_FFFF) as u32);
            return BOOL(0);
        }
        BOOL(1)
    }

    let mut id: Option<u32> = None;
    // SAFETY: `id` outlives the synchronous `EnumDisplayMonitors` call.
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(cb), LPARAM((&raw mut id) as isize));
    }
    id
}

/// Resolves a `CaptureSource::Window { id }` (lower 32 bits of `HWND`) to its DWM extended frame
/// bounds in virtual-desktop pixels.
#[must_use]
pub fn window_rect(window_id: u32) -> Option<Rect> {
    struct Search {
        target_id: u32,
        found: Option<HWND>,
    }

    unsafe extern "system" fn cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let state = &mut *(lparam.0 as *mut Search);
        if (hwnd.0 as usize as u32) == state.target_id {
            state.found = Some(hwnd);
            return BOOL(0);
        }
        BOOL(1)
    }

    let mut search = Search {
        target_id: window_id,
        found: None,
    };
    // SAFETY: `search` outlives `EnumWindows`.
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM((&raw mut search) as isize));
    }
    let hwnd = search.found?;
    let mut r = RECT::default();
    // SAFETY: `r` is a valid out-pointer to `RECT`.
    unsafe {
        let ok = DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            (&raw mut r).cast(),
            std::mem::size_of::<RECT>() as u32,
        )
        .is_ok()
            || GetWindowRect(hwnd, &mut r).is_ok();
        if !ok {
            return None;
        }
    }
    let w = f64::from(r.right - r.left).max(0.0);
    let h = f64::from(r.bottom - r.top).max(0.0);
    (w > 0.0 && h > 0.0).then(|| Rect::new(f64::from(r.left), f64::from(r.top), w, h))
}

/// RAII guard requesting 1 ms Windows multimedia timer resolution so the 120 Hz sampling loop does
/// not quantize to the default 15.6 ms system tick.
pub struct TimerPeriodGuard {
    period_ms: u32,
    active: bool,
}

impl TimerPeriodGuard {
    #[must_use]
    pub fn request_1ms() -> Self {
        // SAFETY: paired with `timeEndPeriod(1)` in `Drop` when return value is 0 (`TIMERR_NOERROR`).
        let ok = unsafe { timeBeginPeriod(1) == 0 };
        Self {
            period_ms: 1,
            active: ok,
        }
    }
}

impl Drop for TimerPeriodGuard {
    fn drop(&mut self) {
        if self.active {
            // SAFETY: balanced with `timeBeginPeriod(self.period_ms)`.
            unsafe {
                let _ = timeEndPeriod(self.period_ms);
            }
        }
    }
}
