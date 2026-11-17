import CoreGraphics
import Foundation

// MARK: - Conventions
//
// * All times are `MediaTime`: Double seconds from recording start.
// * All positions are normalized 0...1 in SOURCE space (the recorded screen),
//   origin TOP-LEFT, x to the right, y downward. (CoreGraphics global display
//   coordinates are bottom-left in AppKit but top-left in CGEvent; capture and
//   input code must convert to this convention before it reaches NSSCore types.)
// * Motion is a pure function of time: no wall-clock state.

/// Seconds from recording start.
public typealias MediaTime = Double

/// A point in normalized source space: 0...1, origin top-left.
public struct NormalizedPoint: Codable, Sendable, Equatable, Hashable {
    public var x: Double
    public var y: Double
    public init(x: Double, y: Double) {
        self.x = x
        self.y = y
    }
    public static let center = NormalizedPoint(x: 0.5, y: 0.5)
    public static let zero = NormalizedPoint(x: 0, y: 0)

    public var clamped: NormalizedPoint {
        NormalizedPoint(x: min(max(x, 0), 1), y: min(max(y, 0), 1))
    }
}

/// A size in normalized units (fraction of the container).
public struct NormalizedSize: Codable, Sendable, Equatable, Hashable {
    public var width: Double
    public var height: Double
    public init(width: Double, height: Double) {
        self.width = width
        self.height = height
    }
}

/// RGBA color, components 0...1, sRGB.
public struct RGBAColor: Codable, Sendable, Equatable, Hashable {
    public var r: Double
    public var g: Double
    public var b: Double
    public var a: Double
    public init(r: Double, g: Double, b: Double, a: Double = 1) {
        self.r = r
        self.g = g
        self.b = b
        self.a = a
    }
    public static let black = RGBAColor(r: 0, g: 0, b: 0)
    public static let white = RGBAColor(r: 1, g: 1, b: 1)
    public static let clear = RGBAColor(r: 0, g: 0, b: 0, a: 0)
}

/// Integer pixel size.
public struct PixelSize: Codable, Sendable, Equatable, Hashable {
    public var width: Int
    public var height: Int
    public init(width: Int, height: Int) {
        self.width = width
        self.height = height
    }
    public var aspect: Double { height == 0 ? 1 : Double(width) / Double(height) }
}
