# NSSRender (Windows/wgpu): how a frame is drawn

`crates/nss-render` is the port of upstream `NSSRender`. `Renderer` implements `FrameRenderer`, and the
preview and the export both use it, so a frame looks the same in both. The translation decisions and
their trade-offs are in `docs/ADR/0011-renderer-parity.md`; this file is the operator's view.

## Using it

```rust
// Falls back to `PassthroughRenderer` only when there is no adapter at all; any other setup error is
// returned so the caller can report it.
let mut renderer = Renderer::new()?;
renderer.set_asset_base_url(Some(bundle_path)); // relative Background::Image(file) names
let frame: Frame = renderer.render(
    &screen,          // &Image, RGBA8 sRGB, tightly packed
    camera.as_ref(),  // Option<&Image>; None when there is no camera track
    &motion.frame_state(t),
    &style,
    PixelSize::new(1920, 1080),
)?;
```

- Frames are tightly packed **RGBA8 sRGB**, both in and out (`frame::Image`, `BYTES_PER_PIXEL = 4`).
  Upstream took IOSurface-backed `32BGRA`; here each frame is one `write_texture`, and
  `Image::to_bgra` converts once at the FFmpeg / Windows-compositor boundary. Any other layout is a
  `RenderError::Frame`/`Image`, not a silent misread.
- The output size is an argument, not a property of a buffer. For export, size it with
  `style.aspect_ratio.output_size_for(preset.resolution)` (16:9 gives 1920x1080 or 3840x2160, 1:1 gives
  1080x1080 or 2160x2160, 9:16 gives 1080x1920 or 2160x3840). For the preview, any size works, but keep
  the aspect ratio: `RenderLayout::output_size_for` derives it.
- `render` is synchronous and takes `&mut self`. It returns once the GPU has finished writing the
  staging buffer, so the returned `Frame` is complete. Each consumer (preview, exporter) owns its own
  renderer; the compiled shader and the cursor atlas are cheap to rebuild but the textures and caches
  are per-renderer.
- Pass `None` for the camera when there is no camera track. The bubble is skipped.
- Relative `Background::Image(file)` names resolve against the asset base URL. The image is decoded
  once (at most 4096 px on its longer side, with linear-light mipmaps) and cached. The cache key is
  the path plus the file's modification time and size, so a file replaced at the same path, or one
  that appears after a frame drew the grey fallback, is picked up automatically. A missing image draws
  mid-grey and does not error.
- **Editor:** call `preload(&style)` on a worker thread when the user picks a background image, not on
  the UI thread: decoding a large image takes tens of milliseconds and the first preview frame pays it
  otherwise. If a file is rewritten in place with exactly the same size within one timestamp tick,
  call `invalidate_background_image()`.
- **Callers resolve bundle images.** The preview and the exporter render `style.resolving_assets(...)`,
  which turns a bundle-relative image into an absolute path. Picked images are copied into the bundle
  under a content-hashed name, so the shared renderer needs no per-project base URL.
- `Background::SystemWallpaper(path)` (a desktop picture, absolute path) is drawn exactly like an
  image. Multi-image HEIC wallpapers are a macOS concept; on Windows the path is whatever the capture
  side found, and a `.heic` simply fails to decode into the grey fallback.
- `Background::Mesh` is a soft multi-colour gradient: up to 8 colour points blended in OKLab with a
  gentle sine warp. The legacy `Background::Wallpaper(name)` (schema v1) is drawn as the mesh from
  `background_presets::legacy_wallpaper(name)` (unknown names draw `aurora`), and the project migrator
  rewrites it to that mesh. `background_presets::gradients()` holds the built-in gradients.
- `Style::background_blur` (0...1) blurs image and wallpaper backgrounds; see "Background blur".
- `rasterizer::BackgroundRasterizer` draws colour and gradient backgrounds on the CPU with the same
  maths (`BackgroundShading` feeds both the uniform block and the rasterizer). The picker uses
  `BackgroundRasterizer::rgba` for thumbnails, and `tests/background_render.rs` checks the GPU against
  it for every preset.
- `CursorState::opacity` (0...1) multiplies with `visible`; the motion model ramps it to fade the
  cursor in and out. `CursorState::shape` picks the image, and `Unknown` draws the arrow.
  `CursorState::press_scale` squishes the cursor around its hotspot (1.0 is normal size).
- The camera bubble shrinks while the view is zoomed in. The multiplier is
  `CameraBubble::size_multiplier(camera_scale)`. Use `RenderLayout::camera_bubble_rect` for the bubble
  as it is drawn at a given camera.
- Nothing here is HDR yet: upstream's `Rgba16Float` tone-map path waits for `nss-capture` to decide
  whether WGC ever hands us HDR frames. Inputs and outputs are sRGB RGBA8.

## Geometry (`RenderLayout`)

`RenderLayout` is the pure geometry every uniform comes from, and the parity contract. The editor can
use it for hit testing and overlays:

- `content_rect`: the screen content, fitted inside the output minus `padding * output_height` on
  every side. It keeps the source aspect and is centred.
- `camera` is the camera after `clamped_camera`, which delegates to `nss_motion::clamped_to_source`.
  The scale is at least 1 and the centre stays `0.5 / scale` away from every source edge, which is the
  same rule the motion model uses. So the content can never show a gap or a stretched edge even when a
  caller passes a camera outside that range. Non-finite values fall back to identity.
- `output_point` / `source_point`: the camera transform. `CameraState::CENTER` lands in the middle of
  `content_rect`, and `scale` magnifies.
- `camera_bubble_full_rect`: the bubble at full size, centred on `bubble.position`. The width is
  `size * W`, and the height and corner radius come from `CameraBubbleGeometry` (circle and square 1:1,
  tall 3:4, wide 16:9, radius = roundness * shorter side / 2, circle always fully round).
- `camera_bubble_rect`: the bubble as drawn at this layout's camera. It is the full rect scaled by
  `camera_bubble_size_multiplier` toward an anchor. `bubble_anchor` picks the anchor per axis from the
  bubble position: the left or top edge in the first third, the right or bottom edge in the last
  third, a linear blend in the middle third. A bubble in a corner shrinks into that corner, one on an
  edge shrinks toward that edge, and because the anchor is continuous, dragging while zoomed never
  makes it jump.
- `cursor_height`, `cursor_pixels_per_point` (= height / 19), `ripple_radius`, `ripple_opacity`,
  `ripple_thickness` and `cursor_blur_vector`: the cursor and ripple sizes. They scale with the content
  size, the zoom and `CursorStyle::size`.

## The pass

Each frame is one render pass with one full-screen triangle. A single fragment shader
(`src/shader.wgsl`) does all of the work below per output pixel: no intermediate textures, no extra
passes.

1. **Background.** A solid colour; a linear gradient (up to 8 stops, interpolated in OKLab so
   midpoints stay vivid, `angle_degrees` 0 = left to right, 90 = top to bottom); an aspect-filled image
   or wallpaper (sampled trilinearly from mipmaps, optionally pre-blurred); or a mesh gradient (colour
   points blended in OKLab with a gentle warp).
2. **Screen shadow.** An analytic Gaussian: `erf` of the rounded-rect signed distance, sigma =
   `shadow.radius * H / 2`, offset by `offset_x/offset_y * H`, scaled by `opacity`. No blur pass.
3. **Screen content.** The source is cropped and zoomed by the clamped camera. Every texture tap is
   also clamped to 0...1, so nothing outside the source is read. To downscale (4K source to 1080p
   output, say) a footprint filter takes `ceil(texels_per_pixel)` squared bilinear taps (at most 6x6)
   spread across the output pixel: a box filter with no mipmap copy. Corners are a rounded-rect SDF
   with 1 px anti-aliasing.
4. **Click ripples** (at most 8, only when `click_ripples` is on). Each is an anti-aliased ring with a
   faint fill and a soft dark halo so it reads on light and dark interfaces. The radius eases out and
   opacity is `(1 - progress)^1.5`. Ripples are clipped to the content.
5. **Cursor.** An image from the cursor atlas at the hotspot on `CursorState::position`, mapped through
   the camera, drawn at `cursor_pixels_per_point * press_scale` output pixels per point, so the press
   squish scales around the hotspot. The atlas is sampled trilinearly at a level computed on the CPU
   (`log2(8 / pixels_per_point)`, clamped to 0..=4), so a small cursor is filtered properly and a large
   one stays sharp. Motion blur averages up to 16 atlas samples along the velocity vector, centred on
   the position, for every shape. The shutter is `motion_blur / 30` s and the blur is capped at 3x the
   cursor height; a still cursor gets one sample and stays crisp. The cursor is evaluated only inside
   its bounding box and is clipped to the content.
6. **Camera bubble.** Every shape is one rounded-box SDF with the half size and corner radius from
   `camera_bubble_rect` and the shared bubble geometry, with 1 px anti-aliasing. A circle is a square
   with radius equal to half its side. The bubble has its own analytic shadow (the same `Style::shadow`
   values) from the same SDF, an aspect-fill crop of the camera frame with the same footprint filter,
   an optional horizontal mirror, and a border drawn inside the edge (`border.width * H`). While the
   bubble shrinks during a zoom, the border width, shadow blur and shadow offset are multiplied by the
   same size multiplier, so the whole bubble scales as one piece.
7. **Output.** All blending above is in linear light. Textures are read as plain `rgba8unorm` and the
   shader does the sRGB decode itself, matching upstream's `_srgb` views. The result is sRGB-encoded in
   the shader and a triangular ±1 LSB dither is added to **background pixels only** (never screen or
   camera pixels, so text stays exact), then written to an `rgba8unorm` target.

## Background blur

`Style::background_blur` (0...1) becomes a sigma by `image_blur::sigma_for`, which quantises first and
returns `None` below the visible threshold, so a slider at 0 really is a sharp image. The plan is
upstream's: `MAX_WORKING_SIGMA = 6.0` per axis, so the kernel stays around 19 taps by starting from a
mip level chosen by `image_blur::plan`. Both a full-size and a blurred variant of an image are cached;
`tests/background_render.rs` checks that a blur looks the same at every output size and that two
renderers produce byte-identical blurred frames.

The blur itself is `src/blur.wgsl` as two fullscreen passes (horizontal, then vertical) into
`rgba16float` targets, sampled back at LOD 0. One encoder and one `queue.submit` per pass is
deliberate: `queue.write_buffer` is ordered against submissions, so a shared encoder lets the second
pass's parameters overwrite the first's.