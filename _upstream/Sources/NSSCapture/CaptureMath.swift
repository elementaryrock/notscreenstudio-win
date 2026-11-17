import CoreGraphics
import CoreMedia
import Foundation
import NSSCore

/// Pure, hardware-free helpers for configuring and timing a capture. Everything here is unit-tested.
public enum CaptureMath {
    public static let frameRate = 60

    /// Round to the nearest even pixel count (hardware encoders want even dimensions), minimum 2.
    public static func evenPixels(_ value: Double) -> Int {
        let v = value.isFinite ? value : 2
        return max(2, Int((v / 2).rounded()) * 2)
    }

    /// Native (Retina) pixel size for a size in points at `scale` pixels per point.
    public static func pixelSize(points: CGSize, scale: Double) -> PixelSize {
        let s = scale > 0 ? scale : 1
        return PixelSize(width: evenPixels(points.width * s), height: evenPixels(points.height * s))
    }

    /// Clamp an area (display points, origin top-left) to the display bounds. nil if nothing is left.
    public static func clampedArea(_ rect: CGRect, displaySize: CGSize) -> CGRect? {
        let bounds = CGRect(origin: .zero, size: displaySize)
        let r = rect.standardized.intersection(bounds)
        guard !r.isNull, r.width >= 2, r.height >= 2 else { return nil }
        return r
    }

    /// Target video bitrate: high quality for screen content (lots of sharp text), clamped.
    public static func videoBitrate(width: Int, height: Int, frameRate: Double, bitsPerPixel: Double = 0.12) -> Int {
        let raw = Double(width) * Double(height) * max(frameRate, 1) * bitsPerPixel
        return Int(min(max(raw, 8_000_000), 200_000_000))
    }

    /// Whether a running app belongs to this process (so it is excluded from the capture).
    public static func isOwnApp(pid: Int32, bundleID: String?, ownPID: Int32, ownBundleID: String?) -> Bool {
        if pid == ownPID { return true }
        if let b = bundleID, let o = ownBundleID, !b.isEmpty, b == o { return true }
        return false
    }

    /// Whether a window should be offered in the source picker.
    public static func isPickableWindow(layer: Int, frame: CGRect, isOnScreen: Bool, hasApp: Bool) -> Bool {
        layer == 0 && isOnScreen && hasApp && frame.width >= 50 && frame.height >= 50
    }

    public static func hostNow() -> Double {
        CMClockGetTime(CMClockGetHostTimeClock()).seconds
    }

    public static func hostTime(_ seconds: Double) -> CMTime {
        CMTime(seconds: seconds, preferredTimescale: 1_000_000_000)
    }
}

/// Maps host-clock times onto the shared recording timeline (t = 0 is the first screen frame).
public struct TimeOrigin: Sendable, Equatable {
    public let hostTime: Double
    public init(hostTime: Double) { self.hostTime = hostTime }

    /// Seconds from the origin, never negative.
    public func relative(_ hostSeconds: Double) -> MediaTime { max(0, hostSeconds - hostTime) }

    /// True when a sample ends at or before the origin and so must be dropped.
    public func isEntirelyBefore(start: Double, duration: Double) -> Bool {
        duration > 0 ? start + duration <= hostTime : start < hostTime
    }

    public func duration(endingAt endHost: Double) -> MediaTime { max(0, endHost - hostTime) }
}

/// Thread-safe, write-once origin shared by all writers. Set by the first complete screen frame.
final class SharedOrigin: @unchecked Sendable {
    private let lock = NSLock()
    private var value: Double?

    var current: TimeOrigin? { lock.withLock { value.map(TimeOrigin.init(hostTime:)) } }

    /// Returns true if this call set the origin.
    @discardableResult
    func setIfUnset(_ hostSeconds: Double) -> Bool {
        lock.withLock {
            guard value == nil else { return false }
            value = hostSeconds
            return true
        }
    }
}

/// File names inside the bundle for a given set of options.
public struct CaptureFilePlan: Sendable, Equatable {
    public var screen: String
    public var camera: String?
    public var mic: String?

    public init(options: RecordingOptions) {
        screen = RecordingAssets.defaultScreenFile
        camera = options.cameraDeviceID == nil ? nil : RecordingAssets.defaultCameraFile
        mic = options.micDeviceID == nil ? nil : RecordingAssets.defaultMicFile
    }

    public func url(_ name: String, in bundle: URL) -> URL { bundle.appendingPathComponent(name) }
}
