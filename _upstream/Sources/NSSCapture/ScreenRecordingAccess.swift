import CoreGraphics
import Foundation
import os
@preconcurrency import ScreenCaptureKit
import Security

/// What `SCShareableContent` said when asked for the shareable content.
public enum ShareableOutcome: Sendable, Equatable {
    case ok
    case failed(domain: String, code: Int)
}

/// Result of one Screen Recording check. `SCShareableContent` is authoritative; `CGPreflightScreenCaptureAccess`
/// is only a fast hint, because it can say false for a process that can in fact capture.
public struct ScreenRecordingAccess: Sendable, Equatable {
    public var preflight: Bool
    /// nil when the authoritative probe was not run.
    public var shareable: ShareableOutcome?

    public init(preflight: Bool, shareable: ShareableOutcome?) {
        self.preflight = preflight
        self.shareable = shareable
    }

    public var isGranted: Bool {
        switch shareable {
        case .ok: return true
        case .failed: return false
        case nil: return preflight
        }
    }

    /// A clear message for the user when not granted, nil when granted.
    public var message: String? {
        guard !isGranted else { return nil }
        guard case .failed(let domain, let code)? = shareable else {
            return Self.settingsHint
        }
        return Self.message(domain: domain, code: code)
    }

    static let settingsHint = "Screen recording permission is required. Enable notscreenstudio in System Settings > Privacy & Security > Screen & System Audio Recording, then relaunch the app."

    /// Maps the error from `SCShareableContent` to a clear sentence.
    public static func message(domain: String, code: Int) -> String {
        if domain == SCStreamErrorDomain {
            switch SCStreamError.Code(rawValue: code) {
            case .userDeclined:
                return settingsHint
            case .failedApplicationConnectionInvalid, .failedApplicationConnectionInterrupted:
                return "macOS could not reach the screen capture service (error \(code)). Relaunch the app and try again."
            case .noDisplayList, .noWindowList, .noCaptureSource:
                return "No displays or windows are available to capture right now (error \(code))."
            case .missingEntitlements:
                return "This build is not allowed to use screen capture (missing entitlements, error \(code))."
            default:
                break
            }
        }
        if domain == NSCocoaErrorDomain && code == NSUserCancelledError {
            return "Screen recording was cancelled."
        }
        return "Screen recording could not be checked (\(domain) \(code)). If it is turned on in System Settings, relaunch the app."
    }

    /// The decision, with both checks injected so it needs no hardware or permission.
    ///
    /// `SCShareableContent` is TCC-gated and can itself show the system prompt for an app with no decision
    /// yet, so it only runs when the preflight is already true or the user has asked for the permission once
    /// (`asked`). Otherwise the result is the preflight hint alone.
    public static func evaluate(
        asked: Bool,
        preflight: @Sendable () -> Bool,
        shareable: @Sendable () async -> ShareableOutcome
    ) async -> ScreenRecordingAccess {
        let hint = preflight()
        guard hint || asked else { return ScreenRecordingAccess(preflight: hint, shareable: nil) }
        let outcome = await shareable()
        return ScreenRecordingAccess(preflight: hint, shareable: outcome)
    }
}

extension ScreenRecordingAccess {
    /// The answer when the gate refuses a probe. A real granted verdict is never downgraded by a false preflight
    /// (preflight can stay false in a process that can in fact capture); with no verdict yet the hint decides.
    static func refused(preflight: Bool, lastGranted: Bool?) -> ScreenRecordingAccess {
        ScreenRecordingAccess(preflight: preflight, shareable: lastGranted == true ? .ok : nil)
    }
}

/// Last verdict of the authoritative check, shared so synchronous callers can use it as a hint.
enum ScreenRecordingCache {
    private static let state = OSAllocatedUnfairLock<Bool?>(initialState: nil)
    static var lastGranted: Bool? { state.withLock { $0 } }
    static func store(_ granted: Bool) { state.withLock { $0 = granted } }
}

extension CapturePermissions {
    /// The authoritative check: asks ScreenCaptureKit for shareable content, which can show the system prompt for
    /// an app that is not approved. `ScreenRecordingGate` decides whether it may run: only when the preflight is
    /// true or the user clicked Allow / Check Again, and `.automatic` checks at most once per app activation.
    /// Otherwise the answer is the preflight hint alone. It never calls `CGRequestScreenCaptureAccess`.
    public static func checkScreenRecording(_ trigger: ScreenRecordingGate.Trigger = .automatic) async -> ScreenRecordingAccess {
        let gate = ScreenRecordingGate.system
        guard gate.authorizeProbe(trigger) else {
            // Not allowed to probe: keep the last real verdict, and leave the cache alone.
            return ScreenRecordingAccess.refused(preflight: CGPreflightScreenCaptureAccess(), lastGranted: ScreenRecordingCache.lastGranted)
        }
        let access = await ScreenRecordingAccess.evaluate(
            asked: true,
            preflight: { CGPreflightScreenCaptureAccess() },
            shareable: { await probeShareableContent() })
        ScreenRecordingCache.store(access.isGranted)
        gate.recordProbeResult(granted: access.shareable == .ok)
        if PermissionDiagnostics.isEnabled { PermissionDiagnostics.log(access) }
        return access
    }

    static func probeShareableContent() async -> ShareableOutcome {
        do {
            _ = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true)
            return .ok
        } catch {
            let ns = error as NSError
            return .failed(domain: ns.domain, code: ns.code)
        }
    }
}

/// `NSS_PERMISSION_DEBUG=1` prints permission facts to stderr.
public enum PermissionDiagnostics {
    public static var isEnabled: Bool { ProcessInfo.processInfo.environment["NSS_PERMISSION_DEBUG"] == "1" }

    private static let printedHeader = OSAllocatedUnfairLock(initialState: false)

    static func log(_ access: ScreenRecordingAccess) {
        let first = printedHeader.withLock { done -> Bool in
            let was = done
            done = true
            return !was
        }
        if first { printHeader() }
        let shareable: String
        switch access.shareable {
        case .ok?: return emit("CGPreflightScreenCaptureAccess=\(access.preflight) SCShareableContent=ok verdict=granted")
        case .failed(let domain, let code)?: shareable = "error domain=\(domain) code=\(code)"
        case nil: shareable = "not run"
        }
        emit("CGPreflightScreenCaptureAccess=\(access.preflight) SCShareableContent=\(shareable) verdict=\(access.isGranted ? "granted" : "not granted")")
    }

    private static func printHeader() {
        emit("bundle path=\(Bundle.main.bundleURL.path)")
        emit("bundle id=\(Bundle.main.bundleIdentifier ?? "none") pid=\(ProcessInfo.processInfo.processIdentifier)")
        for line in signingInfo() { emit(line) }
    }

    /// Identifier, cdhash, team id and flags of this running process, like `codesign -dvvv`.
    static func signingInfo() -> [String] {
        var code: SecCode?
        guard SecCodeCopySelf([], &code) == errSecSuccess, let code else { return ["signing: SecCodeCopySelf failed"] }
        var lines: [String] = []
        let validity = SecCodeCheckValidity(code, [], nil)
        lines.append("signing validity=\(validity == errSecSuccess ? "valid" : "invalid (OSStatus \(validity))")")
        var staticCode: SecStaticCode?
        guard SecCodeCopyStaticCode(code, [], &staticCode) == errSecSuccess, let staticCode else {
            return lines + ["signing: SecCodeCopyStaticCode failed"]
        }
        var info: CFDictionary?
        let status = SecCodeCopySigningInformation(staticCode, SecCSFlags(rawValue: kSecCSSigningInformation), &info)
        guard status == errSecSuccess, let dict = info as? [String: Any] else {
            return lines + ["signing: SecCodeCopySigningInformation failed (OSStatus \(status))"]
        }
        let identifier = dict[kSecCodeInfoIdentifier as String] as? String ?? "none"
        let team = dict[kSecCodeInfoTeamIdentifier as String] as? String ?? "none"
        let flags = (dict[kSecCodeInfoFlags as String] as? NSNumber)?.uint32Value ?? 0
        let cdhash = (dict[kSecCodeInfoUnique as String] as? Data).map { $0.map { String(format: "%02x", $0) }.joined() } ?? "none"
        let certs = (dict[kSecCodeInfoCertificates as String] as? [Any])?.count ?? 0
        let adhoc = flags & 0x2 != 0
        lines.append("signing identifier=\(identifier) cdhash=\(cdhash) teamid=\(team) flags=0x\(String(flags, radix: 16)) adhoc=\(adhoc) certificates=\(certs)")
        return lines
    }

    private static func emit(_ line: String) {
        FileHandle.standardError.write(Data("[NSS_PERMISSION_DEBUG] \(line)\n".utf8))
    }
}
