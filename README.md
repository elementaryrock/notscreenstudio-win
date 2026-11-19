# notscreenstudio for Windows

A free, completely offline Windows screen recorder in the style of Screen Studio: record a
display, a window or a dragged area, then polish the video with auto-zoom on clicks, a smoothed
cursor with motion blur, click ripples, backgrounds (solid, gradient, OKLab mesh, Windows
wallpapers, custom blurred images) with rounded corners and soft shadows, and a shaped camera
bubble. Export to MP4 (H.264 / HEVC, 30 or 60 fps) at 1080p or 4K.

> **Status: Complete Windows port (Stages 0–10 implemented).** All seven Rust crates, the Tauri 2
> Windows shell (tray, global hotkeys, excluded HUD overlays, live wgpu preview), and the React 19
> Home + Editor UI are implemented and verified. See [`PLAN.md`](PLAN.md), [`progress.md`](progress.md),
> [`docs/RENDERING.md`](docs/RENDERING.md), [`docs/MANUAL_TEST.md`](docs/MANUAL_TEST.md),
> [`docs/RELEASE.md`](docs/RELEASE.md), and the [ADRs](docs/ADR/).

This is a native Windows port of [dotslashsimran/notscreenstudio](https://github.com/dotslashsimran/notscreenstudio)
(MIT), originally written in Swift 6 / SwiftUI / Metal for macOS 15. The upstream source is
vendored read-only at `_upstream/` (revision in `_upstream/UPSTREAM_REVISION`) for reference.

## Features

- **Capture (`nss-capture`):** Windows Graphics Capture (`Direct3D11CaptureFramePool`) for full
  displays, individual windows, or cropped screen areas; 60 Hz CFR pacing for static desktops;
  countdown warm-up gate; pause/resume with zero gaps; WASAPI microphone and Media Foundation
  webcam capture; automatic own-window exclusion via `WDA_EXCLUDEFROMCAPTURE`.
- **120 Hz Cursor & Click Tracking (`nss-input`):** Low-level mouse hook (`WH_MOUSE_LL`) + 120 Hz
  high-resolution sampler (`timeBeginPeriod(1)`) aligned to the shared recording clock, mixed-DPI
  and multi-monitor negative-coordinate support, and 30 Hz cursor shape detection (`IDC_*` handles
  + SHA-256/16×16 bitmap fingerprinting).
- **Motion & Auto-Zoom (`nss-motion`):** Critically-damped spring camera and cursor smoothing,
  automatic click clustering into non-overlapping zoom blocks, press squish, click ripples, idle
  cursor fade, and a mathematically verified zero-edge-gap viewport clamp.
- **Single-Pass GPU Compositor (`nss-render`):** wgpu/WGSL translation of upstream's Metal shader
  with OKLab mesh gradients, separable linear-light Gaussian background blur, 18-shape vector
  cursor atlas with mipmaps, rounded corners, analytical soft shadows, and camera bubble shapes
  (`circle`, `square`, `tall`, `wide`) with shrink-on-zoom.
- **Shared Preview & 60 fps MP4 Export (`nss-export` + `src-tauri`):** Live editor preview and
  offline MP4 export share the exact same `SpringMotionModel` + `AssetFrameSource` (at-or-before
  frame selection) + `Renderer` pipeline. Exports 1080p and 4K across `16:9`, `1:1`, and `9:16`
  with H.264/HEVC, 48 kHz stereo AAC audio mix, atomic sibling-temp-file finalize, and instant
  cancellation.
- **Windows Shell & Studio UI (`src-tauri` + `ui`):** System tray with idle/recording/paused icons
  and elapsed timer tooltip, configurable global hotkeys (`Ctrl+Shift+R`, `Ctrl+Shift+P`) with
  conflict detection, borderless always-on-top excluded HUD overlays, OneDrive-aware Videos library
  (`SHGetKnownFolderPath`), Recycle Bin delete (`SHFileOperationW`), debounced autosave with
  backup recovery, and a full dark-themed Home + Editor + Timeline + Inspector interface.

## Requirements

- Windows 10 2004 (build 19041) or newer (Windows 11 recommended for borderless capture).
- Rust 1.86+ with the **MSVC** toolchain (`rust-toolchain.toml` pins the exact version).
- Visual Studio 2022 Build Tools (C++ workload) and the Windows 10/11 SDK.
- Node.js 22+ for the UI (`ui/`).
- WebView2 runtime (preinstalled on Windows 11 and modern Windows 10).
- **FFmpeg** (`ffmpeg.exe` & `ffprobe.exe`, tested with **8.1.1**) on `PATH` or next to the
  executable (see [ADR 0002](docs/ADR/0002-ffmpeg-packaging.md)).

## Build, Test & Package

With PowerShell:

```powershell
./scripts/dev.ps1      # hot-reload Tauri + Vite dev loop
./scripts/test.ps1     # full CI gate: fmt, clippy, cargo test, UI typecheck/test/build, smoke test
./scripts/build.ps1    # release build, smoke test, MSI + NSIS installers
```

Non-interactive self-check (no GUI session needed):

```powershell
cargo run -p notscreenstudio-win-app --bin notscreenstudio -- --smoke-test
```

## Workspace Layout

```
crates/nss-core       platform-neutral models, serde wire format, contracts (NSSCore)
crates/nss-motion     spring camera, cursor, ripples, auto-zoom planner     (NSSMotion)
crates/nss-project    .nssproj bundles, atomic save/backup, Recycle Bin     (NSSProject)
crates/nss-capture    Windows Graphics Capture, WASAPI, Media Foundation    (NSSCapture)
crates/nss-input      WH_MOUSE_LL mouse hook, 120 Hz sampler, cursor shapes (NSSInput)
crates/nss-render     wgpu/WGSL single-pass compositor, blur, cursor atlas  (NSSRender)
crates/nss-export     60 fps MP4 exporter, FFmpeg mux, at-or-before decode  (NSSExport)
src-tauri             tray, HUD overlays, global hotkeys, live preview IPC
ui                    React 19 + strict TypeScript Home, Editor & HUD views
docs/ADR              architecture decision records (0001–0012)
docs/RENDERING.md     wgpu/WGSL frame pipeline specification
docs/MANUAL_TEST.md   Windows 10/11 manual QA checklist
docs/RELEASE.md       packaging, FFmpeg licensing & release checklist
```

## Platform Differences & Known Limitations

These stem from Windows platform APIs and v1 scope choices; each is documented in an ADR:

- **Yellow capture border:** Windows Graphics Capture draws a yellow border around captured
  windows/displays. On Windows 11 (22000+) you can request borderless capture once in Settings; if
  declined or on Windows 10, Windows keeps the yellow border visible ([ADR 0007](docs/ADR/0007-capture-backend.md)).
- **Elevated apps & secure desktop (UAC):** A non-elevated `WH_MOUSE_LL` mouse hook cannot observe
  input over administrator-elevated windows or the UAC secure desktop; the cursor holds cleanly
  until focus returns to normal windows.
- **Custom app cursor shapes:** Windows exposes no stable cursor ID; shapes are identified via
  `LoadCursorW(IDC_*)` handles and 16×16 / SHA-256 bitmap fingerprints, falling back to the arrow
  for unrecognised custom app cursors ([ADR 0008](docs/ADR/0008-cursor-shapes.md)).
- **Single-monitor area capture:** Area capture crops out of a single display capture; dragging an
  area across two monitors is clamped to the starting display.
- **SDR exports:** Exports and live previews render in 8-bit sRGB / SDR in v1 ([ADR 0005](docs/ADR/0005-codec-and-hdr.md)).
- **Global shortcuts:** Default shortcuts are **Ctrl+Shift+R** (Record / Stop) and
  **Ctrl+Shift+P** (Pause / Resume), configurable in Settings with conflict detection.
- **Installer signing:** Local builds produce unsigned MSI/NSIS installers unless an Authenticode
  certificate is configured (see [`docs/RELEASE.md`](docs/RELEASE.md)).

## Privacy

Nothing leaves your machine. Zero accounts, zero network calls, zero telemetry, zero crash
reporting. `.github/workflows/ci.yml` fails the build if any network crate is added to workspace
manifests or if `unsafe` escapes the four audited Windows FFI modules.

## Licence

MIT, matching upstream. See [`LICENSE`](LICENSE).
