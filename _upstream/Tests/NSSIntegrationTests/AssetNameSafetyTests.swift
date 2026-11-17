import Foundation
import Testing
import NSSCore
import NSSProject

/// Asset file names from `project.json` are joined onto the bundle path, so they must not escape it.
@Suite struct AssetNameSafetyTests {
    @Test(arguments: ["../x.jpg", "a/b.jpg", "/etc/passwd", "..", ".", "", "a\\b", "x\0y"])
    func rejectsUnsafeNames(_ name: String) {
        #expect(!RecordingAssets.isSafeFileName(name))
    }

    @Test(arguments: ["thumbnail.jpg", "screen.mov", "mic.m4a", "my file (1).mov", "..hidden"])
    func acceptsPlainNames(_ name: String) {
        #expect(RecordingAssets.isSafeFileName(name))
    }

    @Test func everyAssetNameIsChecked() {
        let base = RecordingAssets(sourcePixelSize: PixelSize(width: 10, height: 10), frameRate: 60, duration: 1)
        #expect(base.unsafeFileName == nil)
        for edit: (inout RecordingAssets) -> Void in [
            { $0.screenFile = "../s.mov" }, { $0.cursorFile = "a/c.json" }, { $0.cameraFile = "../c.mov" },
            { $0.micFile = "/m.m4a" }, { $0.thumbnailFile = "../../thumb.jpg" },
        ] {
            var a = base
            edit(&a)
            #expect(a.unsafeFileName != nil)
        }
    }

    @Test func storeAndLibraryRefuseABundleThatNamesAFileOutsideIt() async throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("nss-names-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: dir) }
        let store = FileProjectStore(libraryURL: dir)
        let ref = try await store.createProject(named: "evil")
        var assets = RecordingAssets(sourcePixelSize: PixelSize(width: 10, height: 10), frameRate: 60, duration: 1)
        assets.thumbnailFile = "../../outside.jpg"
        try await store.save(Project(name: "evil", assets: assets), to: ref)

        await #expect(throws: NSSError.self) { _ = try await store.load(ref) }
        let summaries = try await FileProjectLibrary(libraryURL: dir).summaries()
        #expect(summaries.isEmpty)
    }
}
