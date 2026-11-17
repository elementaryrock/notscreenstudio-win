import CoreGraphics
import Foundation
import NSSCapture
import NSSCore

// MARK: - Value types the shell UI works with (decoupled from NSSCapture so tests need no capture stack)

public struct DisplayItem: Sendable, Equatable, Identifiable {
    public let id: UInt32
    public let name: String
    /// Global display points, origin top-left of the primary display.
    public let frame: CGRect
    public let isMain: Bool
    public init(id: UInt32, name: String, frame: CGRect, isMain: Bool) {
        self.id = id
        self.name = name
        self.frame = frame
        self.isMain = isMain
    }
}

public struct WindowItem: Sendable, Equatable, Identifiable {
    public let id: UInt32
    public let title: String
    public let appName: String
    /// Global display points, origin top-left of the primary display.
    public let frame: CGRect
    public init(id: UInt32, title: String, appName: String, frame: CGRect) {
        self.id = id
        self.title = title
        self.appName = appName
        self.frame = frame
    }
    public var displayName: String { title.isEmpty ? appName : "\(appName) – \(title)" }
}

public struct DeviceItem: Sendable, Equatable, Identifiable, Hashable {
    public let id: String
    public let name: String
    public let isDefault: Bool
    public init(id: String, name: String, isDefault: Bool = false) {
        self.id = id
        self.name = name
        self.isDefault = isDefault
    }
}

public struct SourceListing: Sendable, Equatable {
    public var displays: [DisplayItem]
    public var windows: [WindowItem]
    public init(displays: [DisplayItem] = [], windows: [WindowItem] = []) {
        self.displays = displays
        self.windows = windows
    }
}

// MARK: - Protocols

/// Lists what can be recorded and which devices exist.
public protocol CaptureSourceProviding: Sendable {
    /// Displays and windows. Throws `NSSError.permissionDenied` without screen recording permission.
    func loadSources() async throws -> SourceListing
    func microphones() -> [DeviceItem]
    func cameras() -> [DeviceItem]
}

public enum PermissionKind: String, Sendable, CaseIterable, Identifiable {
    case screenRecording, camera, microphone
    public var id: String { rawValue }
}

public enum PermissionState: Sendable, Equatable {
    case granted, denied, notDetermined
}

/// Checks, requests and explains the three permissions.
public protocol PermissionsProviding: Sendable {
    func status(of kind: PermissionKind) -> PermissionState
    /// Prompts if the system allows it. Returns the granted state afterwards.
    func request(_ kind: PermissionKind) async -> PermissionState
    /// Opens the System Settings pane where the user can fix a denied permission.
    func openSettings(for kind: PermissionKind)
    /// The authoritative check (for screen recording: ScreenCaptureKit). 
    /// The authoritative check. It can raise the macOS prompt for an unapproved app, so `userInitiated` is true
    /// only for a click on Allow or Check Again. Automatic callers (launch, activation) pass false and may be
    /// refused or rate limited, in which case the answer is the last real verdict, or the quick hint if none.
    func probe(_ kind: PermissionKind, userInitiated: Bool) async -> PermissionState
    /// The app became active: allows one more automatic probe.
    func noteActivation()
}

extension PermissionsProviding {
    public func probe(_ kind: PermissionKind) async -> PermissionState { await probe(kind, userInitiated: false) }
    public func noteActivation() {}
}

/// Pure helpers for permissions UI and the settings deep links.
public enum PermissionInfo {
    public static func title(_ kind: PermissionKind) -> String {
        switch kind {
        case .screenRecording: return "Screen Recording"
        case .camera: return "Camera"
        case .microphone: return "Microphone"
        }
    }

    public static func explanation(_ kind: PermissionKind) -> String {
        switch kind {
        case .screenRecording:
            return "Needed to record your screen. After you allow it, quit and reopen notscreenstudio once so macOS applies it."
        case .camera:
            return "Optional. Shows your webcam in the camera bubble and records it."
        case .microphone:
            return "Optional. Records your voice with the screen recording."
        }
    }

    /// `x-apple.systempreferences` deep link for the Privacy & Security pane of this permission.
    public static func settingsURL(_ kind: PermissionKind) -> URL {
        let anchor: String
        switch kind {
        case .screenRecording: anchor = "Privacy_ScreenCapture"
        case .camera: anchor = "Privacy_Camera"
        case .microphone: anchor = "Privacy_Microphone"
        }
        return URL(string: "x-apple.systempreferences:com.apple.preference.security?\(anchor)")!
    }

    /// Screen recording is required; camera and microphone only matter when enabled.
    public static func isRequired(_ kind: PermissionKind) -> Bool { kind == .screenRecording }
}

/// Screen-side effects of recording. The real implementation (`AppKitShellEffects`) owns the HUD,
/// the camera bubble panel, the global shortcut and the area-selection overlay. Tests use a fake.
@MainActor
public protocol ShellEffects: AnyObject {
    /// Shows the control bar (source, camera, microphone, Record) before recording. `sourceRect` is in AppKit
    /// coordinates and picks the screen the bar appears on.
    func showSetupBar(model: ShellModel, sourceRect: CGRect?)
    func hideSetupBar()
    /// Shows the small floating bar (red dot, elapsed time, pause, stop) while recording.
    func showHUD(controls: RecordingControls, sourceRect: CGRect?)
    func hideHUD()
    /// Show the live camera bubble over the source. `frame` is in AppKit screen coordinates.
    func showBubble(deviceID: String?, shape: CameraShape, roundness: Double, frame: CGRect, sourceRect: CGRect)
    /// Current bubble frame in AppKit screen coordinates, nil if no bubble is shown.
    func bubbleFrame() -> CGRect?
    func updateBubble(shape: CameraShape, roundness: Double)
    func hideBubble()
    /// Global shortcut that stops the recording. Registers `handler`, replacing any previous one.
    func registerStopShortcut(_ handler: @escaping @MainActor () -> Void)
    func unregisterStopShortcut()
    /// Shows a large `seconds … 1` countdown centred on `rect` (AppKit coordinates), one second per number.
    /// Returns true when it ran to the end, false when the user cancelled (Esc or a click) or `cancelCountdown()` was called.
    func runCountdown(seconds: Int, over rect: CGRect) async -> Bool
    func cancelCountdown()
    /// Full-screen overlay where the user drags a rectangle. nil if cancelled.
    func selectArea() async -> AreaSelection?
    /// Height of the primary display in points, for coordinate flips.
    var primaryScreenHeight: CGFloat { get }
}

public struct AreaSelection: Sendable, Equatable {
    public var displayID: UInt32
    /// In the display's points, origin top-left.
    public var rect: CGRect
    public init(displayID: UInt32, rect: CGRect) {
        self.displayID = displayID
        self.rect = rect
    }
    public var source: CaptureSource { .area(displayID: displayID, rect: rect) }
}

// MARK: - Real providers

/// `CaptureSourceCatalog` and `CaptureDevices` from NSSCapture.
public struct SystemSourceProvider: CaptureSourceProviding {
    public init() {}

    public func loadSources() async throws -> SourceListing {
        let list = try await CaptureSourceCatalog.load()
        return SourceListing(
            displays: list.displays.map { DisplayItem(id: $0.id, name: $0.name, frame: $0.frame, isMain: $0.isMain) },
            windows: list.windows.map { WindowItem(id: $0.id, title: $0.title, appName: $0.appName, frame: $0.frame) })
    }

    public func microphones() -> [DeviceItem] {
        CaptureDevices.microphones().map { DeviceItem(id: $0.id, name: $0.name, isDefault: $0.isDefault) }
    }

    public func cameras() -> [DeviceItem] {
        CaptureDevices.cameras().map { DeviceItem(id: $0.id, name: $0.name, isDefault: $0.isDefault) }
    }
}

/// `CapturePermissions` from NSSCapture, plus System Settings deep links.
public struct SystemPermissions: PermissionsProviding {
    public init() {}

    public func status(of kind: PermissionKind) -> PermissionState {
        switch kind {
        case .screenRecording:
            if CapturePermissions.screenRecordingGranted { return .granted }
            // The system gives no "not asked yet" answer for screen recording, so remember our own request.
            return CapturePermissions.screenRecordingAsked ? .denied : .notDetermined
        case .camera: return Self.map(CapturePermissions.cameraStatus())
        case .microphone: return Self.map(CapturePermissions.microphoneStatus())
        }
    }

    public func probe(_ kind: PermissionKind, userInitiated: Bool) async -> PermissionState {
        guard kind == .screenRecording else { return status(of: kind) }
        let access = await CapturePermissions.checkScreenRecording(userInitiated ? .user : .automatic)
        if access.isGranted { return .granted }
        return CapturePermissions.screenRecordingAsked ? .denied : .notDetermined
    }

    public func noteActivation() { CapturePermissions.noteAppActivation() }

    public func request(_ kind: PermissionKind) async -> PermissionState {
        switch kind {
        case .screenRecording:
            // One system prompt per click, and only if this signing identity has not asked yet. We do not probe
            // here: ScreenCaptureKit would raise a second prompt while the first is still showing. The model
            // follows up with a state read, and "Check again" or the next launch confirms the grant.
            _ = CapturePermissions.requestScreenRecording()
            return status(of: .screenRecording)
        case .camera: _ = await CapturePermissions.requestCamera()
        case .microphone: _ = await CapturePermissions.requestMicrophone()
        }
        return status(of: kind)
    }

    @MainActor private static func open(_ url: URL) { NSWorkspaceBridge.open(url) }

    public func openSettings(for kind: PermissionKind) {
        let url = PermissionInfo.settingsURL(kind)
        Task { @MainActor in Self.open(url) }
    }

    private static func map(_ s: CapturePermissions.Status) -> PermissionState {
        switch s {
        case .granted: return .granted
        case .denied: return .denied
        case .notDetermined: return .notDetermined
        }
    }
}

// MARK: - Stubs for tests and previews

public struct StubSourceProvider: CaptureSourceProviding {
    public var listing: SourceListing
    public var mics: [DeviceItem]
    public var cams: [DeviceItem]
    public var error: NSSError?
    public init(
        listing: SourceListing = SourceListing(displays: [DisplayItem(id: 1, name: "Main Display", frame: CGRect(x: 0, y: 0, width: 1440, height: 900), isMain: true)]),
        mics: [DeviceItem] = [], cams: [DeviceItem] = [], error: NSSError? = nil
    ) {
        self.listing = listing
        self.mics = mics
        self.cams = cams
        self.error = error
    }
    public func loadSources() async throws -> SourceListing {
        if let error { throw error }
        return listing
    }
    public func microphones() -> [DeviceItem] { mics }
    public func cameras() -> [DeviceItem] { cams }
}

public final class ProbeCounter: @unchecked Sendable {
    private let lock = NSLock()
    private var value = 0
    public init() {}
    public var count: Int { lock.lock(); defer { lock.unlock() }; return value }
    private var userValue = 0
    /// Probes that were started by a user click.
    public var userCount: Int { lock.lock(); defer { lock.unlock() }; return userValue }
    func increment() { lock.lock(); value += 1; lock.unlock() }
    func incrementUser() { lock.lock(); userValue += 1; lock.unlock() }
}

/// Inert permissions for tests and previews. Nothing here touches the system.
public struct StubPermissions: PermissionsProviding {
    public var states: [PermissionKind: PermissionState]
    /// What `request` answers for a kind. Kinds not listed answer with their current state.
    public var requestResults: [PermissionKind: PermissionState]
    /// What `probe` answers for a kind. Kinds not listed answer with their current state, so a test can make the
    /// authoritative answer differ from the quick `status` hint.
    public var probeResults: [PermissionKind: PermissionState]
    public init(
        states: [PermissionKind: PermissionState] = [.screenRecording: .granted, .camera: .granted, .microphone: .granted],
        requestResults: [PermissionKind: PermissionState] = [:],
        probeResults: [PermissionKind: PermissionState] = [:]
    ) {
        self.states = states
        self.requestResults = requestResults
        self.probeResults = probeResults
    }
    /// Counts `probe` calls so tests can check that quick repeated clicks run one probe.
    public let probeCounter = ProbeCounter()
    public func probe(_ kind: PermissionKind, userInitiated: Bool) async -> PermissionState {
        probeCounter.increment()
        if userInitiated { probeCounter.incrementUser() }
        return probeResults[kind] ?? status(of: kind)
    }
    public func status(of kind: PermissionKind) -> PermissionState { states[kind] ?? .notDetermined }
    public func request(_ kind: PermissionKind) async -> PermissionState { requestResults[kind] ?? status(of: kind) }
    public func openSettings(for kind: PermissionKind) {}
}
