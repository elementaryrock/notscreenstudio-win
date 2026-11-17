# ADR 0011 — Renderer parity: what "the same frame" means on wgpu

Status: accepted (Stage 4)

## Context

`NSSRender` is a single-pass Metal compositor: one full-screen fragment shader draws the background,
the screen's drop shadow, the zoomed screen content with rounded corners, the click ripples, the
motion-blurred cursor and the camera bubble. On top of it sit a layout type whose geometry *is* the
preview/export parity contract, a CPU background rasterizer that feeds the picker's thumbnails, a
cursor atlas and an image blur.

Non-negotiable upstream invariant: a preview frame and an export frame of the same instant must be
identical, not merely similar. Everything below exists to keep that true across two completely
different GPU APIs — and, harder, across the GPU and the CPU rasterizer.

## Decision

### 1. Frames are packed RGBA8 sRGB, not IOSurface BGRA

Upstream exchanges `CVPixelBuffer`s (32BGRA, IOSurface-backed) so Metal can wrap them with a texture
cache and copy nothing. wgpu has no equivalent guarantee, so `crate::frame::Image` is tightly packed
RGBA8 in sRGB encoding — the same *values* upstream's `bgra8Unorm` view holds, in a different order
and without a zero-copy path. Consequences:

- Each frame costs one `queue.write_texture`. At 4K that is 33 MB per frame; measured cost is inside
  the budget recorded in `PLAN.md` Stage 4, and the alternative (a platform-specific shared-surface
  path) would be per-vendor code, i.e. exactly the fragility this port is trying to avoid.
- `Image::to_bgra` exists for the FFmpeg pipe and the Windows compositor, so the conversion happens
  once, at the boundary that needs it.
- The output size is a parameter of `FrameRenderer::render`, not a property of the buffer, which is
  what lets the exporter ask for a different size with the same style.

### 2. The output target is `Rgba8Unorm`; the shader encodes sRGB itself

Upstream renders into a `bgra8Unorm` view and calls `linearToSrgb` in the shader, so the dither is
applied to *encoded* values. A wgpu `Rgba8UnormSrgb` target would do the encode in the hardware,
after the shader — a different value for every dithered pixel, and a different value from the CPU
rasterizer. Both matters, so this port keeps upstream's arrangement: target `Rgba8Unorm`, shader
does `linearToSrgb`, dither added afterwards.

### 3. Every sample is explicit: `textureSampleLevel` only

`textureSample` needs implicit derivatives, which depend on neighbouring invocations and therefore on
how the driver schedules them. `crates/nss-render/src/shaders.rs` has a test that fails on any
`textureSample(` in `shader.wgsl`. The mip levels the shader needs are computed on the CPU
(`uniforms::cursor_lod`, `uniforms::taps`, `image_lod`) and passed in the uniform block, so a frame is
a pure function of (frame state, style, output size, source pixels) — no matter which renderer, or
which order the tiles were rasterized in.

Two corollaries:

- The footprint filter's tap count is computed on the CPU for the same reason (six taps maximum,
  matching upstream).
- `textureSampleLevel` with an explicit level also makes the background image's LOD auditable: the
  renderer passes `log2(image_width / short_side)` for a sharp image and `0` for a blurred one.

### 4. The uniform block's slot order is load-bearing

Upstream writes a `struct Uniforms` of `simd_float4`s; this port keeps the same 46 slots in the same
order, 16 bytes each (736 bytes total), with `Slot::name()` and `Slot::index()` shared by Rust and
WGSL through `shaders::defines()`. The numbers are not a private detail: `shader.wgsl` indexes
`u.slots[U_*]` and the Rust writer indexes the same array. A unit test checks every slot's name
appears as a `const U_X: u32 = N;` with the matching index, and another checks the serialised block is
`16 * COUNT` bytes.

### 5. The cursor atlas is baked, not sampled from the OS

macOS gives `NSCursor` images; Windows' equivalents are theme-dependent, DPI-dependent and not
stable enough to hash a frame against. `CursorAtlas` therefore draws all 18 shapes as vector art at
8× (upstream's atlas scale), packs them into one texture with 32 px padding, and builds a full mip
chain in linear premultiplied space. `Cell::is_system_image` is always `false`, which is upstream's
name for "we drew this ourselves".

Two smaller decisions:

- The sampler is clamp-to-edge rather than Metal's clamp-to-zero. The shader already rejects samples
  more than 2 pt outside the image and the padding is transparent, so the two agree where it counts;
  WebGPU core has no clamp-to-zero for filtering samplers.
- The arrow's hotspot is the black body's apex with the 1.3 pt outline reaching the image's
  top-left corner, so the outline is never clipped and the tip lands on the cursor position
  (`tests/cursor_render.rs`).

### 6. Image blur: two render passes instead of a compute kernel, one submit each

Upstream blurs with a Metal compute kernel into a private texture. Here the same separable Gaussian
runs as two fullscreen passes into `Rgba16Float` targets, with the same plan as upstream
(`MAX_WORKING_SIGMA = 6`, `MIN_SIGMA = 0.3`, `radius = ceil(3σ)`), so the kernel stays ~19 taps even
for a 100 px blur and the result is the same at every output size (the radius is relative to the
image, not the frame).

Render targets are used instead of storage textures because `rgba16float` render targets are core
WebGPU while storage-texture formats are not uniformly available.

Each pass gets its own encoder and its own `queue.submit`. This is not style: `queue.write_buffer` is
ordered against *submissions*, so two passes sharing one encoder both read whichever parameters were
written last. That bug produced a "blurred" background identical to the sharp one (and, with a
different mip level, an almost entirely red frame); `tests/background_render.rs` now covers it.

The blurred texture has a single mip and is sampled at LOD 0: it is already low-pass filtered, so
mip selection has nothing left to do.

### 7. The camera clamp is shared with the motion model

`layout::clamped_camera` is a one-line delegation to `nss_motion::clamped_to_source`
(`nss-render` depends on `nss-motion`). Upstream has the clamp in both places; duplicating it here is
exactly how the preview and the exported file would come to disagree by one ULP at the edges of a
zoom, so the renderer uses the function whose property test already proves the viewport stays inside
the source.

### 8. No adapter is a supported state

`Renderer::new()` tries the high-performance adapter, then a fallback (software) adapter, and reports
`RenderError::NoAdapter` when neither exists. The shell then uses `PassthroughRenderer`, which fills
the frame with the style's background colour — the same degradation upstream applies on a Mac without
Metal. Every GPU test starts with `need_gpu!()` and returns early when no adapter is available, so
the suite is green in a session without a GPU and covers real pixels where there is one.

### 9. `render` is `&mut self` and synchronous

The renderer owns its textures, staging buffer and caches, so it needs exclusive access; the preview
and the exporter each own one. Readback is a padded staging buffer (`bytes_per_row` rounded up to
256, the copy alignment) followed by a row-by-row copy into a packed `Frame`.

## Consequences

- Preview and export share one code path, and the parity claims are tested rather than asserted:
  `tests/metal_renderer.rs::render_is_deterministic` compares two renderers byte for byte, and
  `tests/background_render.rs::blur_is_deterministic_across_renderers` does the same for a blurred
  background.
- The GPU is compared against the CPU rasterizer for every built-in gradient
  (`gpu_matches_cpu_rasterizer`): ±2 LSB for linear presets (dither plus rounding) and ±3 for mesh
  presets, which go through `pow`/`sin` in f32 on the GPU and f64 on the CPU. The thumbnails and the
  frame therefore cannot drift apart silently.
- `nss-render` is `#![forbid(unsafe_code)]`. wgpu, `image` and `half` are its only non-workspace
  dependencies, and none of them touches the network.
- The atlas, the shader and the slot table are compiled in (`include_str!`), so there is no runtime
  asset to lose next to the executable.
