import AppKit
import Foundation
import Testing
import NSSCore
@testable import NSSUI

private func summary(_ name: String, at t: Double, thumb: URL? = nil) -> ProjectSummary {
    ProjectSummary(name: name, createdAt: Date(timeIntervalSince1970: t), duration: 12,
                   bundle: ProjectReference(url: URL(fileURLWithPath: "/tmp/\(name).nssproj")), thumbnailURL: thumb)
}

@MainActor @Suite struct HomeModelTests {
    @Test func reloadListsNewestFirst() async {
        let m = HomeModel(library: InMemoryProjectLibrary([summary("a", at: 1), summary("c", at: 3), summary("b", at: 2)]))
        #expect(m.state == .loading)
        await m.reload()
        #expect(m.state == .loaded)
        #expect(m.items.map(\.name) == ["c", "b", "a"])
    }

    @Test func renameAndDeleteUpdateList() async {
        let a = summary("a", at: 1), b = summary("b", at: 2)
        let m = HomeModel(library: InMemoryProjectLibrary([a, b]))
        await m.reload()
        await m.rename(a, to: "  Demo  ")
        #expect(m.items.first { $0.id == a.id }?.name == "Demo")
        await m.rename(a, to: "   ")                      // ignored
        #expect(m.items.first { $0.id == a.id }?.name == "Demo")
        await m.delete(b)
        #expect(m.items.map(\.id) == [a.id])
    }

    @Test func thumbnailLoadsLazilyAndFailureIsRemembered() async throws {
        let png = FileManager.default.temporaryDirectory.appendingPathComponent("home-\(UUID().uuidString).png")
        defer { try? FileManager.default.removeItem(at: png) }
        let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 8, pixelsHigh: 8, bitsPerSample: 8, samplesPerPixel: 4,
                                   hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
        try rep.representation(using: .png, properties: [:])!.write(to: png)
        let withThumb = summary("a", at: 2, thumb: png), without = summary("b", at: 1)
        let m = HomeModel(library: InMemoryProjectLibrary([withThumb, without]))
        await m.reload()
        #expect(m.thumbnails.isEmpty)                      // nothing decoded until a tile asks
        await m.loadThumbnail(for: withThumb)
        await m.loadThumbnail(for: without)
        #expect(m.thumbnails[withThumb.id] != nil)
        #expect(m.failedThumbnails == [without.id])
    }

    @Test func durationLabels() {
        #expect(HomeModel.durationLabel(0) == "0:00")
        #expect(HomeModel.durationLabel(65.4) == "1:05")
        #expect(HomeModel.durationLabel(3725) == "1:02:05")
    }
}

/// A library whose list the test controls, so a bundle can "change" between reloads.
private actor ScriptedLibrary: ProjectLibrary {
    var items: [ProjectSummary]
    init(_ items: [ProjectSummary]) { self.items = items }
    func set(_ items: [ProjectSummary]) { self.items = items }
    func summaries() async throws -> [ProjectSummary] { items }
    func thumbnailURL(for summary: ProjectSummary) async -> URL? { items.first { $0.id == summary.id }?.thumbnailURL }
    func delete(_ summary: ProjectSummary) async throws {}
    func rename(_ summary: ProjectSummary, to name: String) async throws {}
}

@MainActor @Suite struct HomeThumbnailRetryTests {
    @Test func failuresArePrunedWhenDeletedAndClearedWhenTheBundleChanges() async {
        let a = summary("a", at: 2), b = summary("b", at: 1)
        let lib = ScriptedLibrary([a, b])
        let m = HomeModel(library: lib)
        await m.reload()
        await m.loadThumbnail(for: a)
        await m.loadThumbnail(for: b)
        #expect(m.failedThumbnails == [a.id, b.id])

        await m.reload()                                   // nothing changed: still failed, not retried
        #expect(m.failedThumbnails == [a.id, b.id])

        var a2 = a
        a2.thumbnailURL = URL(fileURLWithPath: "/tmp/new-thumb.jpg")   // the bundle changed
        await lib.set([a2])                                // and b was deleted behind our back
        await m.reload()
        #expect(m.failedThumbnails.isEmpty)                // a will retry, b is pruned
    }
}
