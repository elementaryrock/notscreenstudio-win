import Foundation

/// A named, ready-made background shown in the picker.
public struct BackgroundPreset: Sendable, Equatable, Identifiable {
    public var id: String
    public var name: String
    public var background: Background
    public init(id: String, name: String, background: Background) {
        self.id = id
        self.name = name
        self.background = background
    }
}

/// Built-in gradients and colours. Pure data: the renderer draws them from the stored `Background`
/// value, so a project keeps its look even if these presets change later.
public enum BackgroundPresets {
    private static func hex(_ v: UInt32) -> RGBAColor {
        RGBAColor(r: Double((v >> 16) & 0xFF) / 255, g: Double((v >> 8) & 0xFF) / 255, b: Double(v & 0xFF) / 255)
    }

    private static func linear(_ id: String, _ name: String, _ angle: Double, _ colors: [UInt32]) -> BackgroundPreset {
        let n = Double(max(colors.count - 1, 1))
        let stops = colors.enumerated().map { GradientStop(location: Double($0.offset) / n, color: hex($0.element)) }
        return BackgroundPreset(id: "linear." + id, name: name, background: .gradient(stops: stops, angleDegrees: angle))
    }

    private static func mesh(_ id: String, _ name: String, _ mesh: (points: [MeshPoint], warp: Double)) -> BackgroundPreset {
        BackgroundPreset(id: "mesh." + id, name: name, background: .mesh(points: mesh.points, warp: mesh.warp))
    }

    private static func p(_ x: Double, _ y: Double, _ r: Double, _ g: Double, _ b: Double) -> MeshPoint {
        MeshPoint(x: x, y: y, color: RGBAColor(r: r, g: g, b: b))
    }

    /// Smooth linear gradients first, then soft multi-colour mesh gradients.
    public static let gradients: [BackgroundPreset] = [
        linear("dusk", "Dusk", 45, [0x5C4DD9, 0xED6B8C]),
        linear("sky", "Sky", 90, [0x4FACFE, 0x00C6FB]),
        linear("apricot", "Apricot", 135, [0xF6D365, 0xFDA085]),
        linear("mint", "Mint", 45, [0x43E97B, 0x38F9D7]),
        linear("lavender", "Lavender", 90, [0xA18CD1, 0xFBC2EB]),
        linear("ember", "Ember", 45, [0xF83600, 0xF9D423]),
        linear("deepsea", "Deep Sea", 90, [0x0F2027, 0x203A43, 0x2C5364]),
        linear("graphite", "Graphite", 90, [0x3A3D40, 0x181A1B]),
        mesh("aurora", "Aurora", legacyWallpaper(named: "aurora")),
        mesh("sunset", "Sunset", legacyWallpaper(named: "sunset")),
        mesh("ocean", "Ocean", legacyWallpaper(named: "ocean")),
        mesh("forest", "Forest", legacyWallpaper(named: "forest")),
        mesh("midnight", "Midnight", legacyWallpaper(named: "midnight")),
        mesh("peach", "Peach", legacyWallpaper(named: "peach")),
        mesh("candy", "Candy", ([
            p(0.0, 0.0, 0.99, 0.60, 0.80), p(1.0, 0.1, 0.60, 0.70, 1.00),
            p(0.2, 1.0, 1.00, 0.85, 0.60), p(0.9, 0.9, 0.75, 0.55, 0.95), p(0.5, 0.45, 1.00, 0.75, 0.85),
        ], 0.05)),
        mesh("lagoon", "Lagoon", ([
            p(0.0, 0.2, 0.02, 0.18, 0.35), p(0.7, 0.0, 0.00, 0.55, 0.65),
            p(1.0, 0.8, 0.10, 0.80, 0.70), p(0.25, 0.95, 0.05, 0.35, 0.55),
        ], 0.06)),
    ]

    /// Solid colour swatches (the picker adds a free `ColorPicker`).
    public static let colors: [BackgroundPreset] = [
        ("white", "White", 0xFFFFFF), ("silver", "Silver", 0xD9DCE1), ("slate", "Slate", 0x5B6270),
        ("charcoal", "Charcoal", 0x26282C), ("black", "Black", 0x000000), ("red", "Red", 0xE5484D),
        ("orange", "Orange", 0xF76B15), ("amber", "Amber", 0xFFC53D), ("green", "Green", 0x30A46C),
        ("teal", "Teal", 0x12A594), ("sky", "Sky", 0x0090FF), ("indigo", "Indigo", 0x3E63DD),
        ("violet", "Violet", 0x6E56CF), ("pink", "Pink", 0xD6409F), ("sand", "Sand", 0xE9DFCF),
        ("navy", "Navy", 0x14213D),
    ].map { BackgroundPreset(id: "solid." + $0.0, name: $0.1, background: .solid(hex($0.2))) }

    /// Names the v1 `.wallpaper(name:)` case understood (case-insensitive). Unknown names mean "aurora".
    public static let legacyWallpaperNames = ["aurora", "sunset", "ocean", "forest", "midnight", "peach"]

    /// The colour points of a v1 procedural wallpaper. Unknown names fall back to "aurora", as v1 did.
    public static func legacyWallpaper(named name: String) -> (points: [MeshPoint], warp: Double) {
        switch name.lowercased() {
        case "sunset":
            return ([p(0.1, 0.1, 0.98, 0.45, 0.35), p(0.9, 0.2, 0.99, 0.75, 0.40), p(0.2, 0.9, 0.60, 0.20, 0.55), p(0.85, 0.85, 0.95, 0.35, 0.50)], 0.05)
        case "ocean":
            return ([p(0.0, 0.0, 0.05, 0.25, 0.55), p(1.0, 0.1, 0.10, 0.60, 0.80), p(0.3, 1.0, 0.15, 0.75, 0.70), p(0.9, 0.9, 0.05, 0.35, 0.65)], 0.04)
        case "forest":
            return ([p(0.1, 0.2, 0.10, 0.35, 0.25), p(0.8, 0.1, 0.45, 0.70, 0.40), p(0.5, 0.9, 0.15, 0.50, 0.35), p(0.95, 0.75, 0.75, 0.85, 0.50)], 0.05)
        case "midnight":
            return ([p(0.0, 0.0, 0.05, 0.05, 0.15), p(0.7, 0.3, 0.20, 0.15, 0.45), p(0.2, 0.8, 0.08, 0.10, 0.30), p(1.0, 1.0, 0.35, 0.20, 0.55)], 0.03)
        case "peach":
            return ([p(0.1, 0.1, 1.00, 0.85, 0.75), p(0.9, 0.3, 0.98, 0.70, 0.65), p(0.3, 0.9, 0.95, 0.80, 0.90), p(0.95, 0.95, 1.00, 0.90, 0.70)], 0.05)
        default:
            return ([
                p(0.0, 0.1, 0.20, 0.15, 0.55), p(0.6, 0.0, 0.25, 0.70, 0.85), p(1.0, 0.5, 0.55, 0.30, 0.85),
                p(0.3, 1.0, 0.20, 0.80, 0.60), p(0.85, 0.95, 0.90, 0.45, 0.70),
            ], 0.06)
        }
    }

    /// The v2 equivalent of a v1 `.wallpaper(name:)`: the same colour points as a `.mesh`.
    public static func migratedWallpaper(named name: String) -> Background {
        let w = legacyWallpaper(named: name)
        return .mesh(points: w.points, warp: w.warp)
    }
}
