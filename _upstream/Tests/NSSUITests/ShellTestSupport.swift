import CoreGraphics
import Foundation
import NSSCapture
import NSSCore
import NSSProject
@testable import NSSUI

/// Scripted `RecordingSession`. Also pausable, so pause paths can be tested.
final class ShellFakeSession: PausableRecordingSession, @unchecked Sendable {
    private let lock = NSLock()
    private var _state: RecordingState = .idle
    private var continuation: AsyncStream<RecordingState>.Continuation?
    private var _startedOptions: RecordingOptions?
    private var _startedBundle: URL?
    var failStart: NSSError?
    private var _stopCalls = 0
    var stopCalls: Int { lock.withLock { _stopCalls } }
    var failStop: NSSError?
    var stopDuration: MediaTime = 3
    var hostTime: Double = 42

    private var _timeline = PauseTimeline()
    private var _pauseCalls = 0
    private var _resumeCalls = 0
    var pauseCalls: Int { lock.withLock { _pauseCalls } }
    var resumeCalls: Int { lock.withLock { _resumeCalls } }
    /// Host times the fake uses for pause and resume, relative to `hostTime`.
    func pause() async throws { lock.withLock { _pauseCalls += 1; _timeline.pause(at: hostTime + 1) } }
    func resume() async throws { lock.withLock { _resumeCalls += 1; _timeline.resume(at: hostTime + 2) } }
    func pauseTimeline() async -> PauseTimeline { lock.withLock { _timeline } }

    var startedOptions: RecordingOptions? { lock.withLock { _startedOptions } }
    var startedBundle: URL? { lock.withLock { _startedBundle } }

    var stateUpdates: AsyncStream<RecordingState> {
        AsyncStream { c in
            lock.withLock { continuation = c; c.yield(_state) }
        }
    }
    var state: RecordingState { get async { lock.withLock { _state } } }

    func set(_ s: RecordingState) { lock.withLock { _state = s; continuation?.yield(s) } }

    func start(options: RecordingOptions, into bundleURL: URL) async throws {
        if let failStart { throw failStart }
        lock.withLock { _startedOptions = options; _startedBundle = bundleURL }
        set(.recording(startedAt: Date()))
    }

    func stop() async throws -> RecordingAssets {
        lock.withLock { _stopCalls += 1 }
        if let failStop { throw failStop }
        let camera = startedOptions?.cameraDeviceID != nil
        set(.finished)
        return RecordingAssets(
            cameraFile: camera ? "camera.mov" : nil, sourcePixelSize: PixelSize(width: 2880, height: 1800),
            frameRate: 60, duration: stopDuration, startHostTime: hostTime)
    }
}

final class ShellFakeLogger: InputLogger, @unchecked Sendable {
    private let lock = NSLock()
    private var _started: CaptureSource?
    private var _stopOrigin: Double?
    var startedSource: CaptureSource? { lock.withLock { _started } }
    var stopOrigin: Double? { lock.withLock { _stopOrigin } }
    func start(source: CaptureSource) async throws { lock.withLock { _started = source } }
    func stop(recordingStartHostTime: Double) async -> CursorTrack {
        lock.withLock { _stopOrigin = recordingStartHostTime }
        return CursorTrack(samples: [CursorSample(t: 0, x: 0.5, y: 0.5)])
    }
}

@MainActor
final class ShellFakeEffects: ShellEffects {
    var hudShown = false
    var bubbleShown = false
    var bubbleShape: CameraShape?
    var bubbleRoundness: Double?
    var countdownRuns: [Int] = []
    var countdownResult = true
    var countdownRect: CGRect?
    var cancelCountdownCalls = 0
    var onCountdown: (@MainActor () -> Void)?
    var shortcutRegistered = false
    var bubbleFrameValue: CGRect?
    var shownBubbleFrame: CGRect?
    var areaResult: AreaSelection?
    var primaryScreenHeight: CGFloat = 900
    var setupBarShown = false
    var setupBarShowCount = 0
    var setupBarSourceRect: CGRect?
    var hudControls: RecordingControls?
    var hudSourceRect: CGRect?

    func showSetupBar(model: ShellModel, sourceRect: CGRect?) {
        setupBarShown = true; setupBarShowCount += 1; setupBarSourceRect = sourceRect
    }
    func hideSetupBar() { setupBarShown = false }
    func showHUD(controls: RecordingControls, sourceRect: CGRect?) {
        hudShown = true; hudControls = controls; hudSourceRect = sourceRect
    }
    func hideHUD() { hudShown = false }
    func showBubble(deviceID: String?, shape: CameraShape, roundness: Double, frame: CGRect, sourceRect: CGRect) {
        bubbleShown = true; bubbleShape = shape; bubbleRoundness = roundness; shownBubbleFrame = frame
        if bubbleFrameValue == nil { bubbleFrameValue = frame }
    }
    func bubbleFrame() -> CGRect? { bubbleShown ? bubbleFrameValue : nil }
    func updateBubble(shape: CameraShape, roundness: Double) { bubbleShape = shape; bubbleRoundness = roundness }
    func runCountdown(seconds: Int, over rect: CGRect) async -> Bool {
        countdownRuns.append(seconds); countdownRect = rect; onCountdown?()
        return countdownResult
    }
    func cancelCountdown() { cancelCountdownCalls += 1 }
    func hideBubble() { bubbleShown = false }
    func registerStopShortcut(_ handler: @escaping @MainActor () -> Void) { shortcutRegistered = true }
    func unregisterStopShortcut() { shortcutRegistered = false }
    func selectArea() async -> AreaSelection? { areaResult }
}

@MainActor
struct ShellRig {
    let model: ShellModel
    let session: ShellFakeSession
    let warm: ShellFakeWarmSession?
    let logger: ShellFakeLogger
    let effects: ShellFakeEffects
    let store: FileProjectStore
    let library: URL
    let probeCounter: ProbeCounter
    let defaults: UserDefaults
    let suite: String

    static let mainDisplay = DisplayItem(id: 7, name: "Built-in", frame: CGRect(x: 0, y: 0, width: 1440, height: 900), isMain: true)
    static let sideDisplay = DisplayItem(id: 8, name: "Side", frame: CGRect(x: 1440, y: 0, width: 1920, height: 1080), isMain: false)

    init(
        permissions: [PermissionKind: PermissionState] = [.screenRecording: .granted, .camera: .granted, .microphone: .granted],
        mics: [DeviceItem] = [DeviceItem(id: "mic1", name: "Mic", isDefault: true)],
        cams: [DeviceItem] = [DeviceItem(id: "cam1", name: "Cam", isDefault: true)],
        windows: [WindowItem] = [WindowItem(id: 55, title: "Doc", appName: "Notes", frame: CGRect(x: 100, y: 100, width: 800, height: 600))],
        requestResults: [PermissionKind: PermissionState] = [:],
        probeResults: [PermissionKind: PermissionState] = [:],
        initial: ShellSettings? = nil,
        warm useWarm: Bool = false,
        now: @escaping @MainActor () -> Date = { Date() }
    ) {
        suite = "shell-tests-\(UUID().uuidString)"
        defaults = UserDefaults(suiteName: suite)!
        library = FileManager.default.temporaryDirectory.appendingPathComponent("shell-tests-\(UUID().uuidString)")
        store = FileProjectStore(libraryURL: library)
        session = ShellFakeSession()
        warm = useWarm ? ShellFakeWarmSession() : nil
        logger = ShellFakeLogger()
        effects = ShellFakeEffects()
        let stub = StubPermissions(states: permissions, requestResults: requestResults, probeResults: probeResults)
        probeCounter = stub.probeCounter
        let settingsStore = ShellSettingsStore(defaults: defaults)
        if let initial { settingsStore.save(initial) }
        model = ShellModel(
            services: ShellServices(
                session: warm ?? session, inputLogger: logger, store: store,
                sources: StubSourceProvider(
                    listing: SourceListing(displays: [Self.mainDisplay, Self.sideDisplay], windows: windows),
                    mics: mics, cams: cams),
                permissions: stub),
            effects: effects, settingsStore: settingsStore, now: now)
    }

    func cleanup() {
        try? FileManager.default.removeItem(at: library)
        defaults.removePersistentDomain(forName: suite)
    }
}

/// A session that can warm up, recording the order of calls so tests can check the countdown sequence.
final class ShellFakeWarmSession: WarmableRecordingSession, @unchecked Sendable {
    private let lock = NSLock()
    private let inner = ShellFakeSession()
    private var _calls: [String] = []
    var failPrepare: NSSError?
    var failPreflight: NSSError?
    /// Emitted (on `prepare`) as a fatal capture event, to simulate the stream dying during the countdown.
    var fatalDuringPrepare: NSSError?
    private var eventContinuation: AsyncStream<CaptureEvent>.Continuation?
    var calls: [String] { lock.withLock { _calls } }
    var startedOptions: RecordingOptions? { inner.startedOptions }
    private func note(_ c: String) { lock.withLock { _calls.append(c) } }

    var stateUpdates: AsyncStream<RecordingState> { inner.stateUpdates }
    var state: RecordingState { get async { await inner.state } }
    func start(options: RecordingOptions, into bundleURL: URL) async throws {
        note("start"); try await inner.start(options: options, into: bundleURL)
    }
    var captureEvents: AsyncStream<CaptureEvent> {
        AsyncStream { c in lock.withLock { eventContinuation = c } }
    }
    func preflight(options: RecordingOptions) async throws {
        note("preflight")
        if let failPreflight { throw failPreflight }
    }
    func prepare(options: RecordingOptions, into bundleURL: URL) async throws {
        note("prepare")
        if let fatalDuringPrepare {
            lock.withLock { eventContinuation }?.yield(CaptureEvent(component: .screen, error: fatalDuringPrepare, isFatal: true))
        }
        if let failPrepare { throw failPrepare }
        try await inner.start(options: options, into: bundleURL)
    }
    func beginRecording() async throws { note("begin") }
    func cancelPrepared() async { note("cancelPrepared") }
    func refreshExclusions() async { note("refresh") }
    func stop() async throws -> RecordingAssets { note("stop"); return try await inner.stop() }
}
