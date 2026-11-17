import Foundation

/// A `.nssproj` bundle's `project.json`.
public struct Project: Codable, Sendable, Equatable, Identifiable {
    /// v2 (Round 3): `.mesh` and `.systemWallpaper` backgrounds and `Style.backgroundBlur`;
    /// v1 `.wallpaper(name:)` backgrounds migrate to `.mesh` (see `ProjectMigrator`).
    public static let currentSchemaVersion = 2

    public var id: UUID
    public var name: String
    public var createdAt: Date
    public var modifiedAt: Date
    public var assets: RecordingAssets
    public var timeline: Timeline
    public var style: Style
    public var schemaVersion: Int

    public init(
        id: UUID = UUID(),
        name: String,
        createdAt: Date = Date(),
        modifiedAt: Date = Date(),
        assets: RecordingAssets,
        timeline: Timeline = Timeline(),
        style: Style = Style(),
        schemaVersion: Int = Project.currentSchemaVersion
    ) {
        self.id = id
        self.name = name
        self.createdAt = createdAt
        self.modifiedAt = modifiedAt
        self.assets = assets
        self.timeline = timeline
        self.style = style
        self.schemaVersion = schemaVersion
    }
}

/// Reference to a project bundle on disk (a `.nssproj` directory).
public struct ProjectReference: Codable, Sendable, Equatable, Hashable {
    public var url: URL
    public init(url: URL) { self.url = url }
    public static let fileExtension = "nssproj"
    public static let projectFileName = "project.json"
}

public enum ExportResolution: String, Codable, Sendable, Equatable, CaseIterable {
    case p1080, p2160
    /// Shorter side in pixels (1080 or 2160).
    public var shortSide: Int { self == .p1080 ? 1080 : 2160 }
}

public enum VideoCodec: String, Codable, Sendable, Equatable, CaseIterable {
    case h264, hevc
}

public struct ExportPreset: Codable, Sendable, Equatable {
    public var resolution: ExportResolution
    public var fps: Int
    public var codec: VideoCodec
    public init(resolution: ExportResolution = .p1080, fps: Int = 60, codec: VideoCodec = .h264) {
        self.resolution = resolution
        self.fps = fps
        self.codec = codec
    }
    public func outputSize(for aspect: AspectRatio) -> PixelSize { aspect.outputSize(for: resolution) }
}

public enum ExportProgress: Sendable, Equatable {
    case idle
    case running(fraction: Double, framesWritten: Int, totalFrames: Int)
    case finished(URL)
    case failed(message: String)
    case cancelled
}

// Forward compatibility: fields added after v1 decode with defaults when absent (see PLAN.md).
extension Project {
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decodeIfPresent(UUID.self, forKey: .id) ?? UUID()
        name = try c.decode(String.self, forKey: .name)
        createdAt = try c.decodeIfPresent(Date.self, forKey: .createdAt) ?? Date()
        modifiedAt = try c.decodeIfPresent(Date.self, forKey: .modifiedAt) ?? createdAt
        assets = try c.decode(RecordingAssets.self, forKey: .assets)
        timeline = try c.decodeIfPresent(Timeline.self, forKey: .timeline) ?? Timeline()
        style = try c.decodeIfPresent(Style.self, forKey: .style) ?? Style()
        schemaVersion = try c.decodeIfPresent(Int.self, forKey: .schemaVersion) ?? Project.currentSchemaVersion
    }
}
