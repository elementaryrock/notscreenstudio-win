//! Borderless always-on-top HUD overlay windows (`WDA_EXCLUDEFROMCAPTURE` + `WS_EX_TOOLWINDOW`).
//!
//! Manages the five shell overlays (Setup bar, Recording bar, Countdown, Area selector, Camera
//! bubble) and keeps a single authoritative list of every app window HWND so our own windows are
//! never captured in display or area recordings (ADR 0007).

use std::collections::HashMap;

use nss_core::{CaptureSource, Size2};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};

use crate::shell_model::BarPlacement;
use crate::win_shell;

/// Overlay window roles supported by the shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OverlayKind {
    SetupBar,
    RecordingBar,
    Countdown,
    AreaSelection,
    CameraBubble,
}

impl OverlayKind {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::SetupBar => "hud-setup",
            Self::RecordingBar => "hud-recording",
            Self::Countdown => "hud-countdown",
            Self::AreaSelection => "hud-area",
            Self::CameraBubble => "hud-camera-bubble",
        }
    }

    #[must_use]
    pub const fn hash_route(self) -> &'static str {
        match self {
            Self::SetupBar => "index.html#hud-setup",
            Self::RecordingBar => "index.html#hud-recording",
            Self::Countdown => "index.html#hud-countdown",
            Self::AreaSelection => "index.html#hud-area",
            Self::CameraBubble => "index.html#hud-camera-bubble",
        }
    }

    #[must_use]
    pub const fn default_size(self) -> (f64, f64) {
        match self {
            Self::SetupBar => (680.0, 84.0),
            Self::RecordingBar => (340.0, 68.0),
            Self::Countdown => (260.0, 260.0),
            Self::AreaSelection => (1920.0, 1080.0),
            Self::CameraBubble => (220.0, 220.0),
        }
    }

    pub const ALL: [OverlayKind; 5] = [
        Self::SetupBar,
        Self::RecordingBar,
        Self::Countdown,
        Self::AreaSelection,
        Self::CameraBubble,
    ];
}

/// Tracks every live owned HWND (main + HUDs) keyed by window label so closed/recycled HWNDs
/// never linger and `ScreenCaptureRecordingSession` excludes every live app window (ADR 0007).
#[derive(Default)]
pub struct WindowRegistry {
    hwnds: Mutex<HashMap<String, u64>>,
}

impl WindowRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_window(&self, window: &WebviewWindow, is_tool_hud: bool) {
        if let Some(hwnd) = window_hwnd(window) {
            let _ = win_shell::exclude_window_from_capture(hwnd);
            if is_tool_hud {
                win_shell::mark_tool_window(hwnd);
            }
            self.hwnds.lock().insert(window.label().to_owned(), hwnd);
        }
    }

    pub fn unregister_label(&self, label: &str) {
        self.hwnds.lock().remove(label);
    }

    #[must_use]
    pub fn all_hwnds(&self) -> Vec<u64> {
        let mut out: Vec<u64> = self.hwnds.lock().values().copied().collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Re-applies `WDA_EXCLUDEFROMCAPTURE` to every registered window.
    pub fn refresh_all(&self) {
        for hwnd in self.all_hwnds() {
            let _ = win_shell::exclude_window_from_capture(hwnd);
        }
    }
}

/// Computes physical-pixel `(x, y, width, height)` for an overlay on the monitor associated with
/// `source` (or the primary display if `source` is `None` or no longer connected).
#[cfg(windows)]
fn default_physical_bounds(
    kind: OverlayKind,
    source: Option<CaptureSource>,
) -> Option<(i32, i32, u32, u32)> {
    let monitors = nss_capture::win::enumerate::enumerate_monitors();
    if monitors.is_empty() {
        return None;
    }
    let target_display_id = match source {
        Some(CaptureSource::Display { id }) => Some(id),
        Some(CaptureSource::Area { display_id, .. }) => Some(display_id),
        Some(CaptureSource::Window { id }) => {
            let windows = nss_capture::win::enumerate::enumerate_windows();
            windows
                .iter()
                .find(|w| (w.handle as u32) == id)
                .and_then(|w| {
                    let bounds = nss_capture::win::enumerate::window_bounds(w.handle)?;
                    let rects: Vec<nss_core::Rect> =
                        monitors.iter().map(|m| m.monitor.rect).collect();
                    let idx = BarPlacement::screen_index(Some(bounds), &rects);
                    monitors.get(idx).map(|m| m.monitor.id)
                })
        }
        None => None,
    };
    let mon = target_display_id
        .and_then(|id| monitors.iter().find(|m| m.monitor.id == id))
        .or_else(|| monitors.iter().find(|m| m.monitor.is_primary))
        .or_else(|| monitors.first())?;
    let m = &mon.monitor;
    let scale = m.scale();
    let (dw, dh) = kind.default_size();

    match kind {
        OverlayKind::SetupBar | OverlayKind::RecordingBar => {
            let f = BarPlacement::frame(Size2::new(dw * scale, dh * scale), m.work_rect);
            Some((
                f.min_x().round() as i32,
                f.min_y().round() as i32,
                f.width().round().max(1.0) as u32,
                f.height().round().max(1.0) as u32,
            ))
        }
        OverlayKind::Countdown => {
            let pw = (dw * scale).round();
            let ph = (dh * scale).round();
            let x = m.rect.min_x() + (m.rect.width() - pw) * 0.5;
            let y = m.rect.min_y() + (m.rect.height() - ph) * 0.5;
            Some((
                x.round() as i32,
                y.round() as i32,
                pw.max(1.0) as u32,
                ph.max(1.0) as u32,
            ))
        }
        OverlayKind::AreaSelection => Some((
            m.rect.min_x().round() as i32,
            m.rect.min_y().round() as i32,
            m.rect.width().round().max(1.0) as u32,
            m.rect.height().round().max(1.0) as u32,
        )),
        OverlayKind::CameraBubble => {
            let pw = (dw * scale).round();
            let ph = (dh * scale).round();
            let margin = (24.0 * scale).round();
            let x = (m.work_rect.max_x() - pw - margin).max(m.work_rect.min_x());
            let y = (m.work_rect.max_y() - ph - margin).max(m.work_rect.min_y());
            Some((
                x.round() as i32,
                y.round() as i32,
                pw.max(1.0) as u32,
                ph.max(1.0) as u32,
            ))
        }
    }
}

#[cfg(not(windows))]
fn default_physical_bounds(
    _kind: OverlayKind,
    _source: Option<CaptureSource>,
) -> Option<(i32, i32, u32, u32)> {
    None
}

/// Opens (or repositions/focuses if already open) an overlay HUD window and applies
/// `WDA_EXCLUDEFROMCAPTURE` + `WS_EX_TOOLWINDOW` **before** showing it so it never flashes into
/// recordings or Alt+Tab.
///
/// MUST only be called from an async command or background thread (never a synchronous Tauri
/// command on Windows, where `WebviewWindowBuilder::build` would deadlock WebView2).
pub fn open_overlay_for_source(
    app: &AppHandle,
    registry: &WindowRegistry,
    kind: OverlayKind,
    bounds: Option<(f64, f64, f64, f64)>,
    source: Option<CaptureSource>,
) -> Result<(), String> {
    let label = kind.label();
    let phys_bounds = if let Some((x, y, w, h)) = bounds {
        Some((
            x.round() as i32,
            y.round() as i32,
            w.round().max(1.0) as u32,
            h.round().max(1.0) as u32,
        ))
    } else {
        default_physical_bounds(kind, source)
    };

    if let Some(existing) = app.get_webview_window(label) {
        if let Some((px, py, pw, ph)) = phys_bounds {
            let _ = existing.set_size(PhysicalSize::new(pw, ph));
            let _ = existing.set_position(PhysicalPosition::new(px, py));
        }
        registry.register_window(&existing, true);
        let _ = existing.show();
        let _ = existing.set_focus();
        return Ok(());
    }

    let (default_w, default_h) = kind.default_size();
    let route = match (kind, source) {
        (OverlayKind::AreaSelection, Some(CaptureSource::Display { id }))
        | (OverlayKind::AreaSelection, Some(CaptureSource::Area { display_id: id, .. })) => {
            format!("index.html#hud-area?displayId={id}")
        }
        _ => kind.hash_route().to_owned(),
    };

    // Start hidden so WDA_EXCLUDEFROMCAPTURE + WS_EX_TOOLWINDOW + physical monitor bounds are all
    // applied before the window is ever presented on screen.
    let mut builder = WebviewWindowBuilder::new(app, label, WebviewUrl::App(route.into()))
        .title(label)
        .visible(false)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(matches!(kind, OverlayKind::CameraBubble))
        .shadow(false)
        .inner_size(default_w, default_h);

    if phys_bounds.is_none() && !matches!(kind, OverlayKind::AreaSelection) {
        builder = builder.center();
    } else if phys_bounds.is_none() && matches!(kind, OverlayKind::AreaSelection) {
        builder = builder.fullscreen(true);
    }

    let win = builder
        .build()
        .map_err(|e| format!("Failed to create overlay window {label}: {e}"))?;
    registry.register_window(&win, true);
    if let Some((px, py, pw, ph)) = phys_bounds {
        let _ = win.set_size(PhysicalSize::new(pw, ph));
        let _ = win.set_position(PhysicalPosition::new(px, py));
    }
    let _ = win.show();
    if matches!(kind, OverlayKind::AreaSelection | OverlayKind::SetupBar) {
        let _ = win.set_focus();
    }
    Ok(())
}

pub fn close_overlay(app: &AppHandle, registry: &WindowRegistry, kind: OverlayKind) {
    let label = kind.label();
    registry.unregister_label(label);
    if let Some(win) = app.get_webview_window(label) {
        let _ = win.close();
    }
}

pub fn close_all_overlays(app: &AppHandle, registry: &WindowRegistry) {
    for kind in OverlayKind::ALL {
        close_overlay(app, registry, kind);
    }
}

#[cfg(windows)]
fn window_hwnd(window: &WebviewWindow) -> Option<u64> {
    window.hwnd().ok().map(|h| h.0 as usize as u64)
}

#[cfg(not(windows))]
fn window_hwnd(_window: &WebviewWindow) -> Option<u64> {
    None
}
