//! notscreenstudio for Windows — application entry point.
//!
//! Replaces upstream `Sources/NotScreenStudio/App.swift`. This file owns the Tauri
//! runtime and the command surface; everything with real behaviour lives in the
//! `nss-*` crates (see `PLAN.md` Appendix A).
//!
//! Stage 0: the shell opens and proves the IPC bridge works. Capture, the
//! compositor and the editor arrive in stages 4-9.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![forbid(unsafe_code)]

use serde::Serialize;

/// The port stage currently implemented (tracked against `PLAN.md` §5).
const PORT_STAGE: u32 = 0;

#[derive(Debug, Serialize)]
struct CoreInfo {
    version: String,
    platform: String,
    stage: u32,
}

/// Trivial round-trip used by the UI to prove the bridge is alive.
#[tauri::command]
fn ping(message: String) -> String {
    message
}

/// Reports which crates and which port stage this build contains.
#[tauri::command]
fn core_info() -> CoreInfo {
    CoreInfo {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        platform: std::env::consts::OS.to_owned(),
        stage: PORT_STAGE,
    }
}

fn main() {
    // The stage-0 smoke test exits before any window is created, so CI can build
    // and link the whole graph without a display (upstream: `--smoke-test`).
    if std::env::args().any(|a| a == "--smoke-test") {
        match nss_project::smoke_test() {
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

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![ping, core_info])
        .run(tauri::generate_context!())
        .expect("failed to start the notscreenstudio shell");
}
