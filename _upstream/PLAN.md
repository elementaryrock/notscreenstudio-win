# notscreenstudio plan

## Goal

The goal is a free, native macOS 15 screen recorder like Screen Studio. It differs from Screen Studio in four ways. It is free. Recording starts with one click. The preview is smooth at 60 fps. Everything stays on the user's Mac, with no network use, no telemetry and no accounts.

## Frontend

You record in one click from the menu bar. You pick a display, a window or an area. A timeline lets you trim the recording and add zoom blocks. An inspector controls the background, the frame, the cursor and the camera. A camera bubble can be dragged anywhere on the video. Export is available at 1080p or 4K.

## Backend

The app captures the screen, the microphone and the webcam. It logs the mouse 120 times a second, plus clicks. Spring physics drive the zooms and the cursor. A GPU renderer built on Metal draws every frame. A 60 fps MP4 exporter writes the result. Projects are autosaved.

## Must-haves

Auto-zoom on clicks, a smooth cursor with motion blur, click ripples, backgrounds with rounded corners and a shadow, a camera bubble in any shape, and the 16:9, 1:1 and 9:16 aspect ratios.

## Modules

| Target | Responsibility | Owning worker |
| --- | --- | --- |
| NSSCore | Shared models, conventions and protocols (the contracts) | Phase 0 skeleton, then additive changes only |
| NSSCapture | Screen, mic and camera recording into a project bundle | Recording |
| NSSInput | 120 Hz cursor and click logging into a CursorTrack | Input + Projects |
| NSSProject | `.nssproj` bundles: create, load, save, autosave | Input + Projects |
| NSSMotion | Pure Swift MotionModel: spring camera, cursor smoothing, ripples (Opus) | Motion |
| NSSRender | Metal FrameRenderer and shader sources compiled at runtime (Opus) | Renderer |
| NSSExport | 60 fps MP4 exporter using MotionModel and FrameRenderer. Also owns `AssetFrameSource`, the sequential `AVAssetReader`-based `FrameSource` | Export |
| NSSUI/Shell | Menu bar, source picker, recording HUD, camera bubble overlay | App shell UI |
| NotScreenStudio | App entry point and scene wiring | App shell UI |
| NSSUI/Editor | Preview, timeline and inspector. Also owns the real-time `AVPlayerItemVideoOutput`-based `FrameSource` | Editor UI |

## Phases

Phase 0 is the shared skeleton: this PR, with the package layout, the NSSCore contracts, stub implementations, CI, PLAN.md and AGENTS.md.

Phase 1 has seven parallel workers. Each owns its folder and opens its own PR.

1. Recording owns NSSCapture.
2. Input + Projects owns NSSInput and NSSProject.
3. Motion owns NSSMotion (Opus).
4. Renderer owns NSSRender (Opus).
5. Export owns NSSExport.
6. App shell UI owns NSSUI/Shell and NotScreenStudio.
7. Editor UI owns NSSUI/Editor.

Phase 2 is one integration worker. It wires record, edit and export end to end, verifies that the preview holds 60 fps, and exports a test clip in all three aspect ratios at both 1080p and 4K.

## Data flow

Recording writes a `.nssproj` bundle in `~/Movies/notscreenstudio`. The bundle holds the media files, `cursor.json` and `project.json`. The editor loads that bundle. The MotionModel turns the cursor track and the timeline into a FrameState for each frame. The FrameRenderer composites the frame from the screen pixels, the camera pixels, the FrameState and the Style. The preview and the export share the same MotionModel and the same FrameRenderer, so they match exactly.

## Shared contracts worth knowing

- Time origin: t = 0 is the first screen frame. `RecordingAssets.startHostTime` holds its host-clock time (the `CACurrentMediaTime()` clock). `InputLogger.stop(recordingStartHostTime:)` returns a track already rebased to it.
- `FrameSource` picks the frame nearest at-or-before `t` for both screen and camera. Camera returns nil when there is no camera track. `SolidColorFrameSource` in NSSRender is the stub.
- Codable policy: every persisted type decodes with `decodeIfPresent` and a default for every field that is not essential, so an older `project.json` without a newer field still loads. New fields must follow this rule.

## Round 2

Round 1 is merged. The user tested main by hand and found eight parity gaps with Screen Studio. Round 2 closes them.

### The eight gaps

1. No countdown before recording starts.
2. The recorder's own windows (HUD, camera bubble overlay) can show up in the capture.
3. The camera bubble only has old shapes. It needs four shapes (circle, square, tall, wide) with a roundness control, drawn the same in the overlay, the preview and the export.
4. The cursor is always an arrow. Real cursor shapes (I-beam, pointing hand, resize and so on) are not logged or drawn.
5. The cursor does not squish while the button is held down.
6. The camera bubble does not shrink when the view zooms in.
7. There is no home screen. The user needs a list of past projects with thumbnails, plus rename and delete.
8. The playhead jumps instead of easing, and the inspector lacks controls for the new bubble options.

### The three checks

- Auto-zoom never shows gaps at the edges of the source.
- The preview holds 60 fps.
- The export matches the preview exactly, with audio.

### Shared contracts added in NSSCore (this PR)

- `CursorShape`, `CursorShapeChange`, `CursorTrack.shapes` and `CursorTrack.shape(at:)`. `CursorState.shape` and `CursorState.pressScale`.
- `CameraShape` is now circle, square, tall, wide. `CameraBubble.roundness` and `CameraBubble.zoomedScale`. `CameraBubbleGeometry` holds the aspect ratios (tall 3:4, wide 16:9), the corner radius rule and `sizeMultiplier(cameraScale:)` (smoothstep from 1 at camera scale 1 to `zoomedScale` at scale 2 or more). Old shapes migrate when decoded: circle stays circle, roundedRect becomes square with matching roundness, squircle becomes square at 0.6, rectangle becomes wide at 0.
- `RecordingOptions.countdownSeconds` (default 3, 0 is off).
- `ProjectSummary`, the `ProjectLibrary` protocol, `InMemoryProjectLibrary` and `RecordingAssets.thumbnailFile`.

### Owners

| Worker | Owns | Covers |
| --- | --- | --- |
| Recording UX | NSSUI/Shell, NotScreenStudio, capture exclusion | Countdown, exclude own windows from capture, bubble overlay shapes |
| Cursor (Opus) | NSSInput, NSSMotion | Cursor shape logging, press squish, auto-zoom edge guarantee |
| Renderer (Opus) | NSSRender | Draw cursor shapes and squish, bubble shrink and the new bubble shapes |
| Editor + Home | NSSUI/Editor, NSSProject library | Home screen with thumbnails, playhead easing, inspector controls |
| Verification (last) | Tests, MANUAL_TEST.md | Edge gaps, 60 fps, export equals preview with audio |
