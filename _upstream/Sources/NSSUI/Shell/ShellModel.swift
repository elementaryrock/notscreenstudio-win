import AppKit
import CoreGraphics
import Foundation
import NSSCapture
import Observation
import NSSCore

/// The modules the shell talks to, all through NSSCore protocols so any can be swapped for the real one.
public struct ShellServices: Sendable {
    public var session: any RecordingSession
    public var inputLogger: any InputLogger
    public var store: any ProjectStore
    public var sources: any CaptureSourceProviding
    public var permissions: any PermissionsProviding

    public init(
        session: any RecordingSession,
        inputLogger: any InputLogger,
        store: any ProjectStore,
        sources: any CaptureSourceProviding,
        permissions: any PermissionsProviding
    ) {
        self.session = session
        self.inputLogger = inputLogger
        self.store = store
        self.sources = sources
        self.permissions = permissions
    }
}

/// A recording that was just saved, ready for the editor.
public struct SavedRecording: Sendable, Equatable {
    public var reference: ProjectReference
    public var project: Project
    public var cursorTrack: CursorTrack
}

/// View model for the whole shell: settings, the idle → recording → saving → editing cycle, sources and permissions.
@MainActor
@Observable
public final class ShellModel {
    // MARK: Observable state
    public private(set) var phase: ShellPhase = .idle
    public private(set) var settings: ShellSettings
    public private(set) var listing = SourceListing()
    public private(set) var microphones: [DeviceItem] = []
    public private(set) var cameras: [DeviceItem] = []
    public private(set) var permissionStates: [PermissionKind: PermissionState] = [:]
    /// Set when the source list could not be loaded (usually missing screen recording permission).
    public private(set) var sourceError: String?
    /// A non-fatal remark about the last start, for example "Camera skipped: permission denied".
    public private(set) var notice: String?
    public private(set) var lastSaved: SavedRecording?
    /// True while the 3-2-1 countdown is on screen.
    public private(set) var isCountingDown = false
    /// True while the control bar (source, camera, mic, Record) is on screen, before recording starts.
    public private(set) var isSettingUp = false
    /// True while a running recording is paused.
    public private(set) var isPaused = false
    /// Recorded seconds so far, not counting pauses. Refreshed about four times a second while recording.
    public private(set) var elapsed: TimeInterval = 0
    /// What the floating recording bar shows.
    @ObservationIgnored public let controls = RecordingControls()
    /// Newest project bundle in the library (from a previous run or the last recording), so the editor can reopen it.
    public private(set) var latestProject: ProjectReference?
    /// Increments each time the editor should open. Views watch it and call `openWindow`.
    public private(set) var editorRequestID = 0
    /// Increments each time the permissions window should open.
    public private(set) var onboardingRequestID = 0

    // MARK: Dependencies
    @ObservationIgnored private let services: ShellServices
    @ObservationIgnored private let effects: any ShellEffects
    @ObservationIgnored private let settingsStore: ShellSettingsStore
    @ObservationIgnored private let now: @MainActor () -> Date
    @ObservationIgnored private var active: ActiveRecording?
    @ObservationIgnored private var watchTask: Task<Void, Never>?
    @ObservationIgnored private var discarding = false
    @ObservationIgnored private var clock: ElapsedClock?
    @ObservationIgnored private var tickTask: Task<Void, Never>?
    /// The in-flight permission probe started by `beginSetup`. Internal so tests can await it.
    @ObservationIgnored var setupProbeTask: Task<Void, Never>?
    @ObservationIgnored private var screenRecordingProbed = false
    @ObservationIgnored private var activationObserver: NSObjectProtocol?

    struct ActiveRecording {
        var reference: ProjectReference
        var source: CaptureSource
        var sourceRect: CGRect?
        var cameraDeviceID: String?
        /// Chosen once at start, so the bundle name, recents entry and project name always agree.
        var name: String
    }

    public init(
        services: ShellServices,
        effects: any ShellEffects,
        settingsStore: ShellSettingsStore = ShellSettingsStore(),
        now: @escaping @MainActor () -> Date = { Date() }
    ) {
        self.services = services
        self.effects = effects
        self.settingsStore = settingsStore
        self.now = now
        self.settings = settingsStore.load()
    }

    // MARK: Derived

    public var isRecording: Bool { phase.isRecording }
    public var canStart: Bool { phase.canStart && !discarding }

    public var missingRequiredPermissions: [PermissionKind] {
        PermissionKind.allCases.filter { PermissionInfo.isRequired($0) && permissionStates[$0] != .granted }
    }

    /// SF Symbol for the menu bar icon in the current phase.
    public var menuBarSymbol: String {
        switch phase {
        case .idle, .editing: return "record.circle"
        case .preparing: return "record.circle"
        case .recording: return "record.circle.fill"
        case .saving: return "ellipsis.circle"
        case .failed: return "exclamationmark.triangle"
        }
    }

    /// Menu bar title while recording, for example "0:07" (or "Paused 0:07"). nil when not recording.
    public var menuBarTitle: String? {
        guard isRecording else { return nil }
        return ElapsedFormat.string(elapsed)
    }

    /// True when the capture layer can pause and resume cleanly.
    public var canPause: Bool { services.session is any PausableRecordingSession }

    // MARK: Settings

    public func updateSettings(_ change: (inout ShellSettings) -> Void) {
        var s = settings
        change(&s)
        guard s != settings else { return }
        settings = s
        settingsStore.save(s)
    }

    public func selectSource(_ source: CaptureSource?) { updateSettings { $0.source = source } }
    public func setMicEnabled(_ on: Bool) { updateSettings { $0.micEnabled = on } }
    public func selectMicrophone(_ id: String?) { updateSettings { $0.micDeviceID = id } }
    public func setCameraEnabled(_ on: Bool) { updateSettings { $0.cameraEnabled = on } }
    public func selectCamera(_ id: String?) { updateSettings { $0.cameraDeviceID = id } }
    public func selectShape(_ shape: CameraShape) {
        updateSettings { $0.cameraShape = shape }
        effects.updateBubble(shape: shape, roundness: settings.bubbleRoundness)
    }
    public func selectRoundness(_ roundness: Double) {
        updateSettings { $0.bubbleRoundness = min(max(roundness, 0), 1) }
        effects.updateBubble(shape: settings.cameraShape, roundness: settings.bubbleRoundness)
    }
    /// 0 (off), 3 or 5 seconds. Other values are ignored.
    public func setCountdown(_ seconds: Int) {
        guard ShellSettings.countdownChoices.contains(seconds) else { return }
        updateSettings { $0.countdownSeconds = seconds }
    }

    /// Lets the user drag a rectangle on screen, then remembers it as the source.
    public func chooseArea() async {
        guard canStart, let area = await effects.selectArea() else { return }
        selectSource(area.source)
        beginSetup()
    }

    // MARK: Recorder bar (before recording)

    /// Shows the control bar for the current source. Record on the bar starts the countdown.
    public func beginSetup() {
        // An open bar is repositioned over a newly picked source, so only a pending probe blocks a second call.
        guard canStart, setupProbeTask == nil else { return }
        refreshPermissions()
        if permissionStates[.screenRecording] == .granted {
            presentSetupBar()
            return
        }
        // The sync value is only a hint: confirm with ScreenCaptureKit before sending the user to onboarding.
        // One probe at a time, so quick double clicks cannot open two bars.
        setupProbeTask = Task { [weak self] in
            guard let self else { return }
            await self.refreshPermissionsAuthoritative()
            self.setupProbeTask = nil
            guard self.canStart else { return }
            if self.permissionStates[.screenRecording] == .granted { self.presentSetupBar() } else { self.requestOnboarding() }
        }
    }

    private func presentSetupBar() {
        isSettingUp = true
        effects.showSetupBar(model: self, sourceRect: setupSourceRect())
        Task { await refreshSources() }
    }

    /// Picks a source and opens the control bar for it.
    public func pickSource(_ source: CaptureSource?) {
        guard canStart else { return }
        selectSource(source)
        beginSetup()
    }

    /// The bar's close button: nothing is recorded.
    public func cancelSetup() {
        guard isSettingUp else { return }
        isSettingUp = false
        effects.hideSetupBar()
    }

    /// The bar's Record button: hides the bar and runs the countdown, then recording. If the countdown is
    /// cancelled the bar comes back, so the user can try again or close it.
    public func confirmRecord() async {
        guard isSettingUp, canStart else { return }
        isSettingUp = false
        effects.hideSetupBar()
        await startRecording()
        if case .idle = phase, !isRecording { beginSetup() }
    }

    private func setupSourceRect() -> CGRect? {
        let (source, _) = Self.resolve(settings.source, in: listing, fallbackDisplay: CGMainDisplayID())
        return SourceLocator.appKitRect(
            for: source, displays: listing.displays, windows: listing.windows, primaryHeight: effects.primaryScreenHeight)
    }

    // MARK: Pause

    public func pauseRecording() async {
        guard case .recording = phase, !isPaused, let session = services.session as? any PausableRecordingSession else { return }
        do { try await session.pause() } catch { notice = Self.describe(error); return }
        clock?.pause(at: now())
        setPaused(true)
    }

    public func resumeRecording() async {
        guard case .recording = phase, isPaused, let session = services.session as? any PausableRecordingSession else { return }
        do { try await session.resume() } catch { notice = Self.describe(error); return }
        clock?.resume(at: now())
        setPaused(false)
    }

    public func togglePause() async {
        if isPaused { await resumeRecording() } else { await pauseRecording() }
    }

    private func setPaused(_ paused: Bool) {
        isPaused = paused
        controls.isPaused = paused
        refreshElapsed()
    }

    private func refreshElapsed() {
        let value = clock?.elapsed(at: now()) ?? 0
        elapsed = value
        controls.elapsed = value
    }

    private func startTicking(from start: Date) {
        clock = ElapsedClock(startedAt: start)
        isPaused = false
        controls.isPaused = false
        controls.canPause = canPause
        refreshElapsed()
        tickTask?.cancel()
        tickTask = Task { @MainActor [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .milliseconds(250))
                self?.refreshElapsed()
            }
        }
    }

    private func stopTicking() {
        tickTask?.cancel()
        tickTask = nil
        clock = nil
        isPaused = false
        controls.isPaused = false
        elapsed = 0
        controls.elapsed = 0
    }

    // MARK: Sources and permissions

    /// Reads the quick (synchronous) state. For screen recording the hint is used only until the first
    /// authoritative probe; after that the probe result stays the source of truth and is never overwritten here.
    public func refreshPermissions() {
        for kind in PermissionKind.allCases where kind != .screenRecording {
            permissionStates[kind] = services.permissions.status(of: kind)
        }
        if !screenRecordingProbed { permissionStates[.screenRecording] = services.permissions.status(of: .screenRecording) }
    }

    /// Re-checks every permission, using the authoritative ScreenCaptureKit probe for screen recording.
    /// `userInitiated` is true only for a click on Allow or Check Again. Automatic callers may get the quick hint.
    public func refreshPermissionsAuthoritative(userInitiated: Bool = false) async {
        refreshPermissions()
        let state = await services.permissions.probe(.screenRecording, userInitiated: userInitiated)
        screenRecordingProbed = true
        permissionStates[.screenRecording] = state
    }

    /// The Check Again button.
    public func checkAgain() async { await refreshPermissionsAuthoritative(userInitiated: true) }

    /// One explicit click on Allow. Shows the system prompt at most once; it does not probe ScreenCaptureKit
    /// for screen recording (that could raise a second prompt), only reads the quick state.
    public func requestPermission(_ kind: PermissionKind) async {
        let requested = await services.permissions.request(kind)
        if kind == .screenRecording {
            if requested == .granted {
                permissionStates[.screenRecording] = .granted
                await refreshSources()
            } else if permissionStates[.screenRecording] != .granted {
                permissionStates[.screenRecording] = requested
            }
        } else {
            permissionStates[kind] = requested
        }
    }

    /// Polls every 2 seconds while screen recording is not granted. Returns when granted or cancelled.
    /// Views run it from `.task`, so it stops when they disappear. It reads only the quick state
    /// (`CGPreflightScreenCaptureAccess`, which never prompts) and runs the rate-limited probe once it turns true.
    public func watchScreenRecordingPermission() async {
        while !Task.isCancelled {
            if services.permissions.status(of: .screenRecording) == .granted {
                await refreshPermissionsAuthoritative()
                if permissionStates[.screenRecording] == .granted { return }
            }
            await pollSleep(.seconds(2))
        }
    }

    /// The wait between polls. Tests replace it so no wall clock is involved.
    @ObservationIgnored var pollSleep: @Sendable (Duration) async -> Void = { try? await Task.sleep(for: $0) }

    /// Re-checks whenever the app becomes active (for example after the user returns from System Settings).
    func observeActivation() {
        guard activationObserver == nil else { return }
        activationObserver = NotificationCenter.default.addObserver(
            forName: NSApplication.didBecomeActiveNotification, object: nil, queue: .main
        ) { [weak self] _ in
            Task { @MainActor [weak self] in
                guard let self else { return }
                await self.handleActivation()
            }
        }
    }

    /// One automatic, rate-limited probe per activation.
    func handleActivation() async {
        services.permissions.noteActivation()
        await refreshPermissionsAuthoritative()
        if permissionStates[.screenRecording] == .granted, sourceError != nil { await refreshSources() }
    }

    /// Relaunches the same bundle: macOS applies a new Screen Recording grant only to a new process.
    public func relaunchApp() { AppRelauncher.relaunch() }

    public func openSystemSettings(for kind: PermissionKind) { services.permissions.openSettings(for: kind) }

    public func refreshSources() async {
        refreshPermissions()
        microphones = services.sources.microphones()
        cameras = services.sources.cameras()
        do {
            listing = try await services.sources.loadSources()
            sourceError = nil
        } catch {
            sourceError = Self.describe(error)
        }
    }

    /// Call once at launch: reads permissions and asks to show onboarding if screen recording is missing.
    public func launch() async {
        observeActivation()
        latestProject = await newestLoadableProject()
        await refreshSources()
        if !missingRequiredPermissions.isEmpty { requestOnboarding() }
    }

    /// Newest bundle that really loads. A bundle left by a crash during recording (or a failed save) has no
    /// `project.json`, and must not hide older valid recordings.
    private func newestLoadableProject() async -> ProjectReference? {
        for ref in (try? await services.store.listProjects()) ?? [] {
            if (try? await services.store.load(ref)) != nil { return ref }
        }
        return nil
    }

    public func requestOnboarding() { onboardingRequestID += 1 }
    public func requestEditor() { if lastSaved != nil || latestProject != nil { editorRequestID += 1 } }
    public func dismissError() { apply(.dismissError) }
    public func dismissNotice() { notice = nil }
    /// Shows a non-fatal message in the menu, for example a setup problem found at launch.
    public func showNotice(_ text: String) { notice = text }

    // MARK: Recording

    /// The one-click action: starts with the last-used source and devices, no dialogs.
    public func startRecording() async {
        guard canStart else { return }
        cancelSetup()
        await refreshPermissionsAuthoritative()
        guard permissionStates[.screenRecording] == .granted else {
            requestOnboarding()
            return
        }
        guard apply(.startRequested) else { return }
        notice = nil
        var createdRef: ProjectReference?
        var loggerStarted = false
        do {
            if let fresh = try? await services.sources.loadSources() { listing = fresh }
            microphones = services.sources.microphones()
            cameras = services.sources.cameras()

            let (source, fellBack) = Self.resolve(settings.source, in: listing, fallbackDisplay: CGMainDisplayID())
            var notes: [String] = []
            if fellBack { notes.append("The saved source is gone, recording the main display.") }

            var micID: String?
            if settings.micEnabled {
                if !(await deviceAllowed(.microphone)) { notes.append("Microphone skipped: permission denied.") }
                else if let id = Self.pickDevice(preferred: settings.micDeviceID, from: microphones) { micID = id }
                else { notes.append("Microphone skipped: none found.") }
            }
            var camID: String?
            if settings.cameraEnabled {
                if !(await deviceAllowed(.camera)) { notes.append("Camera skipped: permission denied.") }
                else if let id = Self.pickDevice(preferred: settings.cameraDeviceID, from: cameras) { camID = id }
                else { notes.append("Camera skipped: none found.") }
            }
            notice = notes.isEmpty ? nil : notes.joined(separator: " ")

            let name = Self.projectName(for: now())
            let ref = try await services.store.createProject(named: name)
            createdRef = ref
            // The logger starts first so no early movement is lost. `stop` rebases to the session's origin.
            try await services.inputLogger.start(source: source)
            loggerStarted = true
            let seconds = settings.countdownSeconds
            let options = RecordingOptions(source: source, micDeviceID: micID, cameraDeviceID: camID, countdownSeconds: seconds)
            let sourceRect = SourceLocator.appKitRect(
                for: source, displays: listing.displays, windows: listing.windows, primaryHeight: effects.primaryScreenHeight)
            active = ActiveRecording(reference: ref, source: source, sourceRect: sourceRect, cameraDeviceID: camID, name: name)
            // `begin` runs the whole start sequence: preflight, then the camera bubble, then the countdown with
            // capture warming up in parallel, and capture starts as the countdown ends. It throws
            // `StartCancelled` if the user cancels during the countdown.
            try await begin(options: options, into: ref.url, countdown: seconds, over: sourceRect)

            let startedAt = now()
            apply(.started(startedAt))
            startTicking(from: startedAt)
            showOverlays()
            watchSession()
            // The HUD now exists too: make sure the running stream excludes it (and anything else that appeared).
            if let warm = services.session as? any WarmableRecordingSession { Task { await warm.refreshExclusions() } }
        } catch {
            effects.cancelCountdown()
            isCountingDown = false
            effects.hideBubble()
            if loggerStarted { _ = await services.inputLogger.stop(recordingStartHostTime: 0) }
            if let createdRef { Self.removeBundle(createdRef, library: services.store.libraryURL) }
            active = nil
            if error is StartCancelled {
                apply(.startCancelled)
            } else {
                apply(.startFailed(Self.describe(error)))
                if case NSSError.permissionDenied = error { requestOnboarding() }
            }
        }
    }

    private struct StartCancelled: Error {}
    private final class FatalBox { var error: NSSError? }

    /// Counts down (if enabled) while the capture warms up, then starts recording at the end of the countdown.
    /// Sessions that cannot warm up simply start after the countdown. Throws `StartCancelled` if the user cancels.
    private func begin(options: RecordingOptions, into url: URL, countdown seconds: Int, over sourceRect: CGRect?) async throws {
        let area = sourceRect ?? CGRect(x: 0, y: 0, width: 1440, height: 900)
        guard let warm = services.session as? any WarmableRecordingSession else {
            // The bubble is shown before capture is prepared, so the capture filter is built with every
            // window of this app already on screen (and the user can see themselves during the countdown).
            showBubbleOverlay()
            if seconds > 0 {
                isCountingDown = true
                let done = await effects.runCountdown(seconds: seconds, over: area)
                isCountingDown = false
                if !done { throw StartCancelled() }
            }
            try await services.session.start(options: options, into: url)
            return
        }
        // Permissions first: no bubble or countdown appears for a recording that cannot start.
        try await warm.preflight(options: options)
        showBubbleOverlay()
        // A screen stream that dies during the countdown cuts it short and its real error is reported.
        let fatal = FatalBox()
        let events = warm.captureEvents
        let effects0 = self.effects
        let watcher = Task { @MainActor in
            for await e in events where e.isFatal {
                fatal.error = e.error
                effects0.cancelCountdown()
                return
            }
        }
        defer { watcher.cancel() }
        // Warm-up runs during the countdown. If it fails the countdown is cut short and the error is reported.
        let effects = self.effects
        let warmUp = Task { @MainActor () -> Error? in
            do { try await warm.prepare(options: options, into: url); return nil } catch {
                effects.cancelCountdown()
                return error
            }
        }
        var completed = true
        if seconds > 0 {
            isCountingDown = true
            completed = await effects.runCountdown(seconds: seconds, over: area)
            isCountingDown = false
        }
        if let failure = await warmUp.value { throw failure }
        if let error = fatal.error {
            await warm.cancelPrepared()
            throw error
        }
        if !completed {
            await warm.cancelPrepared()
            throw StartCancelled()
        }
        try await warm.beginRecording()
    }

    /// Cancels the countdown. The recording never starts and nothing is saved.
    public func cancelCountdown() {
        guard isCountingDown else { return }
        effects.cancelCountdown()
    }

    public func stopRecording() async {
        guard case .recording = phase, let active, apply(.stopRequested) else { return }
        watchTask?.cancel()
        let bubble = captureBubble(for: active)
        hideOverlays()
        self.active = nil
        do {
            let saved = try await finish(active, bubble: bubble)
            apply(.saved(saved.reference))
            editorRequestID += 1
        } catch {
            // `finish` already cleaned up for a failed stop (bundle removed) and kept the bundle for a failed save.
            apply(.saveFailed(Self.describe(error)))
        }
    }

    /// True if the device's permission is granted. An undecided permission is asked for here, before the
    /// session starts, so the system prompt never interrupts `session.start`. Declined or denied means skip the device.
    private func deviceAllowed(_ kind: PermissionKind) async -> Bool {
        var state = services.permissions.status(of: kind)
        if state == .notDetermined { state = await services.permissions.request(kind) }
        permissionStates[kind] = state
        return state == .granted
    }

    /// Stops the session (finalizing every file), stops the logger, and saves the project and cursor track.
    private func finish(_ active: ActiveRecording, bubble: (center: NormalizedPoint, size: Double)?) async throws -> SavedRecording {
        let assets: RecordingAssets
        do {
            assets = try await services.session.stop()
        } catch {
            // The session deleted its own files, so the bundle is an empty shell. Do not leave a broken project behind.
            _ = await services.inputLogger.stop(recordingStartHostTime: 0)
            Self.removeBundle(active.reference, library: services.store.libraryURL)
            throw error
        }
        // From here the finished recording exists: a failed save keeps the bundle.
        var track = await services.inputLogger.stop(recordingStartHostTime: assets.startHostTime)
        // Paused time is left out of the video, so it is left out of the cursor track too.
        if let pausable = services.session as? any PausableRecordingSession {
            track = await pausable.pauseTimeline().removingPauses(from: track, origin: assets.startHostTime)
        }
        var style = Style()
        // The bubble was placed over the capture source; `Style.camera` is in output space (padding included).
        let geometry = bubble ?? (settings.bubbleCenter, settings.bubbleSize)
        let placed = BubbleGeometry.toOutputSpace(
            center: geometry.0, size: geometry.1,
            outputAspect: style.aspectRatio.ratio, sourceAspect: assets.sourcePixelSize.aspect, padding: style.padding)
        style.camera = Self.cameraBubble(hasCamera: assets.cameraFile != nil, shape: settings.cameraShape, roundness: settings.bubbleRoundness, geometry: placed)
        let project = Project(name: active.name, assets: assets, style: style)
        try await services.store.saveCursorTrack(track, for: active.reference, assets: assets)
        try await services.store.save(project, to: active.reference)
        let saved = SavedRecording(reference: active.reference, project: project, cursorTrack: track)
        lastSaved = saved
        latestProject = saved.reference
        return saved
    }

    /// Stops and throws the recording away, deleting its bundle.
    public func discardRecording() async {
        guard case .recording = phase, let active else { return }
        discarding = true
        defer { discarding = false }
        watchTask?.cancel()
        hideOverlays()
        self.active = nil
        apply(.discardRequested)
        _ = try? await services.session.stop()
        _ = await services.inputLogger.stop(recordingStartHostTime: 0)
        Self.removeBundle(active.reference, library: services.store.libraryURL)
    }

    public func toggleRecording() async {
        if phase.isRecording { await stopRecording() } else { await startRecording() }
    }

    // MARK: Internals

    @discardableResult
    private func apply(_ event: ShellEvent) -> Bool {
        guard let next = ShellStateMachine.reduce(phase, event) else { return false }
        phase = next
        return true
    }

    private func showOverlays() {
        controls.onStop = { [weak self] in Task { await self?.stopRecording() } }
        controls.onDiscard = { [weak self] in Task { await self?.discardRecording() } }
        controls.onTogglePause = { [weak self] in Task { await self?.togglePause() } }
        effects.showHUD(controls: controls, sourceRect: active?.sourceRect)
        effects.registerStopShortcut { [weak self] in Task { await self?.stopRecording() } }
    }

    /// Shows the live camera bubble with the same shape and roundness the editor and export will use.
    private func showBubbleOverlay() {
        guard let active, let camID = active.cameraDeviceID else { return }
        let source = active.sourceRect ?? CGRect(x: 0, y: 0, width: 1440, height: 900)
        let frame = BubbleGeometry.panelFrame(
            center: settings.bubbleCenter, size: settings.bubbleSize, shape: settings.cameraShape, in: source)
        effects.showBubble(
            deviceID: camID, shape: settings.cameraShape, roundness: settings.bubbleRoundness, frame: frame, sourceRect: source)
    }

    private func hideOverlays() {
        stopTicking()
        effects.hideHUD()
        effects.hideBubble()
        effects.unregisterStopShortcut()
    }

    /// Reads where the user left the bubble, saves it as the new default, and returns it.
    private func captureBubble(for active: ActiveRecording) -> (center: NormalizedPoint, size: Double)? {
        guard active.cameraDeviceID != nil, let frame = effects.bubbleFrame(), let rect = active.sourceRect,
              let result = BubbleGeometry.normalize(panelFrame: frame, in: rect)
        else { return nil }
        updateSettings { $0.bubbleCenter = result.center; $0.bubbleSize = result.size }
        return result
    }

    /// The screen stream can end on its own (window closed, display unplugged).
    private func watchSession() {
        watchTask?.cancel()
        let stream = services.session.stateUpdates
        watchTask = Task { [weak self] in
            for await state in stream {
                guard let self, !Task.isCancelled else { return }
                if case .failed(let message) = state, self.phase.isRecording {
                    await self.captureEnded(message)
                    return
                }
            }
        }
    }

    /// The screen stream ended on its own. Stop the session so the mic and camera stop and every file is
    /// finalized, then keep whatever was recorded (or remove the bundle if nothing usable exists).
    private func captureEnded(_ message: String) async {
        guard let active else { return }
        watchTask = nil
        let bubble = captureBubble(for: active)
        hideOverlays()
        self.active = nil
        // The phase changes only after the session is stopped and saved, so `canStart` stays false until then.
        defer { apply(.captureFailed(message)) }
        do {
            let saved = try await finish(active, bubble: bubble)
            notice = "Recording stopped early. The partial recording was saved as \(saved.project.name)."
        } catch {
            // `finish` removes the bundle only when the session produced nothing. If saving the project failed,
            // the finished recording stays on disk, so say where.
            if FileManager.default.fileExists(atPath: active.reference.url.path) {
                notice = "Recording stopped early. The files are kept in \(active.reference.url.lastPathComponent), but the project could not be saved."
            }
        }
    }

    // MARK: Pure helpers (tested)

    /// Picks the source to record, falling back to the main display when the saved one no longer exists.
    static func resolve(
        _ saved: CaptureSource?, in listing: SourceListing, fallbackDisplay: UInt32
    ) -> (source: CaptureSource, fellBack: Bool) {
        let mainID = listing.displays.first(where: \.isMain)?.id ?? listing.displays.first?.id ?? fallbackDisplay
        guard let saved else { return (.display(id: mainID), false) }
        switch saved {
        case .display(let id):
            return listing.displays.contains { $0.id == id } || listing.displays.isEmpty
                ? (saved, false) : (.display(id: mainID), true)
        case .window(let id):
            return listing.windows.contains { $0.id == id } ? (saved, false) : (.display(id: mainID), true)
        case .area(let displayID, _):
            return listing.displays.contains { $0.id == displayID } ? (saved, false) : (.display(id: mainID), true)
        }
    }

    /// The preferred device if it still exists, else the system default, else the first one. nil if there are none.
    static func pickDevice(preferred: String?, from devices: [DeviceItem]) -> String? {
        if let preferred, devices.contains(where: { $0.id == preferred }) { return preferred }
        return (devices.first(where: \.isDefault) ?? devices.first)?.id
    }

    static func cameraBubble(
        hasCamera: Bool, shape: CameraShape, roundness: Double = CameraBubble().roundness,
        geometry: (center: NormalizedPoint, size: Double)
    ) -> CameraBubble {
        CameraBubble(visible: hasCamera, shape: shape, position: geometry.center, size: geometry.size, roundness: roundness)
    }

    static func projectName(for date: Date) -> String {
        let f = DateFormatter()
        f.locale = Locale(identifier: "en_US_POSIX")
        f.dateFormat = "yyyy-MM-dd 'at' HH.mm.ss"
        return "Recording \(f.string(from: date))"
    }

    /// Deletes a bundle this app created. Refuses anything that is not a `.nssproj` directly inside the library.
    @discardableResult
    static func removeBundle(_ ref: ProjectReference, library: URL) -> Bool {
        let url = ref.url.standardizedFileURL
        guard url.pathExtension == ProjectReference.fileExtension,
              url.deletingLastPathComponent().standardizedFileURL == library.standardizedFileURL
        else { return false }
        return (try? FileManager.default.removeItem(at: url)) != nil
    }

    static func describe(_ error: Error) -> String {
        switch error {
        case NSSError.permissionDenied(let m), NSSError.unsupported(let m), NSSError.invalidProject(let m),
             NSSError.renderFailed(let m), NSSError.io(let m), NSSError.notImplemented(let m):
            return m
        default: return error.localizedDescription
        }
    }
}

public enum ElapsedFormat {
    /// "0:07", "12:03", "1:02:03".
    public static func string(_ seconds: TimeInterval) -> String {
        let total = max(0, Int(seconds))
        let h = total / 3600, m = (total % 3600) / 60, s = total % 60
        return h > 0 ? String(format: "%d:%02d:%02d", h, m, s) : String(format: "%d:%02d", m, s)
    }
}
