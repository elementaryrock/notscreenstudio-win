# ADR 0012 — Windows capture & input FFI boundary (`windows` crate and confined `unsafe`)

Status: accepted (Stage 5; applies to Stages 5, 6 and 8)

## Context

Stages 0–4 built the platform-neutral core (`nss-core`, `nss-motion`), the bundle store
(`nss-project`), and the GPU compositor (`nss-render`). In Stage 3 (ADR 0010), `unsafe` was
confined to a single Recycle Bin call (`crates/nss-project/src/windows_trash.rs`), and CI
enforced that nothing else used `unsafe`.

Stages 5, 6 and 8 introduce the real Windows platform integrations:

- **Stage 5 (`nss-capture`)**: `Windows.Graphics.Capture` (WGC) + Direct3D 11 staging readback,
  `EnumDisplayMonitors` / `GetDpiForMonitor` / `EnumWindows` / `DwmGetWindowAttribute`,
  `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`, WASAPI microphone capture, Media Foundation
  camera capture, and Windows privacy registry reads.
- **Stage 6 (`nss-input`)**: `SetWindowsHookExW(WH_MOUSE_LL)`, `GetCursorPos`, `GetCursorInfo`,
  `GetIconInfoExW` / `GetDIBits`.
- **Stage 8 (`src-tauri`)**: `RegisterHotKey`, `SHGetKnownFolderPath(FOLDERID_Videos)`,
  `SHOpenFolderAndSelectItems`, `SetWindowDisplayAffinity`.

The Microsoft `windows` crate (`0.62`, already present in `Cargo.lock` via `tauri` and `wgpu`)
provides typed Win32 and WinRT bindings without adding an unrelated third-party wrapper.

## Decision

1. **Depend on `windows = "0.62"` and `windows-core = "0.62"`** in the workspace manifest, with
   narrow per-crate feature flags. Never depend on `windows` from `nss-core` or `nss-motion`
   (PLAN.md §3 hard rule).
2. **Keep `nss-capture` and `nss-input` split into pure rules + thin `win/` FFI adapters.**
   The crate roots stay `#![deny(unsafe_code)]`; `#[allow(unsafe_code)]` is permitted **only**
   inside:
   - `crates/nss-project/src/windows_trash.rs` (ADR 0010)
   - `crates/nss-capture/src/win/`
   - `crates/nss-input/src/win/`
   - `src-tauri/src/win_shell.rs`
3. **Update the CI `unsafe` audit job** to allow only those four audited paths and fail the
   build if `unsafe` appears anywhere else in the workspace.
4. **Intermediate capture files use FFmpeg pipes (`screen.mov`, `camera.mov`, `mic.m4a`)** via
   ADR 0002's FFmpeg binary, preserving exact upstream `.nssproj` filenames (`RecordingAssets`
   defaults) so projects, thumbnailing, preview decoding, and export all share one standard
   container format without a custom binary container.

## Consequences

- `nss-core`, `nss-motion`, `nss-render`, and `nss-export` remain `#![forbid(unsafe_code)]`.
- Every capture rule (framing, pause clock, countdown gate, filter planning, error mapping,
  geometry) stays unit-testable without hardware or `unsafe`.
- Live WGC / WASAPI / MF / hook tests remain cleanly gated so CI stays green on headless
  runners while running against real hardware locally.
