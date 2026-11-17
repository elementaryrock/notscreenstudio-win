import CoreVideo
import NSSCore

/// CPU fallback for Macs without Metal (see `MetalRenderer.makeDefault()`): clears the output to the
/// style's flat background color. Does not draw the screen, cursor or camera.
/// Expects a 32BGRA output buffer.
public struct PassthroughRenderer: FrameRenderer {
    public init() {}

    public func render(
        screen: CVPixelBuffer,
        camera: CVPixelBuffer?,
        state: FrameState,
        style: Style,
        into output: CVPixelBuffer
    ) throws {
        guard CVPixelBufferGetPixelFormatType(output) == kCVPixelFormatType_32BGRA else {
            throw NSSError.unsupported("PassthroughRenderer needs a 32BGRA output buffer")
        }
        CVPixelBufferLockBaseAddress(output, [])
        defer { CVPixelBufferUnlockBaseAddress(output, []) }
        guard let base = CVPixelBufferGetBaseAddress(output) else {
            throw NSSError.renderFailed("no base address")
        }
        let c = style.backgroundFallbackColor
        func byte(_ v: Double) -> UInt8 { UInt8((min(max(v, 0), 1) * 255).rounded()) }
        let px: [UInt8] = [byte(c.b), byte(c.g), byte(c.r), byte(c.a)]
        let w = CVPixelBufferGetWidth(output), h = CVPixelBufferGetHeight(output)
        let stride = CVPixelBufferGetBytesPerRow(output)
        for y in 0..<h {
            let row = base.advanced(by: y * stride).assumingMemoryBound(to: UInt8.self)
            for x in 0..<w { for k in 0..<4 { row[x * 4 + k] = px[k] } }
        }
    }
}
