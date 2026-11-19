//! Borderless always-on-top HUD overlay windows (`WDA_EXCLUDEFROMCAPTURE` + `WS_EX_TOOLWINDOW`).
//!
//! Manages the five shell overlays (Setup bar, Recording bar, Countdown, Area selector, Camera
//! bubble) and keeps a single authoritative list of every app window HWND so our own windows are
//! never captured in display or area recordings (ADR 0007).

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

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
            Self::SetupBar => (620.0, 74.0),
            Self::RecordingBar => (310.0, 58.0),
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

/// Tracks every owned HWND (main + HUDs) so `ScreenCaptureRecordingSession` excludes them all.
#[derive(Default)]
pub struct WindowRegistry {
    hwnds: Mutex<Vec<u64>>,
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
            let mut list = self.hwnds.lock();
            if !list.contains(&hwnd) {
                list.push(hwnd);
            }
        }
    }

    #[must_use]
    pub fn all_hwnds(&self) -> Vec<u64> {
        self.hwnds.lock().clone()
    }

    /// Re-applies `WDA_EXCLUDEFROMCAPTURE` to every registered window.
    pub fn refresh_all(&self) {
        for hwnd in self.all_hwnds() {
            let _ = win_shell::exclude_window_from_capture(hwnd);
        }
    }
}

/// Opens (or focuses if already open) an overlay HUD window and immediately excludes it from WGC.
pub fn open_overlay(
    app: &AppHandle,
    registry: &WindowRegistry,
    kind: OverlayKind,
    bounds: Option<(f64, f64, f64, f64)>,
) -> Result<(), String> {
    let label = kind.label();
    if let Some(existing) = app.get_webview_window(label) {
        if let Some((x, y, w, h)) = bounds {
            let _ = existing.set_position(tauri::LogicalPosition::new(x, y));
            let _ = existing.set_size(tauri::LogicalSize::new(w, h));
        }
        registry.register_window(&existing, true);
        let _ = existing.show();
        let _ = existing.set_focus();
        return Ok(());
    }

    let (default_w, default_h) = kind.default_size();
    let (x, y, w, h) = bounds.unwrap_or((120.0, 120.0, default_w, default_h));

    let mut builder =
        WebviewWindowBuilder::new(app, label, WebviewUrl::App(kind.hash_route().into()))
            .title(label)
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .resizable(matches!(kind, OverlayKind::CameraBubble))
            .shadow(false)
            .inner_size(w, h);

    if bounds.is_some() {
        builder = builder.position(x, y);
    } else if matches!(kind, OverlayKind::AreaSelection) {
        builder = builder.fullscreen(true);
    } else {
        builder = builder.center();
    }

    let win = builder
        .build()
        .map_err(|e| format!("Failed to create overlay window {label}: {e}"))?;
    registry.register_window(&win, true);
    Ok(())
}

pub fn close_overlay(app: &AppHandle, kind: OverlayKind) {
    if let Some(win) = app.get_webview_window(kind.label()) {
        let _ = win.close();
    }
}

pub fn close_all_overlays(app: &AppHandle) {
    for kind in OverlayKind::ALL {
        close_overlay(app, kind);
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
