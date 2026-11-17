import CoreGraphics
import CoreVideo
import Foundation
import ImageIO
import Testing
import UniformTypeIdentifiers
import NSSCore
@testable import NSSRender

/// Round 3 backgrounds: gradient presets, mesh gradients, system wallpapers and blur.
@Suite(.enabled(if: GPU.available, "needs a Metal device"))
struct BackgroundRenderTests {
    static let W = 320, H = 180

    private func render(_ style: Style, renderer: MetalRenderer? = nil, width: Int = W, height: Int = H) throws -> CVPixelBuffer {
        let out = try PixelBuffers.make(width: width, height: height)
        try (renderer ?? MetalRenderer()).render(screen: try TB.filled(64, 36, TB.green), camera: nil, state: TB.hiddenCursor(), style: style, into: out)
        return out
    }

    private func style(_ bg: Background, blur: Double = 0) -> Style {
        var s = TB.plainStyle(padding: 0.3)
        s.background = bg
        s.backgroundBlur = blur
        return s
    }

    /// The GPU draws every built-in gradient exactly like the CPU rasterizer (the thumbnails), up to the
    /// +-1 LSB dither and float rounding.
    @Test(arguments: BackgroundPresets.gradients.map(\.id))
    func gpuMatchesCPURasterizer(id: String) throws {
        let preset = try #require(BackgroundPresets.gradients.first { $0.id == id })
        let out = try render(style(preset.background))
        let ref = BackgroundRasterizer.rgba(preset.background, width: Self.W, height: Self.H)
        var worst = 0
        // Rows in the top padding band (the screen covers the middle).
        for y in stride(from: 0, to: 40, by: 3) {
            for x in stride(from: 0, to: Self.W, by: 7) {
                let g = TB.pixel(out, x, y)
                let i = (y * Self.W + x) * 4
                worst = max(worst, abs(g.r - Int(ref[i])), abs(g.g - Int(ref[i + 1])), abs(g.b - Int(ref[i + 2])))
            }
        }
        #expect(worst <= 2, "\(id): GPU and CPU differ by \(worst)")
    }

    @Test func meshIsSmoothAndUsesItsColours() throws {
        let bg = Background.mesh(points: [MeshPoint(x: 0, y: 0, color: TB.red), MeshPoint(x: 1, y: 0, color: TB.blue)], warp: 0)
        let out = try render(style(bg))
        #expect(TB.pixel(out, 1, 1).r > 200 && TB.pixel(out, 1, 1).b < 80)
        #expect(TB.pixel(out, Self.W - 2, 1).b > 200 && TB.pixel(out, Self.W - 2, 1).r < 80)
        // No hard steps along the top row.
        var maxStep = 0
        for x in 1..<Self.W { maxStep = max(maxStep, abs(TB.pixel(out, x, 2).luma - TB.pixel(out, x - 1, 2).luma)) }
        #expect(maxStep <= 6)
    }

    @Test func legacyWallpaperDrawsLikeItsMigratedMesh() throws {
        for name in Wallpapers.names {
            let a = try render(style(.wallpaper(name: name)))
            let b = try render(style(BackgroundPresets.migratedWallpaper(named: name)))
            #expect(TB.bytes(a) == TB.bytes(b), "\(name)")
        }
    }

    // MARK: Image backgrounds and blur

    private func withImage(_ body: (URL) throws -> Void) throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("nss-bg-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        // 512x512 checkerboard of 8 px cells, left half yellow/black, right half cyan/black.
        let url = dir.appendingPathComponent("checker.png")
        try writeTestPNG(url, width: 512, height: 512) { x, y in
            guard (x / 8 + y / 8) % 2 == 0 else { return (0, 0, 0) }
            return x < 256 ? (255, 255, 0) : (0, 255, 255)
        }
        try body(url)
    }

    /// Luma spread along the top band (sharp checkerboard: large, blurred: small).
    private func spread(_ pb: CVPixelBuffer) -> Int {
        var lo = 255, hi = 0
        for x in 20..<100 { let l = TB.pixel(pb, x, 10).luma; lo = min(lo, l); hi = max(hi, l) }
        return hi - lo
    }

    @Test func blurSmoothsImagesAndZeroIsSharp() throws {
        try withImage { url in
            let r = try MetalRenderer()
            let sharp = try render(style(.image(file: url.path)), renderer: r)
            let zero = try render(style(.image(file: url.path), blur: 0.004), renderer: r)
            let blurred = try render(style(.image(file: url.path), blur: 0.6), renderer: r)
            #expect(TB.bytes(zero) == TB.bytes(sharp), "blur below 1/100 is no blur")
            #expect(spread(sharp) > 100)
            #expect(spread(blurred) < 25, "blurred spread \(spread(blurred))")
            // Going back to sharp after a blur uses the original texture again.
            #expect(TB.bytes(try render(style(.image(file: url.path)), renderer: r)) == TB.bytes(sharp))
        }
    }

    /// Preview and export use separate renderers: the blurred background must be byte-identical.
    @Test(arguments: [0.15, 0.5, 1.0])
    func blurIsDeterministicAcrossRenderers(amount: Double) throws {
        try withImage { url in
            let a = try render(style(.image(file: url.path), blur: amount), renderer: try MetalRenderer())
            let b = try render(style(.image(file: url.path), blur: amount), renderer: try MetalRenderer())
            #expect(TB.bytes(a) == TB.bytes(b))
        }
    }

    /// The blur radius is relative to the image, so a 2x larger output (export vs preview) shows the same picture.
    @Test func blurLooksTheSameAtEveryOutputSize() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("nss-bg-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let url = dir.appendingPathComponent("halves.png")
        try writeTestPNG(url, width: 640, height: 360) { x, _ in x < 320 ? (255, 40, 40) : (40, 40, 255) }
        let s = style(.image(file: url.path), blur: 0.8)
        let small = try render(s, width: 320, height: 180)
        let big = try render(s, width: 640, height: 360)
        var worst = 0
        for x in stride(from: 100, to: 220, by: 4) {
            let a = TB.pixel(small, x, 10)
            let b = TB.pixel(big, 2 * x, 20)
            worst = max(worst, abs(a.r - b.r), abs(a.b - b.b))
        }
        #expect(worst <= 8, "preview-size and export-size blur differ by \(worst)")
        // And the edge really is blurred (a gradual transition across the middle).
        #expect(TB.pixel(small, 160, 10).r > 80 && TB.pixel(small, 160, 10).b > 80)
    }

    @Test func systemWallpaperDrawsLikeTheSameImage() throws {
        try withImage { url in
            let a = try render(style(.systemWallpaper(path: url.path), blur: 0.3))
            let b = try render(style(.image(file: url.path), blur: 0.3))
            #expect(TB.bytes(a) == TB.bytes(b))
            // A wallpaper that is missing on this Mac draws the grey fallback and does not throw.
            let missing = try render(style(.systemWallpaper(path: "/nonexistent/wall.heic")))
            #expect(TB.pixel(missing, 2, 2).near(RGB(r: 128, g: 128, b: 128), 2))
        }
    }

    @Test func blurPlanKeepsTheKernelSmall() {
        #expect(ImageBlur.plan(sigma: 3, mipLevels: 10) == (0, 3))
        #expect(ImageBlur.plan(sigma: 12, mipLevels: 10) == (1, 6))
        let big = ImageBlur.plan(sigma: 100, mipLevels: 10)
        #expect(big.sigma <= ImageBlur.maxWorkingSigma && big.sigma > ImageBlur.maxWorkingSigma / 2)
        #expect(abs(big.sigma * Double(1 << big.level) - 100) < 1e-9)
        #expect(ImageBlur.plan(sigma: 100, mipLevels: 2).level == 1, "capped at the last mip level")
    }
}

func writeTestPNG(_ url: URL, width: Int, height: Int, _ f: (Int, Int) -> (UInt8, UInt8, UInt8)) throws {
    var data = [UInt8](repeating: 255, count: width * height * 4)
    for y in 0..<height {
        for x in 0..<width {
            let (r, g, b) = f(x, y)
            let i = (y * width + x) * 4
            data[i] = r; data[i + 1] = g; data[i + 2] = b
        }
    }
    let space = CGColorSpace(name: CGColorSpace.sRGB)!
    let provider = CGDataProvider(data: Data(data) as CFData)!
    let image = CGImage(width: width, height: height, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: width * 4, space: space,
                        bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.noneSkipLast.rawValue), provider: provider,
                        decode: nil, shouldInterpolate: false, intent: .defaultIntent)!
    let dest = CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil)!
    CGImageDestinationAddImage(dest, image, nil)
    guard CGImageDestinationFinalize(dest) else { throw NSSError.io("png write failed") }
}
