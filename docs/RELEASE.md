# Windows Release Checklist

This guide documents building, verifying, and packaging **notscreenstudio for Windows** as MSI and NSIS installers.

---

## 1. Prerequisites

- **OS:** Windows 10 2004+ or Windows 11 (x64).
- **Rust:** Toolchain pinned in `rust-toolchain.toml` (`x86_64-pc-windows-msvc`).
- **Visual Studio 2022 Build Tools:** Desktop development with C++ workload + Windows 10/11 SDK.
- **Node.js:** v22+ (with `npm`).
- **Tauri CLI:** `cargo install tauri-cli --version "^2"` (or via `npx --prefix ui tauri`).
- **FFmpeg (runtime):** Tested against **FFmpeg 8.1.1** (`ffmpeg.exe` + `ffprobe.exe` with `libx264`, `libx265`, `aac`). See [ADR 0002](ADR/0002-ffmpeg-packaging.md).
- **Installer toolchains:** WiX Toolset (for `.msi`) and NSIS (for `-setup.exe`), automatically downloaded by `cargo tauri build` on first run if missing.

---

## 2. Pre-Release Automated Verification

Run every check below on a clean git tree before tagging a release:

```powershell
# 1. Full CI-equivalent gate (fmt, clippy, all workspace tests, UI typecheck/test/build, smoke test)
./scripts/test.ps1

# 2. Performance & GPU parity suite with strict budgets enabled
$env:NSS_BENCH = "1"
cargo test -p nss-motion --test performance -- --nocapture
cargo test -p nss-export --test integration_e2e -- --nocapture
Remove-Item Env:NSS_BENCH

# 3. Live WGC hardware capture test (run in an unlocked interactive desktop session)
cargo test -p nss-capture live_records_one_second_of_primary_display -- --ignored --nocapture
```

### Privacy & Unsafe-Code Audit

Verify locally (also enforced by `.github/workflows/ci.yml` on every push/PR):

1. **Zero network dependencies in workspace manifests:**
   Confirm no `reqwest`, `hyper`, `ureq`, `curl`, `sentry`, `opentelemetry`, or analytics crates appear in `crates/*/Cargo.toml` or `src-tauri/Cargo.toml`.
2. **Confined `unsafe` boundary:**
   Confirm `unsafe` keywords appear **only** inside the four audited Windows FFI modules (ADR 0010 & ADR 0012):
   - `crates/nss-project/src/windows_trash.rs`
   - `crates/nss-capture/src/win/`
   - `crates/nss-input/src/win/`
   - `src-tauri/src/win_shell.rs`
3. **Platform-neutral core:**
   Confirm `crates/nss-core/Cargo.toml` and `crates/nss-motion/Cargo.toml` contain zero `windows`, `wgpu`, or `tauri` dependencies and retain `#![forbid(unsafe_code)]`.

---

## 3. Building MSI & NSIS Installers

Use the release script (which compiles the release binary, runs `notscreenstudio.exe --smoke-test`, and invokes `cargo tauri build`):

```powershell
./scripts/build.ps1
```

Outputs are written to:
- `target/release/bundle/msi/notscreenstudio_<version>_x64_en-US.msi`
- `target/release/bundle/nsis/notscreenstudio_<version>_x64-setup.exe`

To build only one installer target:

```powershell
$env:NSS_BUNDLES = "nsis"
./scripts/build.ps1
```

---

## 4. FFmpeg Distribution & Licensing (ADR 0002)

`nss-export` and `nss-capture` resolve `ffmpeg.exe` and `ffprobe.exe` in order:

1. Next to `notscreenstudio.exe` (installed app directory)
2. Inside `bin/` next to `notscreenstudio.exe`
3. On system `PATH`

If FFmpeg is absent, recording/export displays a structured missing-FFmpeg message citing the tested version (`8.1.1`) and download link (`https://www.gyan.dev/ffmpeg/builds/`).

**Licensing note:**
- This repository is **MIT-licensed** and ships no FFmpeg binaries in Git.
- If distributing a bundle that includes a GPL build of FFmpeg (such as `libx264`/`libx265`), include the FFmpeg license text and exact source-code offer required by GPLv2/v3, or ship without FFmpeg bundled so users install/provide their own FFmpeg on `PATH` (default configuration).

---

## 5. Code Signing & SmartScreen

- **Unsigned builds** (default local build): Windows Defender SmartScreen will show *"Windows protected your PC"* on first run for downloaded installers. Document this clearly in release notes.
- **Signed builds:** Configure `bundle.windows.certificateThumbprint` and `bundle.windows.timestampUrl` in `src-tauri/tauri.conf.json` (or via CI environment variables for Azure Trusted Signing / EV Authenticode certificates) before running `./scripts/build.ps1`.

---

## 6. Manual Sign-Off

Before publishing a release installer:

1. Install the `.msi` or `-setup.exe` on a clean Windows 11 VM or machine.
2. Complete [`docs/MANUAL_TEST.md`](MANUAL_TEST.md) end-to-end:
   - First-run permissions & borderless capture consent check
   - Display, Window, and Area recording with Pause/Resume
   - Editor preview, zoom blocks, backgrounds, trim, undo/redo
   - 1080p & 4K MP4 export parity check + cancellation test
   - Uninstall cleanly via Windows Installed Apps
3. Bump `version` in `Cargo.toml`, `src-tauri/tauri.conf.json`, and `ui/package.json`, tag `v<version>`, and attach installers + SHA-256 checksums to the GitHub Release.
