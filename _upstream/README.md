# notscreenstudio

notscreenstudio is a free screen recorder for macOS, in the style of Screen Studio. You record from the menu bar with one click. Then you polish the video in a built-in editor: the view zooms in on your clicks, the cursor glides, and the screen sits on a nice background. The preview runs at a smooth 60 fps. You export to MP4. Everything stays on your Mac. There are no accounts, no network calls and no telemetry.

[![notscreenstudio launch video](docs/launch-preview.gif)](docs/launch-preview.mp4)

Click the preview to watch the launch video with sound ([docs/launch-preview.mp4](docs/launch-preview.mp4)).

The app is written in Swift 6 with SwiftUI, and it draws every frame with Metal.

## Get started

You build the app yourself. There is no download.

1. You need macOS 15 or later and the Xcode Command Line Tools (`xcode-select --install`). Full Xcode is not needed.
2. Clone the repository and open it in Terminal:
   ```sh
   git clone https://github.com/dotslashsimran/notscreenstudio.git
   cd notscreenstudio
   ```
3. Run `./scripts/make-dev-cert.sh` **once**, before your first build. It gives every build the same signature, so macOS keeps the Screen Recording permission across rebuilds and never shows duplicate entries.
4. Run `./scripts/bundle.sh`. It builds the app and creates `build/NotScreenStudio.app`.
5. Run `open build/NotScreenStudio.app`. A circle icon appears in the menu bar.
6. Click the icon, open **Permissions…**, and allow Screen Recording, Camera and Microphone. Then relaunch the app once. macOS only applies Screen Recording to a new process.

**Keep one copy of the app**, the one in `build/NotScreenStudio.app`. Do not copy it to Applications or anywhere else. Each copy can show up as a separate entry in the Screen Recording settings, and macOS may ask for permission again for each one.

## Features

### Recording

- Three sources: a whole **display**, a single **window**, or an **area** you drag out on the screen.
- A **setup bar** floats at the bottom of the screen after you pick a source. It has the source name, a camera toggle, a mic toggle (each with a device menu), a record button and a close button.
- A **countdown** of 3, 2, 1 runs before recording starts. You can set it to Off, 3 s or 5 s. The default is 3 s. Press Escape or click during the countdown to cancel.
- A small **recording bar** shows a pulsing red dot, the elapsed time, a pause button and a stop button. You can pause and resume. Paused time is not in the recording, and clicks made while paused are ignored.
- Record your **microphone** and your **camera** at the same time as the screen.
- The app's own windows (setup bar, recording bar, countdown, camera bubble, area frame and any other window of the app) are kept out of the recording.
- The **menu bar icon** shows the state. It is a circle when idle. While recording it is a red record symbol with the running time. While paused it is a pause symbol.
- A **global shortcut**, Control + Command + R, stops the recording from any app.

### Cursor

- The mouse position and clicks are logged at 120 Hz while you record.
- Cursor **smoothing** gives a gliding path. You can change how strong it is.
- **Motion blur** smears the cursor along fast moves.
- **Click ripples** draw a ring at each click. You can turn them off.
- **Real cursor shapes** are logged and drawn: arrow, I-beam, pointing hand, open and closed hand, crosshair, resize arrows and more.
- The cursor **squishes** a little while the mouse button is held down.
- **Hide when idle** fades the cursor out after about 2 seconds without movement or clicks.
- You can change the cursor size from 0.5× to 4×.

### Zoom

- **Auto-zoom** makes a zoom from your clicks. The view eases in just before a click, holds while you work there, and eases out.
- Each zoom is a **block on the Zoom track**. Auto blocks have a small sparkle. Blocks are saved with the project.
- You can **add, move, resize and delete** blocks. Blocks do not overlap each other, and the shortest block is 0.3 seconds.
- The **Zoom inspector** shows the selected block. You can set its zoom level (1.1× to 4×), its focus, and its ease in and ease out times (0.1 to 2 seconds). The focus is either *Follow cursor* or *Fixed point*. For a fixed point you can type X and Y, or click the point in the preview.
- **Remove all auto zooms** deletes only the auto blocks. **Regenerate auto zooms from clicks** makes them again. Your own blocks, and auto blocks you edited, stay.
- The view never shows a gap past the edge of the screen when it zooms in near a corner or an edge.

### Look

- **Background picker** with four tabs:
  - **Wallpapers**: the macOS desktop pictures found on your Mac.
  - **Gradients**: 8 linear gradients and 8 soft multi-colour mesh gradients. Linear gradients have an angle slider.
  - **Colours**: 16 swatches and a custom colour picker.
  - **Image**: pick your own photo (JPEG, PNG or HEIC). The last 8 images you used are kept as recents.
- A **blur** slider blurs image and wallpaper backgrounds.
- **Padding**, **corner radius** and **shadow** (blur and opacity) shape the frame around the screen.
- Three **aspect ratios**: 16:9, 1:1 and 9:16.

### Camera bubble

- Four **shapes**: circle, square, tall (3:4) and wide (16:9).
- A **roundness** slider goes from sharp corners to fully round. It is off for the circle.
- **Drag** the bubble to move it. You can do this while recording and in the editor preview.
- The bubble **shrinks when the view zooms in**. The *Shrink when zoomed* slider sets how much.
- Optional **border** with a width and a colour, a **mirror** switch, a **size** slider and a **visible** switch.

### Editor

- The editor has a dark theme. The top bar has a *Recordings* button, the project name (click to rename), undo and redo, the aspect ratio picker and the Export button. The preview is in the middle, the inspector is on the right, and the timeline is at the bottom.
- The **inspector** has six collapsible sections: Background, Frame, Cursor, Zoom, Camera and Audio. Open or closed state is remembered.
- The **timeline** has four tracks: Video, Zoom, Camera and Audio. The Video track shows thumbnails, yellow trim handles and an orange dot for each click.
- **Trim** the start and end by dragging the handles. The trim applies to the picture and the sound.
- **Playhead**: click or drag the ruler to scrub. Jumps ease smoothly. Dragging follows the mouse exactly.
- **Undo and redo** cover every edit. A slider drag counts as one step.
- The preview and the export use the same motion code and the same Metal renderer, so they match.

### Home screen

- A grid of all your recordings, newest first. Each card has a thumbnail, name, date and length.
- Right-click a recording to **Rename**, **Reveal in Finder** or **Move to Trash**.
- Click *Recordings* in the editor to come back here.
- With no recordings, it shows "No recordings yet" and a Record button.

### Autosave

Projects are saved on their own, about one second after your last change. Close the app and open the recording again, and your edits are there.

### Export

- Resolution: **1080p** or **4K**. The shorter side is 1080 or 2160 pixels.
- Frame rate: **60 fps**.
- Codec: **H.264** or **HEVC**.
- Audio: your microphone track, cut to your trim.
- The export matches the editor preview: same zooms, cursor, ripples, background and camera bubble.
- The file is written to a temporary file first and moved into place when it is finished. A failed or cancelled export never leaves a half-written file.

Output sizes:

| Aspect ratio | 1080p | 4K |
| --- | --- | --- |
| 16:9 | 1920×1080 | 3840×2160 |
| 1:1 | 1080×1080 | 2160×2160 |
| 9:16 | 1080×1920 | 2160×3840 |

## Requirements

- macOS 15 or later.
- A Mac that can run Metal. The app renders on the GPU.
- Swift 6.1 and the Xcode Command Line Tools. Full Xcode is **not** needed.

## Build and run

```sh
./scripts/bundle.sh
open build/NotScreenStudio.app
```

The script builds a release binary, wraps it into `build/NotScreenStudio.app`, signs it ad hoc, and runs a self check (`--smoke-test`). When it works, the last lines are `smoke-test OK: ...` and `Built build/NotScreenStudio.app`.

The build uses 2 parallel jobs by default, which suits a Mac with little memory. Set `NSS_JOBS=4` to use more. Set `NSS_SKIP_SMOKE=1` to skip the self check (for example over SSH, where there is no GUI session).

**Why a bundle?** macOS only gives Screen Recording, Camera and Microphone permission to a real app bundle with a bundle identifier and an `Info.plist` that explains each permission. A bare binary from `.build/` cannot hold those permissions reliably. The script creates the bundle (`dev.notscreenstudio.app`) and signs it.

The app has no Dock icon. It lives in the menu bar.

## Permissions

The app asks for three permissions. Open **Permissions…** from the menu bar icon, and click Allow on each row.

| Permission | Used for |
| --- | --- |
| Screen Recording | Capturing the display, window or area |
| Camera | The camera bubble |
| Microphone | Narration |

Nothing else is needed. The cursor and click log does not need Accessibility or Input Monitoring, and the app never listens to the keyboard.

- **Relaunch after allowing Screen Recording.** macOS applies the grant only to a new process. The Permissions window shows a **Relaunch notscreenstudio** button until the check passes. The app re-checks when it becomes active and every 2 seconds while the Permissions window is open. The check asks ScreenCaptureKit for the shareable content, which is the real test. `CGPreflightScreenCaptureAccess` is only used as a quick hint.
- **Keep permissions across rebuilds.** Ad-hoc signing gives each build a new code hash, so macOS treats every rebuild as a new app. To avoid that, create a stable signing identity once: run `./scripts/make-dev-cert.sh`. It is opt-in, only runs when you run it, and adds a self-signed certificate named `notscreenstudio-dev` to your login keychain. After that `./scripts/bundle.sh` signs with it automatically. To use another identity, set `NSS_SIGN_IDENTITY="Your Identity Name"`. With neither, the build is signed ad hoc as before. Allow Screen Recording once for the first build signed with the stable identity; later rebuilds keep it.
- **Screenshots.** Only the recorder's own overlays (setup bar, countdown, recording bar, camera bubble, area selection) are hidden from screenshots and recordings. The Permissions window, editor and home can be screenshotted.

## How to use it

### From first launch to an MP4

1. Run `./scripts/bundle.sh` and then `open build/NotScreenStudio.app`.
2. A circle icon appears in the menu bar. Click it.
3. Open **Permissions…** and allow Screen Recording, Camera and Microphone. Quit and reopen the app.
4. Click the menu bar icon again. Pick a source: your display, **Window…**, or **Select Area…**. For an area, drag a rectangle on the screen.
5. The setup bar appears at the bottom of the screen. Turn the camera and the mic on or off. Choose a device from each menu if you need to.
6. Press Return or click the big round record button. The countdown runs, then recording starts.
7. Work as normal. Move the mouse and click. Use pause and resume on the recording bar if you need a break.
8. Stop with the stop button, **Stop Recording** in the menu, or Control + Command + R. The editor opens with your recording.
9. Press Space to play it. Adjust the background, frame, cursor, zooms and camera in the inspector. Trim the start and end on the timeline.
10. Click **Export** (or press Command E). Pick 1080p or 4K and H.264 or HEVC. Click Export.
11. When the progress bar finishes, open the MP4.

### Edit zooms

1. Look at the **Zoom** track. Each click made a block, marked with a sparkle.
2. Click a block to select it. The Zoom section of the inspector shows its settings.
3. Drag the middle of a block to move it. Drag its left or right end to resize it.
4. Double-click an empty part of the track to add a 2× block. You can also click **Add zoom at playhead** in the inspector.
5. Change **Zoom level**, **Focus**, **Ease in** and **Ease out** in the inspector. For a fixed focus, click **Pick point in preview** and click on the picture.
6. Press Delete to remove the selected block. Press Command Z to bring it back.
7. Use the trash button beside the Zoom track label to remove all auto zooms. Use the wand button to make them again from your clicks.

### Choose a background

1. Open the **Background** section of the inspector.
2. Pick a tab: Wallpapers, Gradients, Colours or Image. The first time you open Wallpapers, the thumbnails take a moment to appear.
3. Click a tile. The preview changes at once.
4. On the Image tab, click the **+** tile to pick a photo. Images you used before show up as recents. Right-click a recent to remove it from the list.
5. Use **Blur** for wallpapers and images. It does nothing for gradients and colours.

### Use the camera bubble

1. Turn the camera on before you record. A recording made without a camera has no bubble.
2. While recording, drag the bubble to where you want it.
3. In the editor, open the **Camera** section. Choose a shape, roundness, size, border and mirror.
4. Drag the bubble in the preview to move it.
5. Set **Shrink when zoomed** to make the bubble smaller during zooms. At 100% it keeps its size.

## Keyboard shortcuts

| Keys | Where | What it does |
| --- | --- | --- |
| Control + Command + R | Anywhere, while recording | Stop recording |
| Return | Setup bar | Start recording |
| Escape | Setup bar | Close the bar, record nothing |
| Escape | During the countdown | Cancel the countdown |
| Escape | Area selection | Cancel the selection |
| Space | Editor | Play or pause |
| Left Arrow / Right Arrow | Editor | Previous or next frame |
| Command Z | Editor | Undo |
| Shift + Command Z | Editor | Redo |
| Delete | Editor, with a zoom block selected | Delete the block |
| Command E | Editor | Open the Export sheet |

While you type in the project name field, Space, the arrow keys and Command Z edit the text instead.

## Where files are saved

| What | Where |
| --- | --- |
| Projects | `~/Movies/notscreenstudio/`, one `.nssproj` folder per recording |
| Inside a project | `project.json`, `screen.mov`, `cursor.json`, and (when recorded) `camera.mov` and `mic.m4a`. Images you pick as a background are copied in too. |
| Thumbnails | `thumbnail.jpg` inside each project, made when needed |
| Exports | `~/Movies/<project name>.mp4` by default. You can change the folder and name in the Export sheet. |
| Background thumbnail cache | `~/Library/Caches/notscreenstudio/BackgroundThumbnails` |
| Recent background images, settings | Saved in the app's preferences (`UserDefaults`) |

A `.nssproj` is a normal folder. You can use Show Package Contents in Finder to look inside it. Deleting a recording from the home screen moves it to the Trash.

## Privacy

- The code has no networking. Nothing is uploaded.
- There is no analytics, no crash reporter and no account.
- Camera and microphone data go only into files in your project folder.

## Troubleshooting

**Recording fails, or the source list is empty.**
Open **Permissions…** and check that Screen Recording, Camera and Microphone are allowed. Quit and reopen the app after allowing Screen Recording. If you rebuilt the app, remove it from the three lists in System Settings, add it again and restart it (see [Permissions](#permissions)), or use `./scripts/make-dev-cert.sh` once so rebuilds keep their permissions.

**Screen Recording asks again and again, or the list in System Settings has stale entries.**
Do this one-time fix. Quit the app, then run:

```sh
tccutil reset ScreenCapture dev.notscreenstudio.app
defaults delete dev.notscreenstudio.app notscreenstudio.shell.askedScreenRecording
```

Open the app again, click Allow once when macOS asks, and relaunch the app. If you have not yet run `./scripts/make-dev-cert.sh`, run it first, so later rebuilds keep the permission.

**Screen Recording still says it is not allowed.**
Start the app from Terminal with `NSS_PERMISSION_DEBUG=1 /path/to/NotScreenStudio.app/Contents/MacOS/NotScreenStudio`. It prints, to stderr, the preflight result, the exact ScreenCaptureKit result (error code -3801 means macOS has not granted this build), the bundle path and id, and the code signature (identifier, cdhash, flags). If `adhoc=true` and the cdhash changed since you allowed it, the build is a new app to macOS: use the stable identity above. Run it from Terminal only for diagnosis, because a program started from Terminal can be judged by the permissions of Terminal. Open the app normally with `open` for real use.

**Check that the app's own windows are kept out of the recording.**
Start the app from Terminal with logging on:

```sh
NSS_CAPTURE_DEBUG=1 build/NotScreenStudio.app/Contents/MacOS/NotScreenStudio
```

While you record, the Terminal prints lines that start with `[NSS capture]`. They show the capture filter, the excluded apps and the app's own windows. If you see `WARNING: nothing excluded`, the app's windows are not being excluded. For a single-window recording, the log says no exclusion is needed.

**The build fails or the self check fails.**
Run `swift build` and read the error. Make sure the Command Line Tools are installed (`xcode-select --install`). To build without the self check, run `NSS_SKIP_SMOKE=1 ./scripts/bundle.sh`.

**The menu bar icon is missing.**
The app has no Dock icon and no window at launch. Look for the circle in the menu bar.

## Development

### Package layout

| Module | What it does |
| --- | --- |
| `NSSCore` | Shared models, protocols and conventions (project, style, timeline, cursor, background presets) |
| `NSSCapture` | Screen, microphone and camera recording with ScreenCaptureKit and AVFoundation |
| `NSSInput` | 120 Hz cursor and click logging, cursor shapes |
| `NSSProject` | `.nssproj` bundles: create, load, save, autosave, the project library, migration |
| `NSSMotion` | Pure Swift motion: spring camera, zoom planner, cursor smoothing, ripples |
| `NSSRender` | Metal renderer. Shaders are Swift strings compiled at run time |
| `NSSExport` | 60 fps MP4 exporter |
| `NSSUI` | SwiftUI views: `Shell` (menu bar, recording bars, countdown), `Editor`, `Home` |
| `NotScreenStudio` | The app entry point |

There are no `.metal` files. This keeps the project buildable with only the Command Line Tools.

### Build and test

```sh
swift build
swift test
```

On a small Mac, add `-j 2`. Tests use Swift Testing. Tests that need a GPU, a camera or a permission skip themselves when it is not available.

Extra switches:

| Variable | Effect |
| --- | --- |
| `NSS_BENCH=1` | Runs the preview timing test, for example `NSS_BENCH=1 swift test -j 2 --filter PreviewPerformanceTests` |
| `NSS_E2E_4K=1` | Includes the 4K exports, for example `NSS_E2E_4K=1 swift test -j 2 --filter EndToEndExportTests` |
| `NSS_CAPTURE_DEBUG=1` | Prints capture filter details at run time |
| `NSS_JOBS` | Number of build jobs used by `bundle.sh` (default 2) |
| `NSS_SKIP_SMOKE=1` | Makes `bundle.sh` skip the self check |

You can also run the self check yourself: `swift run NotScreenStudio --smoke-test`.

### CI

GitHub Actions runs on every pull request and on pushes to `main`. It runs on `macos-15`: `swift build`, `swift test` and the app smoke test.

### Project documents

- [AGENTS.md](AGENTS.md): coding rules for contributors.
- [PLAN.md](PLAN.md): goals, module ownership and design notes.
- [MANUAL_TEST.md](MANUAL_TEST.md): a step-by-step checklist to test the app by hand.

## Known limitations

- Wallpapers that macOS lists only as downloads (`.madesktop` files such as Big Sur and Catalina) are not shown, because the full image is not on the Mac. Dynamic wallpapers show one still image.
- Rebuilding the app can reset its permissions, because the app is signed ad hoc (see [Permissions](#permissions)).
- The recorder can capture system audio, but the app has no switch for it yet. Recordings contain your microphone only.
- Export is MP4 only, at 60 fps, in 1080p or 4K.
- There is no packaged release. You build the app yourself.

## Contributing

1. Fork the repository and create a branch.
2. Make your change. Run `swift build` and `swift test` (add `-j 2` on a small Mac).
3. Open a pull request against `main`. The CI check must pass.
4. Read [AGENTS.md](AGENTS.md) first. It has the coding rules.
5. Contributions are accepted under the same license (MIT).

## Video credits

The memes in the launch video are made for it, using photos from Wikimedia Commons: Ed Sheeran by Harald Krichel (CC BY-SA 3.0), Eva Rinaldi (CC BY-SA 2.0), Patrick Cristiano (CC BY-SA 4.0), Sean Reynolds (CC BY 2.0) and Drew de F Fawkes (CC BY 2.0); cats by DGtal (CC BY-SA 4.0), Keren Dvir (CC BY-SA 4.0) and Sebastian Rosa (CC0); carrots by Evan-Amos (public domain). The desktop wallpaper is Apple's Sequoia Sunrise.

## License

notscreenstudio is released under the MIT License. See [LICENSE](LICENSE).
