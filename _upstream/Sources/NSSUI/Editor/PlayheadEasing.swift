import Foundation
import NSSCore

/// Pure model for the drawn playhead. The real playhead (and the seek) always jump at once; this only
/// decides where the red line is drawn. A jump glides over `duration` seconds with an ease-out curve;
/// `snap` (playback, dragging) follows exactly. Deterministic: no clock, the caller feeds `dt`.
public struct PlayheadEasing: Equatable, Sendable {
    /// Glide time for jumps (ruler click, frame step, keyboard, undo and redo).
    public static let defaultDuration: Double = 0.2

    public private(set) var value: MediaTime
    public private(set) var target: MediaTime
    public let duration: Double
    private var start: MediaTime
    private var elapsed: Double

    public init(at time: MediaTime = 0, duration: Double = PlayheadEasing.defaultDuration) {
        value = time
        target = time
        start = time
        elapsed = 0
        self.duration = max(duration, 0.001)
    }

    public var isAnimating: Bool { value != target }

    /// Cubic ease-out: fast start, soft landing. `p` is clamped to 0...1.
    public static func easeOut(_ p: Double) -> Double {
        let q = 1 - min(max(p, 0), 1)
        return 1 - q * q * q
    }

    /// Draw exactly at `time`, no animation.
    public mutating func snap(to time: MediaTime) {
        value = time
        target = time
        start = time
        elapsed = 0
    }

    /// Glide to `time` from wherever the drawn playhead is now (also when a glide is already running).
    public mutating func glide(to time: MediaTime) {
        guard time != target else { return }
        start = value
        target = time
        elapsed = 0
    }

    /// Moves the drawn playhead forward by `dt` seconds of display time.
    public mutating func advance(by dt: Double) {
        guard value != target else { return }
        elapsed += max(dt, 0)
        if elapsed >= duration {
            value = target
        } else {
            value = start + (target - start) * Self.easeOut(elapsed / duration)
        }
    }
}
