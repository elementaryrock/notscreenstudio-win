# Manual Test Checklist (Windows 10 / 11)

Port of upstream `_upstream/MANUAL_TEST.md` adapted for Windows (Tauri 2, Windows Graphics Capture, WASAPI, Media Foundation, wgpu/D3D12, FFmpeg).

Recordings are saved in your Windows Videos library (`%USERPROFILE%\Videos\notscreenstudio\`, or your redirected OneDrive `Videos\notscreenstudio\` via `SHGetKnownFolderPath`) as `.nssproj` folders. Exports default to the library folder unless a default export directory is set in Settings.

---

## 1. Build, Self-Check & Launch

1. Run `./scripts/test.ps1` in PowerShell.
   - **Good:** `cargo fmt`, `cargo clippy -D warnings`, `cargo test --workspace`, UI `typecheck`, UI `test`, UI `build`, and `notscreenstudio.exe --smoke-test` all pass (`All checks passed.`).
2. Launch the app (`./scripts/dev.ps1` or installed `notscreenstudio.exe`).
   - **Good:** The main dark-themed Home window opens showing the **Recordings** library (or empty state with **New Recording** and **Sample Project** buttons).
   - **Good:** A crisp circular icon appears in the Windows system tray (notification area) with tooltip `notscreenstudio — Ready`.
3. Right-click the system tray icon.
   - **Good:** Context menu displays **New Recording (Ctrl+Shift+R)**, **Pause / Resume (Ctrl+Shift+P)**, **Stop Recording**, **Open Recordings Library**, **Permissions & Settings**, and **Quit notscreenstudio**.

---

## 2. Permissions, Borderless Capture & FFmpeg Check

4. Click **Settings** in the Home header (or **Permissions & Settings** from the tray menu).
   - **Good:** Screen Capture (WGC) shows `granted`, Microphone and Camera reflect Windows Settings (`Settings -> Privacy & security -> Microphone / Camera`), and **FFmpeg Encoder** shows `Ready` with path and `H.264 ✓ • HEVC ✓` (or `Missing` with download guidance if FFmpeg is not on `PATH`).
5. On Windows 11 (build 22000+), click **Request Borderless Capture**.
   - **Good:** Windows asks once for borderless capture consent. If granted, the yellow WGC capture border disappears on future recordings. If declined (or on Windows 10), recording still works with the standard Windows yellow capture border and the Settings note explains why.
6. Test configurable global hotkeys: inspect **Record / Stop** (`Ctrl+Shift+R`) and **Pause / Resume** (`Ctrl+Shift+P`).
   - **Good:** No conflict banner appears unless another running app already holds that shortcut. If another app holds it, a clear conflict warning appears in Settings rather than failing silently.

---

## 3. Recording Flow (Display, Window, Area & Pause)

7. Click **New Recording** (or press **Ctrl+Shift+R**). Choose your primary **Display**, toggle Microphone and Camera as desired, set Countdown to **3 s**, and click **Record**.
   - **Good:** A centered 3-2-1 countdown overlay appears. Pressing **Escape** or clicking it cancels cleanly with no `.nssproj` created.
   - **Good:** When countdown reaches 0, recording begins immediately with no frozen frame or countdown digits baked into frame 0.
   - **Good:** The floating recording bar appears (red pulsing dot, elapsed timer, Pause, Stop, Discard) and the system tray icon switches to a crimson recording dot with a 1-second updating elapsed tooltip.
8. Move the mouse across different UI elements (text box for I-beam, link/button for pointing hand, window border for resize cursor, plain desktop for arrow), click 4–5 times spaced ~1 s apart, and speak into the mic.
   - **Good:** Desktop remains smooth at 60 fps.
9. Click **Pause** (or press **Ctrl+Shift+P**). Wait ~5 seconds, move the mouse and click several times, then click **Resume**.
   - **Good:** Recording bar dot turns amber and the elapsed timer pauses; tray icon switches to amber pause bars.
   - **Good:** After resuming and stopping, the paused interval is completely absent from video duration, audio, and cursor clicks (no frozen gap, no clicks from the paused span).
10. Click **Stop** (or press **Ctrl+Shift+R**).
    - **Good:** The recording saves to `Videos\notscreenstudio\<name>.nssproj` (`project.json`, `screen.mov`, `cursor.json`, plus `camera.mov` / `mic.m4a` when enabled) and opens directly in the Editor.
11. **Own-window exclusion check (`WDA_EXCLUDEFROMCAPTURE`):** Scrub the recording from start to finish in the Editor.
    - **Good:** Neither the Setup bar, Countdown overlay, Recording bar, Area selection overlay, nor the Camera bubble overlay ever appears baked into `screen.mov`.
12. **Window capture:** Start a new recording selecting a single **Window**. Drag that window across the screen while clicking inside it, then stop.
    - **Good:** Only that window is recorded, and the cursor + click ripples stay aligned to the exact spot inside the window even after moving it.
13. **Area capture:** Select the **Area** tab -> **Select Area on Screen**, drag a rectangle (live `W × H` readout appears; **Esc** cancels), record for 5 s, click inside the area, then stop.
    - **Good:** Only the cropped area appears in the Editor, and clicks land accurately inside the crop.
14. **Floating Setup Bar:** Click **Floating Bar** in the Setup modal to pop out the translucent capsule overlay (`#hud-setup`). Toggle Mic/Cam and start or close (`✕`).
    - **Good:** Bar stays always-on-top, does not appear in the taskbar (`WS_EX_TOOLWINDOW`), and is excluded from capture.

---

## 4. Home Library

15. Return to **Recordings** from the Editor top bar.
    - **Good:** All `.nssproj` bundles appear newest-first with thumbnail, title, local date/time, and duration badge.
16. Open the `⋯` menu on a card and test **Rename**, **Show in Explorer**, and **Move to Recycle Bin**.
    - **Good:** Rename persists across app restarts.
    - **Good:** **Show in Explorer** opens File Explorer with the `.nssproj` folder highlighted (`SHOpenFolderAndSelectItems`).
    - **Good:** **Move to Recycle Bin** removes the card and moves the bundle to the Windows Recycle Bin (restorable via Recycle Bin -> Restore).
17. Click **Sample Project** (or test on an empty library).
    - **Good:** Creates a synthetic sample bundle and opens it in the Editor immediately.

---

## 5. Editor: Preview, Timeline & Inspector

18. **Playback & Keyboard Shortcuts:**
    - Press **Space** to play/pause; **Left/Right Arrow** to step ±1 frame; verify renderer pill shows `wgpu` and low frame render time (ms).
    - Edit the project title in the top bar: verify pressing Space/Arrows/Ctrl+Z edits text rather than triggering playback/frame-step/project undo.
19. **Playhead Easing:**
    - Click a point on the time ruler -> playhead smoothly glides (~0.2 s cubic ease-out).
    - Click-and-drag across the ruler -> playhead tracks the mouse 1:1 immediately with zero lag.
20. **Trim Handles:**
    - Drag the yellow left and right trim handles on the Video filmstrip track.
    - **Good:** Excluded regions dim, minimum trimmed length is enforced at 0.5 s, playback clamps to `[trim.start, trim.end]`, and **Ctrl+Z** reverts the whole drag in a single undo step.
21. **Zoom Track & Inspector:**
    - Double-click empty space on the Zoom track (or click **+ Add Zoom**) -> inserts a 2.0× manual zoom block without overlapping neighbors (0.3 s minimum floor).
    - Drag block body to move, drag left/right edges to resize, press **Delete** to remove, press **Ctrl+Z** / **Ctrl+Shift+Z** to undo/redo.
    - Click **✦ Regenerate Auto** and **Clear Auto** -> only `✦ Auto` blocks are replaced/removed; manual blocks stay untouched.
    - Select a block, switch Focus to **Fixed point**, and click anywhere on the preview canvas or drag Focus X/Y sliders -> reticle moves and preview focuses that point.
    - Adjust **Zoom level** (`1.1×–4.0×`), **Ease in**, and **Ease out** (`0.1–2.0 s`).
22. **Backgrounds:**
    - Test all 4 tabs: **Wallpapers** (system wallpapers from `C:\Windows\Web\Wallpaper` & Themes), **Gradients** (8 linear gradients with live **Angle** slider + 8 mesh presets with rendered swatches), **Colors** (solid presets + custom color picker), and **Image** (PNG/JPEG path + up to 8 persisted recents).
    - Adjust **Blur** slider on an image/wallpaper background (disabled for solid/gradient backgrounds).
23. **Frame, Cursor & Camera Sections:**
    - Collapse/expand each of the 6 sections, quit and relaunch -> open/closed states persist.
    - Drag **Padding**, **Corner radius**, **Shadow opacity/blur/offset**, **Cursor size/smoothing/motion blur**, and toggle **Click ripples** / **Hide when idle** -> preview updates live and each drag undoes in one **Ctrl+Z**.
    - Toggle **Show camera bubble**, switch between **circle**, **square**, **tall**, and **wide**, drag **Roundness** (disabled on Circle), **Size**, **Border width/color**, **Mirror**, **Shrink while zoomed**, and drag the `CAM` handle directly on the preview canvas.
24. **Aspect Ratios:**
    - Switch top bar segmented control between **16:9**, **1:1**, and **9:16** -> preview updates immediately and screen stays cleanly framed with zero edge gaps even when zoomed into corners.

---

## 6. Export & Preview Parity

25. Click **Export** in the top bar.
    - Test **1080p** (1920×1080 for 16:9, 1080×1080 for 1:1, 1080×1920 for 9:16) and **4K** (3840×2160 / 2160×2160 / 2160×3840) at 30 fps and 60 fps, with both **H.264** and **HEVC**.
    - **Good:** Progress bar updates smoothly with frame counter (`framesWritten / totalFrames`).
    - **Good:** Clicking **Cancel** mid-export stops FFmpeg immediately and leaves no `.partial.mp4` or truncated output file on disk.
    - **Good:** On completion, **Show in Explorer** highlights the exported `.mp4`.
26. Play the exported `.mp4` in Windows Media Player / VLC and inspect properties (`ffprobe`).
    - **Good:** Resolution, frame rate, duration (matching trim), background/blur, cursor/ripples, zoom timing, camera bubble, and 48 kHz stereo AAC audio sync match the Editor preview.
27. **Autosave & Crash Recovery:**
    - Make edits in the Editor, quit the app immediately without manually saving, relaunch, and reopen the project.
    - **Good:** All edits were flushed on exit and reload cleanly.

---

## 7. Automated Parity & Benchmark Verification

Run the automated test suites to verify regressions before release:

```powershell
# Full CI-equivalent gate (fmt, clippy, workspace tests, UI typecheck/test/build, smoke test)
./scripts/test.ps1

# Cross-crate integration & parity tests (with GPU timing readout)
$env:NSS_BENCH = "1"
cargo test -p nss-export --test integration_e2e -- --nocapture
cargo test -p nss-export --test export_parity -- --nocapture
Remove-Item Env:NSS_BENCH

# Live hardware WGC 1-second capture test (requires unlocked desktop session)
cargo test -p nss-capture live_records_one_second_of_primary_display -- --ignored --nocapture
```
