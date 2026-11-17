import Foundation
import NSSCore

/// `.nssproj` bundle store. A bundle is a folder holding `project.json`, the media files and `cursor.json`.
///
/// - Writes are atomic (`AtomicFile`), and the last good `project.json` is kept as `project.json.bak`.
/// - `scheduleAutosave` debounces: the write happens 1 s after the last call. Call `flushPendingAutosaves()`
///   (or `installTerminationFlush()` once) so nothing is lost on quit.
/// - `load` migrates older schemas, and falls back to the backup when `project.json` is missing or corrupt.
public actor FileProjectStore: ProjectStore {
    public nonisolated let libraryURL: URL
    public static let backupFileName = "project.json.bak"
    public static let recentsFileName = ".recent-projects.json"

    private let migrator: ProjectMigrator
    private let debouncer: AutosaveDebouncer
    private let now: @Sendable () -> Date

    public init(
        libraryURL: URL = FileProjectStore.defaultLibraryURL,
        migrator: ProjectMigrator = .standard,
        autosaveDelay: Double = 1,
        autosaveClock: any DebounceClock = SystemDebounceClock(),
        onAutosaveError: AutosaveDebouncer.ErrorHandler? = nil,
        now: @escaping @Sendable () -> Date = { Date() }
    ) {
        self.libraryURL = libraryURL
        self.migrator = migrator
        self.now = now
        self.debouncer = AutosaveDebouncer(
            delay: autosaveDelay,
            clock: autosaveClock,
            onError: onAutosaveError,
            writer: { project, ref in try FileProjectStore.writeProject(project, to: ref) }
        )
    }

    public static var defaultLibraryURL: URL {
        FileManager.default.urls(for: .moviesDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("notscreenstudio", isDirectory: true)
    }

    // MARK: Coding

    static func encoder() -> JSONEncoder {
        let e = JSONEncoder()
        e.outputFormatting = [.prettyPrinted, .sortedKeys]
        // Default (deferredToDate) keeps full Double precision, so load(save(p)) == p.
        return e
    }

    /// Compact (no whitespace) for the large `cursor.json`.
    static func compactEncoder() -> JSONEncoder { JSONEncoder() }

    static func decoder() -> JSONDecoder { JSONDecoder() }

    // MARK: Create

    public func createProject(named name: String) async throws -> ProjectReference {
        let fm = FileManager.default
        try fm.createDirectory(at: libraryURL, withIntermediateDirectories: true)
        let base = Self.sanitize(name)
        var n = 1
        while true {
            let leaf = n == 1 ? base : "\(base) \(n)"
            let url = libraryURL.appendingPathComponent("\(leaf).\(ProjectReference.fileExtension)", isDirectory: true)
            do {
                // Non-intermediate create fails if it exists, so an existing bundle is never reused.
                try fm.createDirectory(at: url, withIntermediateDirectories: false)
                let ref = ProjectReference(url: url)
                touchRecent(ref, name: name)
                return ref
            } catch let e as CocoaError where e.code == .fileWriteFileExists {
                n += 1
            }
        }
    }

    /// Makes `name` safe as one path component: no separators, no leading dots, never empty.
    static func sanitize(_ name: String) -> String {
        var s = String(name.map { "/:\\\0".contains($0) ? "-" : $0 })
        s = s.trimmingCharacters(in: .whitespacesAndNewlines)
        while s.hasPrefix(".") { s.removeFirst() }
        return s.isEmpty ? "Untitled" : s
    }

    // MARK: Load / save

    public func load(_ ref: ProjectReference) async throws -> Project {
        let fm = FileManager.default
        var isDir: ObjCBool = false
        guard fm.fileExists(atPath: ref.url.path, isDirectory: &isDir), isDir.boolValue else {
            throw NSSError.invalidProject("Project bundle not found: \(ref.url.lastPathComponent)")
        }
        let primary = ref.url.appendingPathComponent(ProjectReference.projectFileName)
        let backup = ref.url.appendingPathComponent(Self.backupFileName)
        let project: Project
        do {
            project = try readProject(at: primary)
        } catch let primaryError {
            // A newer-schema file must not be silently replaced by an older backup.
            if primaryError is ProjectMigrationError { throw NSSError.invalidProject("\(primaryError)") }
            guard let recovered = try? readProject(at: backup) else { throw primaryError }
            project = recovered
        }
        touchRecent(ref, name: project.name)
        return project
    }

    private func readProject(at url: URL) throws -> Project {
        let data: Data
        do {
            data = try Data(contentsOf: url)
        } catch {
            throw NSSError.invalidProject("Missing \(url.lastPathComponent)")
        }
        let migrated = try migrator.migrate(data)
        let project: Project
        do {
            project = try Self.decoder().decode(Project.self, from: migrated)
        } catch {
            throw NSSError.invalidProject("Corrupt \(url.lastPathComponent): \(error.localizedDescription)")
        }
        // Asset names are joined onto the bundle path: they must not point outside it.
        if let bad = project.assets.unsafeFileName {
            throw NSSError.invalidProject("\(url.lastPathComponent) names a file outside the project: \(bad)")
        }
        return project
    }

    public func save(_ project: Project, to ref: ProjectReference) async throws {
        // An explicit save is newer than anything scheduled before it. `exclusive` waits for an in-flight
        // autosave write, drops the pending one, and makes this the last write for the bundle.
        try debouncer.exclusive(ref) { try Self.writeProject(project, to: ref) }
        touchRecent(ref, name: project.name)
    }

    /// Atomic write, keeping the previous valid `project.json` as a backup. Synchronous and nonisolated so the
    /// debouncer can call it from a background task and from `flush()` at quit.
    static func writeProject(_ project: Project, to ref: ProjectReference) throws {
        try FileManager.default.createDirectory(at: ref.url, withIntermediateDirectories: true)
        let file = ref.url.appendingPathComponent(ProjectReference.projectFileName)
        let data = try encoder().encode(project)
        if let old = try? Data(contentsOf: file), (try? decoder().decode(Project.self, from: old)) != nil {
            try? AtomicFile.write(old, to: ref.url.appendingPathComponent(backupFileName))
        }
        try AtomicFile.write(data, to: file)
    }

    // MARK: Cursor track

    /// A missing `cursor.json` yields an empty track (the video is still usable); a corrupt one throws.
    public func loadCursorTrack(for ref: ProjectReference, assets: RecordingAssets) async throws -> CursorTrack {
        let url = ref.url.appendingPathComponent(assets.cursorFile)
        guard let data = try? Data(contentsOf: url) else { return .empty }
        do {
            return try Self.decoder().decode(CursorTrack.self, from: data)
        } catch {
            throw NSSError.invalidProject("Corrupt \(assets.cursorFile): \(error.localizedDescription)")
        }
    }

    public func saveCursorTrack(_ track: CursorTrack, for ref: ProjectReference, assets: RecordingAssets) async throws {
        let data = try Self.compactEncoder().encode(track)
        try AtomicFile.write(data, to: ref.url.appendingPathComponent(assets.cursorFile))
    }

    // MARK: Autosave

    public nonisolated func scheduleAutosave(_ project: Project, to ref: ProjectReference) {
        debouncer.schedule(project, to: ref)
    }

    /// Writes all pending autosaves now. Call on app quit.
    @discardableResult
    public nonisolated func flushPendingAutosaves() -> Int { debouncer.flush() }

    public nonisolated func hasPendingAutosave(for ref: ProjectReference) -> Bool { debouncer.hasPending(for: ref) }

    /// Flushes pending autosaves when the app terminates. Call once from the app. Uses the notification name
    /// as a string so this module needs no AppKit.
    public nonisolated func installTerminationFlush() {
        let debouncer = self.debouncer
        NotificationCenter.default.addObserver(
            forName: Notification.Name("NSApplicationWillTerminateNotification"), object: nil, queue: nil
        ) { _ in debouncer.flush() }
    }

    // MARK: Listing and recents

    /// All bundles in the library, newest first.
    public func listProjects() async throws -> [ProjectReference] {
        let items = (try? FileManager.default.contentsOfDirectory(
            at: libraryURL, includingPropertiesForKeys: [.contentModificationDateKey]
        )) ?? []
        let bundles = items.filter { $0.pathExtension == ProjectReference.fileExtension }
        func modified(_ u: URL) -> Date {
            (try? u.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate) ?? .distantPast
        }
        return bundles.sorted { modified($0) > modified($1) }.map(ProjectReference.init)
    }

    /// Recently opened projects, newest first, with bundles that no longer exist removed.
    public func recentProjects() -> [RecentProject] {
        var list = readRecents()
        if list.prune(exists: { FileManager.default.fileExists(atPath: $0) }) { writeRecents(list) }
        return list.items
    }

    public func removeFromRecents(_ ref: ProjectReference) {
        var list = readRecents()
        list.remove(path: ref.url.standardizedFileURL.path)
        writeRecents(list)
    }

    private var recentsURL: URL { libraryURL.appendingPathComponent(Self.recentsFileName) }

    private func readRecents() -> RecentProjects {
        guard let data = try? Data(contentsOf: recentsURL),
              let list = try? Self.decoder().decode(RecentProjects.self, from: data)
        else { return RecentProjects() }
        return list
    }

    private func writeRecents(_ list: RecentProjects) {
        guard let data = try? Self.encoder().encode(list) else { return }
        try? FileManager.default.createDirectory(at: libraryURL, withIntermediateDirectories: true)
        try? AtomicFile.write(data, to: recentsURL)
    }

    private func touchRecent(_ ref: ProjectReference, name: String) {
        var list = readRecents()
        list.touch(path: ref.url.standardizedFileURL.path, name: name, at: now())
        writeRecents(list)
    }
}
