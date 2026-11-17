import CoreGraphics
import CoreVideo
import Foundation
import ImageIO
import Testing
import UniformTypeIdentifiers
import NSSCore
@testable import NSSRender

/// Pixel tests on the real GPU. Skipped cleanly when Metal is unavailable.
@Suite(.enabled(if: GPU.available, "needs a Metal device"))
struct MetalRendererTests {
    let W = 1920, H = 1080

    @Test func cornersShowBackgroundAndCentreShowsScreen() throws {
        let r = try MetalRenderer()
        let screen = try TB.filled(1920, 1080, TB.green)
        let out = try PixelBuffers.make(width: W, height: H)
        try r.render(screen: screen, camera: nil, state: TB.hiddenCursor(), style: TB.plainStyle(), into: out)
        for (x, y) in [(0, 0), (W - 1, 0), (0, H - 1), (W - 1, H - 1)] {
            #expect(TB.pixel(out, x, y).near(RGB(r: 255, g: 0, b: 0)), "corner \(x),\(y) = \(TB.pixel(out, x, y))")
        }
        #expect(TB.pixel(out, W / 2, H / 2).near(RGB(r: 0, g: 255, b: 0), 1))
    }

    @Test func roundedCornerIsAntiAliased() throws {
        let r = try MetalRenderer()
        let screen = try TB.filled(1920, 1080, TB.green)
        let out = try PixelBuffers.make(width: W, height: H)
        let style = TB.plainStyle(padding: 0.1, corner: 0.05)
        try r.render(screen: screen, camera: nil, state: TB.hiddenCursor(), style: style, into: out)

        let l = RenderLayout(outputSize: PixelSize(width: W, height: H), sourceSize: PixelSize(width: 1920, height: 1080), style: style, camera: .identity)
        let rad = l.cornerRadius
        let cx = l.contentRect.minX + rad, cy = l.contentRect.minY + rad
        // Just inside the content's bounding box, but outside the rounded corner: background.
        #expect(TB.pixel(out, Int(l.contentRect.minX) + 1, Int(l.contentRect.minY) + 1).near(RGB(r: 255, g: 0, b: 0)))
        // On the straight top edge: screen.
        #expect(TB.pixel(out, Int(cx + 10), Int(l.contentRect.minY) + 1).near(RGB(r: 0, g: 255, b: 0), 1))
        // A pixel whose centre sits on the arc is a partial blend of both.
        // Pick the pixel in the corner square whose centre is closest to the arc's 45-degree point.
        var best = (x: 0, y: 0, d: Double.infinity)
        let diag = (cx - rad * 0.7071, cy - rad * 0.7071)
        for y in Int(diag.1) - 3...Int(diag.1) + 3 {
            for x in Int(diag.0) - 3...Int(diag.0) + 3 {
                let px = Double(x) + 0.5, py = Double(y) + 0.5
                let d = ((cx - px) * (cx - px) + (cy - py) * (cy - py)).squareRoot() - rad
                if abs(d) < abs(best.d) { best = (x, y, d) }
            }
        }
        #expect(abs(best.d) < 0.2)
        let edge = TB.pixel(out, best.x, best.y)
        #expect(edge.r > 100 && edge.r < 240 && edge.g > 100 && edge.g < 240, "edge pixel \(edge)")
    }

    @Test func dropShadowDarkensBackgroundBelowContent() throws {
        let r = try MetalRenderer()
        let screen = try TB.filled(1920, 1080, TB.green)
        let out = try PixelBuffers.make(width: W, height: H)
        var style = TB.plainStyle(padding: 0.1, corner: 0.02)
        style.background = .solid(.white)
        style.shadow = Shadow(radius: 0.04, opacity: 0.6, offsetX: 0, offsetY: 0.015)
        try r.render(screen: screen, camera: nil, state: TB.hiddenCursor(), style: style, into: out)
        let l = RenderLayout(outputSize: PixelSize(width: W, height: H), sourceSize: PixelSize(width: 1920, height: 1080), style: style, camera: .identity)
        let below = TB.pixel(out, W / 2, Int(l.contentRect.maxY) + 6)
        let far = TB.pixel(out, 2, 2)
        #expect(below.luma < far.luma - 40, "below \(below) far \(far)")
        #expect(far.near(RGB(r: 255, g: 255, b: 255), 2))
    }

    @Test(arguments: AspectRatio.allCases)
    func rendersEveryAspectRatio(aspect: AspectRatio) throws {
        let r = try MetalRenderer()
        let size = aspect.outputSize(for: .p1080)
        let out = try PixelBuffers.make(width: size.width, height: size.height)
        var style = TB.plainStyle()
        style.aspectRatio = aspect
        try r.render(screen: try TB.filled(1920, 1080, TB.green), camera: nil, state: TB.hiddenCursor(), style: style, into: out)
        #expect(CVPixelBufferGetWidth(out) == size.width && CVPixelBufferGetHeight(out) == size.height)
        #expect(TB.pixel(out, 1, 1).near(RGB(r: 255, g: 0, b: 0)))
        #expect(TB.pixel(out, size.width / 2, size.height / 2).near(RGB(r: 0, g: 255, b: 0), 1))
    }

    @Test func downscaleAveragesTheFootprintInLinearLight() throws {
        // Stripes of period 3 (white, black, black) shrunk 3x must average to 1/3 linear = sRGB ~156,
        // which a single bilinear tap cannot produce.
        let src = try TB.make(1920, 1080) { x, _ in x % 3 == 0 ? .white : .black }
        let out = try PixelBuffers.make(width: 640, height: 360)
        try MetalRenderer().render(screen: src, camera: nil, state: TB.hiddenCursor(), style: TB.plainStyle(padding: 0), into: out)
        for x in [100, 320, 500] {
            let p = TB.pixel(out, x, 180)
            #expect(abs(p.g - 156) <= 6, "pixel \(x) = \(p)")
        }
    }

    @Test func cameraZoomCropsTheSource() throws {
        // Left half blue, right half green; zooming 2x on the right-centre shows only green.
        let src = try TB.make(1920, 1080) { x, _ in x < 960 ? TB.blue : TB.green }
        let out = try PixelBuffers.make(width: W, height: H)
        let state = FrameState(time: 0, camera: CameraState(scale: 2, center: NormalizedPoint(x: 0.75, y: 0.5)), cursor: CursorState(visible: false))
        try MetalRenderer().render(screen: src, camera: nil, state: state, style: TB.plainStyle(padding: 0), into: out)
        #expect(TB.pixel(out, 20, H / 2).near(RGB(r: 0, g: 255, b: 0), 1))
        #expect(TB.pixel(out, W - 20, H / 2).near(RGB(r: 0, g: 255, b: 0), 1))
    }

    @Test func rippleDrawsARing() throws {
        let grey = RGBAColor(r: 0.2, g: 0.2, b: 0.2)
        let screen = try TB.filled(1920, 1080, grey)
        let out = try PixelBuffers.make(width: W, height: H)
        let style = TB.plainStyle()
        let progress = 0.3
        let state = FrameState(time: 0, cursor: CursorState(visible: false, ripples: [RippleState(position: .center, progress: progress)]))
        try MetalRenderer().render(screen: screen, camera: nil, state: state, style: style, into: out)
        let l = RenderLayout(outputSize: PixelSize(width: W, height: H), sourceSize: PixelSize(width: 1920, height: 1080), style: style, camera: .identity)
        let c = l.outputPoint(forSource: .center)
        let rad = l.rippleRadius(progress: progress)
        let ring = TB.pixel(out, Int(c.x + rad), Int(c.y))
        let inside = TB.pixel(out, Int(c.x + rad * 0.4), Int(c.y))
        let outside = TB.pixel(out, Int(c.x + l.rippleMaxRadius * 1.6), Int(c.y))
        #expect(ring.luma > inside.luma + 80, "ring \(ring) inside \(inside)")
        #expect(abs(outside.luma - 51) <= 2, "outside \(outside)")  // untouched grey
        // Ripples off: no ring.
        var off = style
        off.cursor.clickRipples = false
        try MetalRenderer().render(screen: screen, camera: nil, state: state, style: off, into: out)
        #expect(abs(TB.pixel(out, Int(c.x + rad), Int(c.y)).luma - 51) <= 2)
    }

    // MARK: Cursor

    private func cursorScene(_ cursor: CursorState, motionBlur: Double = 0.4, screenColor: RGBAColor = RGBAColor(r: 0.5, g: 0.5, b: 0.5)) throws -> (CVPixelBuffer, CGPoint, Double) {
        let screen = try TB.filled(1920, 1080, screenColor)
        let out = try PixelBuffers.make(width: W, height: H)
        var style = TB.plainStyle()
        style.cursor.size = 4
        style.cursor.motionBlur = motionBlur
        try MetalRenderer().render(screen: screen, camera: nil, state: FrameState(time: 0, cursor: cursor), style: style, into: out)
        let l = RenderLayout(outputSize: PixelSize(width: W, height: H), sourceSize: PixelSize(width: 1920, height: 1080), style: style, camera: .identity)
        return (out, l.outputPoint(forSource: cursor.position), l.cursorPixelsPerPoint(style: style.cursor))
    }

    @Test func cursorIsDrawnAndRespectsVisibility() throws {
        let (out, tip, unit) = try cursorScene(CursorState())
        let body = CGPoint(x: tip.x + 3 * unit, y: tip.y + 8 * unit)
        // The system arrow: black body, white outline along its left edge (just left of the body).
        #expect(TB.pixel(out, body).luma < 15, "body \(TB.pixel(out, body))")
        let outline: Int = (0..<Int(4 * unit)).map { (i: Int) -> Int in TB.pixel(out, Int(tip.x + 3 * unit) - i, Int(body.y)).luma }.max() ?? 0
        #expect(outline > 230, "outline \(outline)")

        let (hidden, _, _) = try cursorScene(CursorState(visible: false))
        #expect(abs(TB.pixel(hidden, body).luma - 128) <= 2)

        let (half, _, _) = try cursorScene(CursorState(opacity: 0.5))
        let mid = TB.pixel(half, body).luma
        #expect(mid > 70 && mid < 115, "half-faded body \(mid)")  // black at 50% over grey, blended in linear light
    }

    @Test func cursorMotionBlurSmearsAlongVelocity() throws {
        let fast = NormalizedPoint(x: 3, y: 0)
        let (sharp, tip, unit) = try cursorScene(CursorState(), screenColor: .white)
        let (blurred, _, _) = try cursorScene(CursorState(velocity: fast), motionBlur: 1, screenColor: .white)
        let body = CGPoint(x: tip.x + 3 * unit, y: tip.y + 8 * unit)
        #expect(TB.pixel(blurred, body).luma > TB.pixel(sharp, body).luma + 40)
        let ahead = CGPoint(x: tip.x + 3 * unit + 50, y: tip.y + 8 * unit)
        #expect(TB.pixel(sharp, ahead).luma >= 253)
        #expect(TB.pixel(blurred, ahead).luma < 245, "ahead \(TB.pixel(blurred, ahead))")

        // motionBlur 0 means no blur even when moving.
        let (noBlur, _, _) = try cursorScene(CursorState(velocity: fast), motionBlur: 0, screenColor: .white)
        #expect(TB.bytes(noBlur) == TB.bytes(sharp))
    }

    // MARK: Camera bubble

    enum ShapeCase: String, CaseIterable, Sendable {
        case circle, squircle, roundedRect, rectangle
        var shape: CameraShape {
            switch self {
            case .circle: .circle
            case .squircle, .roundedRect: .square
            case .rectangle: .wide
            }
        }
        /// Expected camera coverage at (0.8, 0.8) and (0.97, 0.97) of the bubble half-size.
        var expectInside: (Bool, Bool) {
            switch self {
            case .circle: (false, false)
            case .squircle: (true, false)
            case .roundedRect: (true, false)
            case .rectangle: (true, true)
            }
        }
    }

    @Test(arguments: ShapeCase.allCases)
    func bubbleIsMaskedToItsShape(_ shapeCase: ShapeCase) throws {
        let screen = try TB.filled(1920, 1080, TB.green)
        let cam = try TB.filled(640, 480, TB.blue)
        let out = try PixelBuffers.make(width: W, height: H)
        var style = TB.plainStyle(padding: 0)
        style.camera = CameraBubble(visible: true, shape: shapeCase.shape, position: .center, size: 0.3, border: nil, mirror: false, roundness: shapeCase == .rectangle ? 0 : (shapeCase == .squircle ? 0.6 : 0.5))
        try MetalRenderer().render(screen: screen, camera: cam, state: TB.hiddenCursor(), style: style, into: out)
        let l = RenderLayout(outputSize: PixelSize(width: W, height: H), sourceSize: PixelSize(width: 1920, height: 1080), style: style, camera: .identity)
        let r = l.cameraBubbleRect(style.camera, cameraSize: PixelSize(width: 640, height: 480))
        func at(_ f: Double) -> RGB { TB.pixel(out, Int(r.midX + f * r.width / 2), Int(r.midY + f * r.height / 2)) }
        let blue = RGB(r: 0, g: 0, b: 255), green = RGB(r: 0, g: 255, b: 0)
        #expect(at(0).near(blue, 1))
        #expect(at(0.5).near(blue, 1))
        let (mid, corner) = shapeCase.expectInside
        #expect(at(0.8).near(mid ? blue : green, 1), "\(shapeCase) at 0.8 = \(at(0.8))")
        #expect(at(0.97).near(corner ? blue : green, 1), "\(shapeCase) at 0.97 = \(at(0.97))")
    }

    @Test func bubbleBorderAndMirror() throws {
        let screen = try TB.filled(1920, 1080, TB.green)
        // Camera: left half blue, right half red.
        let cam = try TB.make(640, 480) { x, _ in x < 320 ? TB.blue : TB.red }
        let out = try PixelBuffers.make(width: W, height: H)
        var style = TB.plainStyle(padding: 0)
        style.camera = CameraBubble(visible: true, shape: .circle, position: .center, size: 0.3, border: CameraBorder(width: 0.01, color: .white), mirror: false)
        let renderer = try MetalRenderer()
        try renderer.render(screen: screen, camera: cam, state: TB.hiddenCursor(), style: style, into: out)
        let l = RenderLayout(outputSize: PixelSize(width: W, height: H), sourceSize: PixelSize(width: 1920, height: 1080), style: style, camera: .identity)
        let r = l.cameraBubbleRect(style.camera, cameraSize: PixelSize(width: 640, height: 480))
        let left = CGPoint(x: r.midX - r.width / 4, y: r.midY)
        #expect(TB.pixel(out, left).near(RGB(r: 0, g: 0, b: 255), 1))
        // Border: 0.01 * 1080 = 10.8 px inside the edge.
        #expect(TB.pixel(out, Int(r.midX), Int(r.minY) + 5).near(RGB(r: 255, g: 255, b: 255), 2))

        style.camera.mirror = true
        try renderer.render(screen: screen, camera: cam, state: TB.hiddenCursor(), style: style, into: out)
        #expect(TB.pixel(out, left).near(RGB(r: 255, g: 0, b: 0), 1))

        style.camera.visible = false
        try renderer.render(screen: screen, camera: cam, state: TB.hiddenCursor(), style: style, into: out)
        #expect(TB.pixel(out, left).near(RGB(r: 0, g: 255, b: 0), 1))
    }

    // MARK: Backgrounds

    @Test func linearGradientRunsAlongItsAngle() throws {
        let out = try PixelBuffers.make(width: W, height: H)
        var style = TB.plainStyle(padding: 0.2)
        style.background = .gradient(stops: [GradientStop(location: 0, color: TB.red), GradientStop(location: 1, color: TB.blue)], angleDegrees: 0)
        try MetalRenderer().render(screen: try TB.filled(64, 36, TB.green), camera: nil, state: TB.hiddenCursor(), style: style, into: out)
        let left = TB.pixel(out, 1, H / 2), right = TB.pixel(out, W - 2, H / 2)
        #expect(left.r > 240 && left.b < 20, "left \(left)")
        #expect(right.b > 240 && right.r < 20, "right \(right)")
        // Vertical gradient: top vs bottom differ, left vs right of the same row agree.
        style.background = .gradient(stops: [GradientStop(location: 0, color: .white), GradientStop(location: 1, color: .black)], angleDegrees: 90)
        try MetalRenderer().render(screen: try TB.filled(64, 36, TB.green), camera: nil, state: TB.hiddenCursor(), style: style, into: out)
        #expect(TB.pixel(out, 5, 1).luma > 240 && TB.pixel(out, 5, H - 2).luma < 15)
        #expect(abs(TB.pixel(out, 5, 30).luma - TB.pixel(out, W - 5, 30).luma) <= 2)
    }

    @Test(arguments: Wallpapers.names)
    func wallpapersRender(name: String) throws {
        let out = try PixelBuffers.make(width: 960, height: 540)
        var style = TB.plainStyle(padding: 0.2)
        style.background = .wallpaper(name: name)
        try MetalRenderer().render(screen: try TB.filled(64, 36, TB.green), camera: nil, state: TB.hiddenCursor(), style: style, into: out)
        #expect(TB.pixel(out, 1, 1) != TB.pixel(out, 958, 538))
        #expect(TB.pixel(out, 480, 270).near(RGB(r: 0, g: 255, b: 0), 1))
    }

    @Test func imageBackgroundIsLoadedAndAspectFilled() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("nss-render-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        // 200x100 image: left half yellow, right half cyan. Aspect-filled into 1:1 the centre column stays.
        let url = dir.appendingPathComponent("bg.png")
        try writePNG(url, width: 200, height: 100) { x, _ in x < 100 ? (255, 255, 0) : (0, 255, 255) }
        let out = try PixelBuffers.make(width: 600, height: 600)
        var style = TB.plainStyle(padding: 0.3)
        style.background = .image(file: "bg.png")
        let r = try MetalRenderer(assetBaseURL: dir)
        try r.render(screen: try TB.filled(64, 36, TB.green), camera: nil, state: TB.hiddenCursor(), style: style, into: out)
        #expect(TB.pixel(out, 2, 300).near(RGB(r: 255, g: 255, b: 0), 2))
        #expect(TB.pixel(out, 597, 300).near(RGB(r: 0, g: 255, b: 255), 2))
        // Missing file falls back to grey instead of throwing.
        style.background = .image(file: "/nonexistent/nope.png")
        try r.render(screen: try TB.filled(64, 36, TB.green), camera: nil, state: TB.hiddenCursor(), style: style, into: out)
        #expect(TB.pixel(out, 2, 2).near(RGB(r: 128, g: 128, b: 128), 3))
    }

    @Test func imageBackgroundReloadsWhenTheFileChangesOrAppears() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("nss-render-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let url = dir.appendingPathComponent("bg.png")
        let out = try PixelBuffers.make(width: 320, height: 180)
        var style = TB.plainStyle(padding: 0.3)
        style.background = .image(file: "bg.png")
        let r = try MetalRenderer(assetBaseURL: dir)
        let screen = try TB.filled(64, 36, TB.green)

        // Rendered before the file exists: grey fallback, and the miss must not stick.
        try r.render(screen: screen, camera: nil, state: TB.hiddenCursor(), style: style, into: out)
        #expect(TB.pixel(out, 2, 2).near(RGB(r: 128, g: 128, b: 128), 3))

        try writePNG(url, width: 32, height: 18) { _, _ in (255, 255, 0) }
        try r.render(screen: screen, camera: nil, state: TB.hiddenCursor(), style: style, into: out)
        #expect(TB.pixel(out, 2, 2).near(RGB(r: 255, g: 255, b: 0), 2))

        // Same path rewritten with new pixels (and a different size so the stamp changes even on
        // filesystems with coarse timestamps): the new image is shown.
        try writePNG(url, width: 64, height: 36) { _, _ in (0, 255, 255) }
        try r.render(screen: screen, camera: nil, state: TB.hiddenCursor(), style: style, into: out)
        #expect(TB.pixel(out, 2, 2).near(RGB(r: 0, g: 255, b: 255), 2))

        // Same size, explicit invalidation.
        try writePNG(url, width: 64, height: 36) { _, _ in (255, 0, 255) }
        r.invalidateBackgroundImage()
        try r.render(screen: screen, camera: nil, state: TB.hiddenCursor(), style: style, into: out)
        #expect(TB.pixel(out, 2, 2).near(RGB(r: 255, g: 0, b: 255), 2))
    }

    private func writePNG(_ url: URL, width: Int, height: Int, _ f: (Int, Int) -> (UInt8, UInt8, UInt8)) throws {
        var data = [UInt8](repeating: 255, count: width * height * 4)
        for y in 0..<height {
            for x in 0..<width {
                let (r, g, b) = f(x, y)
                let i = (y * width + x) * 4
                data[i] = r; data[i + 1] = g; data[i + 2] = b
            }
        }
        let space = CGColorSpace(name: CGColorSpace.sRGB)!
        let img = try data.withUnsafeMutableBytes { raw in
            let ctx = try #require(CGContext(data: raw.baseAddress, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width * 4,
                                             space: space, bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue))
            return try #require(ctx.makeImage())
        }
        let dest = try #require(CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil))
        CGImageDestinationAddImage(dest, img, nil)
        #expect(CGImageDestinationFinalize(dest))
    }

    // MARK: Contract

    @Test func renderIsDeterministic() throws {
        let screen = try TB.make(1280, 800) { x, y in RGBAColor(r: Double(x % 256) / 255, g: Double(y % 256) / 255, b: 0.4) }
        let cam = try TB.filled(640, 480, TB.blue)
        let state = FrameState(
            time: 1.25,
            camera: CameraState(scale: 1.7, center: NormalizedPoint(x: 0.4, y: 0.6)),
            cursor: CursorState(position: NormalizedPoint(x: 0.45, y: 0.55), velocity: NormalizedPoint(x: 0.8, y: -0.3),
                                ripples: [RippleState(position: NormalizedPoint(x: 0.42, y: 0.58), progress: 0.4)])
        )
        let a = try PixelBuffers.make(width: 1280, height: 720), b = try PixelBuffers.make(width: 1280, height: 720)
        try MetalRenderer().render(screen: screen, camera: cam, state: state, style: Style(), into: a)
        try MetalRenderer().render(screen: screen, camera: cam, state: state, style: Style(), into: b)
        #expect(TB.bytes(a) == TB.bytes(b))
    }

    @Test func makeDefaultReturnsMetalWhenADeviceExists() throws {
        #expect(try MetalRenderer.makeDefault() is MetalRenderer)
    }

    @Test func rejectsNonBGRABuffers() throws {
        var pb: CVPixelBuffer?
        CVPixelBufferCreate(nil, 16, 16, kCVPixelFormatType_32ARGB, [kCVPixelBufferIOSurfacePropertiesKey: [:] as [String: Any]] as CFDictionary, &pb)
        let out = try PixelBuffers.make(width: 16, height: 16)
        #expect(throws: NSSError.self) { try MetalRenderer().render(screen: pb!, camera: nil, state: FrameState(time: 0), style: Style(), into: out) }
    }

    // MARK: Benchmark

    @Test func benchmark4KFrame() throws {
        let r = try MetalRenderer()
        let screen = try TB.make(3840, 2160) { x, y in RGBAColor(r: Double(x & 255) / 255, g: Double(y & 255) / 255, b: 0.5) }
        let cam = try TB.filled(1920, 1080, TB.blue)
        let out = try PixelBuffers.make(width: 3840, height: 2160)
        let style = Style()  // gradient, shadow, rounded corners, bordered circle bubble
        let state = FrameState(
            time: 0,
            camera: CameraState(scale: 1.6, center: NormalizedPoint(x: 0.4, y: 0.4)),
            cursor: CursorState(position: NormalizedPoint(x: 0.42, y: 0.41), velocity: NormalizedPoint(x: 1.5, y: 0.6),
                                ripples: (0..<3).map { RippleState(position: NormalizedPoint(x: 0.4 + 0.02 * Double($0), y: 0.4), progress: 0.2 * Double($0 + 1)) })
        )
        for _ in 0..<5 { try r.render(screen: screen, camera: cam, state: state, style: style, into: out) }
        var times: [Double] = []
        for _ in 0..<30 {
            let t0 = DispatchTime.now().uptimeNanoseconds
            try r.render(screen: screen, camera: cam, state: state, style: style, into: out)
            times.append(Double(DispatchTime.now().uptimeNanoseconds - t0) / 1e6)
        }
        times.sort()
        let best = times[0], median = times[times.count / 2]
        print("MetalRenderer 4K frame: best \(String(format: "%.2f", best)) ms, median \(String(format: "%.2f", median)) ms")
        // Wall-clock budgets are flaky on loaded dev machines and CI VMs: only assert when asked to
        // (`NSS_BENCH=1 swift test --filter benchmark4KFrame`). Otherwise just report the timing.
        if ProcessInfo.processInfo.environment["NSS_BENCH"] == "1" {
            #expect(median < 8, "median 4K frame \(median) ms over the 8 ms budget")
        }
    }
}
