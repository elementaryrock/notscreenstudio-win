import CoreVideo
import NSSCore

public enum PixelBuffers {
    /// Makes an IOSurface-backed, Metal-compatible 32BGRA buffer.
    public static func make(width: Int, height: Int) throws -> CVPixelBuffer {
        var pb: CVPixelBuffer?
        let attrs: [CFString: Any] = [
            kCVPixelBufferIOSurfacePropertiesKey: [:] as [String: Any],
            kCVPixelBufferMetalCompatibilityKey: true,
        ]
        let st = CVPixelBufferCreate(nil, width, height, kCVPixelFormatType_32BGRA, attrs as CFDictionary, &pb)
        guard st == kCVReturnSuccess, let pb else { throw NSSError.renderFailed("CVPixelBufferCreate failed: \(st)") }
        return pb
    }
}
