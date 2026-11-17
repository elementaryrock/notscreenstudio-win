# Manual test checklist

Follow these steps on your Mac (macOS 15). Each step says what "good" looks like. If a step does not match, note the step number and what you saw.

Recordings are saved in `~/Movies/notscreenstudio/` as `.nssproj` folders. Exports go to `~/Movies/` by default.

## 1. Build and launch

1. In the repo folder run `./scripts/bundle.sh`. It builds a release app, signs it ad hoc, and runs a self check.
   - Good: the last lines are `smoke-test OK: renderer=Metal sources=wired` and `Built build/NotScreenStudio.app`.
2. Run `open build/NotScreenStudio.app`.
   - Good: a record icon (a circle) appears in the menu bar. There is no Dock icon and no window yet.
3. If no permission window opens by itself, click the menu bar icon.
   - Good: the menu shows a big Record button, a source choice, a microphone choice and a camera choice.

## 2. Permissions

4. Open the Permissions window from the menu (or let it open on first launch). Click Allow for Screen Recording, Camera and Microphone.
   - Good: the system asks for each one. After you allow them, each row shows as granted.
5. Quit the app (menu bar icon, then Quit) and open it again once. Screen Recording only takes effect after a restart.
   - Good: the source list in the menu now shows your displays and windows, with no permission warning.

4a. First launch on a clean state: run `tccutil reset ScreenCapture dev.notscreenstudio.app` and `defaults delete dev.notscreenstudio.app`, then open the app.
   - Good: no system Screen Recording dialog appears at launch. The Permissions window explains it and shows Allow. The dialog appears only after you click Allow, and only once. Needs a real-machine check: this has not been run on a clean TCC state yet. Expected: nothing prompts until the first Allow click, and no further prompt after that.
5a. If Screen Recording is still not granted, the Screen Recording row shows a **Relaunch notscreenstudio** button. Turn the app on in System Settings, come back, and watch the row.
   - Good: the row turns to Allowed within a few seconds without clicking, or after you click Relaunch and the app comes back.
5b. Start the app from Terminal with `NSS_PERMISSION_DEBUG=1`.
   - Good: stderr shows the preflight result, the SCShareableContent result or error code, the bundle path and the signing identifier, cdhash and flags.
5c. Screenshot scope: with the Permissions window open (not recording), take a screenshot (Shift-Command-4, then Space, click the window).
   - Good: the window appears in the screenshot. While the setup bar, countdown or recording bar is on screen, those bars must not appear in the recording.

5d. Single prompt: reset Screen Recording for the app (`tccutil reset ScreenCapture <bundle id>`), launch it, and leave the Permissions window open for at least 30 seconds without clicking.
   - Good: no system dialog appears while you wait. Click Allow once: the dialog "would like to record this computer's screen and audio" appears exactly once. Switch away and back to the app a few times, and click Check Again: no second dialog appears until you have turned it on in System Settings and relaunched. Relaunch after granting: no dialog at launch.
   - Also check after a rebuild signed differently (for example ad hoc after a dev-cert build): the app behaves as never asked, shows Allow, and prompts only after you click it.

Note: by default `bundle.sh` signs ad hoc, so macOS may treat each rebuild as a new app. To keep permissions across rebuilds, run `./scripts/make-dev-cert.sh` once (it creates the `notscreenstudio-dev` identity in your login keychain; `bundle.sh` then uses it, or set `NSS_SIGN_IDENTITY`). Allow the permissions once for the first build signed that way, rebuild, and check that Screen Recording is still allowed.

## 3. Record

6. Choose your main display. Turn on the microphone and the camera in the menu. Click Record.
   - Good: a big 3, 2, 1 countdown shows first (3 seconds by default), then a small recording bar with a running timer appears. A camera bubble shows your face over the screen. No extra dialog.
   - Good: the countdown, the recording bar and the camera bubble never appear in the finished recording (check this in step 10a below).
   - Also try: press Escape or Stop during the countdown. Good: nothing is recorded and no project is created. Set the countdown to 0 if the menu offers it. Good: recording starts at once.
7. Move the mouse around, click a few different places in different corners of the screen (wait about one second between clicks), and speak a sentence.
   - Good: nothing lags and the screen stays smooth.
8. While recording, drag the camera bubble to another corner and make it larger or smaller if you can.
   - Good: the bubble follows the mouse and stays where you drop it.
9. Stop with the Stop button on the recording bar (or the global stop shortcut shown in the menu).
   - Good: the editor window opens by itself within a few seconds, showing your recording.
10. In Finder open `~/Movies/notscreenstudio/` and right-click the newest `.nssproj` folder, then Show Package Contents.
    - Good: it holds `project.json`, `screen.mov`, `camera.mov`, `mic.m4a` and `cursor.json`. `cursor.json` is more than a few kilobytes. A `thumbnail.jpg` appears a moment after the editor opens.
10a. Play the recording in the editor and step through the first and last seconds.
    - Good: no recording bar, no countdown and no camera bubble overlay are baked into the screen video. The camera bubble you see is the one drawn by the editor, and it follows your Camera settings.
10b. Countdown with warm-up: record your main display with the 3 second countdown and with something moving on screen (a clock or a video playing).
    1. Click Record and watch the 3, 2, 1 numbers.
    2. As soon as the countdown ends, the recording bar must appear at once, with no extra pause and no second countdown.
    3. Stop after about 5 seconds and open the recording in the editor. Press Left Arrow at the very start (playhead at 0:00).
    - Good: the first frame already shows the screen (not black, not a frozen frame from before you clicked Record). The first frame is from the moment the countdown ended, so the countdown numbers are not in it.
    - Good: the first mouse click you made right after the countdown ends is at the right time in the timeline (the ripple starts when the click happened).
10c. Own windows never in the recording: the recording bar (stop bar), the camera bubble, the 3, 2, 1 countdown and the area-selection overlay must not appear in any recording of a display or an area.
    1. Start the app from Terminal with logging on: `NSS_CAPTURE_DEBUG=1 build/NotScreenStudio.app/Contents/MacOS/NotScreenStudio`.
    2. Record a whole display with the camera on and the 3 second countdown. While recording, drag the camera bubble around and click Stop only after 5 seconds.
    3. Record an area: choose Area, drag a rectangle that covers the place where the recording bar and the camera bubble sit, and record for 5 seconds.
    4. During one of the recordings, open a new window of this app mid-recording (for example choose Permissions from the menu bar icon, or open the editor for an older recording), and leave it open for 2 seconds over the recorded area.
    5. Open each new recording in the editor and step through it from start to end (Right Arrow, or drag the playhead slowly).
    - Good: in no frame do you see the recording bar, the camera bubble drawn by the overlay (the editor's own bubble is expected), the countdown numbers, the area-selection frame or the extra window you opened mid-recording. There is no black hole or outline where they were.
    - Good: the Terminal shows lines like `[NSS capture] filter (...) target=... ownPID=...` with `excluded apps (pid): [...]` and a list of this app's windows (for example the bar and the bubble). It must not show `WARNING: nothing excluded`. A line appears again after a new window opens mid-recording.
    - Also try: a single-window recording. Good: the log says `single-window capture: nothing else is visible, no exclusion needed`.
11. Record again, this time choosing a single window as the source. Drag that window around while recording, click inside it, then stop.
    - Good: the editor shows only that window. In the editor the cursor stays on the right spot inside the window even though the window moved.
12. Record again, this time choosing Area and dragging a rectangle over part of the screen. Click inside the area, then stop.
    - Good: the editor shows only that area, and clicks land where you made them.

## 4. Edit

Open each recording in the editor and check the following.

13. Press Space to play.
    - Good: playback is smooth (no stutter), your voice plays in sync with the picture, and the camera bubble moves with the video.
14. Auto-zoom: watch the moment after each click.
    - Good: the view eases in toward the click, holds for a moment, then eases out. It does not jump. It should not zoom for clicks that are far apart in a way that makes the picture swing wildly.
15. Smooth cursor: watch the arrow while you moved the mouse.
    - Good: the cursor glides along a smooth path instead of jittering. Raise Smoothing in the inspector and the path gets softer.
16. Motion blur: find a fast mouse move and pause on it, then raise Motion blur in the inspector.
    - Good: the arrow smears along the direction it moved. A still cursor stays sharp.
17. Click ripples: step through frames right after a click (Right Arrow).
    - Good: a ring grows out from the click point and fades. Turning Click ripples off in the inspector removes it.
18. Background: change the background to a solid color, a gradient and a wallpaper.
    - Good: the preview updates at once, with no flicker. A gradient shows no visible banding.
19. Rounded corners and shadow: raise Corner radius, Shadow blur and Shadow opacity in the inspector, and change Padding.
    - Good: the screen has rounded corners, a soft shadow sits under it, and more padding shrinks the screen inside the frame.
20. Camera bubble shapes: in the inspector pick Circle, Square, Tall and Wide in turn. Drag the bubble in the preview.
    - Good: each shape draws cleanly with smooth edges (Tall is 3:4, Wide is 16:9), the bubble sits where you left it while recording (mapped over the same part of the screen), and it follows your drag. The shape in the preview matches the one in the recording overlay and in the export.
20a. Roundness: with Square, Tall or Wide selected, drag the Roundness slider from left to right.
    - Good: corners go from sharp to fully round (a circle for Square, a pill for Tall and Wide). With Circle selected the slider is greyed out. One Undo (Command Z) reverts the whole drag, not one tick of it.
20b. Shrink when zoomed: set "Shrink when zoomed" to about 50%, then play over a click that zooms in.
    - Good: the bubble smoothly gets smaller as the view zooms in and grows back when it zooms out. At 100% it never changes size. One Undo reverts the whole slider drag. Check the same in the export (step 25).
21. Aspect ratio: switch between 16:9, 1:1 and 9:16.
    - Good: the preview changes shape. The screen stays centered and fits, the zoom still follows your clicks, and the camera bubble stays inside the picture.
22. Trim and zoom blocks: drag the trim handles on the timeline to cut the start and end. Then add a zoom block, drag it, change its zoom level and pick its focus point in the preview.
    - Good: playback stays inside the trim. The new zoom block shows on the timeline and the preview zooms to the point you picked during that block. Undo and Redo (Command Z, Shift Command Z) revert each change.

### Cursor, squish and playhead

22a. Cursor shapes: record yourself moving over a text field (I-beam), a link or button (pointing hand), a window edge (resize arrows) and plain desktop (arrow), then open it in the editor.
    - Good: the cursor in the editor changes to the matching shape at the same moments and is not always an arrow.
22b. Press squish: hold the mouse button down for about a second on a click, then play it back.
    - Good: the cursor shrinks a little while the button is held and springs back on release.
22c. Playhead while playing: press Space and watch the red playhead on the timeline.
    - Good: it glides smoothly, with no stepping. Pause and it stops where the picture is.
22d. Playhead jumps: click a spot on the ruler, press Right Arrow and Left Arrow, and use Undo after a Trim.
    - Good: the picture changes at once, and the red playhead glides there in about a fifth of a second. Pressing the arrow quickly several times keeps it gliding without jumping back.
22e. Playhead drag: press and drag on the ruler or thumbnail strip.
    - Good: the red playhead follows the mouse exactly, with no lag or easing, and the preview scrubs with it.

## 5. Export

23. Choose Export. Pick 1080p and click Export.
    - Good: a progress bar fills, and when it ends the file exists at the chosen path. It does not take much longer than the video itself.
24. Open the MP4 in QuickTime Player and check the details (Command I).
    - Good for 16:9: 1920×1080 at 60 fps. For 1:1: 1080×1080. For 9:16: 1080×1920. Voice and picture are in sync, and the video matches the trim you set.
25. Compare the MP4 with the editor preview: zoom timing, cursor, ripples, background and the camera bubble position.
    - Good: they look the same.
25a. Edges and clicks in the corners: record clicking close to all four corners and the middle of all four edges of the screen (wait one second between clicks), then play it in the editor in 16:9, 1:1 and 9:16.
    - Good: each time the view zooms in on a corner or an edge, the screen fills its rounded frame: there is no strip of background color, no smeared or stretched pixels along the frame edge. Check the same in the export.
25b. Audio and sync: record yourself clapping (or tapping the desk) while a clock or an animation runs on screen. Trim about one second off the start, export at 1080p, and open the MP4 in QuickTime Player.
    - Good: the clap is heard at the moment the picture shows it (no more than about one frame of 1/60 s off), and the first second you trimmed away is missing in both the picture and the sound.
26. Export again at 4K.
    - Good: sizes are 3840×2160, 2160×2160 and 2160×3840 for the three shapes. The file plays smoothly. It takes longer than 1080p, but the app stays responsive.
27. Quit and reopen the app, then choose Open Last Recording in the menu.
    - Good: your edits (trim, style, zoom blocks) were saved automatically.

## 5a. Home screen

27a. Close the editor window, then open it from the menu bar icon (Open Last Recording), and click "Recordings" at the top left of the editor.
    - Good: the home screen shows a grid of your recordings, newest first, each with a thumbnail, name, date and length. Older recordings get their thumbnails a moment after they scroll into view.
27b. Click a recording. Then click "Recordings" to go back.
    - Good: it opens in the editor. Going back returns to the grid with no delay, and your edits are kept.
27c. Right-click a recording and choose Rename…, type a new name and confirm.
    - Good: the name updates in the grid and stays after you quit and reopen the app.
27d. Right-click a recording and choose Reveal in Finder.
    - Good: Finder opens with that `.nssproj` folder selected.
27e. Right-click a recording and choose Move to Trash, then confirm.
    - Good: it disappears from the grid and is in the Trash (open Trash and use Put Back to restore it, then reopen Home to see it again).
27f. Delete (or move away) every recording.
    - Good: the home screen shows "No recordings yet" and a Record button. Click it.
    - Good: recording starts (with the countdown), and when you stop, the editor opens the new recording.
27g. With 30 or more recordings (copy one `.nssproj` folder several times in Finder), scroll the grid.
    - Good: scrolling stays smooth and the app stays responsive while thumbnails fill in.

## Round 3: Backgrounds

Open any recording in the editor. All checks are in the inspector's Background section and the preview.

B1. Look at the Background section.
    - Good: a segmented control with Wallpapers, Gradients, Colours and Image, and a grid of thumbnails under it. The tab that is open matches the current background (a new project opens on Gradients).
B2. Click Wallpapers.
    - Good: a spinner shows briefly, then the macOS wallpapers on this Mac appear (for example Sonoma, Sequoia Sunrise, Sonoma Horizon, iMac Blue … Yellow, Radial Sky Blue). Wallpapers that macOS only lists as downloads (`.madesktop` such as Big Sur, Catalina) are not shown, because their full image is not on the Mac. The editor stays responsive while thumbnails fill in.
B3. Click a wallpaper.
    - Good: it gets a blue ring and the preview shows it behind the recording right away (a large HEIC may take a fraction of a second the first time). Dynamic wallpapers show one still image.
B4. Drag the Blur slider from 0 to 100 % and back.
    - Good: the wallpaper blurs smoothly while you drag, the preview stays fluid, and 0 % is fully sharp again. Undo (Cmd-Z) after the drag undoes the whole drag in one step.
B5. Close and reopen the Wallpapers tab, then quit and relaunch the app and open it again.
    - Good: the thumbnails appear almost instantly the second time (they are cached on disk in `~/Library/Caches/notscreenstudio/BackgroundThumbnails`).
B6. Click Gradients.
    - Good: 16 thumbnails: 8 smooth linear gradients, then 8 soft multi-colour mesh gradients. Clicking one shows it in the preview immediately. When a linear gradient is selected an Angle slider appears and rotates it.
B7. Click Colours. Click a few swatches, then use "Custom colour" to pick any colour.
    - Good: each pick shows at once. A custom colour that matches no swatch leaves no swatch ringed.
B8. Click Image, then the + tile. Pick a large photo (JPEG, PNG or HEIC).
    - Good: the photo becomes the background, aspect-filled, with no grey frame. A tile for the current image is ringed, and the photo is listed after it in the recents.
B9. Set Blur to about 40 %.
    - Good: the photo blurs. The Blur slider is greyed out on the Wallpapers and Image tabs while a gradient or colour is the background.
B10. Pick a second, different image that has the same file name as the first (from another folder). Then press Cmd-Z.
    - Good: the second image shows; undo brings the first one back (they do not overwrite each other inside the project).
B11. Open a different recording and go to Image.
    - Good: the images you picked before are listed as recents (most recent first, at most 8). Clicking one uses it in this project. Right-click a recent and choose Remove from Recents to drop it.
B12. Export the recording at 1080p with a blurred wallpaper, then again with a blurred picked image, and with a mesh gradient.
    - Good: in each export the background looks exactly like the preview, including the amount of blur (blur is relative to the image, so 1080p and 4K look the same).
B13. Quit, relaunch and reopen the project.
    - Good: the background and the blur amount are as you left them.
B14. Open a project saved by the Round 2 app with a Wallpaper background (for example "Sunset").
    - Good: it opens without an error and looks the same as before; the Gradients tab now shows it as a mesh gradient (the project file is migrated to schema 2).

## 6. Quick regression check for the automated tests

28. In the repo run `swift test -j 2`. Optionally run `NSS_E2E_4K=1 swift test -j 2 --filter EndToEndExportTests` for the 4K exports, and `NSS_BENCH=1 swift test -j 2 --filter PreviewPerformanceTests` for the preview timing budget. `swift test -j 2 --filter ParityTests` prints the export-versus-preview numbers, `--filter EdgeGapTests` checks the corner and edge clicks, and `NSS_BENCH=1 ... --filter PreviewPerformanceTests` prints the frame time report.
    - Good: all tests pass.

## Round 3: Editor layout and design system

R3D-1. Open a recording in the editor.
    - Good: dark theme everywhere. Top bar with Recordings button, editable project name, undo/redo, a 16:9 / 1:1 / 9:16 picker and a blue Export button. Preview in the centre, inspector on the right, timeline across the bottom.
R3D-2. Click the project name, type a new name and press Return. Press Cmd-Z.
    - Good: the name changes, then changes back. After going to Recordings, the home grid shows the current name. An empty name is ignored.
    - Good: while the name field is being edited, typing a space does not play or pause, the arrow keys move the caret and do not step frames, and Cmd-Z undoes typing rather than the last edit. After Return the field loses focus, and Cmd-Z then reverts the name and the field shows the old name.
R3D-3. Switch the aspect ratio picker between 16:9, 1:1 and 9:16.
    - Good: the preview reshapes at once. Export at each and check the file has that shape.
R3D-4. Look at the inspector.
    - Good: sections in order Background, Frame, Cursor, Zoom, Camera, Audio, each with an icon and chevron. Click a header to collapse or expand; the state is remembered after quitting. All sliders, toggles and colour wells look the same across sections.
R3D-5. Drag a slider, then press Cmd-Z.
    - Good: the whole drag undoes in one step.
R3D-6. Look at the timeline.
    - Good: labelled tracks Video, Zoom, Camera, Audio. Video shows thumbnails with yellow trim handles and orange click dots. Camera and Audio show a block when the recording has them, otherwise a short note.
R3D-7. Double-click the Zoom track, then drag the block's middle, its left edge and its right edge. Press Delete.
    - Good: middle moves, edges resize, the block is purple and gets a white outline when selected. Delete removes it.
R3D-8. Drag the playhead, play, trim the start and end.
    - Good: playhead stays smooth, trim still works, playback stays at 60 fps.
R3D-9. Open Export… and the home screen.
    - Good: both use the same dark theme, button styles and spacing as the editor.

## Round 3: recording flow (recorder bar and pause)

1. Click the menu bar icon, then pick a display, a window (Window… menu) or Select Area…. The menu closes and a dark floating bar appears at the bottom centre of the screen that holds the source. It shows the source name, a camera toggle with a device menu, a mic toggle with a device menu, a big round record button and a close button.
2. Toggle the camera and mic and open each device menu. Choosing a device turns that input on. Press Esc or the close button: the bar goes away and nothing is recorded.
3. Pick a source again and press Return or the record button. The bar hides, the 3-2-1 countdown runs, then recording starts. Cancelling the countdown (Esc or a click) brings the setup bar back.
4. While recording, a small bar shows a pulsing red dot, the elapsed time, a pause button and a stop button. Check it stays on top of full-screen apps and follows you across Spaces. Drag it to move it.
5. The menu bar icon turns into a red record symbol with the running time (for example 0:07).
6. Press pause. The dot turns orange, the timer and the menu bar time stop, and the menu bar symbol changes to a pause symbol. Wait about 10 seconds, move the mouse and click, then press resume. The timer continues from where it stopped. Clicks made while paused must not show up in the editor (no click marker, no zoom from them).
7. Stop with the stop button, with Stop Recording in the menu, and (in separate runs) with Control+Command+R. The editor opens with the new recording loaded.
8. In the editor, check the recording length matches the timer (the paused time is not in it), the video plays with no frozen gap where you paused, and the cursor and clicks line up with the screen after the pause. With a mic on, the audio has no silent gap.
9. Open the exported or played video and confirm neither bar, nor the countdown, nor the menu popover appears in it.
10. The menu shows Pause or Resume and Discard while recording. Discard removes the recording and returns to idle.

## Round 3: Zoom blocks on the Zoom track

Use a recording with several clicks in different places (step 7). Rebuild with `./scripts/bundle.sh` first.

Z1. Open a recording made before Round 3 (or a new one) in the editor.
    - Good: the Zoom track (the row labelled "Zoom", under Video) shows one block for each auto-zoom the clicks make. Each block sits just before its click and lasts while you kept working. Auto blocks have a small sparkle in their top right corner; otherwise they look like any other zoom.
    - Good: playing the recording zooms exactly where the blocks are, and feels the same as Round 2 (smooth spring, no jumps).
Z2. Close the editor and open the same recording again.
    - Good: the same blocks are there (they were saved with the project, not recomputed).
Z3. Click a block.
    - Good: it gets a white outline, and the Zoom section of the inspector shows its time range, Zoom level, Focus (Follow cursor or Fixed point), Ease in and Ease out.
Z4. With the block selected, press Delete (Backspace). Then press Command Z.
    - Good: the block disappears and the preview no longer zooms there at once (play over that spot). Undo brings it back.
Z5. Drag a block left and right, into its neighbours and past the start and end of the timeline.
    - Good: it follows the mouse, stops at neighbouring blocks and at the timeline ends, and never overlaps another block. One Command Z puts it back where it was.
Z6. Drag the left and right ends of a block (the mouse turns into a resize arrow over them).
    - Good: the block gets longer or shorter. It stops at neighbours and the timeline ends and never gets shorter than about 0.3 seconds.
Z7. Double-click an empty part of the Zoom track.
    - Good: a new 2× zoom block (no sparkle) appears there and is selected. Double-clicking where there is no room does nothing.
Z8. With a block selected, drag Zoom level, then switch Focus to Fixed point. Drag Focus X and Focus Y, then click "Pick point in preview" and click somewhere in the preview.
    - Good: the preview updates as you drag. With Fixed point the view stays on that point while the zoom is on, even when the cursor moves away. With Follow cursor the view pans to keep the cursor in frame.
    - Good: an auto block you edit loses its sparkle (it is now yours).
Z9. Set Ease in to 0.1 s and play into the block, then set it to 2 s and play again. Do the same with Ease out at the end of the block.
    - Good: a short ease zooms in (or out) quickly, a long one slowly. Both stay smooth and spring-like, with no bounce past the target and no sudden stop.
Z10. Click the trash button next to the "Zoom" track label ("Remove all auto zooms"; the same action is in the Zoom inspector section).
    - Good: only the sparkle blocks disappear; the ones you added or edited stay. One Command Z brings them all back. The button is greyed out when there are no auto zooms.
Z11. Add a zoom of your own over one of the clicks, then click the wand button next to the "Zoom" track label ("Regenerate auto zooms from clicks"; also in the Zoom inspector section).
    - Good: the auto blocks come back from the clicks, your own blocks stay unchanged, and no auto block overlaps your blocks. One Command Z undoes it. On a recording with no clicks the button is greyed out.
Z12. Export (step 23) after these edits and compare it with the preview.
    - Good: the zooms in the MP4 are exactly the blocks on the Zoom track: deleted zooms are gone, edited ones use the new level, focus and ease.
