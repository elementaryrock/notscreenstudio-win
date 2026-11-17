import AppKit
import Foundation
import ImageIO
import NSSCore
import Observation

/// State behind the home screen: the list of recordings plus lazily loaded thumbnails.
/// All disk and decode work happens inside the library actor or a detached task, never on the main thread.
@MainActor
@Observable
public final class HomeModel {
    public enum LoadState: Equatable { case loading, loaded, failed(String) }

    public private(set) var items: [ProjectSummary] = []
    public private(set) var state: LoadState = .loading
    /// Decoded thumbnails by project id. A missing key means "not loaded yet"; see `failedThumbnails`.
    public private(set) var thumbnails: [UUID: NSImage] = [:]
    public private(set) var failedThumbnails: Set<UUID> = []
    /// Shown as an alert when a rename or delete fails.
    public var actionError: String?

    @ObservationIgnored private let library: any ProjectLibrary
    @ObservationIgnored private var loading: Set<UUID> = []

    public init(library: any ProjectLibrary) { self.library = library }

    public func reload() async {
        do {
            let list = try await library.summaries()
            let before = Dictionary(items.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
            items = list
            state = .loaded
            let alive = Set(list.map(\.id))
            // Prune deleted projects. A project whose bundle changed (new thumbnail file, new duration, rename)
            // forgets its failure, and a cached image for a different file, so its tile loads again.
            let changed = Set(list.filter { before[$0.id] != nil && before[$0.id] != $0 }.map(\.id))
            failedThumbnails = failedThumbnails.filter { alive.contains($0) && !changed.contains($0) }
            let byID = Dictionary(list.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
            thumbnails = thumbnails.filter { id, _ in alive.contains(id) && before[id]?.thumbnailURL == byID[id]?.thumbnailURL }
        } catch {
            state = .failed(error.localizedDescription)
        }
    }

    /// Called by each tile when it appears, so only visible tiles load their thumbnail.
    public func loadThumbnail(for summary: ProjectSummary) async {
        let id = summary.id
        guard thumbnails[id] == nil, !loading.contains(id) else { return }
        loading.insert(id)
        defer { loading.remove(id) }
        guard let url = await library.thumbnailURL(for: summary) else { failedThumbnails.insert(id); return }
        let cg = await Task.detached(priority: .utility) { Self.decode(url) }.value
        if let cg {
            thumbnails[id] = NSImage(cgImage: cg, size: NSSize(width: cg.width, height: cg.height))
            failedThumbnails.remove(id)
        } else {
            failedThumbnails.insert(id)
        }
    }

    public func rename(_ summary: ProjectSummary, to name: String) async {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, trimmed != summary.name else { return }
        do {
            try await library.rename(summary, to: trimmed)
            if let i = items.firstIndex(where: { $0.id == summary.id }) { items[i].name = trimmed }
        } catch {
            actionError = "Couldn't rename: \(error.localizedDescription)"
        }
    }

    /// Moves the recording to the Trash.
    public func delete(_ summary: ProjectSummary) async {
        do {
            try await library.delete(summary)
            items.removeAll { $0.id == summary.id }
            thumbnails[summary.id] = nil
            failedThumbnails.remove(summary.id)
        } catch {
            actionError = "Couldn't delete: \(error.localizedDescription)"
        }
    }

    /// Downscaled decode so a grid of large JPEGs stays cheap in memory.
    nonisolated static func decode(_ url: URL, maxEdge: Int = 520) -> CGImage? {
        guard let src = CGImageSourceCreateWithURL(url as CFURL, nil) else { return nil }
        let opts: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: maxEdge,
        ]
        return CGImageSourceCreateThumbnailAtIndex(src, 0, opts as CFDictionary)
    }

    // MARK: Formatting

    public static func durationLabel(_ seconds: MediaTime) -> String {
        let total = Int(max(seconds, 0).rounded())
        let h = total / 3600, m = (total % 3600) / 60, s = total % 60
        return h > 0 ? String(format: "%d:%02d:%02d", h, m, s) : String(format: "%d:%02d", m, s)
    }
}
