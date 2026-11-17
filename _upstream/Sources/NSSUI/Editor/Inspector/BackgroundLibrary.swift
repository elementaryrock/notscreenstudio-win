import CryptoKit
import Foundation
import NSSCore

/// One macOS desktop picture that is available on this Mac as a full image file.
public struct SystemWallpaper: Sendable, Hashable, Identifiable {
    public var name: String
    public var url: URL
    public var id: String { url.path }
}

/// Finds the macOS wallpapers on this Mac.
///
/// - Plain image files at the top of each folder (`.heic`, `.jpg`, `.png`, ...) are used as they are. Dynamic
///   HEIC files (light/dark or time of day) draw their primary image.
/// - `.wallpapers/<Name>/` folders (the stills shipped next to the moving wallpapers) contribute their still image.
/// - `.madesktop` entries are skipped: they only point at a download (with a 214 px thumbnail), so there is no
///   full image to draw. Sub-folders such as "Solid Colors" are skipped too; the Colours tab covers them.
public enum SystemWallpaperCatalog {
    public static let defaultRoots: [URL] = [
        URL(fileURLWithPath: "/System/Library/Desktop Pictures", isDirectory: true),
        URL(fileURLWithPath: "/Library/Desktop Pictures", isDirectory: true),
    ]

    static let imageExtensions: Set<String> = ["heic", "heif", "jpg", "jpeg", "png", "tif", "tiff"]

    /// Scans `roots` (blocking file I/O: call off the main thread). Sorted by name, one entry per name.
    public static func scan(roots: [URL] = defaultRoots) -> [SystemWallpaper] {
        let fm = FileManager.default
        var byName: [String: SystemWallpaper] = [:]
        func add(_ url: URL) {
            let name = url.deletingPathExtension().lastPathComponent
            if byName[name.lowercased()] == nil { byName[name.lowercased()] = SystemWallpaper(name: name, url: url) }
        }
        for root in roots {
            guard let items = try? fm.contentsOfDirectory(at: root, includingPropertiesForKeys: [.isDirectoryKey]) else { continue }
            for item in items.sorted(by: { $0.lastPathComponent < $1.lastPathComponent }) {
                let isDir = (try? item.resourceValues(forKeys: [.isDirectoryKey]).isDirectory) ?? false
                if !isDir, !item.lastPathComponent.hasPrefix("."), isImage(item) {
                    add(item)
                } else if isDir, item.lastPathComponent == ".wallpapers" {
                    let folders = (try? fm.contentsOfDirectory(at: item, includingPropertiesForKeys: nil)) ?? []
                    for folder in folders.sorted(by: { $0.lastPathComponent < $1.lastPathComponent }) {
                        if let still = stillImage(in: folder) { add(still) }
                    }
                }
            }
        }
        return byName.values.sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
    }

    static func isImage(_ url: URL) -> Bool { imageExtensions.contains(url.pathExtension.lowercased()) }

    /// The still image of a `.wallpapers/<Name>/` folder: `<Name>.heic` (or another image named like the
    /// folder), else the first non-thumbnail image in it.
    static func stillImage(in folder: URL) -> URL? {
        let files = ((try? FileManager.default.contentsOfDirectory(at: folder, includingPropertiesForKeys: nil)) ?? [])
            .filter { isImage($0) && !$0.lastPathComponent.lowercased().contains("thumbnail") }
            .sorted { $0.lastPathComponent < $1.lastPathComponent }
        let folderName = folder.lastPathComponent
        return files.first { $0.deletingPathExtension().lastPathComponent == folderName } ?? files.first
    }
}

/// The last few background images the user picked, remembered across projects.
///
/// Stored in user defaults as bookmarks (security-scoped when the system allows it, plain otherwise) plus the
/// path, so a file that is moved is still found and a sandboxed build keeps read access.
public struct RecentBackgroundImages: Sendable {
    public static let defaultsKey = "recentBackgroundImages"
    public static let limit = 8

    /// nil means `UserDefaults.standard`.
    public var suiteName: String?
    public var limit: Int

    public init(suiteName: String? = nil, limit: Int = RecentBackgroundImages.limit) {
        self.suiteName = suiteName
        self.limit = limit
    }

    private var defaults: UserDefaults { suiteName.flatMap(UserDefaults.init(suiteName:)) ?? .standard }

    struct Entry: Codable, Equatable {
        var path: String
        var bookmark: Data?
    }

    func entries() -> [Entry] {
        guard let data = defaults.data(forKey: Self.defaultsKey) else { return [] }
        return (try? JSONDecoder().decode([Entry].self, from: data)) ?? []
    }

    private func write(_ entries: [Entry]) {
        defaults.set(try? JSONEncoder().encode(entries), forKey: Self.defaultsKey)
    }

    /// Each entry with its current location: the bookmark's target when it resolves to an existing file
    /// (so a moved or renamed file is followed), else the stored path if that file exists, else nil.
    /// This one location is the entry's identity for `load`, `add` and `remove`.
    private func resolvedEntries() -> [(entry: Entry, path: String?)] {
        let fm = FileManager.default
        return entries().map { e in
            if let bm = e.bookmark, let url = Self.resolve(bm) {
                let p = url.standardizedFileURL.path
                if fm.fileExists(atPath: p) { return (e, p) }
            }
            return (e, fm.fileExists(atPath: e.path) ? e.path : nil)
        }
    }

    /// Most recent first, only files that still exist, one per file.
    public func load() -> [URL] {
        var seen = Set<String>()
        return resolvedEntries().compactMap { r in
            guard let p = r.path, seen.insert(p).inserted else { return nil }
            return URL(fileURLWithPath: p)
        }
    }

    /// Moves `url` to the front (no duplicates, also of entries whose bookmark now points at it) and keeps
    /// at most `limit` entries. Kept entries are rewritten with their current path.
    public func add(_ url: URL) {
        let target = url.standardizedFileURL.path
        var list = others(than: target)
        list.insert(Entry(path: target, bookmark: Self.bookmark(url)), at: 0)
        write(Array(list.prefix(max(limit, 0))))
    }

    /// Removes every entry for `url`, matched by its current location or its stored path.
    public func remove(_ url: URL) {
        write(others(than: url.standardizedFileURL.path))
    }

    /// Entries that are not `target`, updated to their current path; entries for missing files are kept as
    /// they are (the file may come back, for example on an external disk).
    private func others(than target: String) -> [Entry] {
        resolvedEntries().compactMap { r in
            if r.entry.path == target || r.path == target { return nil }
            var e = r.entry
            if let p = r.path { e.path = p }
            return e
        }
    }

    private static func bookmark(_ url: URL) -> Data? {
        (try? url.bookmarkData(options: .withSecurityScope, includingResourceValuesForKeys: nil, relativeTo: nil))
            ?? (try? url.bookmarkData(options: [], includingResourceValuesForKeys: nil, relativeTo: nil))
    }

    private static func resolve(_ data: Data) -> URL? {
        var stale = false
        if let url = try? URL(resolvingBookmarkData: data, options: .withSecurityScope, relativeTo: nil, bookmarkDataIsStale: &stale) {
            return url
        }
        return try? URL(resolvingBookmarkData: data, options: [], relativeTo: nil, bookmarkDataIsStale: &stale)
    }
}

/// Copies a picked image into the project bundle, so the project stays self-contained and exports anywhere.
public enum BackgroundImageImport {
    /// Bundle-relative name for `source`: `background-<hash>-<file name>`. The hash comes from the file's
    /// contents, so two different images with the same name never overwrite each other (undo stays correct),
    /// and picking the same image again reuses the copy.
    public static func bundleName(for source: URL, contents: Data) -> String {
        let digest = SHA256.hash(data: contents).prefix(5).map { String(format: "%02x", $0) }.joined()
        return "background-\(digest)-\(source.lastPathComponent)"
    }

    /// Copies `source` into `bundle` (blocking I/O: call off the main thread) and returns the relative name
    /// to store in `Background.image(file:)`.
    public static func copy(_ source: URL, into bundle: URL) throws -> String {
        let scoped = source.startAccessingSecurityScopedResource()
        defer { if scoped { source.stopAccessingSecurityScopedResource() } }
        let data = try Data(contentsOf: source, options: .mappedIfSafe)
        let name = bundleName(for: source, contents: data)
        let dest = bundle.appendingPathComponent(name)
        if !FileManager.default.fileExists(atPath: dest.path) {
            try data.write(to: dest, options: .atomic)
        }
        return name
    }
}
