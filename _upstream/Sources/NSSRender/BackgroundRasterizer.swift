import CoreGraphics
import Foundation
import NSSCore

/// The procedural part of a background reduced to shader inputs. `MetalRenderer` writes these into
/// uniforms and `BackgroundRasterizer` evaluates the same maths on the CPU, so the two cannot drift.
enum BackgroundShading: Equatable {
    /// Linear-light premultiplied colour.
    case solid(SIMD4<Double>)
    /// OKLab + alpha stops sorted by location; `dir` is the unit direction, `invLength` = 1 / gradient length (px).
    case linear(stops: [SIMD4<Double>], locations: [Double], dir: SIMD2<Double>, invLength: Double)
    /// OKLab colour points at normalized positions, with the sine warp amount.
    case mesh(colors: [SIMD3<Double>], positions: [SIMD2<Double>], warp: Double)
    /// Drawn from an image texture (or mid-grey when the image is missing).
    case image

    static let missingImageGrey = RGBAColor(r: 0.5, g: 0.5, b: 0.5)

    static func solidColor(_ c: RGBAColor) -> BackgroundShading {
        let lin = ColorMath.linear(c)
        return .solid(SIMD4(lin.x, lin.y, lin.z, max(0, min(1, c.a))))
    }

    /// `W`, `H`: output size in pixels (the gradient direction depends on the aspect ratio).
    static func make(_ bg: Background, W: Double, H: Double) -> BackgroundShading {
        switch bg {
        case .solid(let c):
            return solidColor(c)
        case .gradient(let rawStops, let angle):
            let stops = Array(rawStops.sorted { $0.location < $1.location }.prefix(Slot.maxStops))
            guard let first = stops.first else { return solidColor(.black) }
            if stops.count == 1 { return solidColor(first.color) }
            let a = angle * .pi / 180
            let dx = cos(a), dy = sin(a)
            let halfExtent = 0.5 * (W * abs(dx) + H * abs(dy))
            return .linear(
                stops: stops.map { let l = ColorMath.oklab($0.color); return SIMD4(l.x, l.y, l.z, max(0, min(1, $0.color.a))) },
                locations: stops.map(\.location),
                dir: SIMD2(dx, dy),
                invLength: 1 / max(2 * halfExtent, 1))
        case .mesh(let points, let warp):
            return mesh(points, warp: warp)
        case .wallpaper(let name):
            let w = BackgroundPresets.legacyWallpaper(named: name)
            return mesh(w.points, warp: w.warp)
        case .image, .systemWallpaper:
            return .image
        }
    }

    private static func mesh(_ raw: [MeshPoint], warp: Double) -> BackgroundShading {
        let points = Array(raw.prefix(Slot.maxStops))
        guard let first = points.first else { return solidColor(.black) }
        if points.count == 1 { return solidColor(first.color) }
        return .mesh(colors: points.map { ColorMath.oklab($0.color) }, positions: points.map { SIMD2($0.x, $0.y) },
                     warp: warp.isFinite ? warp : 0)
    }

    /// Linear-light colour at pixel centre `p` of a `size` output: the CPU twin of the shader's `background()`.
    func linearColor(at p: SIMD2<Double>, size: SIMD2<Double>) -> SIMD3<Double> {
        switch self {
        case .solid(let c):
            return SIMD3(c.x, c.y, c.z) * c.w
        case .linear(let stops, let locs, let dir, let invLength):
            let d = p - 0.5 * size
            let t = min(max((d.x * dir.x + d.y * dir.y) * invLength + 0.5, 0), 1)
            var c = stops[0]
            for i in 1..<stops.count {
                let l0 = locs[i - 1], l1 = locs[i]
                let f = t >= l0 ? min(max((t - l0) / max(l1 - l0, 1e-5), 0), 1) : 0
                c = c + (stops[i] - c) * f
            }
            return ColorMath.oklabToLinear(SIMD3(c.x, c.y, c.z)) * c.w
        case .mesh(let colors, let positions, let warp):
            var uv = p / size
            uv += warp * SIMD2(sin(uv.y * 5.1 + 1.3), sin(uv.x * 4.3 + 0.7))
            let aspect = size / max(size.x, size.y)
            var acc = SIMD3<Double>(repeating: 0)
            var wsum = 0.0
            for i in colors.indices {
                let d = (uv - positions[i]) * aspect
                let w = 1 / pow(d.x * d.x + d.y * d.y + 0.015, 1.6)
                acc += colors[i] * w
                wsum += w
            }
            return ColorMath.oklabToLinear(acc / wsum)
        case .image:
            return ColorMath.linear(Self.missingImageGrey)
        }
    }
}

/// CPU rendering of colour and gradient backgrounds, using exactly the renderer's maths. The picker uses it for
/// thumbnails (no GPU round trip); tests use it as the reference the GPU output is checked against.
public enum BackgroundRasterizer {
    /// sRGB 8-bit pixels (RGBA, row-major, no dither) of `background` at `width` x `height`.
    /// Image backgrounds come out mid-grey: use the image file for their thumbnails.
    public static func rgba(_ background: Background, width: Int, height: Int) -> [UInt8] {
        let w = max(width, 1), h = max(height, 1)
        let shading = BackgroundShading.make(background, W: Double(w), H: Double(h))
        let size = SIMD2(Double(w), Double(h))
        var out = [UInt8](repeating: 255, count: w * h * 4)
        for y in 0..<h {
            for x in 0..<w {
                let c = shading.linearColor(at: SIMD2(Double(x) + 0.5, Double(y) + 0.5), size: size)
                let i = (y * w + x) * 4
                out[i] = byte(c.x); out[i + 1] = byte(c.y); out[i + 2] = byte(c.z)
            }
        }
        return out
    }

    /// A `CGImage` thumbnail of `background` (see `rgba`).
    public static func cgImage(_ background: Background, width: Int, height: Int) -> CGImage? {
        let w = max(width, 1), h = max(height, 1)
        let data = Data(rgba(background, width: w, height: h)) as CFData
        guard let provider = CGDataProvider(data: data), let space = CGColorSpace(name: CGColorSpace.sRGB) else { return nil }
        return CGImage(
            width: w, height: h, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: w * 4, space: space,
            bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.noneSkipLast.rawValue), provider: provider,
            decode: nil, shouldInterpolate: true, intent: .defaultIntent)
    }

    private static func byte(_ linear: Double) -> UInt8 {
        UInt8(max(0, min(255, (ColorMath.linearToSrgb(linear) * 255).rounded())))
    }
}
