import CoreGraphics
import CoreVideo
import Foundation
import NSSCore
@testable import NSSRender

struct RGB: Equatable, CustomStringConvertible {
    var r: Int, g: Int, b: Int
    var description: String { "(\(r), \(g), \(b))" }
    func near(_ o: RGB, _ tol: Int = 3) -> Bool { abs(r - o.r) <= tol && abs(g - o.g) <= tol && abs(b - o.b) <= tol }
    var luma: Int { (r + g + b) / 3 }
}

enum TB {
    static let red = RGBAColor(r: 1, g: 0, b: 0)
    static let green = RGBAColor(r: 0, g: 1, b: 0)
    static let blue = RGBAColor(r: 0, g: 0, b: 1)

    static func filled(_ w: Int, _ h: Int, _ c: RGBAColor) throws -> CVPixelBuffer {
        try make(w, h) { _, _ in c }
    }

    /// Builds a 32BGRA buffer from a per-pixel colour function.
    static func make(_ w: Int, _ h: Int, _ f: (Int, Int) -> RGBAColor) throws -> CVPixelBuffer {
        let pb = try PixelBuffers.make(width: w, height: h)
        CVPixelBufferLockBaseAddress(pb, [])
        defer { CVPixelBufferUnlockBaseAddress(pb, []) }
        let base = CVPixelBufferGetBaseAddress(pb)!.assumingMemoryBound(to: UInt8.self)
        let stride = CVPixelBufferGetBytesPerRow(pb)
        func byte(_ v: Double) -> UInt8 { UInt8(max(0, min(1, v)) * 255 + 0.5) }
        for y in 0..<h {
            for x in 0..<w {
                let c = f(x, y)
                let p = base + y * stride + x * 4
                p[0] = byte(c.b); p[1] = byte(c.g); p[2] = byte(c.r); p[3] = byte(c.a)
            }
        }
        return pb
    }

    static func pixel(_ pb: CVPixelBuffer, _ x: Int, _ y: Int) -> RGB {
        CVPixelBufferLockBaseAddress(pb, .readOnly)
        defer { CVPixelBufferUnlockBaseAddress(pb, .readOnly) }
        let base = CVPixelBufferGetBaseAddress(pb)!.assumingMemoryBound(to: UInt8.self)
        let p = base + y * CVPixelBufferGetBytesPerRow(pb) + x * 4
        return RGB(r: Int(p[2]), g: Int(p[1]), b: Int(p[0]))
    }

    static func pixel(_ pb: CVPixelBuffer, _ pt: CGPoint) -> RGB { pixel(pb, Int(pt.x), Int(pt.y)) }

    static func bytes(_ pb: CVPixelBuffer) -> [UInt8] {
        CVPixelBufferLockBaseAddress(pb, .readOnly)
        defer { CVPixelBufferUnlockBaseAddress(pb, .readOnly) }
        let n = CVPixelBufferGetBytesPerRow(pb) * CVPixelBufferGetHeight(pb)
        let base = CVPixelBufferGetBaseAddress(pb)!.assumingMemoryBound(to: UInt8.self)
        return Array(UnsafeBufferPointer(start: base, count: n))
    }

    /// A plain style: solid red background, no shadow, no camera bubble, no ripples.
    static func plainStyle(padding: Double = 0.1, corner: Double = 0) -> Style {
        var s = Style()
        s.background = .solid(red)
        s.padding = padding
        s.cornerRadius = corner
        s.shadow.opacity = 0
        s.camera.visible = false
        s.camera.border = nil
        return s
    }

    static func hiddenCursor() -> FrameState {
        FrameState(time: 0, cursor: CursorState(visible: false))
    }
}

enum GPU {
    static let available = MetalRenderer.isSupported
}
