//! notscreenstudio for Windows — application entry point.
//!
//! Replaces upstream `Sources/NotScreenStudio/App.swift`. Wires the Tauri runtime, tray icon,
//! global hotkeys, capture-excluded HUD windows, live wgpu preview engine, and IPC command surface.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![deny(unsafe_code)]

// Prevent ever shipping a release binary that points at `http://localhost:1420` instead of
// embedding `ui/dist` via Tauri's `custom-protocol` feature.
#[cfg(all(not(debug_assertions), dev))]
compile_error!(
    "Release builds of notscreenstudio must enable `custom-protocol` (run `tauri build` or \
     `cargo build --release -p notscreenstudio-win-app --features custom-protocol`) so the UI is \
     embedded inside the standalone EXE."
);

mod app_model;
mod overlay;
mod preview;
mod settings;
mod shell_model;
mod tray;
mod win_shell;

use std::path::PathBuf;

use app_model::AppState;
use tauri::{Manager, RunEvent, WindowEvent};

fn error_log_path() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("notscreenstudio")
        .join("last-error.log")
}

fn record_fatal_error(detail: &str, show_dialog: bool) {
    let log_path = error_log_path();
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&log_path, detail);
    eprintln!("notscreenstudio fatal error: {detail}");

    if show_dialog && std::env::var_os("NSS_NO_ERROR_DIALOG").is_none() {
        let hint = if detail.contains("WebView2") || detail.contains("0x80070002") {
            "\n\nPlease make sure Microsoft Edge WebView2 Runtime is installed and no other copy of notscreenstudio is locked in the background."
        } else {
            ""
        };
        let msg = format!(
            "notscreenstudio could not start.\n\n{detail}{hint}\n\nDetails were saved to:\n{}",
            log_path.display()
        );
        win_shell::show_error_dialog("notscreenstudio — Startup Error", &msg);
    }
}

fn configure_dev_webview_profile() {
    #[cfg(dev)]
    if std::env::var_os("WEBVIEW2_USER_DATA_FOLDER").is_none() {
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            let dev_dir = PathBuf::from(local)
                .join("dev.notscreenstudio.app")
                .join("EBWebView-dev");
            // Called once on the main thread before any worker threads or WebView2 start.
            std::env::set_var("WEBVIEW2_USER_DATA_FOLDER", dev_dir);
        }
    }
}

fn main() {
    win_shell::enable_per_monitor_dpi_awareness();

    // Headless smoke test (`--smoke-test`): exercises core, motion, project bundle I/O, and
    // library listing without creating any GUI window, so CI builds can always verify the graph.
    if std::env::args().any(|a| a == "--smoke-test") {
        match run_smoke_test() {
            Ok(summary) => {
                println!("smoke-test OK: {summary}");
                return;
            }
            Err(e) => {
                eprintln!("smoke-test FAILED: {e}");
                std::process::exit(1);
            }
        }
    }

    configure_dev_webview_profile();

    #[cfg(dev)]
    const INSTANCE_MUTEX: &str = "Local\\notscreenstudio-dev-instance-v1";
    #[cfg(not(dev))]
    const INSTANCE_MUTEX: &str = "Local\\notscreenstudio-app-instance-v1";

    let Some(_instance_guard) = win_shell::SingleInstanceGuard::acquire(INSTANCE_MUTEX) else {
        win_shell::focus_existing_main_window("notscreenstudio");
        return;
    };

    #[cfg(not(debug_assertions))]
    {
        let prev_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            record_fatal_error(&info.to_string(), true);
            prev_hook(info);
        }));
    }

    let state = AppState::new();

    let app = match tauri::Builder::default()
        .manage(state)
        .setup(|app| {
            let handle = app.handle().clone();
            let app_state = handle.state::<AppState>();
            if let Some(main_win) = handle.get_webview_window("main") {
                // ADR 0007: exclude the main app window from capture so recordings of the display
                // never include the recorder's own UI.
                app_state.windows.register_window(&main_win, false);
            }
            let _ = tray::setup_tray(&handle);
            app_state.install_hotkeys(handle.clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_model::ping,
            app_model::core_info,
            app_model::get_shell_snapshot,
            app_model::get_settings,
            app_model::save_settings,
            app_model::list_capture_sources,
            app_model::list_devices,
            app_model::get_permissions,
            app_model::request_borderless_access,
            app_model::check_ffmpeg,
            app_model::list_projects,
            app_model::get_project_thumbnail,
            app_model::rename_project,
            app_model::delete_project,
            app_model::reveal_path,
            app_model::create_demo_project,
            app_model::start_recording,
            app_model::cancel_countdown,
            app_model::pause_recording,
            app_model::resume_recording,
            app_model::stop_recording,
            app_model::discard_recording,
            app_model::dismiss_error,
            app_model::open_overlay,
            app_model::close_overlay,
            app_model::close_all_overlays,
            app_model::select_area_rect,
            app_model::open_editor_project,
            app_model::update_editor_project,
            app_model::plan_auto_zooms,
            app_model::render_preview_frame,
            app_model::get_preview_layout,
            app_model::get_timeline_thumbnails,
            app_model::list_background_presets,
            app_model::list_system_wallpapers,
            app_model::render_background_preview,
            app_model::start_export,
            app_model::cancel_export,
        ])
        .on_window_event(|window, event| match event {
            WindowEvent::CloseRequested { api, .. } if window.label() == "main" => {
                let handle = window.app_handle();
                let st = handle.state::<AppState>();
                if st.is_busy_recording() {
                    // Never destroy the main shell mid-recording; hide to system tray instead.
                    api.prevent_close();
                    let _ = window.hide();
                } else {
                    overlay::close_all_overlays(handle, &st.windows);
                    st.shutdown();
                    handle.exit(0);
                }
            }
            WindowEvent::Destroyed => {
                let st = window.app_handle().state::<AppState>();
                st.windows.unregister_label(window.label());
            }
            _ => {}
        })
        .build(tauri::generate_context!())
    {
        Ok(app) => app,
        Err(e) => {
            record_fatal_error(
                &format!("Failed to initialize the notscreenstudio shell: {e}"),
                !cfg!(debug_assertions),
            );
            std::process::exit(1);
        }
    };

    app.run(|handle, event| {
        if let RunEvent::Exit = event {
            handle.state::<AppState>().shutdown();
        }
    });
}

fn run_smoke_test() -> Result<String, String> {
    let base = nss_project::smoke_test()?;
    // Also verify motion planning + CPU passthrough renderer + shell state machine.
    let mut track = nss_core::CursorTrack::default();
    track.clicks.push(nss_core::ClickEvent::new(
        0.5,
        0.5,
        0.5,
        nss_core::MouseButton::Left,
        nss_core::ClickKind::Down,
    ));
    let zooms = nss_motion::ZoomPlanner::auto_zoom_blocks(
        &track,
        3.0,
        16.0 / 9.0,
        &[],
        &nss_motion::MotionTuning::default(),
    );
    if zooms.is_empty() {
        return Err("ZoomPlanner produced no zoom block for a valid click".into());
    }
    let frame = nss_render::FrameRenderer::render(
        &mut nss_render::PassthroughRenderer::new(),
        &nss_render::Image::filled(32, 18, nss_core::RGBAColor::WHITE),
        None,
        &nss_core::FrameState::new(0.0),
        &nss_core::Style::default(),
        nss_core::PixelSize::new(64, 36),
    )
    .map_err(|e| e.to_string())?;
    if frame.width != 64 || frame.height != 36 {
        return Err("PassthroughRenderer returned wrong dimensions".into());
    }
    Ok(format!("{base} + motion + render + shell"))
}
