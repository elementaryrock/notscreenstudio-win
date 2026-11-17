import SwiftUI
import NSSCore

/// Pure conversions between `Style` values and inspector control values.
public enum InspectorBindings {
    public static let wallpapers = ["Sunset", "Ocean", "Forest", "Graphite", "Aurora"]

    public enum BackgroundKind: String, CaseIterable, Identifiable, Sendable {
        case solid = "Solid", gradient = "Gradient", image = "Image", wallpaper = "Wallpaper"
        public var id: String { rawValue }
    }

    public static func kind(of b: Background) -> BackgroundKind {
        switch b {
        case .solid: .solid
        case .gradient, .mesh: .gradient
        case .image: .image
        case .wallpaper, .systemWallpaper: .wallpaper
        }
    }

    /// Default background when the user switches kind, reusing the current colors where possible.
    public static func background(switchingTo kind: BackgroundKind, from current: Background) -> Background {
        if self.kind(of: current) == kind { return current }
        let base = Style().backgroundFallbackColor
        let color: RGBAColor
        switch current {
        case .solid(let c): color = c
        case .gradient(let s, _): color = s.first?.color ?? base
        default: color = base
        }
        switch kind {
        case .solid: return .solid(color)
        case .gradient:
            return .gradient(stops: [GradientStop(location: 0, color: color), GradientStop(location: 1, color: RGBAColor(r: 0.1, g: 0.1, b: 0.2))], angleDegrees: 45)
        case .image: return .image(file: "")
        case .wallpaper: return .wallpaper(name: wallpapers[0])
        }
    }

    public enum ShapeKind: String, CaseIterable, Identifiable, Sendable {
        case circle = "Circle", square = "Square", tall = "Tall", wide = "Wide"
        public var id: String { rawValue }
    }

    /// Range of the "shrink when zoomed" slider (1 = bubble keeps its size when the view zooms in).
    public static let zoomedScaleRange: ClosedRange<Double> = 0.3...1

    public static func kind(of s: CameraShape) -> ShapeKind {
        switch s {
        case .circle: .circle
        case .square: .square
        case .tall: .tall
        case .wide: .wide
        }
    }

    public static func shape(_ k: ShapeKind) -> CameraShape {
        switch k {
        case .circle: .circle
        case .square: .square
        case .tall: .tall
        case .wide: .wide
        }
    }

    public static func color(_ c: RGBAColor) -> Color { Color(.sRGB, red: c.r, green: c.g, blue: c.b, opacity: c.a) }

    public static func rgba(_ color: Color) -> RGBAColor {
        let ns = NSColor(color).usingColorSpace(.sRGB) ?? NSColor.black
        return RGBAColor(r: Double(ns.redComponent), g: Double(ns.greenComponent), b: Double(ns.blueComponent), a: Double(ns.alphaComponent))
    }

    public static func label(for a: AspectRatio) -> String {
        switch a {
        case .r16x9: "16:9"
        case .r1x1: "1:1"
        case .r9x16: "9:16"
        }
    }
}

extension EditorModel {
    /// A binding that reads `project.style[keyPath]` and writes through `setStyle` (undo + autosave + preview).
    func styleBinding<V: Equatable>(_ keyPath: WritableKeyPath<Style, V>, name: String = "Change Style") -> Binding<V> {
        Binding(get: { self.project.style[keyPath: keyPath] }, set: { self.setStyle(keyPath, $0, name: name) })
    }
}
