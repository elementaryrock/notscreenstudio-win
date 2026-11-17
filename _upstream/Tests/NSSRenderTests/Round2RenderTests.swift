import CoreGraphics
import CoreVideo
import Foundation
import Testing
import NSSCore
@testable import NSSRender

/// Round 2 pixel tests on the real GPU: cursor shapes and squish, bubble shapes and shrink, edges and
/// preview/export determinism. Skipped cleanly when Metal is unavailable.
@Suite(.enabled(if: GPU.available, "needs a Metal device"))
struct Round2RenderTests {
    let W = 1920, H = 1080

    // MARK: Cursor

    private func cursorFrame(_ cursor: CursorState, motionBlur: Double = 0.4, screen: RGBAColor = .white) throws -> (CVPixelBuffer, RenderLayout, Style) {
        let out = try PixelBuffers.make(width: W, height: H)
        var style = TB.plainStyle()
        style.cursor.size = 4
        style.cursor.motionBlur = motionBlur
        try MetalRenderer().render(screen: try TB.filled(1920, 1080, screen), camera: nil, state: FrameState(time: 0, cursor: cursor), style: style, into: out)
        let l = RenderLayout(outputSize: PixelSize(width: W, height: H), sourceSize: PixelSize(width: 1920, height: 1080), style: style, camera: .identity)
        return (out, l, style)
    }

    /// Bounding box of pixels darker than `luma` within `radius` px of `c`.
    private func darkBox(_ pb: CVPixelBuffer, around c: CGPoint, radius: Int = 200, luma: Int = 60) -> CGRect? {
        var minX = Int.max, minY = Int.max, maxX = Int.min, maxY = Int.min
        for y in max(0, Int(c.y) - radius)..<min(H, Int(c.y) + radius) {
            for x in max(0, Int(c.x) - radius)..<min(W, Int(c.x) + radius) where TB.pixel(pb, x, y).luma < luma {
                minX = min(minX, x); minY = min(minY, y); maxX = max(maxX, x); maxY = max(maxY, y)
            }
        }
        return minX == .max ? nil : CGRect(x: minX, y: minY, width: maxX - minX + 1, height: maxY - minY + 1)
    }

    @Test func arrowTipSitsOnTheCursorPosition() throws {
        let (out, l, style) = try cursorFrame(CursorState(position: NormalizedPoint(x: 0.4, y: 0.4)))
        let tip = l.outputPoint(forSource: NormalizedPoint(x: 0.4, y: 0.4))
        let box = try #require(darkBox(out, around: tip))
        let ppp = l.cursorPixelsPerPoint(style: style.cursor)
        // The black arrow starts at the hotspot (within its white outline, about 1.5 pt).
        #expect(abs(box.minX - tip.x) < 2.5 * ppp && abs(box.minY - tip.y) < 2.5 * ppp, "box \(box) tip \(tip)")
        #expect(box.minX >= tip.x - 1 && box.minY >= tip.y - 1)
        // Body is black, the outline next to it is white.
        #expect(TB.pixel(out, Int(tip.x + 3 * ppp), Int(tip.y + 8 * ppp)).luma < 20)
    }

    @Test func everyShapeDrawsItsOwnImageAndUnknownIsTheArrow() throws {
        var images: [CursorShape: [UInt8]] = [:]
        let pos = NormalizedPoint(x: 0.5, y: 0.5)
        let grey = RGBAColor(r: 0.5, g: 0.5, b: 0.5)
        let (blank, l, _) = try cursorFrame(CursorState(visible: false), screen: grey)
        let blankBytes = TB.bytes(blank)
        let hot = l.outputPoint(forSource: pos)
        for shape in CursorShape.allCases {
            let (out, _, _) = try cursorFrame(CursorState(position: pos, shape: shape), screen: grey)
            let bytes = TB.bytes(out)
            #expect(bytes != blankBytes, "\(shape) drew nothing")
            images[shape] = bytes
            // Drawn near the hotspot: some pixel within 60 px changed.
            var changed = false
            for y in stride(from: Int(hot.y) - 60, to: Int(hot.y) + 60, by: 2) where !changed {
                for x in stride(from: Int(hot.x) - 60, to: Int(hot.x) + 60, by: 2) where TB.pixel(out, x, y) != TB.pixel(blank, x, y) { changed = true; break }
            }
            #expect(changed, "\(shape) not drawn at its hotspot")
        }
        #expect(images[.unknown] == images[.arrow])
        let atlas = CursorAtlas.build()
        for shape in CursorShape.allCases where shape != .arrow && shape != .unknown && atlas.cells[shape] != nil {
            #expect(images[shape] != images[.arrow], "\(shape) looks like the arrow")
        }
    }

    @Test(arguments: [CursorShape.arrow, .iBeam, .pointingHand, .resizeLeftRight, .closedHand])
    func motionBlurWorksForEveryShape(_ shape: CursorShape) throws {
        let (sharp, l, style) = try cursorFrame(CursorState(shape: shape), motionBlur: 1)
        let (blurred, _, _) = try cursorFrame(CursorState(velocity: NormalizedPoint(x: 3, y: 0), shape: shape), motionBlur: 1)
        let (still, _, _) = try cursorFrame(CursorState(velocity: NormalizedPoint(x: 3, y: 0), shape: shape), motionBlur: 0)
        #expect(TB.bytes(still) == TB.bytes(sharp))
        let s = try #require(darkBox(sharp, around: l.outputPoint(forSource: .center), luma: 200))
        let b = try #require(darkBox(blurred, around: l.outputPoint(forSource: .center), radius: 400, luma: 250))
        let blurLen = l.cursorBlurVector(velocity: NormalizedPoint(x: 3, y: 0), style: style.cursor).dx
        #expect(b.width > s.width + blurLen * 0.5, "\(shape): blurred \(b.width) sharp \(s.width) blur \(blurLen)")
        #expect(abs(b.height - s.height) < 12)
    }

    @Test(arguments: [CursorShape.arrow, .pointingHand])
    func pressSquishScalesAroundTheHotspot(_ shape: CursorShape) throws {
        let pos = NormalizedPoint(x: 0.5, y: 0.5)
        let (normal, l, style) = try cursorFrame(CursorState(position: pos, shape: shape))
        let (pressed, _, _) = try cursorFrame(CursorState(position: pos, shape: shape, pressScale: 0.5))
        let hot = l.outputPoint(forSource: pos)
        let a = try #require(darkBox(normal, around: hot, luma: 200))
        let b = try #require(darkBox(pressed, around: hot, luma: 200))
        let ppp = l.cursorPixelsPerPoint(style: style.cursor)
        #expect(abs(b.width / a.width - 0.5) < 0.12 && abs(b.height / a.height - 0.5) < 0.12, "\(a) -> \(b)")
        // Every edge moves halfway toward the hotspot.
        for (e0, e1, h) in [(a.minX, b.minX, hot.x), (a.maxX, b.maxX, hot.x), (a.minY, b.minY, hot.y), (a.maxY, b.maxY, hot.y)] {
            #expect(abs((e1 - h) - 0.5 * (e0 - h)) < 1.5 * ppp, "edge \(e0) -> \(e1), hotspot \(h)")
        }
        let (same, _, _) = try cursorFrame(CursorState(position: pos, shape: shape, pressScale: 1))
        #expect(TB.bytes(same) == TB.bytes(normal))
    }

    // MARK: Camera bubble

    struct BubbleCase: Sendable, CustomTestStringConvertible {
        var shape: CameraShape
        var roundness: Double
        /// Expected camera coverage at 0.8 and 0.97 of the half-size along the diagonal.
        var inside: (Bool, Bool)
        var testDescription: String { "\(shape) r=\(roundness)" }
    }

    @Test(arguments: [
        BubbleCase(shape: .circle, roundness: 0, inside: (false, false)),
        BubbleCase(shape: .square, roundness: 0.5, inside: (true, false)),
        BubbleCase(shape: .square, roundness: 0, inside: (true, true)),
        BubbleCase(shape: .square, roundness: 1, inside: (false, false)),
        BubbleCase(shape: .tall, roundness: 0.3, inside: (true, false)),
        BubbleCase(shape: .wide, roundness: 0, inside: (true, true)),
        BubbleCase(shape: .wide, roundness: 1, inside: (false, false)),
    ])
    func bubbleShapesAreMaskedByTheSharedGeometry(_ c: BubbleCase) throws {
        let screen = try TB.filled(1920, 1080, TB.green)
        let cam = try TB.filled(640, 480, TB.blue)
        let out = try PixelBuffers.make(width: W, height: H)
        var style = TB.plainStyle(padding: 0)
        style.camera = CameraBubble(visible: true, shape: c.shape, position: .center, size: 0.25, border: nil, mirror: false, roundness: c.roundness)
        try MetalRenderer().render(screen: screen, camera: cam, state: TB.hiddenCursor(), style: style, into: out)
        let l = RenderLayout(outputSize: PixelSize(width: W, height: H), sourceSize: PixelSize(width: 1920, height: 1080), style: style, camera: .identity)
        let r = l.cameraBubbleRect(style.camera, cameraSize: PixelSize(width: 640, height: 480))
        #expect(abs(r.width / r.height - CameraBubbleGeometry.aspect(for: c.shape)) < 1e-9)
        func at(_ fx: Double, _ fy: Double) -> RGB { TB.pixel(out, Int(r.midX + fx * r.width / 2), Int(r.midY + fy * r.height / 2)) }
        let blue = RGB(r: 0, g: 0, b: 255), green = RGB(r: 0, g: 255, b: 0)
        #expect(at(0, 0).near(blue, 1))
        // Straight edges reach the bounding box on both axes.
        #expect(at(0.97, 0).near(blue, 1) && at(0, 0.97).near(blue, 1) && at(0, -0.97).near(blue, 1))
        #expect(TB.pixel(out, Int(r.maxX) + 3, Int(r.midY)).near(green, 1))
        #expect(TB.pixel(out, Int(r.midX), Int(r.minY) - 3).near(green, 1))
        #expect(at(0.8, 0.8).near(c.inside.0 ? blue : green, 1), "0.8 = \(at(0.8, 0.8))")
        #expect(at(0.97, -0.97).near(c.inside.1 ? blue : green, 1), "0.97 = \(at(0.97, -0.97))")
        // Anti-aliased: a pixel on the rounded corner arc is a blend (skip sharp corners).
        let rad = l.cameraBubbleCornerRadius(style.camera, rect: r)
        if rad > 4 {
            let cx = r.maxX - rad, cy = r.minY + rad
            var best = (x: 0, y: 0, d: Double.infinity)
            for y in Int(cy - rad * 0.7071) - 3...Int(cy - rad * 0.7071) + 3 {
                for x in Int(cx + rad * 0.7071) - 3...Int(cx + rad * 0.7071) + 3 {
                    let d = hypot(Double(x) + 0.5 - cx, Double(y) + 0.5 - cy) - rad
                    if abs(d) < abs(best.d) { best = (x, y, d) }
                }
            }
            let e = TB.pixel(out, best.x, best.y)
            #expect(e.b > 60 && e.b < 245 && e.g > 60 && e.g < 245, "edge \(e)")
        }
    }

    @Test(arguments: CameraShape.allCases)
    func bubbleShrinksWhenZoomedWithBorderAndShadow(_ shape: CameraShape) throws {
        let screen = try TB.filled(1920, 1080, TB.green)
        let cam = try TB.filled(640, 480, TB.blue)
        var style = TB.plainStyle(padding: 0)
        style.camera = CameraBubble(visible: true, shape: shape, position: NormalizedPoint(x: 0.85, y: 0.75), size: 0.2,
                                    border: CameraBorder(width: 0.01, color: .white), mirror: false, roundness: 0.3, zoomedScale: 0.5)
        style.shadow = Shadow(radius: 0.03, opacity: 0.6, offsetX: 0, offsetY: 0.02)
        let zoom = CameraState(scale: 2, center: .center)
        let out = try PixelBuffers.make(width: W, height: H)
        try MetalRenderer().render(screen: screen, camera: cam, state: FrameState(time: 0, camera: zoom, cursor: CursorState(visible: false)), style: style, into: out)
        let l = RenderLayout(outputSize: PixelSize(width: W, height: H), sourceSize: PixelSize(width: 1920, height: 1080), style: style, camera: zoom)
        let full = l.cameraBubbleFullRect(style.camera), r = l.cameraBubbleRect(style.camera, cameraSize: nil)
        #expect(abs(r.width - full.width * 0.5) < 1e-6)
        let blue = RGB(r: 0, g: 0, b: 255), white = RGB(r: 255, g: 255, b: 255)
        // Inside the shrunk bubble: camera. The anchored corner stays put.
        #expect(TB.pixel(out, Int(r.midX), Int(r.midY)).near(blue, 1))
        // Where the full-size bubble's top-left quarter was: screen (shadow-darkened at most).
        let old = TB.pixel(out, Int(full.minX + full.width * 0.25), Int(full.minY + full.height * 0.25))
        #expect(old.g > 150 && old.b < 10, "old area \(old)")
        // Border is scaled: full border would be 10.8 px, the shrunk one is 5.4 px.
        let bw = 0.01 * Double(H) * 0.5
        #expect(TB.pixel(out, Int(r.midX), Int(r.minY + bw * 0.5)).near(white, 3))
        #expect(TB.pixel(out, Int(r.midX), Int(r.minY + bw * 1.6)).near(blue, 3))
        // Shadow follows the shape below the shrunk bubble, and has gone from below the old top edge area.
        let below = TB.pixel(out, Int(r.midX), Int(r.maxY + 0.02 * Double(H) * 0.5))
        #expect(below.g < 235, "shadow below \(below)")
        let farLeft = TB.pixel(out, Int(full.minX - 0.06 * Double(H)), Int(full.midY))
        #expect(farLeft.near(RGB(r: 0, g: 255, b: 0), 2))
    }

    // MARK: Edges

    enum Corner: String, CaseIterable, Sendable { case topLeft, topRight, bottomLeft, bottomRight }

    struct EdgeCase: Sendable, CustomTestStringConvertible {
        var aspect: AspectRatio
        var corner: Corner
        var testDescription: String { "\(aspect.rawValue) \(corner.rawValue)" }
    }

    static let edgeCases: [EdgeCase] = AspectRatio.allCases.flatMap { a in Corner.allCases.map { EdgeCase(aspect: a, corner: $0) } }

    /// Zooms into each corner (and past it) and checks every pixel along the four content edges against
    /// the source position it should show: a gap would show the magenta background, a smear would
    /// repeat the edge value instead of following the source ramp.
    @Test(arguments: edgeCases)
    func zoomedCornersShowNoGapsOrSmearedEdges(_ c: EdgeCase) throws {
        let sw = 1920, sh = 1080
        let src = try TB.make(sw, sh) { x, y in RGBAColor(r: Double(x) / Double(sw - 1), g: Double(y) / Double(sh - 1), b: 0.5) }
        let size = c.aspect.outputSize(for: .p1080)
        let out = try PixelBuffers.make(width: size.width, height: size.height)
        var style = TB.plainStyle(padding: 0.05)
        style.background = .solid(RGBAColor(r: 1, g: 0, b: 1))
        style.aspectRatio = c.aspect
        let renderer = try MetalRenderer()
        for scale in [1.0, 1.5, 2.5, 6] {
            for overshoot in [0.0, 0.3] {
                let half = 0.5 / scale
                let lo = half - overshoot, hi = 1 - half + overshoot
                let center: NormalizedPoint = switch c.corner {
                case .topLeft: NormalizedPoint(x: lo, y: lo)
                case .topRight: NormalizedPoint(x: hi, y: lo)
                case .bottomLeft: NormalizedPoint(x: lo, y: hi)
                case .bottomRight: NormalizedPoint(x: hi, y: hi)
                }
                let cam = CameraState(scale: scale, center: center)
                try renderer.render(screen: src, camera: nil, state: FrameState(time: 0, camera: cam, cursor: CursorState(visible: false)), style: style, into: out)
                let l = RenderLayout(outputSize: size, sourceSize: PixelSize(width: sw, height: sh), style: style, camera: cam)
                let rect = l.contentRect
                // Fully covered edge pixels (the partially covered ones blend with the background by design).
                let x0 = Int(rect.minX.rounded(.up)), x1 = Int(rect.maxX.rounded(.down)) - 1
                let y0 = Int(rect.minY.rounded(.up)), y1 = Int(rect.maxY.rounded(.down)) - 1
                var pts: [(Int, Int)] = []
                for x in stride(from: x0, through: x1, by: 3) { pts.append((x, y0)); pts.append((x, y1)) }
                for y in stride(from: y0, through: y1, by: 3) { pts.append((x0, y)); pts.append((x1, y)) }
                var worst = 0, worstAt = (0, 0)
                for (x, y) in pts {
                    let s = l.sourcePoint(forOutput: CGPoint(x: Double(x) + 0.5, y: Double(y) + 0.5))
                    #expect(s.x >= 0 && s.x <= 1 && s.y >= 0 && s.y <= 1)
                    // Texel centres sit at (i + 0.5) / n; value i / (n - 1).
                    let er = min(max((s.x * Double(sw) - 0.5) / Double(sw - 1), 0), 1) * 255
                    let eg = min(max((s.y * Double(sh) - 0.5) / Double(sh - 1), 0), 1) * 255
                    let p = TB.pixel(out, x, y)
                    let err = Int(max(abs(Double(p.r) - er), abs(Double(p.g) - eg)).rounded(.up))
                    if err > worst { worst = err; worstAt = (x, y) }
                    #expect(abs(p.b - 128) <= 2, "background showing at \(x),\(y): \(p)")
                }
                #expect(worst <= 3, "scale \(scale) overshoot \(overshoot): worst error \(worst) at \(worstAt)")
            }
        }
    }

    // MARK: Determinism

    private func pool(width: Int, height: Int, alignment: Int) throws -> CVPixelBufferPool {
        let attrs: [CFString: Any] = [
            kCVPixelBufferPixelFormatTypeKey: kCVPixelFormatType_32BGRA,
            kCVPixelBufferWidthKey: width, kCVPixelBufferHeightKey: height,
            kCVPixelBufferIOSurfacePropertiesKey: [:] as [String: Any],
            kCVPixelBufferMetalCompatibilityKey: true,
            kCVPixelBufferBytesPerRowAlignmentKey: alignment,
        ]
        var p: CVPixelBufferPool?
        let st = CVPixelBufferPoolCreate(nil, [kCVPixelBufferPoolMinimumBufferCountKey: 3] as CFDictionary, attrs as CFDictionary, &p)
        return try #require(st == kCVReturnSuccess ? p : nil)
    }

    private func rows(_ pb: CVPixelBuffer) -> [UInt8] {
        CVPixelBufferLockBaseAddress(pb, .readOnly)
        defer { CVPixelBufferUnlockBaseAddress(pb, .readOnly) }
        let w = CVPixelBufferGetWidth(pb) * 4, h = CVPixelBufferGetHeight(pb), stride = CVPixelBufferGetBytesPerRow(pb)
        let base = CVPixelBufferGetBaseAddress(pb)!.assumingMemoryBound(to: UInt8.self)
        var out: [UInt8] = []
        out.reserveCapacity(w * h)
        for y in 0..<h { out.append(contentsOf: UnsafeBufferPointer(start: base + y * stride, count: w)) }
        return out
    }

    @Test func previewAndExportPoolsGiveByteIdenticalFrames() throws {
        let (w, h) = (1080, 1920)  // 9:16, so the row strides of the two pools differ
        let screen = try TB.make(1440, 900) { x, y in RGBAColor(r: Double(x % 200) / 199, g: Double(y % 150) / 149, b: 0.3) }
        let cam = try TB.make(640, 480) { x, y in RGBAColor(r: 0.2, g: Double(x) / 639, b: Double(y) / 479) }
        var style = Style()
        style.aspectRatio = .r9x16
        style.camera.shape = .tall
        style.camera.roundness = 0.45
        style.cursor.motionBlur = 0.8
        let states = [
            FrameState(time: 1.5, camera: CameraState(scale: 1.8, center: NormalizedPoint(x: 0.35, y: 0.6)),
                       cursor: CursorState(position: NormalizedPoint(x: 0.4, y: 0.62), velocity: NormalizedPoint(x: 1.2, y: -0.4),
                                           ripples: [RippleState(position: NormalizedPoint(x: 0.41, y: 0.6), progress: 0.35)],
                                           opacity: 0.9, shape: .pointingHand, pressScale: 0.82)),
            FrameState(time: 2, camera: .identity, cursor: CursorState(position: NormalizedPoint(x: 0.7, y: 0.2), shape: .iBeam)),
        ]
        let previewPool = try pool(width: w, height: h, alignment: 16)
        let exportPool = try pool(width: w, height: h, alignment: 256)
        let preview = try MetalRenderer(), export = try MetalRenderer()
        for state in states {
            var a: CVPixelBuffer?, b: CVPixelBuffer?
            CVPixelBufferPoolCreatePixelBuffer(nil, previewPool, &a)
            CVPixelBufferPoolCreatePixelBuffer(nil, exportPool, &b)
            let pa = try #require(a), pb = try #require(b)
            try preview.render(screen: screen, camera: cam, state: state, style: style, into: pa)
            try export.render(screen: screen, camera: cam, state: state, style: style, into: pb)
            #expect(rows(pa) == rows(pb), "frame at \(state.time) differs between preview and export")
            // And rendering again on the same renderer is stable.
            try preview.render(screen: screen, camera: cam, state: state, style: style, into: pa)
            #expect(rows(pa) == rows(pb))
        }
    }
}
