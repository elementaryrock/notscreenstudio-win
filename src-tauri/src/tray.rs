//! System tray icon (idle / recording / paused states) and context menu (PLAN.md §4.9 & §5 Stage 8).
//!
//! Renders three crisp 32x32 RGBA tray icons once at startup and swaps them on state transitions,
//! with a 1 s elapsed-time tooltip while recording.

use tauri::image::Image as TauriImage;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager};

use crate::shell_model::{format_elapsed, ShellPhase};

pub const TRAY_ID: &str = "nss-tray";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayVisual {
    Idle,
    Recording,
    Paused,
}

/// Generates a 32x32 RGBA tray icon for the requested visual state with anti-aliased SDF edges.
#[must_use]
pub fn render_tray_icon(state: TrayVisual) -> TauriImage<'static> {
    const SIZE: usize = 32;
    let mut rgba = vec![0u8; SIZE * SIZE * 4];
    let center = (SIZE as f32) * 0.5;
    let outer_r = 13.5f32;
    let inner_r = 6.0f32;

    for y in 0..SIZE {
        for x in 0..SIZE {
            let px = x as f32 + 0.5 - center;
            let py = y as f32 + 0.5 - center;
            let dist = (px * px + py * py).sqrt();

            // Outer dark studio badge with subtle ring.
            let outer_alpha = (outer_r + 0.7 - dist).clamp(0.0, 1.0);
            if outer_alpha <= 0.0 {
                continue;
            }
            let ring = (dist - (outer_r - 2.0)).clamp(0.0, 1.0);
            let mut r = 24.0 + ring * 38.0;
            let mut g = 25.0 + ring * 40.0;
            let mut b = 30.0 + ring * 45.0;
            let a = outer_alpha;

            match state {
                TrayVisual::Idle => {
                    // Crisp white lens dot.
                    let dot = (inner_r + 0.6 - dist).clamp(0.0, 1.0);
                    r = r * (1.0 - dot) + 236.0 * dot;
                    g = g * (1.0 - dot) + 238.0 * dot;
                    b = b * (1.0 - dot) + 244.0 * dot;
                }
                TrayVisual::Recording => {
                    // Bright crimson recording dot.
                    let dot = (inner_r + 1.2 - dist).clamp(0.0, 1.0);
                    r = r * (1.0 - dot) + 239.0 * dot;
                    g = g * (1.0 - dot) + 68.0 * dot;
                    b = b * (1.0 - dot) + 68.0 * dot;
                }
                TrayVisual::Paused => {
                    // Amber pause bars.
                    let in_y = py.abs() <= 5.5;
                    let in_bars =
                        in_y && ((-4.8..=-1.6).contains(&px) || (1.6..=4.8).contains(&px));
                    if in_bars {
                        r = 245.0;
                        g = 178.0;
                        b = 36.0;
                    }
                }
            }

            let off = (y * SIZE + x) * 4;
            rgba[off] = r.clamp(0.0, 255.0) as u8;
            rgba[off + 1] = g.clamp(0.0, 255.0) as u8;
            rgba[off + 2] = b.clamp(0.0, 255.0) as u8;
            rgba[off + 3] = (a * 255.0).clamp(0.0, 255.0) as u8;
        }
    }

    TauriImage::new_owned(rgba, SIZE as u32, SIZE as u32)
}

/// Builds and attaches the application's system tray icon and menu.
pub fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let record_item = MenuItem::with_id(
        app,
        "tray_record",
        "New Recording (Ctrl+Shift+R)",
        true,
        None::<&str>,
    )?;
    let pause_item = MenuItem::with_id(
        app,
        "tray_pause",
        "Pause / Resume (Ctrl+Shift+P)",
        true,
        None::<&str>,
    )?;
    let stop_item = MenuItem::with_id(app, "tray_stop", "Stop Recording", true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let open_item = MenuItem::with_id(
        app,
        "tray_open",
        "Open Recordings Library",
        true,
        None::<&str>,
    )?;
    let settings_item = MenuItem::with_id(
        app,
        "tray_settings",
        "Permissions & Settings",
        true,
        None::<&str>,
    )?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let quit_item =
        MenuItem::with_id(app, "tray_quit", "Quit notscreenstudio", true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[
            &record_item,
            &pause_item,
            &stop_item,
            &sep1,
            &open_item,
            &settings_item,
            &sep2,
            &quit_item,
        ],
    )?;

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(render_tray_icon(TrayVisual::Idle))
        .tooltip("notscreenstudio — Ready")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "tray_record" => {
                let _ = app.emit("nss://tray-action", "record");
            }
            "tray_pause" => {
                let _ = app.emit("nss://tray-action", "pause");
            }
            "tray_stop" => {
                let _ = app.emit("nss://tray-action", "stop");
            }
            "tray_open" => {
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.show();
                    let _ = win.set_focus();
                }
                let _ = app.emit("nss://tray-action", "open");
            }
            "tray_settings" => {
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.show();
                    let _ = win.set_focus();
                }
                let _ = app.emit("nss://tray-action", "settings");
            }
            "tray_quit" => {
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.show();
                    let _ = win.set_focus();
                }
            }
        })
        .build(app)?;

    Ok(())
}

/// Updates the tray icon image and tooltip text to reflect the live shell phase and elapsed time.
pub fn update_tray(app: &AppHandle, phase: &ShellPhase, paused: bool, elapsed_secs: f64) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let (visual, tooltip) = match phase {
        ShellPhase::Recording { .. } if paused => (
            TrayVisual::Paused,
            format!(
                "notscreenstudio — Paused ({})",
                format_elapsed(elapsed_secs)
            ),
        ),
        ShellPhase::Recording { .. } => (
            TrayVisual::Recording,
            format!(
                "notscreenstudio — Recording {}",
                format_elapsed(elapsed_secs)
            ),
        ),
        ShellPhase::Preparing => (
            TrayVisual::Recording,
            "notscreenstudio — Preparing...".to_owned(),
        ),
        ShellPhase::Saving => (TrayVisual::Idle, "notscreenstudio — Saving...".to_owned()),
        _ => (TrayVisual::Idle, "notscreenstudio — Ready".to_owned()),
    };
    let _ = tray.set_icon(Some(render_tray_icon(visual)));
    let _ = tray.set_tooltip(Some(tooltip));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_icons_render_distinct_states_at_32x32() {
        let idle = render_tray_icon(TrayVisual::Idle);
        let rec = render_tray_icon(TrayVisual::Recording);
        let paused = render_tray_icon(TrayVisual::Paused);
        assert_eq!((idle.width(), idle.height()), (32, 32));
        assert_ne!(idle.rgba(), rec.rgba());
        assert_ne!(rec.rgba(), paused.rgba());
    }
}
