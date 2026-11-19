# notscreenstudio for Windows

A free, completely offline Windows screen recorder and studio editor in the style of Screen Studio. Record a full display, a single window, or a dragged area, then polish the recording with auto-zoom on clicks, a smoothed cursor with motion blur, click ripples, styled backgrounds (solid colors, linear gradients, OKLab mesh gradients, Windows wallpapers, custom blurred images) with rounded corners and soft shadows, and a shaped webcam bubble. Export to MP4 (H.264 / HEVC, 30 or 60 fps) at 1080p or 4K.

Ported to Windows (Rust + Tauri 2 + wgpu/WGSL + React 19/TypeScript) and modified by **elementaryrock (Maanas MS)** from the original macOS project [dotslashsimran/notscreenstudio](https://github.com/dotslashsimran/notscreenstudio) (MIT).

## Features

- **Native Windows Capture (`nss-capture`):** Windows Graphics Capture (`Direct3D11CaptureFramePool`) with per-monitor DXGI adapter matching for hybrid-GPU / multi-monitor setups, Per-Monitor DPI v2 physical-pixel resolution, 60 Hz CFR pacing for static desktops, warm countdown gate, gapless pause/resume, WASAPI microphone & system audio loopback recording, Media Foundation webcam capture, and own-window exclusion via `WDA_EXCLUDEFROMCAPTURE`.
- **120 Hz Cursor & Click Tracking (`nss-input`):** Low-level mouse hook (`WH_MOUSE_LL`) + 120 Hz high-resolution sampler (`timeBeginPeriod(1)`) aligned to the shared recording clock, mixed-DPI and multi-monitor negative-coordinate support, and 30 Hz cursor shape detection (`IDC_*` handles + SHA-256 / 16x16 bitmap fingerprinting).
- **Motion & Auto-Zoom (`nss-motion`):** Critically-damped spring camera and cursor smoothing, automatic click clustering into non-overlapping zoom blocks, press squish, click ripples, idle cursor fade, and a zero-edge-gap viewport clamp.
- **Single-Pass GPU Compositor (`nss-render`):** wgpu/WGSL compositor with OKLab mesh gradients, separable linear-light Gaussian background blur, 18-shape vector cursor atlas with mipmaps, rounded corners, analytical soft shadows, and camera bubble shapes (`circle`, `square`, `tall`, `wide`) with shrink-on-zoom.
- **Shared Live Preview & 60 fps MP4 Export (`nss-export` + `src-tauri`):** Live editor preview and offline MP4 export share the exact same `SpringMotionModel` + `AssetFrameSource` (at-or-before frame selection) + `Renderer` pipeline. Exports 1080p and 4K across `16:9`, `1:1`, and `9:16` with H.264/HEVC, 48 kHz stereo AAC audio mix, atomic sibling-temp-file finalize, and instant cancellation.
- **Windows Shell & Studio UI (`src-tauri` + `ui`):** System tray with idle/recording/paused icons and elapsed timer tooltip, configurable global hotkeys (`Ctrl+Shift+R`, `Ctrl+Shift+P`) with conflict detection, translucent capture-excluded HUD overlays, OneDrive-aware Videos library (`SHGetKnownFolderPath`), Recycle Bin delete (`SHFileOperationW`), debounced autosave with `.bak` recovery, and a dark-themed Home + Editor + Timeline + Inspector interface.

## Requirements

- Windows 10 2004 (build 19041) or newer (Windows 11 recommended for borderless capture).
- Rust 1.86+ with the **MSVC** toolchain (`rust-toolchain.toml` pins the exact toolchain).
- Visual Studio 2022 Build Tools (C++ workload) and the Windows 10/11 SDK.
- Node.js 22+ for the frontend UI (`ui/`).
- WebView2 Runtime (preinstalled on Windows 11 and modern Windows 10).
- **FFmpeg** (`ffmpeg.exe` & `ffprobe.exe`, tested with **8.1.1**) on `PATH` or placed next to `notscreenstudio.exe`.

## Build, Test & Package

Run from PowerShell at the repository root:

```powershell
./scripts/dev.ps1      # hot-reload Tauri + Vite dev loop
./scripts/test.ps1     # full CI gate: fmt, clippy, cargo test, UI typecheck/test/build, smoke test
./scripts/build.ps1    # release build, smoke test, MSI + NSIS installers
```

Headless self-check (no GUI session needed):

```powershell
cargo run -p notscreenstudio-win-app --bin notscreenstudio -- --smoke-test
```

## Workspace Layout

```text
crates/nss-core       platform-neutral models, serde wire format, contracts
crates/nss-motion     spring camera, cursor, ripples, auto-zoom planner
crates/nss-project    .nssproj bundles, atomic save/backup, Recycle Bin
crates/nss-capture    Windows Graphics Capture, WASAPI, Media Foundation
crates/nss-input      WH_MOUSE_LL mouse hook, 120 Hz sampler, cursor shapes
crates/nss-render     wgpu/WGSL single-pass compositor, blur, cursor atlas
crates/nss-export     60 fps MP4 exporter, FFmpeg mux, at-or-before decode
src-tauri             tray, HUD overlays, global hotkeys, live preview IPC
ui                    React 19 + strict TypeScript Home, Editor & HUD views
scripts               PowerShell dev, test, and release installer scripts
```

## Platform Notes & Known Limitations

- **Yellow capture border:** Windows Graphics Capture draws a yellow border around captured windows/displays. On Windows 11 (22000+) you can request borderless capture once in Settings; if declined or on Windows 10, Windows keeps the yellow border visible.
- **Elevated apps & secure desktop (UAC):** A non-elevated `WH_MOUSE_LL` mouse hook cannot observe input over administrator-elevated windows or the UAC secure desktop; the cursor holds cleanly until focus returns to normal windows.
- **Custom app cursor shapes:** Shapes are identified via `LoadCursorW(IDC_*)` handles and 16x16 / SHA-256 bitmap fingerprints, falling back to the standard arrow for unrecognised custom cursors.
- **Single-monitor area capture:** Area capture crops out of a single display capture; dragging an area across two monitors is clamped to the starting display.
- **SDR output:** Live previews and exports render in 8-bit sRGB / SDR.
- **Global shortcuts:** Defaults are `Ctrl+Shift+R` (Record / Stop) and `Ctrl+Shift+P` (Pause / Resume), configurable in Settings with conflict detection.

## Privacy

Nothing leaves your machine. Zero accounts, zero network calls, zero telemetry, zero crash reporting. GitHub Actions CI (`.github/workflows/ci.yml`) fails the build if any network crate is added to workspace manifests or if `unsafe` escapes the four audited Windows FFI modules.

## License

Licensed under the **MIT License**. See [LICENSE](LICENSE).

- Original macOS version: Copyright (c) 2026 [dotslashsimran](https://github.com/dotslashsimran/notscreenstudio)
- Windows port and modifications: Copyright (c) 2026 [elementaryrock](https://github.com/elementaryrock) (Maanas MS)
