# notscreenstudio-win — Port Progress Tracker

Master checklist tracking the full port of [`dotslashsimran/notscreenstudio`](https://github.com/dotslashsimran/notscreenstudio) (vendored read-only at `_upstream/`) to Windows (Tauri 2 + Rust + wgpu/WGSL + React/TypeScript).

See [PLAN.md](./PLAN.md) for architecture, milestones, and invariants, and [CONTRIBUTING.md](./CONTRIBUTING.md) for hard rules.

> **Handoff note for any AI / contributor picking this up:**
> Run `./scripts/test.ps1` (or `cargo test --workspace`) to verify the current baseline, check `git log -n 5 --oneline` and `git status`, then resume from the first unchecked `[ ]` item below. Always keep `progress.md` and `PLAN.md` updated as items finish.

---

## Stage 0 — Repository & Toolchain Bootstrap (`3e60b6e`, `afdc201`)

- [x] Vendor upstream read-only at `_upstream/`
- [x] Pin Rust MSVC toolchain in `rust-toolchain.toml` (`x86_64-pc-windows-msvc`)
- [x] Scaffold Cargo workspace (7 crates + `src-tauri`) with `#![deny(warnings)]`
- [x] Scaffold Tauri 2 + React 19 + TypeScript (strict) UI in `ui/`
- [x] Add `scripts/dev.ps1`, `scripts/build.ps1`, `scripts/test.ps1` (ASCII-only)
- [x] Add GitHub Actions CI (`.github/workflows/ci.yml`: core, app, unsafe audit, privacy audit)
- [x] Write `CONTRIBUTING.md`, `README.md`, and ADRs 0001–0008
- [x] Verify Tauri window launches and `--smoke-test` passes

## Stage 1 — `nss-core` (`c9a7a6b`)

- [x] `basics.rs` (`MediaTime`, `NormalizedPoint`, `NormalizedSize`, `RGBAColor`, `PixelSize`, `Rect`, `Point2`, `Size2`, `Timestamp`)
- [x] `style.rs` (`Background`, `Shadow`, `AspectRatio`, `CursorStyle`, `CameraBubble`, `CameraShape`, `FrameStyle`)
- [x] `timeline.rs` (`Timeline`, `ZoomBlock`, `ZoomFocus`, `Trim`)
- [x] `cursor.rs` (`CursorTrack`, `CursorSample`, `ClickEvent`, `CursorShape`, `ClickKind`, `MouseButton`)
- [x] `frame_state.rs` (`FrameState`, `CameraState`, `RippleState`, `CursorState`)
- [x] `project.rs` (`Project`, `ProjectReference`, `ProjectSummary`)
- [x] `recording.rs` (`RecordingOptions`, `RecordingAssets`, `CaptureSource`, `RecordingState`)
- [x] `background_presets.rs`
- [x] `protocols.rs` (sync traits per ADR 0009, `NssError`)
- [x] `tests/compat.rs` (65 tests pinning Swift wire-format compatibility, `float_roundtrip`, uppercase UUIDs, CF epoch)
- [x] ADR 0009 (`docs/ADR/0009-core-traits.md`)

## Stage 2 — `nss-motion` (`8276f9c`)

- [x] `spring.rs` (closed-form critically/under/over-damped spring + matrix `SpringStepper`)
- [x] `motion_tuning.rs` (upstream defaults + guard rails)
- [x] `viewport.rs` (edge-gap guarantee with `next_up`/`next_down` clamping)
- [x] `zoom_planner.rs` (click clustering, merge/split, 0.3 s min block, non-overlap, `1.1..=4.0`)
- [x] `spring_motion_model.rs` (fixed-step simulation table, dead-zone follow, idle fade, ripples, `stable_shapes`, press squish)
- [x] `linear_motion_model.rs` + shared `factory()` seam
- [x] Unit + `viewport_guarantee` + `stored_zoom` + `performance` tests (62 tests)

## Stage 3 — `nss-project` (`a4804d1`)

- [x] `atomic_file.rs` (temp + `sync_all` + rename with Windows sharing-violation retry)
- [x] `debounce.rs` (`AutosaveDebouncer` worker thread + `DebounceClock` / `ManualClock`)
- [x] `migrator.rs` (`ProjectMigrator` v1 -> v2)
- [x] `store.rs` (`FileProjectStore`: create/save/load/cursor/autosave/list/recents, Windows name sanitising)
- [x] `library.rs` (`FileProjectLibrary`: summaries, FNV-1a IDs, `Thumbnailer` trait + `FfmpegThumbnailer`, rename, delete)
- [x] `recent_projects.rs`
- [x] `windows_trash.rs` (Recycle Bin via `SHFileOperationW`, single `unsafe` block per ADR 0010)
- [x] Unit + `background_migration` + `crash_recovery` tests (70 tests)
- [x] ADR 0010 (`docs/ADR/0010-windows-file-semantics.md`)

## Stage 4 — `nss-render` (`8305f91`)

- [x] `layout.rs` (1:1 port of `RenderLayout` geometry contract)
- [x] `shaders/frame.wgsl` + `uniforms.rs` (46-slot single-pass compositor: background, shadow, screen, ripples, cursor, camera bubble, sRGB + dither)
- [x] `shaders/blur.wgsl` + `image_blur.rs` (two-pass separable Gaussian blur in linear light + cache)
- [x] `cursor_atlas.rs` (18 vector cursors at 8x with 32 px padding + mip chain + `CursorAtlasCache`)
- [x] `rasterizer.rs` (CPU `BackgroundRasterizer` twin)
- [x] `frame.rs` (`Image`, `Frame`, `FrameSource`, `SolidColorFrameSource`)
- [x] `passthrough.rs` (`PassthroughRenderer` software fallback)
- [x] `renderer.rs` (`Renderer` wgpu compositor + readback + caches)
- [x] `wallpapers.rs` (Windows wallpaper discovery)
- [x] Unit + `metal_renderer` + `background_render` + `cursor_render` parity tests (95 tests)
- [x] `docs/RENDERING.md` & ADR 0011 (`docs/ADR/0011-renderer-parity.md`)

---

## Stage 5 — `nss-capture` (DONE)

### Platform-neutral capture logic
- [x] `capture_math.rs` (frame rate, even pixels, pixel size, area clamping, file plan, shared origin)
- [x] `pause_clock.rs` (`PauseTimeline`, `PauseClock`, `PausedFrameHold`, cursor track pause stripping)
- [x] `frame_gate.rs` (`FrameGate` countdown hold/arm/open)
- [x] `capture_filter.rs` (`ContentSnapshot`, `FilterPlan`, `ExclusionPlan`, own-window matching)
- [x] `capture_errors.rs` (`map_code`, `map_error`, HRESULT -> user-facing `NssError`)
- [x] Fix deadlock in `state_hub.rs` (`StateHub::subscribe` calls listener outside hub lock; re-entrant safe)
- [x] `win/mod.rs` (pure Windows geometry/monitor/window value types + `snapshot`, `monitor_at`, `area_in_display`)

### Windows capture implementation & session orchestration
- [x] Add Windows FFI dependencies (`windows 0.62` crate for Win32/WinRT WGC/D3D11/DWM/WASAPI/MF) and ADR 0012 (`docs/ADR/0012-windows-capture-and-input.md`) + update CI `unsafe` audit rule
- [x] `catalog.rs` + `win/enumerate.rs`: `CaptureSourceCatalog` (`EnumDisplayMonitors`, `GetDpiForMonitor`, `EnumWindows`, `DwmGetWindowAttribute(DWMWA_EXTENDED_FRAME_BOUNDS / DWMWA_CLOAKED)`, `WS_EX_TOOLWINDOW`, process image name)
- [x] `permissions.rs` + `win/permissions.rs`: Windows camera/mic privacy registry check + one-shot borderless capture consent gate
- [x] `win/exclusion.rs`: `WDA_EXCLUDEFROMCAPTURE` per-HWND exclusion helper + refresh
- [x] `device.rs` + `win/devices.rs`: enumerate microphones (WASAPI `IMMDeviceEnumerator`) and cameras (Media Foundation `MFEnumDeviceSources`), plus `MicRecorder` (WASAPI -> `mic.m4a`) and `CameraRecorder` (MF `IMFSourceReader` -> `camera.mov`)
- [x] `writer.rs`: `RawVideoWriter` (60 Hz CFR pacer + pause gate -> FFmpeg `screen.mov`/`camera.mov`) & `PcmAudioWriter` (`mic.m4a`)
- [x] `win/wgc.rs`: `Windows.Graphics.Capture` (`Direct3D11CaptureFramePool`, `GraphicsCaptureSession`, `SetIsCursorCaptureEnabled(false)`, best-effort `SetIsBorderRequired(false)`, `Closed` event, D3D11 staging readback, display + window + area crop)
- [x] `recorder.rs`: `ScreenStreamPlan`, `WarmableRecordingSession`, `NoopRecordingSession`, `ScreenCaptureRecordingSession`
- [x] Port upstream capture tests (79 unit tests + 1 live WGC hardware test verified on Windows) + removed placeholder `gate.rs`/`pause.rs`
- [x] Verify `./scripts/test.ps1` green, update `PLAN.md` & `progress.md`, commit Stage 5

---

## Stage 6 — `nss-input` (DONE)

- [x] `geometry.rs` (`SourceGeometry`: virtual-desktop coords -> normalised `0..1` source space, mixed-DPI, negative multi-monitor coords, area crop support)
- [x] `schedule.rs` (`SamplingSchedule`, `SamplingStats`: 120 Hz sampling, monotonic clock alignment via shared `nss_core::host_now()`, rebase to `t = 0`, drop/clamp pre-origin)
- [x] `shapes.rs` (`CursorFingerprint`, `bitmap_hash`, `CursorShapeIdentifier`: `LoadCursorW(IDC_*)` handle match -> SHA-256 hash + 16x16 fingerprint -> `Unknown`/`Arrow` fallback, 30 Hz polling)
- [x] `hook.rs` + `win/hook.rs` (`WH_MOUSE_LL` low-level mouse hook on dedicated Win32 message-loop thread + `GetCursorPos` 120 Hz sampler with `timeBeginPeriod(1)`)
- [x] `recorder.rs` (`CursorRecorder`, `SystemInputLogger`, `NoopInputLogger` implementing `nss_core::InputLogger`)
- [x] Port upstream tests (`NSSInputTests`, `CursorShapeTests`, `CursorMappingTests`, 21 tests incl. 120 Hz timer pacing test)
- [x] Verify `./scripts/test.ps1` green, update `PLAN.md` & `progress.md`, commit Stage 6

---

## Stage 7 — `nss-export` (DONE)

- [x] `ffmpeg.rs` (FFmpeg/FFprobe discovery per ADR 0002: next to exe -> PATH -> structured missing-FFmpeg error with tested version 8.1.1 + encoder capability detection `libx264`/`libx265`/hw fallback)
- [x] `frame_source.rs` (`AssetFrameSource`: FFmpeg demux/decode with exact **at-or-before** frame selection for screen & camera, bounded 2-frame memory, keyframe-aware backward/forward seeking)
- [x] `audio_pump.rs` (`AudioMixPlan`: trim + 48 kHz stereo resample + drift compensation `async=1` + AAC 192 kbps)
- [x] `exporter.rs` (`Mp4Exporter`: `t = trim.start + i / fps` -> `MotionModel::frame_state(t)` -> `AssetFrameSource` -> `FrameRenderer::render` -> FFmpeg child process `-movflags +faststart`, sibling temp file + atomic rename on clean finish, progress callback + atomic `CancelToken`)
- [x] Resolution presets (1080p / 4K by shorter side across 16:9, 1:1, 9:16; H.264 & HEVC)
- [x] Port upstream tests (`tests/export_parity.rs`: 13 real FFmpeg/wgpu tests + 4 unit tests covering 1080p/4K/HEVC/trim/slow-source/audio/cancel/missing-file/backward-seek/decode-failure/preview-vs-export frame-for-frame parity)
- [x] Verify clippy & tests green, update `PLAN.md` & `progress.md`, commit Stage 7

---

## Stage 8 — Shell: Tray, HUD Windows, Hotkeys, Overlays, Settings (`src-tauri`) (DONE)

- [x] `settings.rs` (persisted `ShellSettings`: source, camera/mic devices, countdown, hotkeys, aspect, output dir, 8 recent background images, inspector open state; atomic JSON store; `SHGetKnownFolderPath(FOLDERID_Videos)` library root closing Stage 3 gap)
- [x] `shell_model.rs` (`ShellPhase`, `ShellEvent`, `ShellStateMachine`, `ElapsedClock`, `BarPlacement`, `BubbleGeometry`, `AreaSelectionMath`)
- [x] `win_shell.rs` (`RegisterHotKey` global shortcuts: Ctrl+Shift+R record/stop, Ctrl+Shift+P pause/resume with conflict detection; `SHOpenFolderAndSelectItems`; `WDA_EXCLUDEFROMCAPTURE` + `WS_EX_TOOLWINDOW`)
- [x] `tray.rs` (system tray icon with 32x32 idle/recording/paused states, 1 s elapsed tooltip, menu: Record, Pause/Resume, Stop, Open Library, Settings, Quit)
- [x] `overlay.rs` (borderless always-on-top `WS_EX_TOOLWINDOW` + `WDA_EXCLUDEFROMCAPTURE` HUD windows: Setup bar, Recording bar, Countdown, Area selection, Camera bubble + central `WindowRegistry`)
- [x] `preview.rs` (`PreviewEngine` sharing `SpringMotionModel` + `Renderer` + `AssetFrameSource` with `Mp4Exporter`, timeline thumbnail extraction, `BackgroundRasterizer` swatches, Windows wallpaper discovery)
- [x] `app_model.rs` (36 typed Tauri IPC commands + raw binary `ipc::Response` RGBA preview transport + async worker export streaming `nss://export-progress` + autosave flush on exit)
- [x] Port shell tests (`shell_model`, `settings`, `tray`, `win_shell` unit tests + extended `--smoke-test`)
- [x] Verify clippy, unit tests & `--smoke-test` green, update `PLAN.md` & `progress.md`, commit Stage 8

---

## Stage 9 — UI: Home + Editor + Overlays (React + TypeScript) (DONE)

- [x] Architecture & stores (`ui/src/strings.ts`, strict TypeScript wire types `ui/src/types.ts`, typed IPC bridge `ui/src/ipc.ts` with raw binary `ArrayBuffer` RGBA unpacking, pure editor logic & coalescing undo/redo stack `ui/src/editor/editorLogic.ts` where slider drags coalesce to 1 step)
- [x] **Home view**: responsive recording card grid, thumbnails, name/date/duration, newest-first, context menu (Rename, Show in Explorer, Delete to Recycle Bin), live recording banner, Sample Project action, empty state with Record CTA (`HomeView.tsx`)
- [x] **Editor shell**: dark studio theme (`ui/src/styles/app.css`), top bar (Recordings back button, editable title, undo/redo, aspect picker, Export), center preview, right inspector, bottom timeline (`EditorView.tsx`)
- [x] **Preview**: Rust-rendered 60 fps GPU preview surface via `nss-motion` + `nss-render` (shared frame pipeline with export per ADR 0003), frame-time HUD, click-to-set manual zoom focus, draggable camera bubble handle (`PreviewCanvas.tsx`)
- [x] **Timeline**: 4 tracks (Video thumbnail filmstrip, Zoom blocks with `✦` auto-sparkle & drag/resize/select, Camera track, Audio track), yellow trim handles, orange click dots, eased playhead (`PlayheadEasing`: 0.2 s cubic ease-out on jump, instant snap on drag/playback) (`TimelinePanel.tsx`)
- [x] **Inspector**: 6 collapsible sections (Background, Frame, Cursor, Zoom, Camera, Audio) with persisted open state; Background picker tabs (Windows Wallpapers, Gradients + meshes with CPU-rasterized swatches + angle slider, Colors + custom picker, Image + 8 recents); Frame blur/padding/corner/shadow; Cursor size/smoothing/blur/ripples/hide-when-idle; Zoom level/focus/ease in/out/follow-vs-fixed (X/Y sliders + click preview); Camera bubble shape/roundness/size/border width & color/mirror/shrink-on-zoom/position; Audio status (`InspectorPanel.tsx`)
- [x] **Export sheet**: resolution (1080p/4K with exact aspect dimensions), fps (30/60), codec (H.264/HEVC), destination, live progress + cancel, Show in Explorer when done, missing-FFmpeg warning (`ExportModal.tsx`)
- [x] **Overlay views & modals**: Setup modal (`SetupModal.tsx`), Permissions & Settings modal (`SettingsModal.tsx`), and hash-routed translucent HUD windows (`HudOverlays.tsx`: Setup bar, Recording bar, Countdown, Area selection drag overlay, Camera bubble)
- [x] **Keyboard shortcuts**: Space play/pause, Left/Right arrows ±1 frame nudge, Delete/Backspace remove selected zoom block, Ctrl+Z / Ctrl+Shift+Z / Ctrl+Y undo/redo
- [x] UI logic tests (`ui/src/editor/editorLogic.test.ts`: 8 native Node TS tests wired into `scripts/test.ps1` and `.github/workflows/ci.yml`)
- [x] Verify `npm --prefix ui run typecheck`, `npm --prefix ui run test`, `npm --prefix ui run build`, and `./scripts/test.ps1` green; commit Stage 9

---

## Stage 10 — Integration, Parity, Packaging & Release Readiness (DONE)

- [x] End-to-end cross-crate integration tests (`crates/nss-export/tests/integration_e2e.rs`: `AssetNameSafetyTests`, `CursorMappingTests`, `BackgroundParityTests`, `EdgeGapTests`, `RendererHeadroomTests` — 1080p busy-scene GPU p50 3.33 ms / p95 4.04 ms; plus 13 FFmpeg/wgpu tests in `export_parity.rs` and live WGC capture test in `nss-capture`)
- [x] Wire-format alignment audit: verified `ui/src/types.ts`, `editorLogic.ts`, and `InspectorPanel.tsx` match `nss-core` serde JSON keys/enums (`r16x9`/`p1080`/`solid`/`image.file`/`manual`/`displayID`/`cursorFile`/`mirror`/`zoomedScale`) and normalized style units (`0..1` fractions)
- [x] Port `_upstream/MANUAL_TEST.md` to `docs/MANUAL_TEST.md` for Windows 10/11
- [x] Privacy audit (zero network dependencies in manifests), unsafe audit (strictly confined to the 4 audited Windows FFI modules), platform-neutral core/motion check, and ASCII-only `.ps1` audit — all green
- [x] Release build verified (`cargo build --release -p notscreenstudio-win-app` + `target/release/notscreenstudio.exe --smoke-test` exit 0; MSI/NSIS config in `src-tauri/tauri.conf.json` & `scripts/build.ps1`)
- [x] Write `docs/RELEASE.md` & update `README.md` with complete features, build/package steps, and documented Windows limitations (`PORT_STAGE = 10`)
- [x] Final full gate (`./scripts/test.ps1`) green, update `PLAN.md` & `progress.md`, final commit
