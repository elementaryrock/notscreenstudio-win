import Foundation

/// Virtual camera over the SOURCE screen. `scale` 1 = whole source visible;
/// `center` is the normalized source point shown at the center of the content area.
/// The motion model is responsible for clamping so the viewport never leaves 0...1.
public struct CameraState: Codable, Sendable, Equatable {
    public var scale: Double
    public var center: NormalizedPoint
    public init(scale: Double = 1, center: NormalizedPoint = .center) {
        self.scale = scale
        self.center = center
    }
    public static let identity = CameraState()
}

public struct RippleState: Codable, Sendable, Equatable {
    /// Normalized source position of the click.
    public var position: NormalizedPoint
    /// 0 = just clicked, 1 = fully faded.
    public var progress: Double
    public init(position: NormalizedPoint, progress: Double) {
        self.position = position
        self.progress = progress
    }
}

public struct CursorState: Codable, Sendable, Equatable {
    /// Smoothed position, normalized source space.
    public var position: NormalizedPoint
    /// Normalized source units per second (used for motion blur direction/length).
    public var velocity: NormalizedPoint
    public var visible: Bool
    public var ripples: [RippleState]
    /// 0...1 cursor opacity (idle fade). Multiply into the cursor's alpha; 1 = fully opaque.
    /// `visible == false` implies nothing is drawn regardless of opacity.
    public var opacity: Double
    /// Cursor image to draw.
    public var shape: CursorShape
    /// Scale of the cursor while the button is down (1 = normal, below 1 = squished).
    public var pressScale: Double
    public init(
        position: NormalizedPoint = .center,
        velocity: NormalizedPoint = .zero,
        visible: Bool = true,
        ripples: [RippleState] = [],
        opacity: Double = 1,
        shape: CursorShape = .arrow,
        pressScale: Double = 1
    ) {
        self.position = position
        self.velocity = velocity
        self.visible = visible
        self.ripples = ripples
        self.opacity = opacity
        self.shape = shape
        self.pressScale = pressScale
    }
}

// Forward compatibility: fields added after v1 decode with defaults when absent (see PLAN.md).
extension CursorState {
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        let d = CursorState()
        position = try c.decodeIfPresent(NormalizedPoint.self, forKey: .position) ?? d.position
        velocity = try c.decodeIfPresent(NormalizedPoint.self, forKey: .velocity) ?? d.velocity
        visible = try c.decodeIfPresent(Bool.self, forKey: .visible) ?? d.visible
        ripples = try c.decodeIfPresent([RippleState].self, forKey: .ripples) ?? d.ripples
        opacity = try c.decodeIfPresent(Double.self, forKey: .opacity) ?? d.opacity
        shape = try c.decodeIfPresent(CursorShape.self, forKey: .shape) ?? d.shape
        pressScale = try c.decodeIfPresent(Double.self, forKey: .pressScale) ?? d.pressScale
    }
}

/// Everything the renderer needs, besides pixels and Style, to draw one frame at `time`.
public struct FrameState: Codable, Sendable, Equatable {
    public var time: MediaTime
    public var camera: CameraState
    public var cursor: CursorState
    public init(time: MediaTime, camera: CameraState = .identity, cursor: CursorState = CursorState()) {
        self.time = time
        self.camera = camera
        self.cursor = cursor
    }
}
