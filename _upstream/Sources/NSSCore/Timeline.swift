import Foundation

/// Visible part of the recording. `end == nil` means "until the end of the recording".
public struct TrimRange: Codable, Sendable, Equatable {
    public var start: MediaTime
    public var end: MediaTime?
    public init(start: MediaTime = 0, end: MediaTime? = nil) {
        self.start = start
        self.end = end
    }
    public func resolvedEnd(duration: MediaTime) -> MediaTime { min(end ?? duration, duration) }
    public func length(duration: MediaTime) -> MediaTime { max(0, resolvedEnd(duration: duration) - start) }
}

public enum ZoomFocus: Codable, Sendable, Equatable {
    /// Follow the cursor / click targets automatically.
    case auto
    /// Fixed focus point (normalized source space).
    case manual(NormalizedPoint)
}

public struct ZoomBlock: Codable, Sendable, Equatable, Identifiable {
    /// Ease duration (seconds) used when a block does not say otherwise. Matches the Round 2 zoom spring.
    public static let defaultEase: MediaTime = 0.75

    public var id: UUID
    public var start: MediaTime
    public var end: MediaTime
    /// 1 = no zoom, 2 = 2x magnification.
    public var scale: Double
    /// `.auto` follows the cursor; `.manual(p)` holds a fixed point.
    public var focus: ZoomFocus
    /// True for blocks generated from clicks ("Regenerate auto zooms" replaces only these). Editing a
    /// block by hand makes it manual.
    public var isAuto: Bool
    /// How long the zoom takes to arrive (spring response, seconds), starting at `start`.
    public var easeIn: MediaTime
    /// How long the zoom takes to return to 1x (spring response, seconds), starting at `end`.
    public var easeOut: MediaTime

    public init(
        id: UUID = UUID(), start: MediaTime, end: MediaTime, scale: Double = 2, focus: ZoomFocus = .auto,
        isAuto: Bool = false, easeIn: MediaTime = ZoomBlock.defaultEase, easeOut: MediaTime = ZoomBlock.defaultEase
    ) {
        self.id = id
        self.start = start
        self.end = end
        self.scale = scale
        self.focus = focus
        self.isAuto = isAuto
        self.easeIn = easeIn
        self.easeOut = easeOut
    }
    public func contains(_ t: MediaTime) -> Bool { t >= start && t < end }
}

public struct Timeline: Codable, Sendable, Equatable {
    public var trim: TrimRange
    public var zoomBlocks: [ZoomBlock]
    /// Legacy flag. When true, the motion model derives auto-zooms from clicks on the fly, in addition to
    /// `zoomBlocks`. The editor materializes those auto-zooms into `zoomBlocks` (with `isAuto`) when a
    /// project is first opened and then clears this flag, so the stored blocks are the single source of truth.
    public var autoZoomEnabled: Bool

    public init(trim: TrimRange = TrimRange(), zoomBlocks: [ZoomBlock] = [], autoZoomEnabled: Bool = true) {
        self.trim = trim
        self.zoomBlocks = zoomBlocks
        self.autoZoomEnabled = autoZoomEnabled
    }
}

// Forward compatibility: fields added after v1 decode with defaults when absent (see PLAN.md).
extension Timeline {
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        let d = Timeline()
        trim = try c.decodeIfPresent(TrimRange.self, forKey: .trim) ?? d.trim
        zoomBlocks = try c.decodeIfPresent([ZoomBlock].self, forKey: .zoomBlocks) ?? d.zoomBlocks
        autoZoomEnabled = try c.decodeIfPresent(Bool.self, forKey: .autoZoomEnabled) ?? d.autoZoomEnabled
    }
}

extension ZoomBlock {
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decodeIfPresent(UUID.self, forKey: .id) ?? UUID()
        start = try c.decode(MediaTime.self, forKey: .start)
        end = try c.decode(MediaTime.self, forKey: .end)
        scale = try c.decodeIfPresent(Double.self, forKey: .scale) ?? 2
        focus = try c.decodeIfPresent(ZoomFocus.self, forKey: .focus) ?? .auto
        isAuto = try c.decodeIfPresent(Bool.self, forKey: .isAuto) ?? false
        easeIn = try c.decodeIfPresent(MediaTime.self, forKey: .easeIn) ?? ZoomBlock.defaultEase
        easeOut = try c.decodeIfPresent(MediaTime.self, forKey: .easeOut) ?? ZoomBlock.defaultEase
    }
}
