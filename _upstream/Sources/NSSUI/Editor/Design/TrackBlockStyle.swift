import SwiftUI

/// The timeline tracks, top to bottom.
public enum TimelineTrackKind: String, CaseIterable, Identifiable, Sendable {
    case video, zoom, camera, audio
    public var id: String { rawValue }
    public var title: String {
        switch self {
        case .video: "Video"
        case .zoom: "Zoom"
        case .camera: "Camera"
        case .audio: "Audio"
        }
    }
    public var symbol: String {
        switch self {
        case .video: "film"
        case .zoom: "plus.magnifyingglass"
        case .camera: "video.fill"
        case .audio: "waveform"
        }
    }
    /// Block tint for this track.
    public var tint: Color {
        switch self {
        case .video: Color(red: 0.36, green: 0.52, blue: 0.95)
        case .zoom: Color(red: 0.62, green: 0.40, blue: 0.95)
        case .camera: Color(red: 0.20, green: 0.72, blue: 0.62)
        case .audio: Color(red: 0.95, green: 0.60, blue: 0.20)
        }
    }
}

/// Shared look and hit behaviour for every block on every timeline track.
public enum TrackBlockStyle {
    public static let cornerRadius: CGFloat = 6
    public static let trackHeight: CGFloat = 34
    public static let blockInset: CGFloat = 3
    public static var blockHeight: CGFloat { trackHeight - 2 * blockInset }
    /// Width of the invisible grab area on each end of a block.
    public static let edgeHitWidth: CGFloat = 8
    /// Blocks are never drawn narrower than this, so a very short block can still be grabbed.
    public static let minimumWidth: CGFloat = 6
    /// Pointer travel before a press on a block counts as a drag instead of a click.
    public static let dragThreshold: CGFloat = 3

    public enum HitZone: Equatable, Sendable { case startEdge, body, endEdge }

    /// Which part of a block a point at `x` (0...width, relative to the block) hits. Edges are
    /// `edgeHitWidth` wide but never take more than a third of a narrow block, so its body stays grabbable.
    public static func hitZone(atX x: Double, blockWidth: Double) -> HitZone {
        let edge = min(Double(edgeHitWidth), max(blockWidth, 0) / 3)
        if x < edge { return .startEdge }
        if x > blockWidth - edge { return .endEdge }
        return .body
    }

    /// Display width of a block spanning `rawWidth` points.
    public static func displayWidth(_ rawWidth: Double) -> Double { max(rawWidth, Double(minimumWidth)) }
}

/// The chrome of one block: tinted rounded rectangle, selection ring, optional edge grips.
public struct TrackBlockBackground: View {
    let kind: TimelineTrackKind
    let selected: Bool
    let showsGrips: Bool
    public init(kind: TimelineTrackKind, selected: Bool, showsGrips: Bool = false) {
        self.kind = kind
        self.selected = selected
        self.showsGrips = showsGrips
    }
    public var body: some View {
        RoundedRectangle(cornerRadius: TrackBlockStyle.cornerRadius)
            .fill(kind.tint.opacity(selected ? 0.95 : 0.6))
            .overlay(
                RoundedRectangle(cornerRadius: TrackBlockStyle.cornerRadius)
                    .strokeBorder(selected ? Color.white : kind.tint.opacity(0.9), lineWidth: selected ? 1.5 : 1)
            )
            .overlay {
                if showsGrips {
                    HStack {
                        Capsule().fill(.white.opacity(0.85)).frame(width: 2, height: 12)
                        Spacer(minLength: 0)
                        Capsule().fill(.white.opacity(0.85)).frame(width: 2, height: 12)
                    }
                    .padding(.horizontal, 3)
                }
            }
    }
}

/// The left-hand label for a track.
public struct TrackLabel: View {
    let kind: TimelineTrackKind
    public init(_ kind: TimelineTrackKind) { self.kind = kind }
    public var body: some View {
        HStack(spacing: 6) {
            Image(systemName: kind.symbol).font(.system(size: 10)).foregroundStyle(kind.tint).frame(width: 14)
            Text(kind.title).editorLabel(.caption)
            Spacer(minLength: 0)
        }
        .padding(.leading, 8)
    }
}

/// The rail a track's blocks sit on.
public struct TrackRail: View {
    public init() {}
    public var body: some View {
        RoundedRectangle(cornerRadius: EditorTheme.radiusSmall + 1).fill(Color.white.opacity(0.045))
    }
}
