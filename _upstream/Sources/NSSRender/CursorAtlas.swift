import Accelerate
import AppKit
import CoreGraphics
import Foundation
import Metal
import NSSCore

/// A sprite sheet of the real macOS system cursors, rasterized once at `scale` pixels per point.
///
/// Pixels are linear-light sRGB, premultiplied, so the GPU can filter and mipmap them correctly and
/// the shader can composite them straight into its linear-light frame. Each cell keeps the cursor's
/// size in points and its hotspot (points from the image's top-left corner), so the renderer can put
/// the hotspot exactly on `CursorState.position`.
struct CursorAtlas: Sendable {
    struct Cell: Sendable, Equatable {
        /// Image area in atlas pixels at mip level 0 (origin top-left). Surrounded by `padding` clear pixels.
        var x: Int
        var y: Int
        var width: Int
        var height: Int
        /// Cursor image size in points.
        var size: CGSize
        /// Hotspot in points from the image's top-left corner.
        var hotspot: CGPoint
        /// True when the pixels came from `NSCursor` (false for the drawn fallback arrow).
        var isSystemImage: Bool
    }

    /// Atlas pixels per cursor point. AppKit ships the arrow and I-beam up to 10x and the others at 2x,
    /// so 8x keeps the arrow crisp at 4K zoom and gives the 2x cursors a high-quality CPU upscale.
    static let scale = 8.0
    /// Clear border around every cell, in atlas pixels. Keeps bilinear and mipmap taps from bleeding
    /// between neighbours up to `maxLOD` (32 px is 2 px at level 4).
    static let padding = 32
    /// Coarsest mip level the shader samples.
    static let maxLOD = 4.0
    /// Atlas rows wrap at this width.
    static let maxWidth = 2048

    let width: Int
    let height: Int
    /// RGBA, linear premultiplied, top row first, `width * height * 4` values.
    let pixels: [Float]
    let cells: [CursorShape: Cell]

    /// Cell for `shape`. Shapes without an image (including `.unknown`) use the arrow.
    func cell(for shape: CursorShape) -> Cell {
        cells[shape] ?? cells[.arrow]!
    }

    // MARK: - Building

    /// Rasterizes every `CursorShape` that has a system cursor. Pure CPU work (CoreGraphics), safe on any thread.
    static func build() -> CursorAtlas {
        var sources: [(CursorShape, CGImage, CGSize, CGPoint, Bool)] = []
        for shape in CursorShape.allCases where shape != .unknown {
            if let (image, size, hotspot) = systemImage(for: shape) {
                sources.append((shape, image, size, hotspot, true))
            }
        }
        if !sources.contains(where: { $0.0 == .arrow }) {
            let (image, size, hotspot) = fallbackArrow()
            sources.insert((.arrow, image, size, hotspot, false), at: 0)
        }

        // Shelf packing.
        var cells: [CursorShape: Cell] = [:]
        var x = 0, y = 0, rowHeight = 0, atlasWidth = 0
        for (shape, _, size, hotspot, isSystem) in sources {
            let w = Int((size.width * scale).rounded(.up)), h = Int((size.height * scale).rounded(.up))
            let slotW = w + 2 * padding, slotH = h + 2 * padding
            if x > 0, x + slotW > maxWidth {
                y += rowHeight
                x = 0
                rowHeight = 0
            }
            cells[shape] = Cell(x: x + padding, y: y + padding, width: w, height: h, size: size, hotspot: hotspot, isSystemImage: isSystem)
            x += slotW
            rowHeight = max(rowHeight, slotH)
            atlasWidth = max(atlasWidth, x)
        }
        let atlasHeight = y + rowHeight

        var pixels = [Float](repeating: 0, count: atlasWidth * atlasHeight * 4)
        if let space = CGColorSpace(name: CGColorSpace.linearSRGB) {
            let info = CGImageAlphaInfo.premultipliedLast.rawValue | CGBitmapInfo.floatComponents.rawValue
                | CGBitmapInfo.byteOrder32Little.rawValue
            pixels.withUnsafeMutableBytes { raw in
                guard let ctx = CGContext(
                    data: raw.baseAddress, width: atlasWidth, height: atlasHeight, bitsPerComponent: 32,
                    bytesPerRow: atlasWidth * 16, space: space, bitmapInfo: info)
                else { return }
                ctx.interpolationQuality = .high
                for (shape, image, _, _, _) in sources {
                    guard let c = cells[shape] else { continue }
                    // CoreGraphics user space is bottom-left; bitmap row 0 is the top.
                    ctx.draw(image, in: CGRect(x: c.x, y: atlasHeight - c.y - c.height, width: c.width, height: c.height))
                }
            }
        }
        return CursorAtlas(width: atlasWidth, height: atlasHeight, pixels: pixels, cells: cells)
    }

    /// The highest-resolution bitmap of the system cursor for `shape`, its size and hotspot in points.
    static func systemImage(for shape: CursorShape) -> (CGImage, CGSize, CGPoint)? {
        // The arrow and I-beam images load lazily; touching `current` makes AppKit load them even
        // when no NSApplication is running (tests, command-line tools).
        _ = NSCursor.current
        guard let cursor = systemCursor(for: shape) else { return nil }
        let image = cursor.image
        let size = image.size
        guard size.width > 0, size.height > 0 else { return nil }
        let best = image.representations
            .compactMap { $0 as? NSBitmapImageRep }
            .max { $0.pixelsWide < $1.pixelsWide }
        var proposed = CGRect(origin: .zero, size: CGSize(width: size.width * scale, height: size.height * scale))
        guard let cg = best?.cgImage ?? image.cgImage(forProposedRect: &proposed, context: nil, hints: nil) else { return nil }
        return (cg, size, cursor.hotSpot)
    }

    static func systemCursor(for shape: CursorShape) -> NSCursor? {
        switch shape {
        case .arrow: .arrow
        case .iBeam: .iBeam
        case .iBeamVertical: .iBeamCursorForVerticalLayout
        case .pointingHand: .pointingHand
        case .openHand: .openHand
        case .closedHand: .closedHand
        case .crosshair: .crosshair
        case .resizeLeftRight: .resizeLeftRight
        case .resizeUpDown: .resizeUpDown
        case .resizeLeft: .resizeLeft
        case .resizeRight: .resizeRight
        case .resizeUp: .resizeUp
        case .resizeDown: .resizeDown
        case .notAllowed: .operationNotAllowed
        case .dragCopy: .dragCopy
        case .dragLink: .dragLink
        case .contextualMenu: .contextualMenu
        case .unknown: nil
        }
    }

    /// A drawn macOS-style arrow (black fill, white outline, soft shadow) for when AppKit has no arrow
    /// image. Same 17x23 pt frame and (4, 4) hotspot as the system arrow.
    static func fallbackArrow() -> (CGImage, CGSize, CGPoint) {
        let size = CGSize(width: 17, height: 23), hotspot = CGPoint(x: 4, y: 4)
        let w = Int(size.width * scale), h = Int(size.height * scale)
        let space = CGColorSpace(name: CGColorSpace.sRGB)!
        let ctx = CGContext(
            data: nil, width: w, height: h, bitsPerComponent: 8, bytesPerRow: w * 4, space: space,
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
        // Points, y down, origin at the image's top-left.
        ctx.translateBy(x: 0, y: CGFloat(h))
        ctx.scaleBy(x: scale, y: -scale)
        let poly: [CGPoint] = [
            CGPoint(x: 0, y: 0), CGPoint(x: 0, y: 16.6), CGPoint(x: 4.1, y: 12.7), CGPoint(x: 6.9, y: 19),
            CGPoint(x: 9.5, y: 17.9), CGPoint(x: 6.8, y: 11.8), CGPoint(x: 12, y: 11.8),
        ].map { CGPoint(x: $0.x + hotspot.x, y: $0.y + hotspot.y) }
        let path = CGMutablePath()
        path.addLines(between: poly)
        path.closeSubpath()
        ctx.setShadow(offset: CGSize(width: 0, height: -1.2 * scale), blur: 2 * scale, color: CGColor(gray: 0, alpha: 0.35))
        ctx.setLineJoin(.round)
        ctx.addPath(path)
        ctx.setStrokeColor(CGColor(gray: 1, alpha: 1))
        ctx.setLineWidth(2.6)
        ctx.strokePath()
        ctx.setShadow(offset: .zero, blur: 0, color: nil)
        ctx.addPath(path)
        ctx.setFillColor(CGColor(gray: 0, alpha: 1))
        ctx.fillPath()
        return (ctx.makeImage()!, size, hotspot)
    }

    // MARK: - GPU

    /// Uploads the atlas as an `rgba16Float` texture with a full mip chain.
    func makeTexture(device: MTLDevice, queue: MTLCommandQueue) throws -> MTLTexture {
        let desc = MTLTextureDescriptor.texture2DDescriptor(pixelFormat: .rgba16Float, width: width, height: height, mipmapped: true)
        desc.usage = .shaderRead
        desc.storageMode = .shared
        guard let tex = device.makeTexture(descriptor: desc) else { throw NSSError.renderFailed("cursor atlas texture failed") }
        var halves = [UInt16](repeating: 0, count: pixels.count)
        pixels.withUnsafeBufferPointer { src in
            halves.withUnsafeMutableBufferPointer { dst in
                var s = vImage_Buffer(data: UnsafeMutableRawPointer(mutating: src.baseAddress), height: 1, width: vImagePixelCount(src.count), rowBytes: src.count * 4)
                var d = vImage_Buffer(data: dst.baseAddress, height: 1, width: vImagePixelCount(dst.count), rowBytes: dst.count * 2)
                _ = vImageConvert_PlanarFtoPlanar16F(&s, &d, vImage_Flags(kvImageNoFlags))
            }
        }
        tex.replace(region: MTLRegionMake2D(0, 0, width, height), mipmapLevel: 0, withBytes: halves, bytesPerRow: width * 8)
        if tex.mipmapLevelCount > 1 {
            guard let cmd = queue.makeCommandBuffer(), let blit = cmd.makeBlitCommandEncoder() else {
                throw NSSError.renderFailed("cursor atlas mipmap pass failed")
            }
            blit.generateMipmaps(for: tex)
            blit.endEncoding()
            cmd.commit()
            cmd.waitUntilCompleted()
        }
        return tex
    }
}

/// Builds the cursor atlas once per process and uploads it once per device. Only the cell table is
/// kept on the CPU afterwards; the pixels are dropped after the first upload (and rebuilt in the rare
/// case a second GPU appears).
final class CursorAtlasCache: @unchecked Sendable {
    static let shared = CursorAtlasCache()
    private let lock = NSLock()
    private var cells: [CursorShape: CursorAtlas.Cell]?
    private var textures: [UInt64: MTLTexture] = [:]

    func texture(for device: MTLDevice, queue: MTLCommandQueue) throws -> (MTLTexture, [CursorShape: CursorAtlas.Cell]) {
        try lock.withLock {
            if let t = textures[device.registryID], let cells { return (t, cells) }
            let atlas = CursorAtlas.build()
            let t = try atlas.makeTexture(device: device, queue: queue)
            textures[device.registryID] = t
            // Keep the first layout so every device and renderer agrees on it.
            if cells == nil { cells = atlas.cells }
            return (t, cells!)
        }
    }
}
