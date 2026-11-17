import Foundation
import Testing
import NSSCore
@testable import NSSUI

struct BackgroundPickerTests {
    private func tempDir() throws -> URL {
        let d = FileManager.default.temporaryDirectory.appendingPathComponent("nss-bgpick-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: d, withIntermediateDirectories: true)
        return d
    }

    private func touch(_ url: URL, _ text: String = "x") throws {
        try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        try Data(text.utf8).write(to: url)
    }

    @Test func catalogTakesImagesAndStillsAndSkipsMadesktopAndFolders() throws {
        let root = try tempDir()
        defer { try? FileManager.default.removeItem(at: root) }
        try touch(root.appendingPathComponent("Sonoma.heic"))
        try touch(root.appendingPathComponent("iMac Blue.heic"))
        try touch(root.appendingPathComponent("Photo.JPG"))
        try touch(root.appendingPathComponent("Big Sur.madesktop"))
        try touch(root.appendingPathComponent(".hidden.heic"))
        try touch(root.appendingPathComponent("Solid Colors/Black.png"))
        try touch(root.appendingPathComponent(".thumbnails/Big Sur.heic"))
        try touch(root.appendingPathComponent(".wallpapers/Sequoia Sunrise/Sequoia Sunrise Thumbnail.png"))
        try touch(root.appendingPathComponent(".wallpapers/Sequoia Sunrise/Sequoia Sunrise.heic"))
        try touch(root.appendingPathComponent(".wallpapers/Sequoia Sunrise/Sequoia Sunrise.mov"))
        try touch(root.appendingPathComponent(".wallpapers/Only Video/Only Video.mov"))

        let found = SystemWallpaperCatalog.scan(roots: [root, root.appendingPathComponent("missing")])
        #expect(found.map(\.name) == ["iMac Blue", "Photo", "Sequoia Sunrise", "Sonoma"])
        #expect(found.first { $0.name == "Sequoia Sunrise" }?.url.lastPathComponent == "Sequoia Sunrise.heic")
    }

    @Test func catalogDeduplicatesByNameAcrossRoots() throws {
        let a = try tempDir(), b = try tempDir()
        defer { try? FileManager.default.removeItem(at: a); try? FileManager.default.removeItem(at: b) }
        try touch(a.appendingPathComponent("Same.heic"))
        try touch(b.appendingPathComponent("same.jpg"))
        let found = SystemWallpaperCatalog.scan(roots: [a, b])
        #expect(found.count == 1)
        #expect(found[0].url.deletingLastPathComponent().standardizedFileURL == a.standardizedFileURL)
    }

    @Test func recentsAreMostRecentFirstDeduplicatedCappedAndSkipMissingFiles() throws {
        let dir = try tempDir()
        let suite = "nss-test-recents-\(UUID().uuidString)"
        defer {
            try? FileManager.default.removeItem(at: dir)
            UserDefaults().removePersistentDomain(forName: suite)
        }
        let store = RecentBackgroundImages(suiteName: suite, limit: 3)
        let files = try (0..<5).map { i -> URL in
            let u = dir.appendingPathComponent("img\(i).png"); try touch(u); return u
        }
        #expect(store.load().isEmpty)
        for f in files.prefix(4) { store.add(f) }
        store.add(files[2])   // re-picking moves it to the front, no duplicate
        #expect(store.load().map(\.lastPathComponent) == ["img2.png", "img3.png", "img1.png"])
        try FileManager.default.removeItem(at: files[3])
        #expect(store.load().map(\.lastPathComponent) == ["img2.png", "img1.png"])
        store.remove(files[2])
        #expect(store.load().map(\.lastPathComponent) == ["img1.png"])
        // A second store on the same suite (another project) sees the same list.
        #expect(RecentBackgroundImages(suiteName: suite).load().map(\.lastPathComponent) == ["img1.png"])
    }

    @Test func recentsFollowAMovedFileWithOneIdentity() throws {
        let dir = try tempDir()
        let suite = "nss-test-recents-\(UUID().uuidString)"
        defer {
            try? FileManager.default.removeItem(at: dir)
            UserDefaults().removePersistentDomain(forName: suite)
        }
        let store = RecentBackgroundImages(suiteName: suite)
        let old = dir.appendingPathComponent("before.png"), other = dir.appendingPathComponent("other.png")
        try touch(old); try touch(other)
        store.add(old)
        store.add(other)
        let moved = dir.appendingPathComponent("sub/after.png")
        try FileManager.default.createDirectory(at: moved.deletingLastPathComponent(), withIntermediateDirectories: true)
        try FileManager.default.moveItem(at: old, to: moved)

        let names = { store.load().map(\.lastPathComponent) }
        #expect(names() == ["other.png", "after.png"], "the bookmark follows the move, one entry")
        store.add(moved)
        #expect(names() == ["after.png", "other.png"], "re-adding the moved file does not duplicate it")
        #expect(store.load().count == Set(store.load().map(\.path)).count)
        store.remove(store.load()[0])
        #expect(names() == ["other.png"], "removing the moved file removes it for good")
    }

    @Test func importCopiesIntoTheBundleWithAContentHashedName() throws {
        let dir = try tempDir(), bundle = try tempDir()
        defer { try? FileManager.default.removeItem(at: dir); try? FileManager.default.removeItem(at: bundle) }
        let a = dir.appendingPathComponent("a/photo.png"), b = dir.appendingPathComponent("b/photo.png")
        try touch(a, "first image")
        try touch(b, "second image")
        let na = try BackgroundImageImport.copy(a, into: bundle)
        let nb = try BackgroundImageImport.copy(b, into: bundle)
        #expect(na != nb, "same file name, different contents must not overwrite each other")
        #expect(na.hasPrefix("background-") && na.hasSuffix("-photo.png"))
        #expect(try String(contentsOf: bundle.appendingPathComponent(na), encoding: .utf8) == "first image")
        #expect(try BackgroundImageImport.copy(a, into: bundle) == na, "picking the same image again reuses the copy")
    }

    @Test func tabsFollowTheBackgroundKind() {
        #expect(BackgroundTab(for: .systemWallpaper(path: "/a.heic")) == .wallpapers)
        #expect(BackgroundTab(for: BackgroundPresets.gradients[0].background) == .gradients)
        #expect(BackgroundTab(for: .mesh(points: [], warp: 0)) == .gradients)
        #expect(BackgroundTab(for: .wallpaper(name: "aurora")) == .gradients)
        #expect(BackgroundTab(for: .solid(.white)) == .colours)
        #expect(BackgroundTab(for: .image(file: "x.png")) == .image)
        #expect(BackgroundTab.allCases.filter(\.hasBlur) == [.wallpapers, .image])
    }

    @Test func gradientThumbnailsAreMadeAndCached() async throws {
        let cache = BackgroundThumbnails(diskDirectory: nil)
        let bg = BackgroundPresets.gradients[9].background
        let a = try #require(await cache.thumbnail(for: bg, width: 64, height: 40))
        #expect(a.width == 64 && a.height == 40)
        let b = await cache.thumbnail(for: bg, width: 64, height: 40)
        #expect(a === b)
    }
}
