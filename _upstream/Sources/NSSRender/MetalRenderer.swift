import CoreGraphics
import CoreVideo
import Foundation
import ImageIO
import Metal
import NSSCore

/// GPU compositor on Metal. One full-screen fragment pass draws the whole frame: background,
/// screen shadow, zoomed screen content with rounded corners, click ripples, the motion-blurred
/// cursor and the camera bubble. See `RENDERING.md` in this folder.
///
/// Thread safety: `render` may be called from any thread; calls are serialized by an internal lock.
/// Use one renderer per consumer (preview, export) for parallelism.
public final class MetalRenderer: FrameRenderer, @unchecked Sendable {
    /// True when a Metal device exists (false on some CI VMs).
    public static var isSupported: Bool { MTLCreateSystemDefaultDevice() != nil }

    /// Built-in wallpaper names for `Background.wallpaper(name:)`.
    public static var wallpaperNames: [String] { Wallpapers.names }

    /// A `MetalRenderer`, or the CPU `PassthroughRenderer` ONLY when this Mac has no Metal device.
    /// Any other setup failure (shader compile, command queue, texture cache, buffers) is thrown,
    /// so an export never silently produces flat-colour frames.
    public static func makeDefault(assetBaseURL: URL? = nil) throws -> any FrameRenderer {
        guard let device = MTLCreateSystemDefaultDevice() else { return PassthroughRenderer() }
        return try MetalRenderer(device: device, assetBaseURL: assetBaseURL)
    }

    public let device: MTLDevice
    private let queue: MTLCommandQueue
    private let pipeline: MTLRenderPipelineState
    private let textureCache: CVMetalTextureCache
    private let uniforms: MTLBuffer
    private let placeholder: MTLTexture
    private let cursorAtlas: MTLTexture
    private let cursorCells: [CursorShape: CursorAtlas.Cell]
    private let passDescriptor = MTLRenderPassDescriptor()
    private let outputTextureAttributes: CFDictionary

    private let lock = NSLock()
    private var baseURL: URL?
    private var imageCache: (path: String, stamp: ImageStamp, texture: MTLTexture?)?
    /// The blurred copy of `imageCache`'s texture for one blur amount (see `ImageBlur`).
    private var blurCache: (path: String, stamp: ImageStamp, amount: Double, texture: MTLTexture?)?

    /// - Parameter assetBaseURL: folder used to resolve relative `Background.image(file:)` names
    ///   (normally the `.nssproj` bundle).
    public init(device: MTLDevice? = nil, assetBaseURL: URL? = nil) throws {
        guard let device = device ?? MTLCreateSystemDefaultDevice() else {
            throw NSSError.unsupported("Metal is not available on this Mac")
        }
        self.device = device
        guard let queue = device.makeCommandQueue() else { throw NSSError.renderFailed("makeCommandQueue failed") }
        self.queue = queue
        pipeline = try PipelineCache.shared.pipeline(for: device)

        var cache: CVMetalTextureCache?
        let st = CVMetalTextureCacheCreate(nil, nil, device, nil, &cache)
        guard st == kCVReturnSuccess, let cache else { throw NSSError.renderFailed("CVMetalTextureCacheCreate failed: \(st)") }
        textureCache = cache

        guard let buffer = device.makeBuffer(length: Slot.byteCount, options: .storageModeShared) else {
            throw NSSError.renderFailed("uniform buffer allocation failed")
        }
        uniforms = buffer

        let desc = MTLTextureDescriptor.texture2DDescriptor(pixelFormat: .bgra8Unorm_srgb, width: 1, height: 1, mipmapped: false)
        desc.usage = .shaderRead
        guard let tex = device.makeTexture(descriptor: desc) else { throw NSSError.renderFailed("placeholder texture failed") }
        var black: UInt32 = 0xFF00_0000
        tex.replace(region: MTLRegionMake2D(0, 0, 1, 1), mipmapLevel: 0, withBytes: &black, bytesPerRow: 4)
        placeholder = tex

        (cursorAtlas, cursorCells) = try CursorAtlasCache.shared.texture(for: device, queue: queue)

        let usage = MTLTextureUsage([.renderTarget, .shaderRead]).rawValue
        outputTextureAttributes = [kCVMetalTextureUsage: NSNumber(value: usage)] as CFDictionary
        passDescriptor.colorAttachments[0].loadAction = .dontCare
        passDescriptor.colorAttachments[0].storeAction = .store
        baseURL = assetBaseURL
    }

    /// Folder used to resolve relative background image names.
    public var assetBaseURL: URL? {
        get { lock.withLock { baseURL } }
        set { lock.withLock { baseURL = newValue } }
    }

    /// Loads and caches the background image for `style` now, so the first frame that needs it does
    /// not pay the decode cost. Optional: `render` loads lazily otherwise.
    public func preload(style: Style) {
        lock.withLock { _ = backgroundImage(for: style) }
    }

    // MARK: - FrameRenderer

    public func render(screen: CVPixelBuffer, camera: CVPixelBuffer?, state: FrameState, style: Style, into output: CVPixelBuffer) throws {
        try lock.withLock {
            try renderLocked(screen: screen, camera: camera, state: state, style: style, into: output)
        }
    }

    private func renderLocked(screen: CVPixelBuffer, camera: CVPixelBuffer?, state: FrameState, style: Style, into output: CVPixelBuffer) throws {
        let (screenTex, screenRef) = try texture(for: screen, format: .bgra8Unorm_srgb, attributes: nil)
        let (outTex, outRef) = try texture(for: output, format: .bgra8Unorm, attributes: outputTextureAttributes)
        var cameraTex: MTLTexture?
        var cameraRef: CVMetalTexture?
        if let camera, style.camera.visible {
            let pair = try texture(for: camera, format: .bgra8Unorm_srgb, attributes: nil)
            cameraTex = pair.0
            cameraRef = pair.1
        }
        let image = backgroundImage(for: style)

        let layout = RenderLayout(
            outputSize: PixelSize(width: outTex.width, height: outTex.height),
            sourceSize: PixelSize(width: screenTex.width, height: screenTex.height),
            style: style,
            camera: state.camera
        )
        writeUniforms(layout: layout, state: state, style: style, camera: cameraTex, image: image)

        guard let cmd = queue.makeCommandBuffer() else { throw NSSError.renderFailed("makeCommandBuffer failed") }
        passDescriptor.colorAttachments[0].texture = outTex
        guard let enc = cmd.makeRenderCommandEncoder(descriptor: passDescriptor) else {
            throw NSSError.renderFailed("makeRenderCommandEncoder failed")
        }
        enc.setRenderPipelineState(pipeline)
        enc.setFragmentBuffer(uniforms, offset: 0, index: 0)
        enc.setFragmentTexture(screenTex, index: 0)
        enc.setFragmentTexture(cameraTex ?? placeholder, index: 1)
        enc.setFragmentTexture(image ?? placeholder, index: 2)
        enc.setFragmentTexture(cursorAtlas, index: 3)
        enc.drawPrimitives(type: .triangle, vertexStart: 0, vertexCount: 3)
        enc.endEncoding()
        cmd.commit()
        cmd.waitUntilCompleted()
        passDescriptor.colorAttachments[0].texture = nil
        // The CVMetalTextures must outlive the GPU work.
        withExtendedLifetime((screenRef, outRef, cameraRef)) {}
        if let error = cmd.error { throw NSSError.renderFailed("GPU error: \(error.localizedDescription)") }
    }

    // MARK: - Uniforms

    private func writeUniforms(layout: RenderLayout, state: FrameState, style: Style, camera: MTLTexture?, image: MTLTexture?) {
        let u = uniforms.contents().bindMemory(to: SIMD4<Float>.self, capacity: Slot.allCases.count)
        for i in 0..<Slot.allCases.count { u[i] = .zero }
        func set(_ s: Slot, _ x: Double, _ y: Double = 0, _ z: Double = 0, _ w: Double = 0) {
            u[s.rawValue] = SIMD4(Float(x), Float(y), Float(z), Float(w))
        }
        func setAt(_ s: Slot, _ offset: Int, _ x: Double, _ y: Double = 0, _ z: Double = 0, _ w: Double = 0) {
            u[s.rawValue + offset] = SIMD4(Float(x), Float(y), Float(z), Float(w))
        }

        let W = Double(layout.outputSize.width), H = Double(layout.outputSize.height)
        let rect = layout.contentRect
        let cam = layout.camera
        set(.out, W, H, 1)
        set(.content, rect.minX, rect.minY, rect.width, rect.height)
        set(.content2, layout.cornerRadius, cam.scale, cam.center.x, cam.center.y)
        let duvX = 1 / (rect.width * cam.scale), duvY = 1 / (rect.height * cam.scale)
        set(.screenSS, duvX, duvY, Double(Self.taps(texelsPerPixel: layout.sourceTexelsPerOutputPixel)))

        let sh = style.shadow
        set(.shadow, max(0, sh.radius) * H / 2, max(0, min(1, sh.opacity)), sh.offsetX * H, sh.offsetY * H)

        writeBackground(BackgroundShading.make(style.background, W: W, H: H), image: image, W: W, H: H,
                        set: { set($0, $1, $2, $3, $4) }, setAt: { setAt($0, $1, $2, $3, $4, $5) })

        // Cursor: the system cursor image for `shape`, hotspot on the position, squished around the hotspot.
        let cs = state.cursor
        let alpha = cs.visible && cs.opacity.isFinite ? max(0, min(1, cs.opacity)) : 0
        if alpha > 0, let cell = cursorCells[cs.shape] ?? cursorCells[.arrow] {
            let press = cs.pressScale.isFinite ? max(0.05, min(4, cs.pressScale)) : 1
            let ppp = layout.cursorPixelsPerPoint(style: style.cursor) * press
            if ppp > 0 {
                let hot = layout.outputPoint(forSource: cs.position)
                let blur = layout.cursorBlurVector(velocity: cs.velocity, style: style.cursor)
                let len = (blur.dx * blur.dx + blur.dy * blur.dy).squareRoot()
                let samples = len < 1 ? 1 : min(16, Int((len / 2).rounded(.up)) + 1)
                let bx = samples == 1 ? 0 : blur.dx, by = samples == 1 ? 0 : blur.dy
                let lod = Self.cursorLOD(pixelsPerPoint: ppp)
                set(.cursor, hot.x, hot.y, ppp, alpha)
                set(.cursorBlur, bx, by, Double(samples), lod)
                // Image bounds in output px, grown by the shader's 2 pt margin, half the blur and 1 px.
                let mx: Double = 2 * ppp + 1 + Double(abs(bx)) / 2, my: Double = 2 * ppp + 1 + Double(abs(by)) / 2
                let hx = Double(hot.x), hy = Double(hot.y)
                let sx = Double(cell.hotspot.x), sy = Double(cell.hotspot.y)
                let iw = Double(cell.size.width), ih = Double(cell.size.height)
                let minX: Double = hx - sx * ppp - mx
                let maxX: Double = hx + (iw - sx) * ppp + mx
                let minY: Double = hy - sy * ppp - my
                let maxY: Double = hy + (ih - sy) * ppp + my
                set(.cursorBox, minX, minY, maxX, maxY)
                let aw = Double(cursorAtlas.width), ah = Double(cursorAtlas.height)
                set(.cursorCell, Double(cell.x) / aw, Double(cell.y) / ah, Double(cell.width) / aw, Double(cell.height) / ah)
                set(.cursorCell2, cell.hotspot.x, cell.hotspot.y, cell.size.width, cell.size.height)
            }
        }

        // Ripples
        if style.cursor.clickRipples {
            var n = 0
            for r in cs.ripples where n < Slot.maxRipples && r.progress >= 0 && r.progress < 1 {
                let p = layout.outputPoint(forSource: r.position)
                setAt(.ripple0, n, p.x, p.y, layout.rippleRadius(progress: r.progress), layout.rippleOpacity(progress: r.progress))
                n += 1
            }
            set(.rippleParams, Double(n), layout.rippleThickness)
        }

        // Camera bubble: shared geometry, shrunk toward its corner/edge while zoomed. Border and
        // shadow scale with the bubble.
        let bubble = style.camera
        if let camera, bubble.visible, bubble.size > 0 {
            let camSize = PixelSize(width: camera.width, height: camera.height)
            let r = layout.cameraBubbleRect(bubble, cameraSize: camSize)
            let k = layout.cameraBubbleSizeMultiplier(bubble)
            if r.width >= 0.5, r.height >= 0.5 {
                let border = bubble.border.map { max(0, $0.width) * H * k } ?? 0
                set(.bubble, r.midX, r.midY, r.width / 2, r.height / 2)
                set(.bubble2, 0, layout.cameraBubbleCornerRadius(bubble, rect: r), border, 1)
                // Aspect-fill: crop the camera to the bubble aspect.
                let ba = r.width / max(r.height, 1e-6), ca = camSize.aspect
                let sx = ca > ba ? ba / ca : 1, sy = ca > ba ? 1 : ca / ba
                let texelsPerPx = Double(camera.width) * sx / max(r.width, 1)
                set(.bubbleUV, sx, sy, bubble.mirror ? 1 : 0, Double(Self.taps(texelsPerPixel: texelsPerPx)))
                set(.bubbleSS, sx / max(r.width, 1), sy / max(r.height, 1))
                if let bc = bubble.border?.color {
                    let lin = ColorMath.linear(bc)
                    set(.borderColor, lin.x, lin.y, lin.z, max(0, min(1, bc.a)))
                }
                let bs = style.shadow
                set(.bubbleShadow, max(0, bs.radius) * H / 2 * k, max(0, min(1, bs.opacity)), bs.offsetX * H * k, bs.offsetY * H * k)
            }
        }
    }

    private func writeBackground(
        _ shading: BackgroundShading, image: MTLTexture?, W: Double, H: Double,
        set: (Slot, Double, Double, Double, Double) -> Void,
        setAt: (Slot, Int, Double, Double, Double, Double) -> Void
    ) {
        func solid(_ c: SIMD4<Double>) {
            set(.bg, 0, 0, 0, 0)
            set(.solid, c.x, c.y, c.z, c.w)
        }
        switch shading {
        case .solid(let c):
            solid(c)
        case .linear(let stops, let locations, let dir, let invLength):
            set(.bg, 1, Double(stops.count), 0, 0)
            set(.bgGrad, dir.x, dir.y, invLength, 0)
            for (i, s) in stops.enumerated() {
                setAt(.stop0, i, s.x, s.y, s.z, s.w)
                setAt(.loc0, i, locations[i], 0, 0, 0)
            }
        case .mesh(let colors, let positions, let warp):
            set(.bg, 3, Double(colors.count), 0, 0)
            set(.wallpaper, warp, 0, 0, 0)
            for i in colors.indices {
                setAt(.stop0, i, colors[i].x, colors[i].y, colors[i].z, 1)
                setAt(.loc0, i, positions[i].x, positions[i].y, 0, 0)
            }
        case .image:
            guard let image else {
                let g = ColorMath.linear(BackgroundShading.missingImageGrey)
                return solid(SIMD4(g.x, g.y, g.z, 1))
            }
            let oa = W / H, ia = Double(image.width) / Double(max(image.height, 1))
            let sx = ia > oa ? oa / ia : 1, sy = ia > oa ? 1 : ia / oa
            set(.bg, 2, 0, 0, 0)
            set(.bgImage, sx, sy, 0, 0)
        }
    }

    /// Atlas mip level for a cursor drawn at `ppp` output pixels per point (the atlas holds
    /// `CursorAtlas.scale` pixels per point at level 0), clamped to the levels the padding protects.
    static func cursorLOD(pixelsPerPoint ppp: Double) -> Double {
        guard ppp.isFinite, ppp > 0 else { return 0 }
        return min(max(log2(CursorAtlas.scale / ppp), 0), CursorAtlas.maxLOD)
    }

    /// Taps per axis for the footprint filter: enough to cover the source texels under one output pixel.
    static func taps(texelsPerPixel r: Double) -> Int {
        guard r.isFinite, r > 1.05 else { return 1 }
        return min(6, Int(r.rounded(.up)))
    }

    // MARK: - Textures

    private func texture(for buffer: CVPixelBuffer, format: MTLPixelFormat, attributes: CFDictionary?) throws -> (MTLTexture, CVMetalTexture) {
        guard CVPixelBufferGetPixelFormatType(buffer) == kCVPixelFormatType_32BGRA else {
            throw NSSError.unsupported("MetalRenderer needs 32BGRA pixel buffers")
        }
        guard CVPixelBufferGetIOSurface(buffer) != nil else {
            throw NSSError.unsupported("MetalRenderer needs IOSurface-backed pixel buffers")
        }
        let w = CVPixelBufferGetWidth(buffer), h = CVPixelBufferGetHeight(buffer)
        var cvTex: CVMetalTexture?
        let st = CVMetalTextureCacheCreateTextureFromImage(nil, textureCache, buffer, attributes, format, w, h, 0, &cvTex)
        guard st == kCVReturnSuccess, let cvTex, let tex = CVMetalTextureGetTexture(cvTex) else {
            throw NSSError.renderFailed("CVMetalTextureCacheCreateTextureFromImage failed: \(st)")
        }
        return (tex, cvTex)
    }

    /// Drops the cached background image so the next frame reloads it from disk. The cache already
    /// reloads when the file's path, modification time or size changes, so this is only needed if a
    /// file is rewritten in place with identical size within the filesystem's timestamp resolution.
    public func invalidateBackgroundImage() {
        lock.withLock {
            imageCache = nil
            blurCache = nil
        }
    }

    /// Identity of the file on disk: (mtime seconds, mtime nanoseconds, size); all -1 when missing.
    /// A plain `stat` per frame: no allocation, so it is safe on the hot path.
    private static func fileStamp(_ path: String) -> ImageStamp {
        var st = stat()
        guard stat(path, &st) == 0 else { return ImageStamp(sec: -1, nsec: -1, size: -1) }
        return ImageStamp(sec: Int64(st.st_mtimespec.tv_sec), nsec: Int64(st.st_mtimespec.tv_nsec), size: Int64(st.st_size))
    }

    /// Called with `lock` held. The texture to draw for an image background: the decoded image, or its
    /// blurred copy when `style.backgroundBlur` > 0. Both are cached; a slider drag re-blurs only when the
    /// amount (rounded to 1/100) changes, and the decode itself is reused.
    private func backgroundImage(for style: Style) -> MTLTexture? {
        let file: String
        switch style.background {
        case .image(let f): file = f
        case .systemWallpaper(let path): file = path
        default: return nil
        }
        guard let (path, stamp, base) = decodedImage(file) else { return nil }
        let amount = BackgroundBlur.quantized(style.backgroundBlur)
        guard amount > 0, let base else { return base }
        if let b = blurCache, b.path == path, b.stamp == stamp, b.amount == amount { return b.texture ?? base }
        let blurred = try? ImageBlur.blur(base, amount: amount, device: device, queue: queue)
        blurCache = (path, stamp, amount, blurred)
        return blurred ?? base
    }

    /// Decodes once per (path, mtime, size) and caches the result, including a failed load; a file that
    /// appears, changes or disappears later gets a new stamp and is reloaded.
    private func decodedImage(_ file: String) -> (String, ImageStamp, MTLTexture?)? {
        guard !file.isEmpty else { return nil }
        let url: URL
        if file.hasPrefix("/") {
            url = URL(fileURLWithPath: file)
        } else if let baseURL {
            url = baseURL.appendingPathComponent(file)
        } else {
            url = URL(fileURLWithPath: file)
        }
        let path = url.path
        let stamp = Self.fileStamp(path)
        if let cached = imageCache, cached.path == path, cached.stamp == stamp { return (path, stamp, cached.texture) }
        let tex = stamp.size < 0 ? nil : loadImageTexture(url)
        imageCache = (path, stamp, tex)
        return (path, stamp, tex)
    }

    private func loadImageTexture(_ url: URL) -> MTLTexture? {
        guard let src = CGImageSourceCreateWithURL(url as CFURL, nil) else { return nil }
        let opts: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: 4096,
        ]
        // Dynamic (light/dark, time-of-day) HEIC wallpapers hold several images: draw the primary one.
        let index = CGImageSourceGetPrimaryImageIndex(src)
        guard let img = CGImageSourceCreateThumbnailAtIndex(src, index, opts as CFDictionary) else { return nil }
        let w = img.width, h = img.height
        guard w > 0, h > 0, let space = CGColorSpace(name: CGColorSpace.sRGB) else { return nil }
        let info = CGImageAlphaInfo.premultipliedFirst.rawValue | CGBitmapInfo.byteOrder32Little.rawValue
        guard let ctx = CGContext(data: nil, width: w, height: h, bitsPerComponent: 8, bytesPerRow: w * 4, space: space, bitmapInfo: info),
            let data = ctx.data
        else { return nil }
        ctx.interpolationQuality = .high
        ctx.draw(img, in: CGRect(x: 0, y: 0, width: w, height: h))

        let desc = MTLTextureDescriptor.texture2DDescriptor(pixelFormat: .bgra8Unorm_srgb, width: w, height: h, mipmapped: true)
        desc.usage = .shaderRead
        desc.storageMode = .shared
        guard let tex = device.makeTexture(descriptor: desc) else { return nil }
        tex.replace(region: MTLRegionMake2D(0, 0, w, h), mipmapLevel: 0, withBytes: data, bytesPerRow: w * 4)
        if tex.mipmapLevelCount > 1, let cmd = queue.makeCommandBuffer(), let blit = cmd.makeBlitCommandEncoder() {
            blit.generateMipmaps(for: tex)
            blit.endEncoding()
            cmd.commit()
            cmd.waitUntilCompleted()
        }
        return tex
    }
}

struct ImageStamp: Equatable {
    var sec: Int64
    var nsec: Int64
    var size: Int64
}

/// Compiles the shader source once per device (it takes ~100 ms) and caches the pipeline state.
final class PipelineCache: @unchecked Sendable {
    static let shared = PipelineCache()
    private let lock = NSLock()
    private var pipelines: [UInt64: MTLRenderPipelineState] = [:]
    private var blurPipelines: [UInt64: MTLComputePipelineState] = [:]

    /// The background blur kernel (`ImageBlur.source`), compiled once per device.
    func blurPipeline(for device: MTLDevice) throws -> MTLComputePipelineState {
        try lock.withLock {
            if let p = blurPipelines[device.registryID] { return p }
            let library: MTLLibrary
            do {
                library = try device.makeLibrary(source: ImageBlur.source, options: nil)
            } catch {
                throw NSSError.renderFailed("blur shader compile failed: \(error)")
            }
            guard let fn = library.makeFunction(name: "nss_blur") else { throw NSSError.renderFailed("blur entry point missing") }
            let p = try device.makeComputePipelineState(function: fn)
            blurPipelines[device.registryID] = p
            return p
        }
    }

    func pipeline(for device: MTLDevice) throws -> MTLRenderPipelineState {
        try lock.withLock {
            if let p = pipelines[device.registryID] { return p }
            let library: MTLLibrary
            do {
                library = try device.makeLibrary(source: ShaderSource.source, options: nil)
            } catch {
                throw NSSError.renderFailed("shader compile failed: \(error)")
            }
            guard let vfn = library.makeFunction(name: "nss_vertex"), let ffn = library.makeFunction(name: "nss_fragment") else {
                throw NSSError.renderFailed("shader entry points missing")
            }
            let desc = MTLRenderPipelineDescriptor()
            desc.vertexFunction = vfn
            desc.fragmentFunction = ffn
            desc.colorAttachments[0].pixelFormat = .bgra8Unorm
            let p = try device.makeRenderPipelineState(descriptor: desc)
            pipelines[device.registryID] = p
            return p
        }
    }
}
