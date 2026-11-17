import Foundation

public struct GradientStop: Codable, Sendable, Equatable {
    public var location: Double
    public var color: RGBAColor
    public init(location: Double, color: RGBAColor) {
        self.location = location
        self.color = color
    }
}

public enum Background: Codable, Sendable, Equatable {
    case solid(RGBAColor)
    /// Linear gradient; `angleDegrees` 0 = left->right, 90 = top->bottom.
    case gradient(stops: [GradientStop], angleDegrees: Double)
    /// Image file inside the project bundle (relative name) or an absolute path.
    case image(file: String)
    /// Legacy (schema v1) built-in procedural wallpaper identified by name. Schema v2 projects store
    /// `.mesh` instead; the migrator converts these, and the renderer still draws them via
    /// `BackgroundPresets.legacyWallpaper(named:)`.
    case wallpaper(name: String)
    /// Soft multi-colour "mesh" gradient: up to 8 colour points blended smoothly in OKLab,
    /// with a gentle sine warp (`warp` 0 = none, ~0.06 = soft).
    case mesh(points: [MeshPoint], warp: Double)
    /// A macOS desktop picture, by absolute path (for example `/System/Library/Desktop Pictures/Sonoma.heic`).
    /// Drawn exactly like `.image`; kept separate so the picker knows which tab it came from.
    case systemWallpaper(path: String)

    /// True for backgrounds drawn from an image file (the ones `Style.backgroundBlur` applies to).
    public var isImageBased: Bool {
        switch self {
        case .image, .systemWallpaper: true
        default: false
        }
    }
}

/// One colour point of a `.mesh` background. Position is normalized output space (0...1, origin top-left).
public struct MeshPoint: Codable, Sendable, Equatable, Hashable {
    public var x: Double
    public var y: Double
    public var color: RGBAColor
    public init(x: Double, y: Double, color: RGBAColor) {
        self.x = x
        self.y = y
        self.color = color
    }
}

public struct Shadow: Codable, Sendable, Equatable {
    /// Blur radius as a fraction of output height (0...~0.1).
    public var radius: Double
    public var opacity: Double
    /// Offset as a fraction of output height.
    public var offsetX: Double
    public var offsetY: Double
    public init(radius: Double = 0.03, opacity: Double = 0.4, offsetX: Double = 0, offsetY: Double = 0.012) {
        self.radius = radius
        self.opacity = opacity
        self.offsetX = offsetX
        self.offsetY = offsetY
    }
}

public enum AspectRatio: String, Codable, Sendable, Equatable, CaseIterable {
    case r16x9, r1x1, r9x16

    /// width / height
    public var ratio: Double {
        switch self {
        case .r16x9: 16.0 / 9.0
        case .r1x1: 1
        case .r9x16: 9.0 / 16.0
        }
    }

    /// Output pixel size; the shorter side equals `resolution.shortSide`.
    /// 16:9 -> 1920x1080 / 3840x2160; 1:1 -> 1080x1080 / 2160x2160; 9:16 -> 1080x1920 / 2160x3840.
    public func outputSize(for resolution: ExportResolution) -> PixelSize {
        let short = resolution.shortSide
        switch self {
        case .r16x9: return PixelSize(width: short * 16 / 9, height: short)
        case .r1x1: return PixelSize(width: short, height: short)
        case .r9x16: return PixelSize(width: short, height: short * 16 / 9)
        }
    }
}

public struct CursorStyle: Codable, Sendable, Equatable {
    /// Multiplier on the default cursor size (1 = native size).
    public var size: Double
    /// 0 = raw cursor, 1 = heavy smoothing.
    public var smoothing: Double
    /// 0 = none, 1 = strong.
    public var motionBlur: Double
    public var clickRipples: Bool
    public var hideWhenIdle: Bool
    public init(size: Double = 1.5, smoothing: Double = 0.5, motionBlur: Double = 0.4, clickRipples: Bool = true, hideWhenIdle: Bool = false) {
        self.size = size
        self.smoothing = smoothing
        self.motionBlur = motionBlur
        self.clickRipples = clickRipples
        self.hideWhenIdle = hideWhenIdle
    }
}

/// Bubble silhouette. Aspect ratios live in `CameraBubbleGeometry` (one place).
public enum CameraShape: String, Codable, Sendable, Equatable, CaseIterable {
    case circle, square, tall, wide

    public init(from decoder: Decoder) throws {
        self = try LegacyCameraShape(from: decoder).shape
    }
}

/// Decodes the new string form and the Round 1 forms (`circle`, `roundedRect(radius)`, `squircle`, `rectangle`).
struct LegacyCameraShape: Decodable {
    var shape: CameraShape
    /// Roundness implied by a legacy shape; nil for new-format values.
    var roundness: Double?

    private enum Keys: String, CodingKey { case circle, roundedRect, squircle, rectangle }
    private enum RectKeys: String, CodingKey { case radius }

    init(from decoder: Decoder) throws {
        if let raw = try? decoder.singleValueContainer().decode(String.self) {
            switch raw {
            case "circle": shape = .circle
            case "square": shape = .square
            case "tall": shape = .tall
            case "wide": shape = .wide
            case "squircle": shape = .square; roundness = 0.6
            case "rectangle": shape = .wide; roundness = 0
            case "roundedRect": shape = .square; roundness = 0.4
            default: shape = .circle
            }
            return
        }
        let c = try decoder.container(keyedBy: Keys.self)
        if c.contains(.roundedRect) {
            let n = try c.nestedContainer(keyedBy: RectKeys.self, forKey: .roundedRect)
            let r = try n.decodeIfPresent(Double.self, forKey: .radius) ?? 0.2
            shape = .square
            roundness = min(max(r / 0.5, 0), 1)
        } else if c.contains(.squircle) {
            shape = .square; roundness = 0.6
        } else if c.contains(.rectangle) {
            shape = .wide; roundness = 0
        } else {
            shape = .circle
        }
    }
}

/// Single source of truth for bubble dimensions, shared by the recording overlay, renderer and inspector.
public enum CameraBubbleGeometry {
    /// Width / height of each shape.
    public static func aspect(for shape: CameraShape) -> Double {
        switch shape {
        case .circle, .square: 1
        case .tall: 3.0 / 4.0
        case .wide: 16.0 / 9.0
        }
    }

    /// Camera scale at (and above) which the bubble reaches `zoomedScale`.
    public static let referenceZoom = 2.0

    public struct Layout: Sendable, Equatable {
        public var width: Double
        public var height: Double
        public var cornerRadius: Double
    }

    /// Dimensions and corner radius for a bubble of `width` (in whatever unit the caller uses).
    /// Radius = roundness * shorter side / 2; circle is always fully round.
    public static func layout(shape: CameraShape, roundness: Double, width: Double) -> Layout {
        let w = max(0, width)
        let h = w / aspect(for: shape)
        let shorter = min(w, h)
        let r = shape == .circle ? 1 : min(max(roundness, 0), 1)
        return Layout(width: w, height: h, cornerRadius: r * shorter / 2)
    }

    public static func layout(_ bubble: CameraBubble, width: Double) -> Layout {
        layout(shape: bubble.shape, roundness: bubble.roundness, width: width)
    }

    /// Size multiplier for the bubble: 1 at camera scale <= 1, `zoomedScale` at `referenceZoom` or more (smoothstep).
    public static func sizeMultiplier(cameraScale: Double, zoomedScale: Double) -> Double {
        let t = min(max((cameraScale - 1) / (referenceZoom - 1), 0), 1)
        let s = t * t * (3 - 2 * t)
        return 1 + (zoomedScale - 1) * s
    }
}

public struct CameraBorder: Codable, Sendable, Equatable {
    /// Width as a fraction of output height.
    public var width: Double
    public var color: RGBAColor
    public init(width: Double = 0.004, color: RGBAColor = .white) {
        self.width = width
        self.color = color
    }
}

public struct CameraBubble: Codable, Sendable, Equatable {
    public var visible: Bool
    public var shape: CameraShape
    /// Center of the bubble in normalized OUTPUT space (0...1, origin top-left).
    public var position: NormalizedPoint
    /// Bubble width as a fraction of output width; height follows `CameraBubbleGeometry.aspect(for: shape)`.
    public var size: Double
    public var border: CameraBorder?
    public var mirror: Bool
    /// 0 = sharp corners, 1 = fully round (circle for square, pill for tall/wide). Ignored for `.circle`.
    public var roundness: Double
    /// Size multiplier when the view is fully zoomed in (1 = no shrink).
    public var zoomedScale: Double
    public init(
        visible: Bool = true,
        shape: CameraShape = .circle,
        position: NormalizedPoint = NormalizedPoint(x: 0.88, y: 0.82),
        size: Double = 0.18,
        border: CameraBorder? = CameraBorder(),
        mirror: Bool = true,
        roundness: Double = 0.3,
        zoomedScale: Double = 0.6
    ) {
        self.visible = visible
        self.shape = shape
        self.position = position
        self.size = size
        self.border = border
        self.mirror = mirror
        self.roundness = roundness
        self.zoomedScale = zoomedScale
    }

    /// Size multiplier for the current camera zoom; shared by preview and export.
    public func sizeMultiplier(cameraScale: Double) -> Double {
        CameraBubbleGeometry.sizeMultiplier(cameraScale: cameraScale, zoomedScale: zoomedScale)
    }
}

/// Free-function form of `CameraBubbleGeometry.sizeMultiplier` using the default `zoomedScale` or a given one.
public func bubbleSizeMultiplier(cameraScale: Double, zoomedScale: Double = 0.6) -> Double {
    CameraBubbleGeometry.sizeMultiplier(cameraScale: cameraScale, zoomedScale: zoomedScale)
}

public struct Style: Codable, Sendable, Equatable {
    public var background: Background
    /// Padding around the screen content as a fraction of output height (0...~0.2).
    public var padding: Double
    /// Corner radius of the screen content as a fraction of output height.
    public var cornerRadius: Double
    public var shadow: Shadow
    public var aspectRatio: AspectRatio
    public var cursor: CursorStyle
    public var camera: CameraBubble
    /// Blur of image backgrounds (`.image`, `.systemWallpaper`), 0 = sharp ... 1 = strongest.
    /// Ignored for colours and gradients. See `BackgroundBlur` for the radius rule.
    public var backgroundBlur: Double

    public init(
        background: Background = .gradient(
            stops: [
                GradientStop(location: 0, color: RGBAColor(r: 0.36, g: 0.30, b: 0.85)),
                GradientStop(location: 1, color: RGBAColor(r: 0.93, g: 0.42, b: 0.55)),
            ],
            angleDegrees: 45
        ),
        padding: Double = 0.06,
        cornerRadius: Double = 0.018,
        shadow: Shadow = Shadow(),
        aspectRatio: AspectRatio = .r16x9,
        cursor: CursorStyle = CursorStyle(),
        camera: CameraBubble = CameraBubble(),
        backgroundBlur: Double = 0
    ) {
        self.background = background
        self.padding = padding
        self.cornerRadius = cornerRadius
        self.shadow = shadow
        self.aspectRatio = aspectRatio
        self.cursor = cursor
        self.camera = camera
        self.backgroundBlur = backgroundBlur
    }

    /// A representative flat color for the background (first gradient stop / solid color / mid-gray for images).
    public var backgroundFallbackColor: RGBAColor {
        switch background {
        case .solid(let c): c
        case .gradient(let stops, _): stops.first?.color ?? .black
        case .mesh(let points, _): points.first?.color ?? .black
        case .wallpaper(let name): BackgroundPresets.legacyWallpaper(named: name).points.first?.color ?? .black
        case .image, .systemWallpaper: RGBAColor(r: 0.5, g: 0.5, b: 0.5)
        }
    }
}

extension Style {
    /// A copy whose bundle-relative `.image(file:)` is an absolute path inside `bundle` (the `.nssproj` folder).
    /// The preview and the exporter both render this, so a picked image (copied into the bundle) resolves the
    /// same way in both, without any per-renderer base URL.
    public func resolvingAssets(relativeTo bundle: URL) -> Style {
        guard case .image(let file) = background, !file.isEmpty, !file.hasPrefix("/") else { return self }
        var s = self
        s.background = .image(file: bundle.appendingPathComponent(file).path)
        return s
    }
}

/// The blur rule shared by the renderer and the UI: one place, so preview and export agree.
public enum BackgroundBlur {
    /// Gaussian sigma at `backgroundBlur == 1`, as a fraction of the image's shorter side.
    /// It is relative to the image (not the output), so the same blur looks the same at every output size.
    public static let maxSigmaFraction = 0.035

    /// Sigma in image pixels for an image whose shorter side is `shortSide` pixels.
    public static func sigma(amount: Double, shortSide: Int) -> Double {
        guard amount.isFinite, amount > 0 else { return 0 }
        return min(amount, 1) * maxSigmaFraction * Double(max(shortSide, 0))
    }

    /// The amount rounded to 1/100, so a slider drag reuses cached blurs and the cache key is stable.
    public static func quantized(_ amount: Double) -> Double {
        guard amount.isFinite else { return 0 }
        return (min(max(amount, 0), 1) * 100).rounded() / 100
    }
}

// Forward compatibility: fields added after v1 decode with defaults when absent (see PLAN.md).
extension Style {
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        let d = Style()
        background = try c.decodeIfPresent(Background.self, forKey: .background) ?? d.background
        padding = try c.decodeIfPresent(Double.self, forKey: .padding) ?? d.padding
        cornerRadius = try c.decodeIfPresent(Double.self, forKey: .cornerRadius) ?? d.cornerRadius
        shadow = try c.decodeIfPresent(Shadow.self, forKey: .shadow) ?? d.shadow
        aspectRatio = try c.decodeIfPresent(AspectRatio.self, forKey: .aspectRatio) ?? d.aspectRatio
        cursor = try c.decodeIfPresent(CursorStyle.self, forKey: .cursor) ?? d.cursor
        camera = try c.decodeIfPresent(CameraBubble.self, forKey: .camera) ?? d.camera
        backgroundBlur = try c.decodeIfPresent(Double.self, forKey: .backgroundBlur) ?? d.backgroundBlur
    }
}

extension CursorStyle {
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        let d = CursorStyle()
        size = try c.decodeIfPresent(Double.self, forKey: .size) ?? d.size
        smoothing = try c.decodeIfPresent(Double.self, forKey: .smoothing) ?? d.smoothing
        motionBlur = try c.decodeIfPresent(Double.self, forKey: .motionBlur) ?? d.motionBlur
        clickRipples = try c.decodeIfPresent(Bool.self, forKey: .clickRipples) ?? d.clickRipples
        hideWhenIdle = try c.decodeIfPresent(Bool.self, forKey: .hideWhenIdle) ?? d.hideWhenIdle
    }
}

extension CameraBubble {
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        let d = CameraBubble()
        visible = try c.decodeIfPresent(Bool.self, forKey: .visible) ?? d.visible
        let legacy = try c.decodeIfPresent(LegacyCameraShape.self, forKey: .shape)
        shape = legacy?.shape ?? d.shape
        roundness = try c.decodeIfPresent(Double.self, forKey: .roundness) ?? legacy?.roundness ?? d.roundness
        zoomedScale = try c.decodeIfPresent(Double.self, forKey: .zoomedScale) ?? d.zoomedScale
        position = try c.decodeIfPresent(NormalizedPoint.self, forKey: .position) ?? d.position
        size = try c.decodeIfPresent(Double.self, forKey: .size) ?? d.size
        // v1 field, optional: a missing key means nil (border off), not the default.
        border = try c.decodeIfPresent(CameraBorder.self, forKey: .border)
        mirror = try c.decodeIfPresent(Bool.self, forKey: .mirror) ?? d.mirror
    }
}
