import CoreVideo
import NSSCore

/// STUB: a FrameSource that returns one solid-colour 32BGRA frame for every `t` (and for the camera
/// when `hasCamera`). Useful for tests and as a placeholder until the real decoders land.
public final class SolidColorFrameSource: FrameSource, @unchecked Sendable {
    private let screen: CVPixelBuffer
    private let camera: CVPixelBuffer?

    public init(width: Int, height: Int, color: RGBAColor, hasCamera: Bool = false) throws {
        screen = try Self.filled(width: width, height: height, color: color)
        camera = hasCamera ? try Self.filled(width: width, height: height, color: color) : nil
    }

    public func screenFrame(at t: MediaTime) async -> CVPixelBuffer? { screen }
    public func cameraFrame(at t: MediaTime) async -> CVPixelBuffer? { camera }

    private static func filled(width: Int, height: Int, color: RGBAColor) throws -> CVPixelBuffer {
        let pb = try PixelBuffers.make(width: width, height: height)
        CVPixelBufferLockBaseAddress(pb, [])
        defer { CVPixelBufferUnlockBaseAddress(pb, []) }
        let base = CVPixelBufferGetBaseAddress(pb)!.assumingMemoryBound(to: UInt8.self)
        let stride = CVPixelBufferGetBytesPerRow(pb)
        func byte(_ v: Double) -> UInt8 { UInt8(max(0, min(1, v)) * 255 + 0.5) }
        for y in 0..<height {
            for x in 0..<width {
                let p = base + y * stride + x * 4
                p[0] = byte(color.b); p[1] = byte(color.g); p[2] = byte(color.r); p[3] = byte(color.a)
            }
        }
        return pb
    }
}
