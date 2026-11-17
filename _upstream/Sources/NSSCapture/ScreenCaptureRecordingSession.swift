import AVFoundation
import CoreMedia
import Foundation
import NSSCore
@preconcurrency import ScreenCaptureKit

/// A session that can warm up (open the stream and devices) and start recording later, so a countdown can
/// run in between. Frames are held back until `beginRecording()`, which makes that instant t = 0.
public protocol WarmableRecordingSession: RecordingSession {
    /// Checks (and asks for) the permissions the options need. Call before showing any recording UI.
    func preflight(options: RecordingOptions) async throws
    /// Runtime problems, including the screen stream ending while the countdown runs.
    var captureEvents: AsyncStream<CaptureEvent> { get }
    /// Gets everything ready without recording. Throws if the sources cannot be captured.
    func prepare(options: RecordingOptions, into bundleURL: URL) async throws
    /// Starts recording now. Returns once the first frame is written (the origin is set).
    func beginRecording() async throws
    /// Abandons a prepared session and removes its files.
    func cancelPrepared() async
    /// Re-reads the shareable content and updates the running filter, so windows that appeared after the
    /// stream started (HUD, camera bubble) stay excluded. Safe to call at any time.
    func refreshExclusions() async
}

/// Real `RecordingSession`: screen (SCStream) + optional mic and camera (AVCaptureSession), written
/// into a bundle directory. All files share one time origin: the host-clock PTS of the first screen frame.
public actor ScreenCaptureRecordingSession: WarmableRecordingSession, PausableRecordingSession {
    private let hub = StateHub()
    private let events = EventHub()

    private var screen: ScreenRecorder?
    private var mic: DeviceRecorder?
    private var camera: DeviceRecorder?
    private var origin = SharedOrigin()
    private var pauseClock = PauseClock()
    private var plan: ScreenStreamPlan?
    private var files: CaptureFilePlan?
    private var fatalError: NSSError?
    private var screenEndedEarly = false
    private var bundleURL: URL?
    /// Files created by the current attempt only. Cleanup never touches anything else.
    private var createdFiles: [URL] = []
    /// True between `prepare` and `beginRecording`.
    private var awaitingBegin = false
    private var lastPlan: FilterPlan?
    private var lastSource: CaptureSource?
    private let ownWindowIDs: @Sendable () async -> [UInt32]

    /// `ownWindowIDs` reports this app's window numbers (`NSWindow.windowNumber`). They are a fallback for
    /// excluding windows when the owning process cannot be matched.
    public init(ownWindowIDs: @escaping @Sendable () async -> [UInt32] = { [] }) {
        self.ownWindowIDs = ownWindowIDs
    }

    public nonisolated var stateUpdates: AsyncStream<RecordingState> { hub.stream() }
    /// Runtime problems (window closed, device unplugged, disk full).
    public nonisolated var captureEvents: AsyncStream<CaptureEvent> { events.stream() }
    public var state: RecordingState { hub.current }

    public func start(options: RecordingOptions, into bundleURL: URL) async throws {
        try await prepare(options: options, into: bundleURL, armed: true)
        hub.set(.recording(startedAt: Date()))
    }

    public func preflight(options: RecordingOptions) async throws {
        do { try await CapturePermissions.ensure(for: options) } catch { throw CaptureErrorMapper.map(error) }
    }

    public func prepare(options: RecordingOptions, into bundleURL: URL) async throws {
        try await prepare(options: options, into: bundleURL, armed: false)
        awaitingBegin = true
    }

    private func prepare(options: RecordingOptions, into bundleURL: URL, armed: Bool) async throws {
        switch hub.current {
        case .preparing, .recording, .stopping:
            throw NSSError.unsupported("A recording is already in progress.")
        default: break
        }
        hub.set(.preparing)
        await discardRecorders() // recorders left over from a failed run
        files = nil; plan = nil; self.bundleURL = nil; createdFiles = []
        fatalError = nil
        awaitingBegin = false
        do {
            try await begin(options: options, bundleURL: bundleURL, armed: armed)
        } catch {
            await teardownAfterFailedStart()
            let mapped = CaptureErrorMapper.map(error)
            hub.set(.failed(message: Self.message(mapped)))
            throw mapped
        }
    }

    public func beginRecording() async throws {
        guard awaitingBegin, let recorder = screen else {
            throw NSSError.unsupported("No prepared recording to start.")
        }
        awaitingBegin = false
        do {
            // The stream may have died during the countdown: report that error, not a generic one.
            if let fatalError { throw fatalError }
            recorder.arm()
            try await waitForFirstFrame(recorder)
            hub.set(.recording(startedAt: Date()))
        } catch {
            await teardownAfterFailedStart()
            let mapped = CaptureErrorMapper.map(error)
            hub.set(.failed(message: Self.message(mapped)))
            throw mapped
        }
    }

    public func pause() async throws {
        guard case .recording = hub.current else { throw NSSError.unsupported("No recording is in progress.") }
        pauseClock.pause(at: CaptureMath.hostNow())
    }

    public func resume() async throws {
        guard case .recording = hub.current else { throw NSSError.unsupported("No recording is in progress.") }
        if let screen { screen.resume() } else { pauseClock.resume(at: CaptureMath.hostNow()) }
    }

    public func pauseTimeline() async -> PauseTimeline { pauseClock.snapshot }

    public func cancelPrepared() async {
        guard awaitingBegin else { return }
        awaitingBegin = false
        await teardownAfterFailedStart()
        hub.set(.idle)
    }

    public func refreshExclusions() async {
        guard let recorder = screen, let source = lastSource, let current = lastPlan, !current.isWindow else { return }
        do {
            let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: false)
            let snapshot = CaptureFilter.snapshot(of: content)
            let updated = try CaptureFilter.plan(
                source: source, snapshot: snapshot, ownPID: ProcessInfo.processInfo.processIdentifier,
                ownBundleID: Bundle.main.bundleIdentifier, ownWindowIDs: await ownWindowIDs())
            let filter = try CaptureFilter.makeFilter(updated, content: content)
            try await recorder.updateFilter(filter)
            lastPlan = updated
            CaptureDebug.log(CaptureDebug.describe(
                updated, snapshot: snapshot, ownPID: ProcessInfo.processInfo.processIdentifier, filterBuiltFrom: "updateContentFilter"))
        } catch {
            CaptureDebug.log("updateContentFilter failed: \(error)")
        }
    }

    private func waitForFirstFrame(_ recorder: ScreenRecorder) async throws {
        // Give the first frame a moment so the caller returns with the origin already set.
        for _ in 0..<300 where !recorder.hasFirstFrame {
            if let fatalError { throw fatalError }
            try? await Task.sleep(for: .milliseconds(10))
        }
        if let fatalError { throw fatalError }
        if !recorder.hasFirstFrame {
            throw NSSError.io("No screen frames arrived. Check that the selected source is visible and try again.")
        }
    }

    /// Stops any recorders still held (best effort) without deleting files.
    private func discardRecorders() async {
        let (s, m, c) = (screen, mic, camera)
        screen = nil; mic = nil; camera = nil
        let now = CaptureMath.hostNow()
        if let s { _ = try? await s.stop(endHost: now) }
        if let m { _ = try? await m.stop(endHost: now) }
        if let c { _ = try? await c.stop(endHost: now) }
    }

    private func begin(options: RecordingOptions, bundleURL: URL, armed: Bool) async throws {
        try await CapturePermissions.ensure(for: options)
        do {
            try FileManager.default.createDirectory(at: bundleURL, withIntermediateDirectories: true)
        } catch {
            throw CaptureErrorMapper.map(error, context: .writer)
        }

        self.bundleURL = bundleURL
        origin = SharedOrigin()
        pauseClock = PauseClock()
        screenEndedEarly = false
        let files = CaptureFilePlan(options: options)
        self.files = files

        let content: SCShareableContent
        do {
            content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: false)
        } catch {
            throw CaptureErrorMapper.map(error)
        }
        // Built here, after the shell has shown its countdown, bubble and HUD windows, from content fetched now.
        let snapshot = CaptureFilter.snapshot(of: content)
        let ownPID = ProcessInfo.processInfo.processIdentifier
        let filterPlan = try CaptureFilter.plan(
            source: options.source, snapshot: snapshot, ownPID: ownPID, ownBundleID: Bundle.main.bundleIdentifier,
            ownWindowIDs: await ownWindowIDs())
        let filter = try CaptureFilter.makeFilter(filterPlan, content: content)
        lastPlan = filterPlan
        lastSource = options.source
        CaptureDebug.log(CaptureDebug.describe(filterPlan, snapshot: snapshot, ownPID: ownPID, filterBuiltFrom: "initial"))
        let info = SCShareableContent.info(for: filter)
        let plan = try ScreenStreamPlan.make(
            contentPoints: info.contentRect.size, scale: Double(info.pointPixelScale),
            area: filterPlan.area, isWindow: filterPlan.isWindow, systemAudio: options.captureSystemAudio)
        self.plan = plan

        let box = WeakSession(self)
        let report: @Sendable (CaptureEvent.Component, Bool) -> @Sendable (NSSError) -> Void = { component, fatal in
            { error in
                let event = CaptureEvent(component: component, error: error, isFatal: fatal)
                Task { await box.session?.handle(event) }
            }
        }

        // Devices first (they warm up slowly). Their samples are dropped until the origin exists.
        if let id = options.micDeviceID, let name = files.mic {
            createdFiles.append(files.url(name, in: bundleURL))
            let r = try DeviceRecorder(kind: .microphone, deviceID: id, url: files.url(name, in: bundleURL), origin: origin, pause: pauseClock, onFailure: report(.microphone, false))
            try await r.start()
            mic = r
        }
        if let id = options.cameraDeviceID, let name = files.camera {
            createdFiles.append(files.url(name, in: bundleURL))
            let r = try DeviceRecorder(kind: .camera, deviceID: id, url: files.url(name, in: bundleURL), origin: origin, pause: pauseClock, onFailure: report(.camera, false))
            try await r.start()
            camera = r
        }
        createdFiles.append(files.url(files.screen, in: bundleURL))
        let recorder = try ScreenRecorder(
            filter: filter, plan: plan, url: files.url(files.screen, in: bundleURL),
            origin: origin, pause: pauseClock, armed: armed, onFailure: report(.screen, true))
        screen = recorder
        try await recorder.start()

        if armed { try await waitForFirstFrame(recorder) }
    }

    private func handle(_ event: CaptureEvent) {
        events.send(event)
        if event.isFatal {
            fatalError = event.error
            screenEndedEarly = true
            if case .recording = hub.current { hub.set(.failed(message: Self.message(event.error))) }
        }
    }

    public func stop() async throws -> RecordingAssets {
        // Serialize against an in-flight start().
        while case .preparing = hub.current, !awaitingBegin { try? await Task.sleep(for: .milliseconds(10)) }
        if awaitingBegin { throw NSSError.unsupported("The recording has not started yet.") }
        // Take ownership synchronously (no await before this point) so a second stop() finds nothing.
        guard let screen, let plan, let files else {
            throw NSSError.unsupported("No recording is in progress.")
        }
        let (mic, camera) = (self.mic, self.camera)
        let created = createdFiles
        self.screen = nil; self.mic = nil; self.camera = nil
        hub.set(.stopping)
        let stopHost = CaptureMath.hostNow()

        var screenResult: ScreenResult?
        var screenError: Error?
        do { screenResult = try await screen.stop(endHost: stopHost) } catch { screenError = error }

        var micHas = false, camHas = false
        if let mic { micHas = (try? await mic.stop(endHost: stopHost)) ?? false }
        if let camera { camHas = (try? await camera.stop(endHost: stopHost)) ?? false }
        // A broken mic/camera file never costs the screen recording.

        guard let result = screenResult, result.frames > 0, let o = origin.current else {
            let err = fatalError ?? screenError.map { CaptureErrorMapper.map($0, context: .writer) }
                ?? NSSError.io("No screen frames were captured.")
            for u in created { try? FileManager.default.removeItem(at: u) }
            hub.set(.failed(message: Self.message(err)))
            throw err
        }
        let assets = RecordingAssets(
            screenFile: files.screen,
            cameraFile: camHas ? files.camera : nil,
            micFile: micHas ? files.mic : nil,
            sourcePixelSize: plan.pixelSize,
            frameRate: Double(plan.frameRate),
            duration: o.duration(endingAt: result.endHost),
            startHostTime: o.hostTime)
        if !micHas, let f = files.mic, let d = bundleURL { try? FileManager.default.removeItem(at: files.url(f, in: d)) }
        if !camHas, let f = files.camera, let d = bundleURL { try? FileManager.default.removeItem(at: files.url(f, in: d)) }
        hub.set(.finished)
        return assets
    }

    private func teardownAfterFailedStart() async {
        await discardRecorders()
        for u in createdFiles { try? FileManager.default.removeItem(at: u) }
        createdFiles = []
    }

    static func message(_ e: NSSError) -> String {
        switch e {
        case .notImplemented(let m), .unsupported(let m), .permissionDenied(let m),
             .invalidProject(let m), .renderFailed(let m), .io(let m): return m
        }
    }
}

final class WeakSession: @unchecked Sendable {
    weak var session: ScreenCaptureRecordingSession?
    init(_ s: ScreenCaptureRecordingSession) { session = s }
}
