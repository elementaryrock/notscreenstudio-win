import CoreGraphics
import CryptoKit
import Foundation
import ImageIO
import NSSCore
import NSSRender
import UniformTypeIdentifiers

/// Picker thumbnails, made off the main thread and cached in memory and on disk.
///
/// Image files (wallpapers, picked images) are decoded with ImageIO at thumbnail size; the cache key includes
/// the file's modification time and size, so a changed file gets a fresh thumbnail. Gradients and colours are
/// drawn by `BackgroundRasterizer`, the CPU twin of the renderer's shader, so they look like the preview.
public actor BackgroundThumbnails {
    public static let shared = BackgroundThumbnails()

    /// Longer side of an image thumbnail, in pixels (tiles are ~64 pt, so this is sharp at 2x).
    public static let maxPixelSize = 192

    private var memory: [String: CGImage] = [:]
    private var order: [String] = []
    private let memoryLimit = 300
    private let diskDirectory: URL?

    public init(diskDirectory: URL? = BackgroundThumbnails.defaultDiskDirectory) {
        self.diskDirectory = diskDirectory
        if let diskDirectory { try? FileManager.default.createDirectory(at: diskDirectory, withIntermediateDirectories: true) }
    }

    public static var defaultDiskDirectory: URL? {
        FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask).first?
            .appendingPathComponent("notscreenstudio/BackgroundThumbnails", isDirectory: true)
    }

    /// Thumbnail of an image file, or nil when it cannot be read.
    /// Decodes run in parallel outside the actor; only the cache bookkeeping is serialized.
    public func thumbnail(forImageAt url: URL) async -> CGImage? {
        guard let key = Self.fileKey(url) else { return nil }
        if let hit = memory[key] { return hit }
        if let disk = readDisk(key) { return remember(disk, key) }
        let decoded = await Task.detached(priority: .userInitiated) { () -> CGImage? in
            let scoped = url.startAccessingSecurityScopedResource()
            defer { if scoped { url.stopAccessingSecurityScopedResource() } }
            return Self.decode(url, maxPixelSize: Self.maxPixelSize)
        }.value
        guard let image = decoded else { return nil }
        if memory[key] == nil { writeDisk(image, key) }
        return remember(image, key)
    }

    /// Thumbnail of a colour or gradient background at `width` x `height` pixels.
    public func thumbnail(for background: Background, width: Int, height: Int) -> CGImage? {
        let key = "bg:\(width)x\(height):" + ((try? Self.keyEncoder.encode(background)).map { String(decoding: $0, as: UTF8.self) } ?? "")
        if let hit = memory[key] { return hit }
        guard let image = BackgroundRasterizer.cgImage(background, width: width, height: height) else { return nil }
        return remember(image, key)
    }

    /// Sorted keys: the same background must always give the same cache key.
    private static let keyEncoder: JSONEncoder = {
        let e = JSONEncoder()
        e.outputFormatting = .sortedKeys
        return e
    }()

    private func remember(_ image: CGImage, _ key: String) -> CGImage {
        if memory[key] == nil { order.append(key) }
        memory[key] = image
        while order.count > memoryLimit { memory[order.removeFirst()] = nil }
        return image
    }

    /// Path + modification time + size: a replaced file gets a new key.
    static func fileKey(_ url: URL) -> String? {
        guard let attrs = try? FileManager.default.attributesOfItem(atPath: url.path) else { return nil }
        let mtime = (attrs[.modificationDate] as? Date)?.timeIntervalSince1970 ?? 0
        let size = (attrs[.size] as? NSNumber)?.int64Value ?? 0
        return "file:\(url.standardizedFileURL.path):\(mtime):\(size)"
    }

    static func decode(_ url: URL, maxPixelSize: Int) -> CGImage? {
        guard let src = CGImageSourceCreateWithURL(url as CFURL, nil) else { return nil }
        let opts: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: maxPixelSize,
        ]
        return CGImageSourceCreateThumbnailAtIndex(src, CGImageSourceGetPrimaryImageIndex(src), opts as CFDictionary)
    }

    private func diskURL(_ key: String) -> URL? {
        let name = SHA256.hash(data: Data(key.utf8)).map { String(format: "%02x", $0) }.joined()
        return diskDirectory?.appendingPathComponent(name + ".jpg")
    }

    private func readDisk(_ key: String) -> CGImage? {
        guard let url = diskURL(key), let src = CGImageSourceCreateWithURL(url as CFURL, nil) else { return nil }
        return CGImageSourceCreateImageAtIndex(src, 0, nil)
    }

    private func writeDisk(_ image: CGImage, _ key: String) {
        guard let url = diskURL(key),
            let dest = CGImageDestinationCreateWithURL(url as CFURL, UTType.jpeg.identifier as CFString, 1, nil)
        else { return }
        CGImageDestinationAddImage(dest, image, [kCGImageDestinationLossyCompressionQuality: 0.85] as CFDictionary)
        CGImageDestinationFinalize(dest)
    }
}
