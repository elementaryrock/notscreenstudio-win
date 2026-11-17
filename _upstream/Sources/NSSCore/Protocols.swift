import CoreMedia
import CoreVideo
import Foundation

/// Pure, deterministic map from time to per-frame state. Built from a `CursorTrack`,
/// `Timeline` and `CursorStyle`. MUST NOT depend on call order or wall-clock: calling
/// `frameState(at:)` with the same `t` always returns the same value, in any order, from any
/// thread. (Spring simulations are therefore integrated from t=0 with a fixed step, or
/// precomputed once at init.) Preview and export share one implementation so they match.
public protocol MotionModel: Sendable {
    func frameState(at t: MediaTime) -> FrameState
}

/// Builds a `MotionModel`. Lives in NSSCore so UI/Export code can depend on the factory shape only.
public typealias MotionModelFactory = @Sendable (_ track: CursorTrack, _ timeline: Timeline, _ cursorStyle: CursorStyle, _ duration: MediaTime) -> any MotionModel

/// Decodes frames of a recording. One instance per asset set. Implementations must be safe to call
/// from a single background queue; they own any AVAssetReader state and caching.
/// Owners (see PLAN.md): NSSExport provides the sequential `AVAssetReader`-based `AssetFrameSource`;
/// NSSUI/Editor provides the real-time `AVPlayerItemVideoOutput`-based source. Both MUST use the
/// at-or-before frame selection rule below. NSSRender ships `SolidColorFrameSource` as a stub.
public protocol FrameSource: Sendable {
    /// Decoded screen frame nearest at-or-before `t`. nil if unavailable.
    func screenFrame(at t: MediaTime) async -> CVPixelBuffer?
    /// Decoded camera frame nearest at-or-before `t` (same rule as `screenFrame(at:)`, so both consumers
    /// agree). nil when there is no camera track.
    func cameraFrame(at t: MediaTime) async -> CVPixelBuffer?
}

/// GPU compositor. DESIGN: pixel-buffer in, pixel-buffer out.
///
/// All three buffers should be IOSurface-backed, Metal-compatible `kCVPixelFormatType_32BGRA`
/// buffers; the renderer wraps them as `MTLTexture`s with a `CVMetalTextureCache` (zero copy).
/// The same call serves the preview (output buffer from a pool sized to the view) and export
/// (output buffer from the `AVAssetWriterInputPixelBufferAdaptor` pool). The renderer allocates
/// nothing per frame: pipelines, samplers and caches are built once at init. `render` is
/// synchronous: when it returns, `output` is fully written (it waits for the GPU).
/// Output size is dictated by `output`'s dimensions; `style.aspectRatio` informs layout only.
public protocol FrameRenderer: Sendable {
    func render(
        screen: CVPixelBuffer,
        camera: CVPixelBuffer?,
        state: FrameState,
        style: Style,
        into output: CVPixelBuffer
    ) throws
}

public enum NSSError: Error, Sendable, Equatable {
    case notImplemented(String)
    case unsupported(String)
    case permissionDenied(String)
    case invalidProject(String)
    case renderFailed(String)
    case io(String)
}

/// Create/load/save/autosave `.nssproj` bundles.
public protocol ProjectStore: Sendable {
    /// Default library folder, e.g. `~/Movies/notscreenstudio`.
    var libraryURL: URL { get }
    /// Create an empty bundle directory for a new recording and return its reference.
    func createProject(named name: String) async throws -> ProjectReference
    func load(_ ref: ProjectReference) async throws -> Project
    func save(_ project: Project, to ref: ProjectReference) async throws
    func loadCursorTrack(for ref: ProjectReference, assets: RecordingAssets) async throws -> CursorTrack
    func saveCursorTrack(_ track: CursorTrack, for ref: ProjectReference, assets: RecordingAssets) async throws
    /// Schedule a debounced autosave; later calls replace earlier pending ones.
    func scheduleAutosave(_ project: Project, to ref: ProjectReference)
    func listProjects() async throws -> [ProjectReference]
}

/// Records screen (+ optional mic/camera) into a project bundle directory.
public protocol RecordingSession: Sendable {
    /// Latest state plus subsequent changes. The stream yields the current state first.
    var stateUpdates: AsyncStream<RecordingState> { get }
    var state: RecordingState { get async }
    /// Start capturing into `bundleURL`. Writes screen.mov / camera.mov / mic.m4a there.
    func start(options: RecordingOptions, into bundleURL: URL) async throws
    /// Stop and finalize files; returns the assets (without cursor data, see `InputLogger`).
    /// `RecordingAssets.startHostTime` is the host-clock time of the first screen frame (the t = 0 origin).
    func stop() async throws -> RecordingAssets
}

/// Logs mouse movement (~120 Hz) and clicks, in normalized source space (see `Basics.swift`).
public protocol InputLogger: Sendable {
    /// `source` determines the normalization rect. The logger stamps every event with the host clock
    /// (`CACurrentMediaTime()` seconds) while running; it does not guess the recording origin.
    func start(source: CaptureSource) async throws
    /// Stops and returns the track REBASED by the logger: each `t` is (event host time minus
    /// `recordingStartHostTime`), so t = 0 is the first screen frame (`RecordingAssets.startHostTime`),
    /// same clock as `CACurrentMediaTime()`. Events before the origin are dropped or clamped to 0.
    func stop(recordingStartHostTime: Double) async -> CursorTrack
}

// MARK: - Project library (home screen)

/// One row on the home screen.
public struct ProjectSummary: Codable, Sendable, Equatable, Identifiable {
    public var id: UUID
    public var name: String
    public var createdAt: Date
    public var duration: MediaTime
    public var bundle: ProjectReference
    public var thumbnailURL: URL?
    public init(
        id: UUID = UUID(), name: String, createdAt: Date = Date(), duration: MediaTime,
        bundle: ProjectReference, thumbnailURL: URL? = nil
    ) {
        self.id = id
        self.name = name
        self.createdAt = createdAt
        self.duration = duration
        self.bundle = bundle
        self.thumbnailURL = thumbnailURL
    }
}


public protocol ProjectLibrary: Sendable {
    /// All projects, newest first.
    func summaries() async throws -> [ProjectSummary]
    /// Thumbnail for a project, generating it if missing. nil if it cannot be made.
    func thumbnailURL(for summary: ProjectSummary) async -> URL?
    /// Moves the project bundle to the Trash.
    func delete(_ summary: ProjectSummary) async throws
    func rename(_ summary: ProjectSummary, to name: String) async throws
}

/// In-memory `ProjectLibrary` for tests and previews.
public actor InMemoryProjectLibrary: ProjectLibrary {
    private var items: [ProjectSummary]
    public init(_ items: [ProjectSummary] = []) { self.items = items }

    public func summaries() async throws -> [ProjectSummary] {
        items.sorted { $0.createdAt > $1.createdAt }
    }
    public func thumbnailURL(for summary: ProjectSummary) async -> URL? {
        items.first { $0.id == summary.id }?.thumbnailURL
    }
    public func delete(_ summary: ProjectSummary) async throws {
        items.removeAll { $0.id == summary.id }
    }
    public func rename(_ summary: ProjectSummary, to name: String) async throws {
        guard let i = items.firstIndex(where: { $0.id == summary.id }) else { return }
        items[i].name = name
    }
}
