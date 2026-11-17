import CoreGraphics
import Foundation

/// Elapsed recording time that stops while paused. Pure: every method takes the current time, so it is
/// fully testable and never reads the wall clock itself.
public struct ElapsedClock: Equatable, Sendable {
    public let startedAt: Date
    private var pausedAt: Date?
    private var pausedTotal: TimeInterval = 0

    public init(startedAt: Date) { self.startedAt = startedAt }

    public var isPaused: Bool { pausedAt != nil }

    /// Recorded seconds at `now`: time since the start minus every pause.
    public func elapsed(at now: Date) -> TimeInterval {
        let end = pausedAt ?? now
        return max(0, end.timeIntervalSince(startedAt) - pausedTotal)
    }

    public mutating func pause(at now: Date) {
        guard pausedAt == nil else { return }
        pausedAt = now
    }

    public mutating func resume(at now: Date) {
        guard let p = pausedAt else { return }
        pausedTotal += max(0, now.timeIntervalSince(p))
        pausedAt = nil
    }
}

/// Where the floating bars sit: bottom centre of a screen's visible area.
public enum BarPlacement {
    public static let bottomMargin: CGFloat = 28

    /// Frame (AppKit coordinates, origin bottom-left) for a bar of `size`, centred above the bottom edge of `visible`.
    /// The bar is shrunk if it is wider than the screen.
    public static func frame(size: CGSize, in visible: CGRect) -> CGRect {
        let width = min(size.width, visible.width)
        let height = min(size.height, visible.height)
        return CGRect(
            x: visible.midX - width / 2, y: visible.minY + bottomMargin, width: width, height: height)
    }

    /// The index of the screen that holds most of `source` (nil source or no overlap means the first screen).
    public static func screenIndex(for source: CGRect?, screens: [CGRect]) -> Int {
        guard let source, !screens.isEmpty else { return 0 }
        var best = 0
        var bestArea: CGFloat = -1
        for (i, s) in screens.enumerated() {
            let r = s.intersection(source)
            let area = r.isNull ? 0 : r.width * r.height
            if area > bestArea { best = i; bestArea = area }
        }
        return best
    }
}
