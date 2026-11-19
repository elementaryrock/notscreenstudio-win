# Contributing

These engineering rules keep the Windows codebase deterministic, private, and maintainable.

## Language and Platform

- Rust 2021, pinned to the MSVC toolchain by `rust-toolchain.toml`. Minimum OS target is Windows 10 2004 (build 19041).
- UI is React 19 + TypeScript (`strict`, `noUncheckedIndexedAccess`, `exactOptionalPropertyTypes`).
- Keep dependencies minimal and strictly offline.

## Toolchain

- No build step may require an interactive GUI session. `--smoke-test` must always work headless.
- WGSL shaders live in `crates/nss-render/src/` (`shader.wgsl`, `blur.wgsl`) and are embedded with `include_str!`.
- FFmpeg (`ffmpeg.exe` / `ffprobe.exe`) is the only external runtime binary.
- **PowerShell scripts stay pure ASCII without UTF-8 BOM.** Windows PowerShell 5.1 parses BOM-less `.ps1` files as ANSI, so non-ASCII punctuation breaks parsing.

## Crate Boundaries & Unsafe Confinement

- `nss-core` and `nss-motion` are **platform-neutral** (`#![forbid(unsafe_code)]`) and must never depend on `windows`, `wgpu`, `tauri`, or any I/O backend.
- `unsafe` is strictly confined to the four audited Windows FFI paths enforced by CI:
  - `crates/nss-project/src/windows_trash.rs`
  - `crates/nss-capture/src/win/`
  - `crates/nss-input/src/win/`
  - `src-tauri/src/win_shell.rs`
- `nss-render` owns the `wgpu::Device` and shaders.
- `nss-project` owns `.nssproj` bundle persistence (`project.json`, `cursor.json`, atomic save/backup).
- `src-tauri` owns Tauri windows, tray, hotkeys, and async IPC commands. Keep blocking, COM/WinRT, GPU, and FFmpeg work off the WebView2 STA UI thread.

## Contracts

- Times are `f64` seconds from the first screen frame (`MediaTime`).
- Positions are normalised `0..=1`, origin top-left.
- Motion is a pure, deterministic function of time shared identically by live preview and MP4 export.
- Every persisted `project.json` field decodes with a backward-compatible default and keeps wire compatibility with upstream macOS `.nssproj` bundles.

## Tests & Verification

- Run `./scripts/test.ps1` locally before opening a PR; CI runs the exact same checks:
  - `cargo fmt --all -- --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo test --workspace`
  - `npm --prefix ui run typecheck`, `test`, and `build`
  - `notscreenstudio --smoke-test`
- Hardware-dependent tests (live WGC capture, live mouse hook) use `#[ignore = "device: ..."]` so headless CI stays green.

## Privacy

- Nothing leaves the machine. No networking crates, analytics, telemetry, crash reporters, or update pingers. CI fails if any network crate is added to workspace `Cargo.toml` manifests.
