import Foundation
import NSSCore

/// Why a project could not be migrated. Typed so callers never match on message text.
public enum ProjectMigrationError: Error, Sendable, Equatable, CustomStringConvertible {
    case newerSchema(found: Int, supported: Int)
    case noMigration(from: Int)

    public var description: String {
        switch self {
        case .newerSchema(let found, let supported):
            return "Project was saved by a newer version (schema \(found), this app reads \(supported))"
        case .noMigration(let from):
            return "No migration from schema \(from)"
        }
    }
}

/// Upgrades an old `project.json` to the current schema before it is decoded.
/// A migration step turns version N JSON into version N+1 JSON. Steps are registered by their source version.
public struct ProjectMigrator: Sendable {
    public typealias Step = @Sendable (inout [String: Any]) throws -> Void

    public let currentVersion: Int
    private let steps: [Int: Step]

    /// v1 -> v2: a `.wallpaper(name:)` background becomes the `.mesh` with the same colour points.
    /// Add the next step as `2: { json in ... }`.
    public static let standard = ProjectMigrator(currentVersion: Project.currentSchemaVersion, steps: [
        1: migrateV1Wallpaper,
    ])

    /// Rewrites `style.background` = `{"wallpaper": {"name": n}}` to the equivalent `{"mesh": ...}`.
    /// Anything else is left alone (decoding fills defaults for the new v2 fields).
    @Sendable static func migrateV1Wallpaper(_ json: inout [String: Any]) throws {
        guard var style = json["style"] as? [String: Any],
            let background = style["background"] as? [String: Any],
            let wallpaper = background["wallpaper"] as? [String: Any]
        else { return }
        let name = wallpaper["name"] as? String ?? ""
        let data = try JSONEncoder().encode(BackgroundPresets.migratedWallpaper(named: name))
        style["background"] = try JSONSerialization.jsonObject(with: data)
        json["style"] = style
    }

    public init(currentVersion: Int, steps: [Int: Step]) {
        self.currentVersion = currentVersion
        self.steps = steps
    }

    /// Returns JSON at `currentVersion`, or `data` untouched when it is already current.
    public func migrate(_ data: Data) throws -> Data {
        guard var json = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else {
            throw NSSError.invalidProject("project.json is not valid JSON")
        }
        // A missing version means v1, matching `Project`'s decoder.
        var version = (json["schemaVersion"] as? Int) ?? 1
        if version == currentVersion { return data }
        guard version < currentVersion else {
            throw ProjectMigrationError.newerSchema(found: version, supported: currentVersion)
        }
        while version < currentVersion {
            guard let step = steps[version] else {
                throw ProjectMigrationError.noMigration(from: version)
            }
            try step(&json)
            version += 1
            json["schemaVersion"] = version
        }
        return try JSONSerialization.data(withJSONObject: json, options: [.sortedKeys])
    }
}
