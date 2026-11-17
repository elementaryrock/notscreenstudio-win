import Foundation
import Testing
import NSSCore
@testable import NSSProject

/// Schema v1 -> v2: legacy `.wallpaper(name:)` backgrounds become the equivalent `.mesh`.
struct BackgroundMigrationTests {
    private func v1JSON(background: Background) throws -> [String: Any] {
        var p = sampleProject(name: "old")
        p.style.background = background
        var obj = try JSONSerialization.jsonObject(with: JSONEncoder().encode(p)) as! [String: Any]
        obj["schemaVersion"] = 1
        var style = obj["style"] as! [String: Any]
        style.removeValue(forKey: "backgroundBlur")   // v1 had no blur field
        obj["style"] = style
        return obj
    }

    private func migrate(_ obj: [String: Any]) throws -> Project {
        let out = try ProjectMigrator.standard.migrate(JSONSerialization.data(withJSONObject: obj))
        return try JSONDecoder().decode(Project.self, from: out)
    }

    @Test(arguments: ["Sunset", "ocean", "Graphite", "aurora"])
    func v1WallpaperBecomesMesh(name: String) throws {
        let p = try migrate(v1JSON(background: .wallpaper(name: name)))
        #expect(p.schemaVersion == 2)
        #expect(p.style.background == BackgroundPresets.migratedWallpaper(named: name))
        #expect(p.style.backgroundBlur == 0)
    }

    @Test(arguments: [
        Background.solid(.white),
        Background.image(file: "background-x.png"),
        Background.gradient(stops: [GradientStop(location: 0, color: .black), GradientStop(location: 1, color: .white)], angleDegrees: 30),
    ])
    func otherBackgroundsAreUnchanged(_ bg: Background) throws {
        let p = try migrate(v1JSON(background: bg))
        #expect(p.schemaVersion == Project.currentSchemaVersion)
        #expect(p.style.background == bg)
    }

    @Test func storeLoadsAV1WallpaperProject() async throws {
        let dir = tempDir()
        defer { try? FileManager.default.removeItem(at: dir) }
        let store = FileProjectStore(libraryURL: dir)
        let ref = try await store.createProject(named: "legacy")
        let obj = try v1JSON(background: .wallpaper(name: "forest"))
        try JSONSerialization.data(withJSONObject: obj).write(to: ref.url.appendingPathComponent("project.json"))
        let loaded = try await store.load(ref)
        #expect(loaded.style.background == BackgroundPresets.migratedWallpaper(named: "forest"))
    }
}
