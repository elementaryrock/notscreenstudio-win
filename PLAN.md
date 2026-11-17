# notscreenstudio → Windows port plan

Port of [`dotslashsimran/notscreenstudio`](https://github.com/dotslashsimran/notscreenstudio) (macOS 15, Swift 6 + SwiftUI + Metal, ~14.5k lines across 8 modules plus 499 tests) to a native-feeling Windows desktop app.

Status: **plan, Stages 0-4 complete.** Upstream is vendored read-only at `_upstream/` for reference.

| Stage | Status |
|---|---|
| 0 — Repository and toolchain bootstrap | **done** |
| 1 — `nss-core` | **done** |
| 2 — `nss-motion` | **done** |
| 3 — `nss-project` | **done** |
| 4 — `nss-render` | **done** |
| 5 — `nss-capture` | next |
| 6-11 | not started |

When a stage lands, flip its row and keep the notes in that stage's section accurate.

---

## 1. Goal and non-goals

### Goal

A free Windows screen recorder in the spirit of Screen Studio, matching upstream's feature surface:

- One-click record of a **display**, a **window**, or a **dragged area**, with optional **microphone** and **webcam**.
- A 3 s **countdown**, a floating **setup bar** and **recording bar** (pause/resume), a draggable **camera bubble** overlay, and a **tray icon** that reflects state.
- The recorder's own windows/non-capture-able overlays are **excluded from the recording**.
- **120 Hz** cursor + click logging, **real cursor shapes**, **press squish**, **idle fade**, **smoothing**, **motion blur**, **click ripples**.
- **Auto-zoom** from clicks planned into non-overlapping **zoom segments**, editable **zoom blocks** on a timeline.
- A GPU-composited frame: background (solid / linear or mesh gradient / image / wallpaper), padded frame with rounded corners and an analytic Gaussian shadow, zoomed screen content, cursor, ripples, camera bubble.
- **Editor**: 60 fps preview, 4-track timeline (video thumbs, zoom, camera, audio), trim handles, easing playhead, inspector with collapsible sections.
- **Home** grid of past recordings with thumbnails, rename / reveal / delete.
- **Autosave** ~1 s after the last edit, project library on disk.
- **MP4 export** at 1080p/4K, 60 fps, H.264/HEVC, with audio, in 16:9 / 1:1 / 9:16.
- **Offline by construction**: no network calls, no accounts, no telemetry.
- **Preview and export must produce identical frames** — same motion model, same renderer.

### Non-goals (deliberately out of scope for v1)

- macOS parity of *code* — this is a reimplementation, not a transpilation.
- Multi-platform builds (Linux/macOS). Windows-only, but see §3 for keeping the core platform-neutral anyway.
- Cloud upload, sharing links, accounts, plugin API, scripting.
- Mobile.
- 120 fps export, variable frame rate, AV1.

---

## 2. What upstream is, module by module

| Upstream target | Lines | Platform deps | Port difficulty |
|---|---|---|---|
| `NSSCore` — models, conventions, protocols | ~1.3k | `CoreGraphics`, `CoreMedia`, `CoreVideo` (types only) | **Low** — nearly mechanical |
| `NSSMotion` — spring camera, cursor smoothing, ripples, zoom planning | ~1.2k | none (pure `Foundation`) | **Low** — mechanical |
| `NSSProject` — `.nssproj` bundle, atomic writes, autosave, migration | ~800 | `AVFoundation`, `ImageIO`, `UniformTypeIdentifiers`, `Darwin` | **Low/medium** |
| `NSSCapture` — SCStream + AVCaptureSession writers | ~2.2k | `ScreenCaptureKit`, `AVFoundation`, `CoreMedia` | **High** — full rewrite |
| `NSSInput` — 120 Hz cursor/click/shape logging | ~900 | `CGEvent`, `NSEvent`, `NSCursor`, `QuartzCore` | **High** — full rewrite |
| `NSSRender` — Metal single-pass compositor + 274-line MSL shader | ~1.8k | `Metal`, `CoreVideo`, `ImageIO`, `Accelerate`, `AppKit` (cursor atlas) | **High** — shader ports well, host code is new |
| `NSSExport` — 60 fps MP4 writer, asset frame source | ~1.0k | `AVFoundation` | **High** — full rewrite |
| `NSSUI/Editor` — preview, timeline, inspector | ~3.2k | SwiftUI, `AVPlayerItemVideoOutput` | **High** — full rewrite |
| `NSSUI/Shell` — menu bar, HUD windows, hotkey, overlays | ~2.6k | AppKit, `Carbon.HIToolbox`, `NSScreen` | **High** — full rewrite |
| `NotScreenStudio` — app entry, wiring, smoke test | ~700 | AppKit, SwiftUI | **High** — full rewrite |

The valuable asset is not the Swift; it is **the behaviour**: the contracts in `NSSCore/Protocols.swift`, the geometry rules in `NSSRender/RENDERING.md`, the zoom-cluster algorithm in `NSSMotion/ZoomPlanner.swift`, the shader maths in `NSSRender/Shaders.swift`, and 499 tests that encode all the subtle parity rules. Those are what we port faithfully.

---

## 3. Target stack

**Tauri 2 + Rust core + React/TypeScript UI + wgpu (WGSL)** — chosen over Electron and .NET.

```
notscreenstudio-win/
├─ PLATFORM.md                  # this plan's companion: platform decision record
├─ crates/
│  ├─ nss-core/                 # models, conventions, protocols   ← port of NSSCore
│  ├─ nss-motion/               # spring, cursor, ripple, zoom plan ← port of NSSMotion
│  ├─ nss-project/              # .nssproj bundle, autosave         ← port of NSSProject
│  ├─ nss-capture/              # WGC + WASAPI capture              ← replaces NSSCapture
│  ├─ nss-input/                # low-level mouse hook, cursor shape ← replaces NSSInput
│  ├─ nss-render/               # wgpu compositor + WGSL            ← replaces NSSRender
│  └─ nss-export/               # frame pump + FFmpeg mux            ← replaces NSSExport
├─ src-tauri/                   # Tauri app: commands, tray, HUD windows, IPC
├─ ui/                          # React editor + home (Vite)
├─ shaders/                     # WGSL sources (embedded at compile time)
├─ scripts/                     # build.ps1, dev.ps1, make-fixtures.ps1
└─ docs/                        # MANUAL_TEST.md, RENDERING.md (ported), ADRs
```

**Toolchain confirmed present on this machine:** Rust 1.95 (msvc + gnu toolchains), VS 2022 build tools, Node 26.7, FFmpeg 8.1.1, WebView2 runtime 157. Note: the active `cargo` default is `stable-x86_64-pc-windows-gnu`; the plan pins **`x86_64-pc-windows-msvc`** and records this in `rust-toolchain.toml`.

**Dependency policy** (mirrors upstream's "no third-party dependencies without approval"):
- Rust: `windows` (WinRT + Win32), `wgpu`, `serde`/`serde_json`, `tokio`, `anyhow`/`thiserror`, `parking_lot`, `image` (decode + resize), `sha2`, `uuid`, `time`.
- UI: `react`, `vite`, `typescript`, `zustand` (state), `tailwind` or CSS modules, `@tauri-apps/api`. No component framework that dictates look and feel.
- External binaries: **FFmpeg** (bundled or discovered) for encoding/muxing. Nothing else.
- Anything new gets an entry in `docs/ADR/`.

### Why Rust/wgpu over Electron

- WGSL is a near-mechanical translation of the MSL shader (both are trivially different from HLSL; the shader is 274 lines with a flat `float4` uniform array and one full-screen triangle).
- `Windows.Graphics.Capture` and the Win32 mouse/DPI APIs are reachable directly through the `windows` crate — no N-API addon boundary in the 120 Hz hot path.
- Capture → GPU → encode can stay on one D3D11/wgpu device end-to-end.

### Where Swift *platform* types leak, and how they are replaced

| Upstream | Windows replacement |
|---|---|
| `CVPixelBuffer` | `nss_render::Frame` — a wgpu texture handle + `BufferIdentity` (an IOSurface analogue) |
| `CVPixelBufferPool` | `FramePool` (recycled wgpu textures) |
| `ScreenCaptureKit` `SCStream` | `Windows.Graphics.Capture` (`Direct3D11CaptureFramePool` + `GraphicsCaptureSession`) |
| `CGEvent` / `NSEvent` monitors | `WH_MOUSE_LL` low-level hook + `GetCursorPos` sampling timer |
| `NSCursor.currentSystem` | `GetCursorInfo` + `GetIconInfoEx` + image hashing (see §6.2) |
| `AVCaptureSession` (mic) | WASAPI capture (`IAudioClient3`) → 48 kHz float PCM |
| `AVCaptureSession` (camera) | Media Foundation source reader (`IMFSourceReader`), BGRA frames |
| `AVAssetWriter` + `AVAssetWriterInputPixelBufferAdaptor` | raw BGRA pipe into FFmpeg (`libx264`/`libx265`) |
| `AVAssetReader` frame source | FFmpeg demux/decode reader (`nss-export::reader`) |
| `AVPlayerItemVideoOutput` (editor preview) | our own frame source + a wgpu preview surface (no player needed) |
| `Metal` `MTLDevice` / shaders | `wgpu` on D3D12; WGSL |
| SwiftUI | React + CSS, staged in a WebView2 window |
| `NSStatusItem` / menu bar | tray icon (`tray-icon`) with dynamic icon + tooltip |
| `NSPanel` HUD overlays | Tauri borderless always-on-top windows with `WS_EX_TOOLWINDOW` + `WDA_EXCLUDEFROMCAPTURE` |
| `Carbon` global hotkey (⌃⌘R) | `RegisterHotKey` (default **Ctrl+Shift+R**, configurable) |
| `NSWorkspace` reveal/trash | `SHOpenFolderAndSelectItems` / `IFileOperation` (Recycle Bin) |
| `~/.Movies/notscreenstudio` | `%USERPROFILE%\Videos\notscreenstudio` |
| `NSHighResolutionCapable` / Retina | per-monitor DPI awareness v2; physical pixels everywhere in the core |

**Hard rule:** `nss-core` and `nss-motion` must not link `windows`, `wgpu`, or `tauri`. Their tests run on any host. This keeps the 60-fps/parity invariants testable in CI without a GPU.

---

## 4. Windows-specific behaviour differences — decide up front

These are real API differences, not porting sloppiness. Each gets a decision and a note in `docs/ADR/`, so nobody "fixes" them into bugs later.

1. **Consent is unavoidable for window capture.** macOS lets SCK enumerate windows and capture silently. `Windows.Graphics.Capture` requires either the system picker (`GraphicsCapturePicker`) or `GraphicsCaptureItem::TryCreateFromWindowId` on a window we are entitled to. **Decision:** ship our own source picker built from `EnumWindows` + `DwmGetWindowAttribute(DWMWA_EXTENDED_FRAME_BOUNDS)`, then construct the item with `TryCreateFromWindowId`, and fall back to the system picker when that is denied. Record which path was used.
2. **The yellow capture border exists.** `GraphicsCaptureSession::SetIsBorderRequired(false)` needs the `graphicsCaptureWithoutBorder` capability and `GraphicsCaptureAccess::RequestAccessAsync` consent. **Decision:** request borderless once at first record, degrade silently to the border, and surface it as a line in Settings ("Windows shows a yellow border while recording — Windows 11 can turn this off"). **Never** try `SetWindowDisplayAffinity` tricks to fake it.
3. **Excluding our own overlays is inverted.** Upstream *excludes* its windows from the capture via SCK's content filter. On Windows, `SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE)` makes a window invisible to WGC — the same effect, per-HWND. **Decision:** apply `WDA_EXCLUDEFROMCAPTURE` to every HUD/overlay/WebView2 window; verify with a self-test that captures a frame and asserts the overlay pixels are absent.
4. **Cursor is either-or.** WGC can *burn the cursor into* the capture; upstream instead *omits* it and draws its own cursor from the log. **Decision:** capture with cursor deliberately excluded, then draw the cursor ourselves from the log — otherwise we would double the cursor. This is the only way to get smoothing, motion blur, shapes and press-squish at all.
5. **Cursor *shape* has no stable identity on Windows.** `GetCursorInfo` returns an `HCURSOR`; there is no public "this is an I-beam". **Decision:** map well-known shared handles (`LoadCursorW(IDC_*)`, matched by handle) first; otherwise hash the cursor bitmap from `GetIconInfoEx` (mask+colour) and classify against a fingerprint table built once from the system cursors. Unrecognised shapes → `arrow`, matching upstream's `.unknown` behaviour. See §6.2 for the fallback ladder.
6. **HDR and pixel format.** With HDR enabled, WGC frames are `R16G16B16A16_FLOAT`, not BGRA. **Decision:** accept both, tone-map `scRGB` → sRGB in the compositor's background pass; export stays SDR 8-bit in v1 (documented).
7. **DPR / mixed-DPI.** A window dragged between monitors changes capture size. **Decision:** capture at physical pixels; store the source pixel size and per-display DPI in the project; normalise all motion coordinates to the source (0...1, top-left origin) exactly as upstream does, so a size change does not shift the zooms.
8. **Audio clock.** macOS ties PTS to one host clock. WASAPI pushes packets on its own clock and drifts. **Decision:** use the **screen** frame stream as the master clock; resample mic audio to 48 kHz and timestamp it against the screen frame counter, with drift compensation (see §7.5).
9. **Menu bar with no Dock icon → tray icon.** Windows tray cannot render a live red dot as cheaply as macOS. **Decision:** two or three pre-rendered icon states (idle / recording / paused) swapped on transition, with the elapsed time in the tooltip (refreshed once a second, not per frame).

---

## 5. Port order, with exit criteria

Eleven stages. Each stage ends in a green build and a checkable result. Stages 1–3 can run without any capture or GPU work; that is deliberate, so the deterministic core is proven before anything platform-specific lands.

### Stage 0 — Repository and toolchain bootstrap
- `git init`, add `_upstream/` as a **read-only vendored reference** (and a `.gitattributes` + `README` note stating it is not built), or skip it and add upstream as a git remote read-only. Decision: keep it vendored for offline reference, excluded from the workspace.
- `rust-toolchain.toml` pinning `x86_64-pc-windows-msvc`.
- Cargo workspace with the seven crates, empty modules, `#![deny(warnings)]` on new code.
- `src-tauri` + `ui` scaffolded with `create-tauri-app` (React + TS), Tauri 2.
- `scripts/dev.ps1`, `scripts/build.ps1`, `scripts/test.ps1`.
- GitHub Actions: Windows runner → `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, `pnpm build`, `cargo tauri build`.
- Port `AGENTS.md` conventions into `CONTRIBUTING.md` (times are `f64` seconds; positions normalised 0...1 top-left; motion is a pure function of time; no per-frame allocation; no network APIs).

**Exit:** `cargo test` and `cargo tauri dev` both run; empty window opens; CI green.

### Stage 1 — `nss-core` (port of `NSSCore`)
Port, field for field, preserving the serialised shape:
- `Basics` (`MediaTime`, `NormalizedPoint`, `NormalizedSize`, `RGBAColor`, `PixelSize`).
- `Style` (`Background` incl. `solid`/`gradient`/`image`/`mesh`/`systemWallpaper`, `Shadow`, `AspectRatio`, `CursorStyle`, `CameraBubble`, `CameraShape`, `FrameStyle`).
- `Project`, `Timeline` (`ZoomBlock`, `ZoomFocus`, `Trim`), `Recording` (`RecordingOptions`, `RecordingAssets`, `CaptureSource`, `RecordingState`), `Cursor` (`CursorTrack`, `CursorSample`, `ClickEvent`, `CursorShape`, `ClickKind`, `MouseButton`), `FrameState` (`CameraState`, `RippleState`, `CursorState`), `BackgroundPresets`.
- Protocols as traits: `MotionModel`, `FrameSource`, `FrameRenderer`, `ProjectStore`, `RecordingSession`, `InputLogger`, `ProjectLibrary`; `NSSError` as an enum.
- **Coding policy preserved:** every persisted field decodes with a default (`#[serde(default)]`), so an older `project.json` still loads. Unknown `CursorShape` raw values decode to `unknown` via a custom `Deserialize`.
- Keep the on-disk **JSON key names identical to upstream** so fixtures from `Tests/` can be reused as golden files.

**Exit:** all upstream `NSSCoreTests` behaviours reproduced (`BackgroundModelTests`, `CursorStateCodingTests`, `ZoomBlockModelTests`, `Round2ContractTests`) as Rust tests, including the exact migration rules (circle→circle, roundedRect→square+roundness, squircle→square@0.6, rectangle→wide@0).

**Done.** 65 tests: one Rust module per ported Swift file (`basics`, `style`, `timeline`, `cursor`, `frame_state`, `project`, `recording`, `background_presets`) plus `tests/compat.rs`, which pins the exact persisted JSON (key sets, enum tags, epochs, uppercase UUIDs) and loads a v1 Mac-era `project.json` fixture. Notes:
- Persisted key names now match Swift exactly; the first draft of `Shadow`, `CursorStyle`, `CursorTrack` and `Timeline` silently emitted `offset_x`, `motion_blur`, `sample_rate`, `zoom_blocks`. `tests/compat.rs` fails loudly on any such rename, so this class of bug cannot recur silently.
- `FrameSource`/`FrameRenderer` are deliberately **not** in `nss-core` (see ADR 0009): they traffic in `CVPixelBuffer` upstream and belong to `nss-render` in Stage 4.
- `serde_json` now enables `float_roundtrip`, found by a property test: without it a stored `f64` could come back 1 ULP off, so save/load equality on user projects was not guaranteed.

### Stage 2 — `nss-motion` (port of `NSSMotion`)
- `Spring` (closed-form critically-damped spring, `MotionTuning`).
- `Viewport` clamping — the "never show a gap past the source edge" rule.
- `ZoomPlanner` — click clustering, merge/split rules, 0.3 s minimum block, non-overlap, `followFrom` dead zone, `1.1...4.0` clamp.
- `SpringMotionModel` / `LinearMotionModel` — cursor smoothing, idle-fade ramp, press squish, ripple lifetime `(1-p)^1.5`, motion-blur vector.
- Determinism: integrate from `t = 0` at a **fixed step**; never read the wall clock. Same `t` ⇒ same `FrameState`, in any order, from any thread.

**Exit:** property tests reproduce `NSSMotionTests` + `ViewportPropertyTests` + `EdgeGapTests`; a fuzz test asserts `frameState(at:)` is order-independent and byte-stable for a fixed seed.

**Status: done** (`crates/nss-motion`, 62 tests: 51 unit + 5 `viewport_guarantee` + 5 `stored_zoom` + 1 `performance`).

Ported: `spring.rs` (analytic `evolve` for all three damping regimes + matrix `SpringStepper`), `motion_tuning.rs` (all upstream defaults and the guard rails: non-finite `simulation_rate` → 240, `max_duration` ≤ 0 → 24 h, `press_scale` clamped `0.1..=1.0`, `max_steps = max_simulation_frames.max(2) - 1`), `viewport.rs`, `zoom_planner.rs`, `spring_motion_model.rs` (fixed-step table, dead-zone follow, idle fade, ripples, `stable_shapes` flicker filter, press squish, `CameraPlanner`/`PressTracker`/`ActivityTracker`/`RawCursor`), `linear_motion_model.rs`, plus the `factory()` seam that preview and export both call so the two cannot drift.

Notes for later stages:

- **`CameraState::clamped_to_source` became a free function in `nss-motion`, not an inherent method** — `CameraState` lives in `nss-core`, which may not grow motion concerns (ADR 0009). `SourceViewport` is reached through the `CameraViewport` trait so renderer code can still write `camera.source_viewport()`.
- **The clamp is a loop of `next_down`/`next_up` nudges**, not a second `clamp`: `0.5/scale` is not exact, so the naive bound can still leave `center + half` one ULP outside `1.0`. This costs MSRV 1.86 (`f64::next_up`/`next_down`), hence the workspace bump from 1.85.
- **The simulation table is the determinism contract.** `frame_state` interpolates between two rows of a `Vec<SimFrame>` (36 bytes/row, 240 Hz ⇒ 5.2 MB per 10 minutes, capped by `max_simulation_frames`). A 10-minute recording builds in ~12 ms and looks up in ~0.1 µs (debug build) — up to 30x under the upstream budget.
- **The strongest guarantee is a test, not a comment.** `tests/viewport_guarantee.rs` replays upstream's property test with adversarial tuning (underdamped springs, `dead_zone` up to 1.5, scales up to 14, 7 source aspects, 3 output aspects, clicks and manual focuses deliberately outside `0..1`) and additionally mirrors `RenderLayout`'s content rect so it checks the corners the renderer actually samples. Keep this test when `nss-render` lands: `nss-render` must reuse `rendered_source_corners`' maths instead of inventing its own.
- `tests/performance.rs` asserts wall-clock budgets only with `NSS_BENCH=1`, matching upstream; a flaky perf gate in a shared CI VM teaches people to ignore the suite.

### Stage 3 — `nss-project` (port of `NSSProject`)
- `.nssproj` bundle = directory with `project.json`, `project.json.bak`, `cursor.json`, media files, thumbnail.
- Atomic write (temp + rename) and backup fallback on corrupt load.
- `AutosaveDebouncer` (1 s, replaces pending, flush on exit).
- `FileProjectLibrary` (summaries newest-first, thumbnail generation, rename, delete → **Recycle Bin**, not `remove_dir_all`), `RecentProjects`.
- `ProjectMigrator` for schema v1 → v2.
- Library root `%USERPROFILE%\Videos\notscreenstudio`.

**Exit:** upstream `NSSProjectTests` + `ProjectLibraryTests` + `BackgroundMigrationTests` behaviours reproduced, including "crash during save leaves a loadable project".

**Status: done** (`crates/nss-project`, 70 tests: 58 unit + 7 `background_migration` + 5 `crash_recovery`).

Ported: `atomic_file.rs` (temp + sync + swap, with a Windows retry window), `debounce.rs` (`AutosaveDebouncer` on one worker thread, injectable `DebounceClock` + `ManualClock`), `migrator.rs` (`ProjectMigrator`, v1 wallpaper → mesh), `store.rs` (`FileProjectStore`: create/save/load/cursor track/autosave/list/recents, name sanitising), `library.rs` (`FileProjectLibrary`: summaries, FNV-1a path ids, thumbnails, rename, delete), `recent_projects.rs`, `windows_trash.rs`.

Notes for later stages (full reasoning in **ADR 0010**):

- **`project.json` is written pretty + sorted keys, `cursor.json` compact.** Byte-identical output to the Mac build is *not* promised (Apple writes `"key" : value`); load-compatibility in both directions is, and is what the tests assert.
- **`nss-project` is `#![deny(unsafe_code)]`, not `forbid`.** The Recycle Bin has no std equivalent, so `windows_trash.rs` holds one `SHFileOperationW` call with an `#[allow(unsafe_code)]` and a safety comment. CI greps for `unsafe` outside that file, so the exemption cannot spread.
- **The autosave debouncer is a thread, not a runtime.** One worker parks outside the state lock in `clock.wait_until(deadline, epoch)`; the `epoch` counter is what makes that wait interruptible so a new schedule or `Drop` returns immediately. `flush()` and `exclusive()` share the worker's `write_lock`, which is what makes "explicit save wins over an in-flight autosave" true rather than probable.
- **Windows quirks are in the API, not hidden:** `ProjectError::is_transient()` for sharing violations (Explorer/Defender hold files for milliseconds), `sanitize_name` strips reserved device names and trailing dots, extensions are compared case-insensitively, and `path_key` folds case so two spellings share one autosave timer.
- **Thumbnails go through the `Thumbnailer` trait** (`FfmpegThumbnailer` in production, `NoopThumbnailer` in tests) because FFmpeg is not on CI and because Stage 4 may substitute the GPU rasteriser. Same seam for `Trash`.
- **Gap left open:** the library root is `%USERPROFILE%\Videos\notscreenstudio` from the environment, so a Videos folder redirected to OneDrive is not followed. Stage 8 replaces it with `SHGetKnownFolderPath(FOLDERID_Videos)`.
- **`--smoke-test` now creates a real bundle** in a scratch directory (save → load → cursor track → library listing) instead of only round-tripping JSON, so a broken bundle layout fails the build.

### Stage 4 — `nss-render` (port of `NSSRender`) — the long pole
1. **WGSL translation of `Shaders.swift`.** One full-screen triangle, one fragment entry point, same pass order (background → screen shadow → screen content → ripples → cursor → camera bubble → sRGB encode + dither on background only). Same `Slot` enum, generated into WGSL `#define`-equivalents (const array indices) so the Rust uniform writer and the shader cannot drift. Translate `oklabToLinear`, `sdRoundBox`, `erfApprox`, `softShadow`, `sampleFootprint`, the trilinear mip sampling, and the linear-light blending exactly.
2. **`RenderLayout`** — a 1:1 port. This crate's geometry is the parity contract: `contentRect`, `clampedCamera`, `outputPoint`/`sourcePoint`, `cameraBubbleFullRect`, `cameraBubbleRect`, `bubbleAnchor`, `cursorHeight`, `rippleRadius`, `cursorBlurVector`.
3. **`FrameRenderer`** on wgpu: pipelines/samplers/textures created once at init, zero per-frame allocation except the unavoidable bind groups; serialised by a lock; `render` returns after GPU completion (fence wait).
4. **`BackgroundRasterizer`** (CPU, same maths) for picker thumbnails, plus the parity test GPU-vs-CPU.
5. **`ImageBlur`** — separable Gaussian in linear light as a WGSL compute kernel, cached per (path, mtime, size, blur rounded to 1/100), mip plan capping ~19 taps.
6. **`CursorAtlas`** — rasterise system cursors once per process at 8 px/pt into shelf-packed `rgba16Float` with a full mip chain, 32 px cell padding, per-cell point size + hotspot, `CursorAtlasCache` per device. Arrow fallback if a shape is unavailable.
7. **`PassthroughRenderer`** as the fallback when no D3D12 adapter is available.
8. **HDR input path**: accept `Rgba16Float`, tone-map in the background pass.

**Status: done.** `nss-render` draws real frames on wgpu: `layout.rs` (the parity contract, a 1:1
port of `RenderLayout`), `shader.wgsl` (the single-pass compositor, translated slot for slot),
`uniforms.rs` (the 46-slot block shared with WGSL), `cursor_atlas.rs` (18 vector cursors at 8x with
padding and a mip chain), `rasterizer.rs` (the CPU twin of the background maths), `image_blur.rs` +
`blur.wgsl` (two-pass separable Gaussian in linear light), `frame.rs` (`Image`/`Frame`/`FrameSource`),
`passthrough.rs` and `renderer.rs` (pipelines, caches, readback). 95 tests: 67 unit, plus
`tests/metal_renderer.rs` (17), `tests/background_render.rs` (8) and `tests/cursor_render.rs` (3)
ported from `MetalRendererTests`/`BackgroundRenderTests`/`Round2RenderTests`. GPU tests skip when no
adapter exists. `docs/ADR/0011-renderer-parity.md` records the translation decisions and
`docs/RENDERING.md` describes the wgpu frame pipeline.

Four deviations, all deliberate:

- **Frames are packed RGBA8 sRGB, not IOSurface BGRA.** One upload per frame instead of a zero-copy
  wrap; that is the price of not writing per-vendor shared-surface code. `Image::to_bgra` converts
  once at the FFmpeg/Windows-compositor boundary. ADR 0011 §1.
- **The blur is two fullscreen passes, not a compute kernel.** Same plan and same maths, and it keeps
  the targets to formats that are core WebGPU. Each pass is its own submit: a shared encoder lets the
  second pass's `write_buffer` overtake the first pass (this was a real bug, caught by the blur
  tests). ADR 0011 §6.
- **The cursor atlas is our own art, not `NSCursor`.** Windows has no stable per-DPI system cursor
  bitmap to hash a frame against. `Cell::is_system_image` is always `false`; the arrow's hotspot is
  the body's apex so the tip lands on the cursor position. ADR 0011 §5.
- **HDR input is not implemented.** Upstream's `Rgba16Float` tone-map path is deferred until
  `nss-capture` (Stage 5) decides whether WGC HDR frames exist on this platform at all; the renderer
  currently takes sRGB RGBA8 in and out. Recorded here so a later stage does not assume it is done.

**Exit:** `RENDERING.md` ported to `docs/RENDERING.md` describing the wgpu reality; `BackgroundRenderTests`/`Round2LayoutTests`/`Round2RenderTests`/`MetalRendererTests` behaviours reproduced (`*RendererTests` renamed `*RendererTests`, GPU tests `.enabled(if:)`-equivalent via `#[ignore]` + a `--ignored gpu` CI job); the `previewAndExportPoolsGiveByteIdenticalFrames` determinism test reproduced; 4K frame under the 8 ms budget on this machine (benchmark, not a CI gate).

### Stage 5 — `nss-capture` (replaces `NSSCapture`)
- `CaptureSourceCatalog`: enumerate displays (`EnumDisplayMonitors`) and windows (`EnumWindows`, visible + non-cloaked + non-tool + non-empty title), with DWM extended frame bounds; APP_USER_MODEL_ID-friendly display names.
- `AreaSelection`: full-screen dimmed topmost overlay for the drag-to-select; result as a virtual-desktop rect; area capture = display capture + crop rect in the compositor (WGC cannot capture an arbitrary rect).
- `ScreenRecorder`: `Direct3D11CaptureFramePool` → `Frame` (persistent-bitmap-resident texture, zero copy where possible), `GraphicsCaptureSession`, `Closed` event → `CaptureEvent::SourceEnded`, `SetIsCursorCaptureEnabled(false)`.
- `WarmableRecordingSession` parity: `preflight` → `prepare` → `beginRecording` (t = 0 at the first screen frame) → `cancelPrepared`, with frames held back during the countdown, exactly as upstream.
- `DeviceRecorder` ×2: WASAPI loop-free mic capture and Media Foundation camera capture, both written as raw/packaged temp files inside the bundle.
- `PauseClock` + `FrameGate`: paused time is not in the recording; clicks while paused are ignored.
- `SampleFileWriter`: screen frames to a compact intermediate (see §7) rather than MOV.
- Own-window exclusion: apply `WDA_EXCLUDEFROMCAPTURE` to every app window; `refreshExclusions` after HUDs appear.
- Permissions: no macOS-style TCC screen permission exists; handle *only* camera/mic consent (Windows Settings privacy keys, readable via registry) and file-write failures.

**Exit:** a 30 s recording of each source type produces a loadable `.nssproj` with correct `startHostTime`, source pixel size, and audio/video that stays in sync (< 1 frame drift over 5 min). `CaptureLogicTests`, `FrameGateTests`, `PauseTimelineTests`, `CaptureFilterTests` behaviours reproduced.

### Stage 6 — `nss-input` (replaces `NSSInput`)
- `SystemInputLogger`: `WH_MOUSE_LL` hook (move/down/up, left/right/other) into a lock-free ring buffer; a `120 Hz` high-resolution timer samples `GetCursorPos` on a dedicated thread.
- Cursor shape sampler at 30 Hz on its own low-priority thread (§4.5 ladder: shared-handle match → bitmap hash → arrow).
- `SourceGeometry`: global virtual-desktop rect → normalised source space (top-left origin), mixed-DPI correct, multi-monitor negative coordinates handled.
- `CursorRecorder` + `SamplingSchedule`: stamps every event on the same monotonic clock as `RecordingAssets.startHostTime`, and `stop(recordingStartHostTime)` **rebases** so t = 0 is the first screen frame; pre-origin events dropped/clamped.
- Admin-elevated foreground apps, secure desktop (UAC) and full-screen exclusive games are not observable by a low-level hook — log as a known limitation, don't crash.

**Exit:** upstream `NSSInputTests` + `CursorShapeTests` + `CursorMappingTests` + `StoredZoomTests` behaviours reproduced; a live test shows ≥ 110 Hz effective sampling with < 2 % dropped samples while the UI is busy.

### Stage 7 — `nss-export` (replaces `NSSExport`)
- `FrameSource` (`AssetFrameSource`): sequential decode, **at-or-before** frame selection for both screen and camera (the shared rule that makes preview and export agree); camera returns `None` when there is no camera track.
- `AudioPump`: trim + resample + drift compensation; AAC 192 kbps.
- `MP4Exporter`: for output frame `i`, `t = trim.start + i / fps` → `motion.frameState(t)` → decode at-or-before → `renderer.render` into a pooled buffer → append. Strictly one frame at a time, gated on the encoder's readiness, bounded memory.
- Encoder: FFmpeg child process (`-f rawvideo -pix_fmt bgra`, `libx264`/`libx265`, `-movflags +faststart`), or `ffmpeg-sidecar`. Output to a temp file next to the destination and **moved into place only on clean finish** — a crash/cancel never leaves a partial file.
- Progress/cancel via an atomic + channel; cancel kills the encoder and removes the temp file.
- Resolution presets: 1080p/4K by shorter side with the documented 16:9, 1:1, 9:16 sizes.

**Exit:** `EndToEndExportTests` + `NSSExportTests` behaviours reproduced; an export equals the preview frame-for-frame on a fixture (SSIM ≥ 0.999 against the preview-rendered PNGs); cancel leaves no file; 60 fps output verified with `ffprobe`.

### Stage 8 — Shell: tray, HUD windows, hotkey, overlays
- Tray icon with state (idle/recording/paused), tooltip with elapsed time, menu: Record, Pause/Resume, Stop, Permissions/Settings, Quit.
- Setup bar (source name, camera toggle + device menu, mic toggle + device menu, Record, Close), Recording bar (pulsing dot, elapsed, pause, stop), Countdown overlay (3/2/1, Off/3 s/5 s, Escape or click cancels).
- Area selection overlay (drag rect, live size readout, Escape cancels).
- Camera bubble overlay: draggable, four shapes, roundness, border, mirror, size; position persists into the project.
- Globals: `RegisterHotKey` default **Ctrl+Shift+R** (configurable, with conflict detection), `RegisterHotKey` for pause.
- All overlays are borderless, always-on-top, `WS_EX_TOOLWINDOW`, `WDA_EXCLUDEFROMCAPTURE`, and DPI-aware.
- Settings persistence (source, devices, countdown, hotkeys, aspect, output dir).

**Exit:** upstream `BarStyleTests`, `ShellCountdownTests`, `ShellLogicTests`, `ShellModelTests`, `RecorderFlowTests` behaviours reproduced; a manual pass shows the HUD never appears in a recording of itself.

### Stage 9 — UI: home + editor (React)
- **Home**: responsive grid, cards with thumbnail/name/date/duration, newest first, context menu (Rename / Show in Explorer / Delete to Recycle Bin), empty state with a Record button.
- **Editor shell**: dark theme; top bar with *Recordings*, editable project name, undo/redo, aspect picker, Export; centre preview; right inspector; bottom timeline.
- **Preview**: an IPC-driven wgpu surface (or a per-frame encoded texture) that renders from the same `nss-motion` + `nss-render` path as export. Target 60 fps; measure with a frame-time HUD in dev, not by eyeballing.
- **Timeline**: four tracks (Video thumbnails, Zoom blocks with auto-sparkle, Camera, Audio), yellow trim handles, orange click dots, easing playhead (eased on jump, exact on drag).
- **Inspector**: six collapsible sections (Background, Frame, Cursor, Zoom, Camera, Audio) with remembered open/closed state; background picker tabs (Wallpapers from `%WINDIR%\Web\Wallpaper` + `%APPDATA%\Microsoft\Windows\Themes`, Gradients with angle slider, Colours + custom picker, Image with 8 recents); blur/padding/corner/shadow; cursor size/smoothing/blur/ripples/hide-when-idle; zoom level/focus/ease in/out with *Follow cursor* or *Fixed point* (type X/Y or click in preview); bubble shape/roundness/size/border/mirror/shrink-on-zoom.
- **Undo/redo** over every edit; a slider drag is one step.
- **Export sheet**: resolution, fps, codec, destination, progress with cancel, "reveal when done".
- Keyboard: Space = play/pause, arrows = nudge, Delete = remove block, Ctrl+Z / Ctrl+Shift+Z.

**Exit:** upstream `EditorDesignTests`, `EditorLogicTests`, `ZoomEditingTests`, `EditorPreviewEngineTests`, `EditorPlayheadAndBubbleTests`, `BackgroundPickerTests`, `HomeModelTests` behaviours reproduced; a UX walkthrough of first-launch → record → edit → export with no dead ends.

### Stage 10 — Integration, parity, packaging
- Wire record → project → editor → export end-to-end with real (not fixture) media, for all three source types and all three aspect ratios at 1080p and 4K.
- Port `MANUAL_TEST.md` to a Windows checklist with screenshots; run it once fully.
- Performance: preview holds 60 fps while recording; export is at least 1× realtime for 1080p H.264 on this machine; no per-frame allocations in the render loop (verify with a dev allocator counter).
- Privacy audit: `grep` for any HTTP/analytics/sentry dependency; CI job fails the build if `reqwest`/`hyper`/network crates appear outside dev-dependencies.
- Packaging: `cargo tauri build` → MSI + NSIS installer; sign if a certificate is available (unsigned is a documented limitation); bundle or detect FFmpeg and state the licence; first-run wizard for camera/mic consent and the borderless-capture consent.
- Release checklist in `docs/RELEASE.md`.

**Exit:** an installed MSI on a clean Windows 11 machine records, edits and exports an MP4 that matches the preview; `MANUAL_TEST.md` fully ticked.

---

## 6. The riskiest five things, and the mitigation

| Risk | Why it bites | Mitigation |
|---|---|---|
1. **Shader parity drift** | MSL → WGSL differences (precision, `select`, clamp semantics, mip bias, sRGB views) silently change colours. | Golden-image tests: render fixed fixtures at 1080p/4K and compare against upstream's own exported PNGs (generated once from a Mac, committed as fixtures) with a per-pixel tolerance. Fail CI on drift. |
2. **Cursor shape detection** | No stable cursor ID on Windows; hashing is heuristic. | Handle-match first (covers `IDC_*`), then a committed fingerprint table, then `arrow`. Log unknown hashes in dev so the table can grow. Ship a test that every `CursorShape` maps to *something* drawable. |
3. **A/V sync drift** | WASAPI and WGC clocks are independent. | Screen frames are the master clock. Measure drift in the manual test (5 min, clap at start and end); assert < 1 frame. Resampler + drift compensation in `AudioPump`. |
4. **Yellow border / consent surprise** | Users will think it is a bug, or think capture is broken when the picker falls back. | Ask for borderless consent at first record with a one-line explanation; if denied, tell them once in the recording bar's tooltip; document in Settings and the README. |
5. **Tauri IPC on the 60 fps preview path** | Serialising 60 frames/s of JavaScript → Rust round trips will stutter. | Never IPC per frame. The preview renders in Rust; the UI sends only state changes (playhead, style edits). If a canvas must be shared, use a shared GPU surface/texture handle, not pixel copies. |

Additional known limitations to write down honestly in the README: no capture of UAC/secure desktop, elevated apps' input is not observable by a non-elevated hook, HDR exports are SDR, and area capture is a cropped display capture (so an area spanning two monitors needs the display that contains it).

---

## 7. Cross-cutting technical decisions

**7.1 Time and coordinates.** `MediaTime = f64` seconds from the first screen frame. Positions normalised 0...1, origin **top-left** in source space. All Win32 bottom-up/screen-space conversions happen in `nss-capture`/`nss-input` and never leak into `nss-core`.

**7.2 Determinism.** Motion is a pure function of `t`. Springs integrated from 0 at a fixed step. Preview and export share one implementation and one renderer — that is the entire parity story, and it is non-negotiable.

**7.3 Persistence format.** Identical `.nssproj` layout and JSON keys to upstream, so Mac projects and upstream test fixtures are loadable as golden data. Media files are ours (BGRA intermediate), so *media* is not cross-compatible even though *projects* are. Documented.

**7.4 Rendering contract.** Pixel-in, pixel-out. Buffers are wgpu textures with `Bgra8Unorm`/`Bgra8UnormSrgb` views, `Rgba16Float` for HDR input, zero copy on one device. No per-frame allocation. `render()` is synchronous (fence-waited) so preview and export behave identically.

**7.5 Audio.** 48 kHz, stereo (or mono upmixed), float internally, AAC 192 kbps on export. Trim applies to picture and sound together. Paused intervals are cut, not stretched.

**7.6 Privacy.** No network APIs in any crate's dependency tree. CI check as described in Stage 10. No crash reporter, no analytics, no update pinger.

**7.7 Accessibility and i18n.** Not in v1, but all UI strings go through a single `strings.ts` so it is not a rewrite later. Respect the system light/dark preference for *chrome* (the editor itself stays dark, as upstream).

---

## 8. Test strategy

- **Unit**: one Rust test module per ported Swift test file, named the same, so coverage can be diffed against upstream.
- **Golden fixtures**: reuse upstream JSON (`project.json`, `cursor.json`) as committed fixtures; commit a set of reference PNG frames rendered on macOS for shader parity.
- **Property tests** (`proptest`): viewport clamping never gaps, zoom segments never overlap and respect the 0.3 s floor, `frameState` order-independence, normalised-point round-trips.
- **GPU tests**: `#[ignore]`d, run on a Windows CI runner with a software adapter (WARP/D3D12 `Microsoft Basic Render Driver`) where possible; skip cleanly when unavailable, exactly like upstream's `.enabled(if:)` policy.
- **Integration**: synthetic bundle → store → motion → render → FFmpeg → probe the output with `ffprobe`. Includes cancel-leaves-no-file and export-equals-preview.
- **Manual**: `MANUAL_TEST.md` ported and kept current (it is 28 KB upstream — treat it as a real artefact, not a stub).

---

## 9. Milestones and what "done" means

| Milestone | Stages | Done when |
|---|---|---|
| M1 — Core proven | 0–3 | `cargo test` reproduces the deterministic half of upstream's suite; no GPU, no capture |
| M2 — Frames are right | 4 | The compositor renders every background/cursor/bubble case; golden images match macOS |
| M3 — It can record | 5–6 | All three source types + mic + camera + 120 Hz input produce a correct project |
| M4 — It can export | 7 | A 1080p and a 4K MP4 match the preview |
| M5 — It is usable | 8–9 | Tray/HUD/overlays and the full editor work without dead ends |
| M6 — It ships | 10 | Signed-or-documented MSI on a clean machine; manual test ticked; privacy audit green |

Rough weight: M1 ~20 %, M2 ~25 %, M3 ~20 %, M4 ~10 %, M5 ~20 %, M6 ~5 %. M2 and M5 dominate; plan the schedule around them.

---

## 10. Open questions to settle before Stage 4

1. **FFmpeg distribution** — bundle a licensed build, or detect an existing install and link to the download? (Affects installer size and the licence file.)
2. **Preview transport** — native child-window with a wgpu surface (best perf, harder layout) vs. a shared texture into the WebView (simpler chrome, needs WebView2 texture interop). Decide with a spike in Stage 4, not later.
3. **Minimum Windows version** — Windows 10 2004+ as the floor (WGC is fine, `IsBorderRequired` absent), or Windows 11 only (cleaner borderless story)?
4. **Codec availability** — ship `libx264` only (universal), or also `libx265`/hardware `h264_nvenc`/`qsv`/`amf` with detection?
5. **Do we keep upstream's `_upstream/` vendored in the repo, or just as a remote?**

---

## Appendix A — mapping table (upstream file → new home)

| Upstream | New |
|---|---|
| `NSSCore/Basics.swift`, `Style.swift`, `Project.swift`, `Timeline.swift`, `Recording.swift`, `Cursor.swift`, `FrameState.swift`, `BackgroundPresets.swift`, `Protocols.swift` | `crates/nss-core/src/*` |
| `NSSMotion/Spring.swift`, `MotionTuning.swift`, `Viewport.swift`, `ZoomPlanner.swift`, `SpringMotionModel.swift`, `LinearMotionModel.swift` | `crates/nss-motion/src/*` |
| `NSSProject/AtomicFile.swift`, `AutosaveDebouncer.swift`, `FileProjectStore.swift`, `FileProjectLibrary.swift`, `ProjectMigrator.swift`, `RecentProjects.swift` | `crates/nss-project/src/*` |
| `NSSCapture/*` | `crates/nss-capture/src/{catalog,recorder,device,wgc,wasapi,mf,pause,gate,writer}.rs` |
| `NSSInput/*` | `crates/nss-input/src/{hook,schedule,geometry,recorder,shapes}.rs` |
| `NSSRender/MetalRenderer.swift`, `Shaders.swift`, `ImageBlur.swift`, `CursorAtlas.swift`, `RenderLayout.swift`, `BackgroundRasterizer.swift`, `PixelBuffers.swift`, `PassthroughRenderer.swift`, `SolidColorFrameSource.swift`, `Wallpapers.swift`, `RENDERING.md` | `crates/nss-render/src/*` + `shaders/frame.wgsl`, `shaders/blur.wgsl`, `docs/RENDERING.md` |
| `NSSExport/MP4Exporter.swift`, `AssetFrameSource.swift`, `AudioPump.swift`, `Exporter.swift` | `crates/nss-export/src/*` |
| `NSSUI/Shell/*` | `src-tauri/src/{tray,hud,hotkey,overlay,settings}.rs` |
| `NSSUI/Editor/*`, `NSSUI/Home/*` | `ui/src/{editor,home}/**` |
| `NotScreenStudio/App.swift`, `AppModel.swift`, `SmokeTest.swift` | `src-tauri/src/main.rs`, `app_model.rs`, `--smoke-test` |
| `scripts/bundle.sh` | `scripts/build.ps1` |
| `MANUAL_TEST.md`, `AGENTS.md`, `PLAN.md` (upstream) | `docs/MANUAL_TEST.md`, `CONTRIBUTING.md`, this file |
| 499 Swift tests | `#[cfg(test)]` modules mirroring upstream file names |

## Appendix B — things that must survive the port verbatim

- The **at-or-before** frame selection rule for both screen and camera.
- The **0.3 s minimum** zoom block and the non-overlap invariant.
- The **edge-gap guarantee**: `clampedCamera` keeps the centre `0.5/scale` from every edge.
- The **bubble anchor** rule (left/top third, right/bottom third, linear blend in the middle) so a dragged bubble never jumps while zoomed.
- The **cursor atlas mip level** computed on the CPU (`log2(8 / pixelsPerPoint)`), not from screen-space derivatives — this is what makes the cursor deterministic.
- **Dither background only**, never the screen/camera pixels, so text stays exact.
- **Atomic save + backup fallback**, and "autosave 1 s after the last change".
- **Temp-file-then-move** export.
- **No network.**
