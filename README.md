# notscreenstudio for Windows

A free Windows screen recorder in the style of Screen Studio: record a display, a window or
an area, then polish the video with auto-zoom on clicks, a smoothed cursor with motion blur,
click ripples, a background with rounded corners and a shadow, and a camera bubble. Export to
MP4 at 1080p or 4K.

> **Status: Stage 0 of the port.** The workspace, the platform-neutral core, the IPC bridge
> and the UI shell are in place and tested. Capture, the compositor and the editor are built
> in stages 4-9. See [`PLAN.md`](PLAN.md) for the full plan and the [ADRs](docs/ADR/) for the
> decisions behind it.

This is a port of [dotslashsimran/notscreenstudio](https://github.com/dotslashsimran/notscreenstudio)
(MIT), a macOS 15 app written in Swift 6 with SwiftUI and Metal. The upstream source is
vendored read-only at `_upstream/` (revision in `_upstream/UPSTREAM_REVISION`) for reference.

## What is here today

- A Cargo workspace with the crate layout the port is built on (`crates/`).
- `nss-core`: the shared conventions — `MediaTime`, normalised points, colours, pixel sizes
  and the error type. Platform-neutral by rule.
- The Tauri 2 shell with the IPC bridge and a `--smoke-test` mode, mirroring upstream's
  `bundle.sh` self check.
- A React UI shell with the editor design tokens ported from upstream `EditorTheme.swift`.
- CI that fails on formatting, lint warnings, failing tests, UI type errors, or the addition
  of a network dependency.

## Requirements

- Windows 10 2004 (build 19041) or newer. Windows 11 recommended for borderless capture.
- Rust 1.85+ with the **MSVC** toolchain (`rust-toolchain.toml` pins it).
- Visual Studio 2022 Build Tools (C++ workload) and the Windows 10/11 SDK.
- Node.js 22+ for the UI.
- WebView2 runtime (preinstalled on Windows 11 and current Windows 10).
- FFmpeg on `PATH` once exporting lands (ADR 0002).

## Build and run

With PowerShell:

```powershell
./scripts/dev.ps1      # hot-reload dev loop
./scripts/test.ps1     # fmt, clippy, tests, UI typecheck — what CI runs
./scripts/build.ps1    # release build, smoke test, MSI + NSIS installers
```

Or directly:

```sh
cargo build -p notscreenstudio-win-app
npm --prefix ui install
cargo tauri dev
```

Non-interactive self check (no GUI needed, suitable for CI):

```sh
cargo run -p notscreenstudio-win-app --bin notscreenstudio -- --smoke-test
```

## Layout

```
crates/nss-core       models, conventions, contracts          (port of NSSCore)
crates/nss-motion     spring camera, cursor, ripples, zoom    (port of NSSMotion)
crates/nss-project    .nssproj bundles, autosave, migration   (port of NSSProject)
crates/nss-capture    Graphics Capture, WASAPI, Media Foundation
crates/nss-input      low-level mouse hook, cursor shapes
crates/nss-render     wgpu compositor, WGSL, cursor atlas
crates/nss-export     60 fps MP4 export through FFmpeg
src-tauri             tray, HUD windows, hotkeys, IPC
ui                    React editor and home screen
shaders               WGSL sources
docs/ADR              decision records
```

## Platform differences from the macOS original

These are Windows API realities, not porting bugs. Each has an ADR.

- **Capture is never silent.** Windows draws a yellow border around a captured window or
  display. On Windows 11 the app asks once for permission to hide it; if refused, or on
  Windows 10, the border stays. ADR 0007.
- **Picking a source is explicit.** There is no silent window enumeration the way
  ScreenCaptureKit allows, so the app builds its own picker and falls back to the system one.
  ADR 0007.
- **Some cursors render as an arrow.** Windows exposes no stable cursor identity, so shapes
  are matched by handle and then by bitmap hash. Unrecognised custom cursors fall back to the
  arrow. ADR 0008.
- **No capture of elevated apps or the secure desktop.** A non-elevated low-level mouse hook
  cannot observe them. Documented, not fixed.
- **Exports are SDR.** HDR capture is tone-mapped for preview and export in v1. ADR 0005.
- **The global shortcut is Ctrl+Shift+R**, not Control+Command+R (there is no Command key).

## Privacy

Nothing leaves the machine. No accounts, no network calls, no telemetry, no crash reporting.
A CI job fails the build if a network dependency is added to any of our manifests.

## Licence

MIT, matching upstream. See [`LICENSE`](LICENSE).
