# NSSRender: how a frame is drawn

`MetalRenderer` implements `FrameRenderer`. The preview and the export both use it, so a frame looks the same in both.

## Using it

```swift
// Falls back to PassthroughRenderer only when there is no Metal device; any other setup error is thrown.
let renderer = try MetalRenderer.makeDefault(assetBaseURL: projectBundleURL)
try renderer.render(screen: screenPB, camera: cameraPB, state: motion.frameState(at: t), style: project.style, into: outputPB)
```

- All three pixel buffers must be IOSurface-backed `kCVPixelFormatType_32BGRA` buffers. Use `PixelBuffers.make(width:height:)`, a `CVPixelBufferPool` with `kCVPixelBufferIOSurfacePropertiesKey` and `kCVPixelBufferMetalCompatibilityKey`, or the `AVAssetWriterInputPixelBufferAdaptor` pool. Any other format throws `NSSError.unsupported`.
- The output size comes from the output buffer. For export, size it with `style.aspectRatio.outputSize(for: preset.resolution)` (16:9 gives 1920x1080 or 3840x2160, 1:1 gives 1080x1080 or 2160x2160, 9:16 gives 1080x1920 or 2160x3840). For the preview, any size works, but keep the same aspect ratio.
- `render` is synchronous. It returns once the GPU has finished writing `output`. Calls on one renderer are serialized by a lock, so the preview and the export should each create their own renderer. The compiled shader is shared between renderers.
- Pass `camera: nil` when there is no camera track. The bubble is skipped.
- Relative `Background.image(file:)` names are resolved against `assetBaseURL` (the `.nssproj` bundle). The image is decoded once (at most 4096 px on its longer side, with mipmaps) and cached. The cache key is the path plus the file's modification time and size, which the renderer reads with one `stat` per frame. So a file that is replaced at the same path, or that appears after a frame drew the grey fallback, is picked up automatically. A missing image draws mid-grey and does not throw.
- **Editor:** when the user picks a new background image, call `renderer.preload(style:)` on a background queue, not the main thread. Decoding a large image takes tens of milliseconds, and without the preload the first preview frame pays that cost. If you rewrite a file in place with exactly the same size within one timestamp tick, call `invalidateBackgroundImage()`.
- **Callers resolve bundle images.** The preview and the exporter render `style.resolvingAssets(relativeTo: bundleURL)`, which turns a bundle-relative `.image(file:)` into an absolute path. Picked images are copied into the bundle under a content-hashed name (`background-<hash>-<name>`), so the shared renderer needs no per-project base URL. `assetBaseURL` still works for callers that set it.
- `Background.systemWallpaper(path:)` (a macOS desktop picture, absolute path) is drawn exactly like `.image`. Multi-image HEIC files (dynamic and light/dark wallpapers) draw their primary image.
- `Background.mesh(points:warp:)` is a soft multi-colour gradient: up to 8 colour points blended in OKLab with a gentle sine warp. The legacy `Background.wallpaper(name:)` (schema v1; names in `MetalRenderer.wallpaperNames`, unknown names draw `aurora`) is drawn as the mesh from `BackgroundPresets.legacyWallpaper(named:)`, and the project migrator rewrites it to that mesh. The built-in gradients are `BackgroundPresets.gradients` in NSSCore.
- `Style.backgroundBlur` (0...1) blurs `.image` and `.systemWallpaper` backgrounds; see "Background blur" below.
- `BackgroundRasterizer` draws colour and gradient backgrounds on the CPU with the same maths (`BackgroundShading` feeds both the uniforms and the rasterizer). The picker uses it for thumbnails, and `BackgroundRenderTests` checks the GPU against it for every preset.
- `CursorState.opacity` (0...1) multiplies with `visible`. The motion model ramps it to fade the cursor in and out.
- `CursorState.shape` picks the cursor image, and `.unknown` draws the arrow. `CursorState.pressScale` squishes the cursor around its hotspot (1 means normal size).
- The camera bubble shrinks while the view is zoomed in. The multiplier is `CameraBubble.sizeMultiplier(cameraScale:)` from NSSCore. Use `RenderLayout.cameraBubbleRect` to get the bubble as it is drawn at a given camera.

## Geometry (`RenderLayout`)

`RenderLayout` is the pure-Swift geometry that every shader uniform comes from. The editor can use it for hit testing and overlays:

- `contentRect`: the screen content, fitted inside the output minus `padding * outputHeight` on every side. It keeps the source aspect and is centered.
- `camera` is the camera after `clampedCamera(_:)`. The scale is at least 1, and the centre stays `0.5 / scale` away from every source edge, which is the same rule the motion model uses. So the screen content can never show a gap or a stretched edge, even when a caller passes a camera outside that range. Non-finite values fall back to identity.
- `outputPoint(forSource:)` and `sourcePoint(forOutput:)`: the camera transform. `CameraState.center` lands at the middle of `contentRect`, and `scale` magnifies.
- `cameraBubbleFullRect(_:)`: the bubble at full size, centred on `bubble.position`. The width is `size * W`, and the height and corner radius come from `CameraBubbleGeometry.layout` (circle and square 1:1, tall 3:4, wide 16:9, radius = roundness * shorter side / 2, circle always fully round).
- `cameraBubbleRect(_:cameraSize:)`: the bubble as drawn at this layout's camera. It is the full rect scaled by `cameraBubbleSizeMultiplier(_:)` toward an anchor point. `bubbleAnchor(_:)` picks the anchor on each axis from the bubble position: the left or top edge in the first third, the right or bottom edge in the last third, and a linear blend in the middle third. A bubble in a corner shrinks into that corner, and a bubble on an edge shrinks toward that edge. Because the anchor is continuous, dragging the bubble while zoomed never makes it jump.
- `cursorHeight(style:)`, `cursorPixelsPerPoint(style:)` (= height / 19), `rippleRadius(progress:)` and `cursorBlurVector(velocity:style:)`: the cursor and ripple sizes. They scale with the content size, the zoom and `CursorStyle.size`.

## The pass

Each frame is one render pass with one full-screen triangle. A single fragment shader does all the work below for each output pixel. There are no intermediate textures and no extra passes.

1. **Background.** This is a solid colour, a linear gradient (up to 8 stops, interpolated in OKLab so the midpoints stay vivid, with `angleDegrees` 0 meaning left to right and 90 meaning top to bottom), an aspect-filled image or system wallpaper (sampled trilinearly from mipmaps, optionally pre-blurred), or a mesh gradient (colour points blended in OKLab with a gentle warp).
2. **Screen shadow.** This is an analytic Gaussian: `erf` of the rounded-rect signed distance, with sigma = `shadow.radius * H / 2`, offset by `offsetX/Y * H` and scaled by `opacity`. It needs no blur pass.
3. **Screen content.** The source is cropped and zoomed by the (clamped) camera. Every texture tap is also clamped to 0...1, so nothing outside the source is ever read. To downscale (for example a 4K source to a 1080p output), a footprint filter takes `ceil(texelsPerPixel)` by `ceil(texelsPerPixel)` bilinear taps (at most 6x6) spread across the output pixel. This is a box filter with no mipmap copy, so the input stays zero-copy. The corners are a rounded-rect SDF with 1 px anti-aliasing.
4. **Click ripples** (at most 8, only when `clickRipples` is on). Each is an anti-aliased ring with a faint fill and a soft dark halo, so it shows on both light and dark interfaces. The radius eases out and the opacity is `(1 - progress)^1.5`. Ripples are clipped to the content.
5. **Cursor.** This is the real macOS cursor image for `CursorState.shape`, taken from the cursor atlas (see below). Its hotspot sits on `CursorState.position`, mapped through the camera. It is drawn at `cursorPixelsPerPoint * pressScale` output pixels per point, so the press squish scales it around the hotspot. The images already contain the system's soft shadow and outline. The atlas is sampled trilinearly at a mip level computed on the CPU (`log2(8 / pixelsPerPoint)`, from 0 to 4), so a small cursor is filtered properly and a large one stays sharp. Motion blur averages up to 16 atlas samples along the velocity vector, centred on the position, and it works the same for every shape. The shutter is `motionBlur / 30` s and the blur is capped at 3x the cursor height. A still cursor gets one sample, so it stays crisp. The cursor is evaluated only inside its bounding box and is clipped to the content.
6. **Camera bubble.** Every shape (circle, square, tall, wide) is one rounded-box SDF with the half size and corner radius from `cameraBubbleRect` and `CameraBubbleGeometry`, with 1 px anti-aliasing. A circle is a square with radius equal to half its side. The bubble has its own analytic shadow (the same `Style.shadow` values) from the same SDF, an aspect-fill crop of the webcam frame with the same footprint filter, an optional horizontal mirror, and a border drawn inside the edge (`border.width * H`). When the bubble shrinks during zoom, the border width, shadow blur and shadow offset are multiplied by the same size multiplier, so the whole bubble scales as one piece.
7. **Output.** All blending above is in linear light. Inputs are read through `bgra8Unorm_srgb` views, so the GPU decodes them. The result is sRGB-encoded in the shader, and a triangular ±1 LSB dither is added to background pixels only, to prevent gradient banding. The screen and the camera pixels are never dithered, so text stays exact. Then the result is written to a `bgra8Unorm` view of the output buffer.

## Background blur

`ImageBlur` blurs an image background once per (file, modification stamp, blur amount rounded to 1/100) and caches the result next to the decoded image, so the per-frame pass is unchanged. It is a separable Gaussian in a compute kernel compiled at runtime from a Swift string (`ImageBlur.source`), in linear light.

- The radius is relative to the image, not the output: sigma = `backgroundBlur * BackgroundBlur.maxSigmaFraction (0.035) * the image's shorter side`. The same blurred texture is used at every output size, so a 2/3-size preview, a 1080p export and a 4K export show the same picture.
- To keep the kernel at most about 19 taps per pass, it reads the mip level where sigma is at most 6 texels (`ImageBlur.plan`): each level halves the sigma in texels. The result is a mipmapped `rgba16Float` texture holding linear-light colour, sampled by the main shader exactly like the sRGB image texture.
- Edges clamp, so the borders keep their colour instead of darkening.
- A slider drag re-blurs only when the rounded amount changes. The decode is reused. A blur below 1/100 draws the sharp image.

## Cursor atlas

`CursorAtlas` (in `CursorAtlas.swift`) rasterizes the system cursors once per process: `NSCursor.arrow`, `.iBeam`, `.iBeamCursorForVerticalLayout`, `.pointingHand`, `.openHand`, `.closedHand`, `.crosshair`, the six resize cursors, `.operationNotAllowed`, `.dragCopy`, `.dragLink` and `.contextualMenu`. It reads `NSCursor.current` first, because AppKit loads the arrow and I-beam images lazily, and without that call they come back empty when no `NSApplication` is running. For each cursor it takes the largest bitmap representation and draws it at 8 pixels per point with high-quality interpolation. AppKit ships the arrow and I-beam up to 10x, so those stay sharp at 4K zoom. The other cursors only exist at 2x, so they are upscaled once on the CPU, which is sharper than magnifying them on the GPU, but they can look slightly soft when the view is zoomed in a lot at 4K.

- Pixels are linear-light sRGB and premultiplied, stored as `rgba16Float` with a full mip chain. That way filtering, mipmaps and blending are all correct in linear light.
- Cells are shelf-packed into rows up to 2048 px wide. Each cell has 32 clear pixels of padding, so the samples never bleed into a neighbouring cell down to mip level 4. The shader also ignores samples more than 2 pt outside the image, so long blur vectors cannot reach a neighbouring cell.
- Each cell keeps the cursor's size in points and its `hotSpot` (in points from the image's top-left corner).
- If AppKit has no image for a cursor, that shape draws the arrow. If even the arrow is missing, a drawn arrow with the same 17x23 pt frame and (4, 4) hotspot is used instead.
- `CursorAtlasCache` builds the atlas and uploads it once per Metal device, then drops the CPU pixels. All renderers on that device share one texture, about 6 MB with its mips.

## Determinism

The output depends only on the inputs: the pixels, `FrameState`, `Style` and the output size. It does not depend on the pixel buffer pool, the row stride or which renderer instance draws it. Every uniform is computed from `RenderLayout` on the CPU, and the cursor mip level is set explicitly instead of coming from screen-space derivatives. The `previewAndExportPoolsGiveByteIdenticalFrames` test renders the same frames through two renderers into two pools with different row alignments and compares the bytes.

## Performance

The renderer allocates nothing per frame except the unavoidable `CVMetalTexture` wrappers and the command buffer. The library is compiled once per device (`PipelineCache`). The uniform buffer, the placeholder texture and the render pass descriptor are created once. Uniforms are a flat array of `float4` slots (`Slot` in `Shaders.swift`). The shader receives `#define U_<SLOT>` indices generated from the Swift enum, so the two layouts can never drift apart. The `benchmark4KFrame` test renders a full 4K frame (4K source, zoom, gradient, shadow, a blurred cursor, 3 ripples and a bordered bubble) and prints the best and median times. It asserts the 8 ms budget only when `NSS_BENCH=1` is set (`NSS_BENCH=1 swift test --filter benchmark4KFrame`), because wall-clock budgets are unreliable on loaded machines and CI VMs. Measured on an M-series MacBook Air under heavy load: about 2.5 ms best and 3.2 ms median.

## Shader source

The shaders are Swift strings in `Shaders.swift`, compiled at runtime with `MTLDevice.makeLibrary(source:options:)`. There are no `.metal` files because this toolchain has no Metal compiler.
