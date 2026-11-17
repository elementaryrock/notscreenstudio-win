import CoreGraphics
import Foundation
import NSSCore

/// Resolves a `CaptureSource` to a rectangle in GLOBAL display coordinates: points, origin at the top-left
/// of the primary display, y pointing down (the CoreGraphics / `CGEvent` convention, NOT AppKit's).
/// Injectable so the coordinate math is unit-testable without real displays.
public protocol ScreenGeometryProvider: Sendable {
    /// Current global top-left rect of the source, or nil when it cannot be resolved (for example a closed window).
    func globalRect(for source: CaptureSource) -> CGRect?
    /// Height in points of the primary display. AppKit global coordinates have their origin at its bottom-left.
    var primaryDisplayHeight: Double { get }
    /// True when the rect of `source` can change during a recording and must be re-read often.
    func isDynamic(_ source: CaptureSource) -> Bool
}

extension ScreenGeometryProvider {
    public func isDynamic(_ source: CaptureSource) -> Bool {
        if case .window = source { return true }
        return false
    }
}

/// Pure coordinate math. Everything here is deterministic and free of system calls.
public enum CoordinateMath {
    /// Converts an AppKit global point (origin bottom-left of the primary display, y up) to the global
    /// top-left convention. Valid on every display of a multi-display setup, because AppKit's global space
    /// is anchored at the primary display.
    public static func appKitToTopLeft(_ p: CGPoint, primaryDisplayHeight: Double) -> CGPoint {
        CGPoint(x: p.x, y: primaryDisplayHeight - p.y)
    }

    /// Normalizes a global top-left point against `rect`. The result is NOT clamped: it can fall outside 0...1
    /// for window and area capture, which the contract allows. Returns nil for an empty rect.
    public static func normalize(_ p: CGPoint, in rect: CGRect) -> NormalizedPoint? {
        guard rect.width > 0, rect.height > 0 else { return nil }
        return NormalizedPoint(x: (p.x - rect.minX) / rect.width, y: (p.y - rect.minY) / rect.height)
    }

    /// Resolves a source to a global top-left rect from raw lookups. An `.area` rect is relative to its
    /// display's top-left corner, so it is offset by the display origin.
    public static func resolve(
        _ source: CaptureSource,
        displayBounds: (UInt32) -> CGRect?,
        windowBounds: (UInt32) -> CGRect?
    ) -> CGRect? {
        switch source {
        case .display(let id):
            return displayBounds(id)
        case .window(let id):
            return windowBounds(id)
        case .area(let displayID, let rect):
            guard let d = displayBounds(displayID) else { return nil }
            return CGRect(x: d.minX + rect.minX, y: d.minY + rect.minY, width: rect.width, height: rect.height)
        }
    }
}

/// Real geometry from CoreGraphics. Needs no permission: display bounds and window bounds are readable
/// without Screen Recording (only window titles are gated).
public struct SystemScreenGeometry: ScreenGeometryProvider {
    public init() {}

    public var primaryDisplayHeight: Double { CGDisplayBounds(CGMainDisplayID()).height }

    public func globalRect(for source: CaptureSource) -> CGRect? {
        let rect = CoordinateMath.resolve(
            source,
            displayBounds: { id in
                let b = CGDisplayBounds(CGDirectDisplayID(id))
                return b.width > 0 && b.height > 0 ? b : nil
            },
            windowBounds: { id in Self.windowBounds(CGWindowID(id)) }
        )
        guard let rect, rect.width > 0, rect.height > 0 else { return nil }
        return rect
    }

    static func windowBounds(_ id: CGWindowID) -> CGRect? {
        guard let list = CGWindowListCopyWindowInfo([.optionIncludingWindow], id) as? [[String: Any]],
              let info = list.first,
              let dict = info[kCGWindowBounds as String] as? NSDictionary
        else { return nil }
        return CGRect(dictionaryRepresentation: dict as CFDictionary)
    }
}
