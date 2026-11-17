import AVFoundation
import Foundation
import ImageIO
import NSSCore
import UniformTypeIdentifiers

/// `ProjectLibrary` over the `.nssproj` bundles in a library folder (the home screen's data source).
///
/// - Listing only reads each bundle's small `project.json`; it never touches media, so it stays fast with
///   many recordings.
/// - Thumbnails (`thumbnail.jpg` in the bundle) are made on demand off the main thread and cached on disk.
///   `AVAssetImageGenerator` does the decoding on its own queues, never on the caller's actor.
/// - Delete moves the bundle to the Trash. Rename changes the project name only; the bundle folder keeps
///   its name so open references stay valid.
public actor FileProjectLibrary: ProjectLibrary {
    public static let thumbnailFileName = "thumbnail.jpg"
    /// Longest edge of a generated thumbnail, in pixels.
    public static let thumbnailMaxEdge: CGFloat = 640

    public nonisolated let libraryURL: URL
    private let migrator: ProjectMigrator
    private let trash: @Sendable (URL) throws -> Void
    private var inFlight: [URL: Task<URL?, Never>] = [:]

    /// `trash` is injectable so tests never touch the real Trash.
    public init(
        libraryURL: URL = FileProjectStore.defaultLibraryURL,
        migrator: ProjectMigrator = .standard,
        trash: @escaping @Sendable (URL) throws -> Void = { try FileManager.default.trashItem(at: $0, resultingItemURL: nil) }
    ) {
        self.libraryURL = libraryURL
        self.migrator = migrator
        self.trash = trash
    }

    // MARK: Listing

    public func summaries() async throws -> [ProjectSummary] {
        let items = (try? FileManager.default.contentsOfDirectory(at: libraryURL, includingPropertiesForKeys: nil)) ?? []
        var result: [ProjectSummary] = []
        var seen = Set<UUID>()
        for url in items where url.pathExtension == ProjectReference.fileExtension {
            guard let project = readProject(in: url) else { continue }
            let thumb = Self.thumbnailURL(in: url, project: project)
            // A bundle copied in Finder keeps the original's project id; give copies a stable id of their own.
            let id = seen.insert(project.id).inserted ? project.id : Self.pathID(url)
            result.append(ProjectSummary(
                id: id, name: project.name, createdAt: project.createdAt,
                duration: project.assets.duration, bundle: ProjectReference(url: url),
                thumbnailURL: FileManager.default.fileExists(atPath: thumb.path) ? thumb : nil))
        }
        return result.sorted { $0.createdAt > $1.createdAt }
    }

    /// Deterministic UUID from a path (FNV-1a, two 64-bit lanes), stable across launches.
    nonisolated static func pathID(_ url: URL) -> UUID {
        var a: UInt64 = 0xcbf29ce484222325, b: UInt64 = 0x84222325cbf29ce4
        for byte in url.standardizedFileURL.path.utf8 {
            a = (a ^ UInt64(byte)) &* 0x100000001b3
            b = (b &+ UInt64(byte)) &* 0x100000001b3 ^ (b >> 29)
        }
        let u = (0..<8).map { UInt8(truncatingIfNeeded: a >> (8 * UInt64($0))) } + (0..<8).map { UInt8(truncatingIfNeeded: b >> (8 * UInt64($0))) }
        return UUID(uuid: (u[0], u[1], u[2], u[3], u[4], u[5], u[6], u[7], u[8], u[9], u[10], u[11], u[12], u[13], u[14], u[15]))
    }

    // MARK: Thumbnails

    public func thumbnailURL(for summary: ProjectSummary) async -> URL? {
        let bundle = summary.bundle.url
        guard let project = readProject(in: bundle) else { return nil }
        let target = Self.thumbnailURL(in: bundle, project: project)
        if FileManager.default.fileExists(atPath: target.path) { return target }
        if let running = inFlight[bundle] { return await running.value }
        let video = bundle.appendingPathComponent(project.assets.screenFile)
        let time = Self.thumbnailTime(duration: project.assets.duration)
        let task = Task.detached(priority: .utility) { () -> URL? in
            await Self.generateThumbnail(video: video, at: time, to: target) ? target : nil
        }
        inFlight[bundle] = task
        let url = await task.value
        inFlight[bundle] = nil
        return url
    }

    /// About 1 s in, or the middle of clips shorter than 2 s.
    public nonisolated static func thumbnailTime(duration: MediaTime) -> MediaTime {
        guard duration > 0 else { return 0 }
        return duration >= 2 ? 1 : duration / 2
    }

    private nonisolated static func thumbnailURL(in bundle: URL, project: Project) -> URL {
        bundle.appendingPathComponent(project.assets.thumbnailFile ?? thumbnailFileName)
    }

    /// Decodes one frame and writes it as a JPEG. Returns false when the video is missing or unreadable.
    static func generateThumbnail(video: URL, at time: MediaTime, to target: URL) async -> Bool {
        guard FileManager.default.fileExists(atPath: video.path) else { return false }
        let generator = AVAssetImageGenerator(asset: AVURLAsset(url: video))
        generator.appliesPreferredTrackTransform = true
        generator.maximumSize = CGSize(width: thumbnailMaxEdge, height: thumbnailMaxEdge)
        generator.requestedTimeToleranceBefore = CMTime(seconds: 0.5, preferredTimescale: 600)
        generator.requestedTimeToleranceAfter = CMTime(seconds: 0.5, preferredTimescale: 600)
        do {
            let cm = CMTime(seconds: time, preferredTimescale: 600)
            let (image, _) = try await generator.image(at: cm)
            return writeJPEG(image, to: target)
        } catch {
            // The requested time can be past the real end of a short file; fall back to the first frame.
            guard let (image, _) = try? await generator.image(at: .zero) else { return false }
            return writeJPEG(image, to: target)
        }
    }

    private nonisolated static func writeJPEG(_ image: CGImage, to url: URL) -> Bool {
        let data = NSMutableData()
        guard let dest = CGImageDestinationCreateWithData(data, UTType.jpeg.identifier as CFString, 1, nil) else { return false }
        CGImageDestinationAddImage(dest, image, [kCGImageDestinationLossyCompressionQuality: 0.8] as CFDictionary)
        guard CGImageDestinationFinalize(dest) else { return false }
        return (try? AtomicFile.write(data as Data, to: url)) != nil
    }

    // MARK: Delete and rename

    public func delete(_ summary: ProjectSummary) async throws {
        inFlight[summary.bundle.url]?.cancel()
        do {
            try trash(summary.bundle.url)
        } catch {
            throw NSSError.io("Couldn't move \(summary.bundle.url.lastPathComponent) to the Trash: \(error.localizedDescription)")
        }
    }

    public func rename(_ summary: ProjectSummary, to name: String) async throws {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { throw NSSError.invalidProject("A name can't be empty") }
        // Primary file only: never rewrite project.json from an older backup (see FileProjectStore.load).
        let primary = summary.bundle.url.appendingPathComponent(ProjectReference.projectFileName)
        guard let data = try? Data(contentsOf: primary) else {
            throw NSSError.invalidProject("Missing \(ProjectReference.projectFileName) in \(summary.bundle.url.lastPathComponent)")
        }
        var project: Project
        do {
            project = try JSONDecoder().decode(Project.self, from: try migrator.migrate(data))
        } catch let e as ProjectMigrationError {
            throw NSSError.invalidProject("\(e)")
        } catch {
            throw NSSError.invalidProject("Corrupt \(ProjectReference.projectFileName): \(error.localizedDescription)")
        }
        guard project.name != trimmed else { return }
        project.name = trimmed
        try FileProjectStore.writeProject(project, to: summary.bundle)
    }

    // MARK: Reading

    private func readProject(in bundle: URL) -> Project? {
        for file in [ProjectReference.projectFileName, FileProjectStore.backupFileName] {
            guard let data = try? Data(contentsOf: bundle.appendingPathComponent(file)),
                  let migrated = try? migrator.migrate(data),
                  let project = try? JSONDecoder().decode(Project.self, from: migrated),
                  project.assets.unsafeFileName == nil
            else { continue }
            return project
        }
        return nil
    }
}
