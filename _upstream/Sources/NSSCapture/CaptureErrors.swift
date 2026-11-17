import AVFoundation
import Foundation
import NSSCore
import ScreenCaptureKit

/// Where an error came from, so the message can be specific.
public enum CaptureErrorContext: Sendable {
    case screen, window, microphone, camera, writer
}

/// Turns system errors into `NSSError`.
public enum CaptureErrorMapper {
    public static func map(_ error: Error, context: CaptureErrorContext = .screen) -> NSSError {
        if let e = error as? NSSError { return e }
        let ns = error as NSError

        if isDiskFull(ns) { return .io("The disk is full. Free up space and record again.") }

        if ns.domain == SCStreamErrorDomain, let code = SCStreamError.Code(rawValue: ns.code) {
            switch code {
            case .userDeclined, .missingEntitlements:
                return .permissionDenied("Screen recording permission is required. Enable it in System Settings > Privacy & Security > Screen & System Audio Recording.")
            case .noWindowList, .noCaptureSource, .noDisplayList, .failedNoMatchingApplicationContext:
                return context == .window
                    ? .io("The captured window was closed.")
                    : .unsupported("The selected capture source is no longer available.")
            case .userStopped:
                return .io("Screen capture was stopped.")
            case .systemStoppedStream:
                return .io("The system stopped screen capture.")
            case .failedToStartAudioCapture, .failedToStartMicrophoneCapture:
                return .io("Audio capture could not start.")
            default:
                return .io("Screen capture failed: \(ns.localizedDescription)")
            }
        }

        if ns.domain == AVFoundationErrorDomain, let code = AVError.Code(rawValue: ns.code) {
            switch code {
            case .diskFull:
                return .io("The disk is full. Free up space and record again.")
            case .deviceWasDisconnected, .deviceNotConnected:
                return .io("\(deviceName(context)) was disconnected.")
            case .applicationIsNotAuthorized, .applicationIsNotAuthorizedToUseDevice:
                return .permissionDenied("\(deviceName(context)) permission is required. Enable it in System Settings > Privacy & Security.")
            case .deviceInUseByAnotherApplication:
                return .io("\(deviceName(context)) is in use by another app.")
            default:
                break
            }
        }
        return .io(ns.localizedDescription)
    }

    static func deviceName(_ context: CaptureErrorContext) -> String {
        switch context {
        case .microphone: return "The microphone"
        case .camera: return "The camera"
        case .window: return "The window"
        default: return "The capture device"
        }
    }

    static func isDiskFull(_ ns: NSError) -> Bool {
        if ns.domain == NSCocoaErrorDomain, ns.code == NSFileWriteOutOfSpaceError { return true }
        if ns.domain == NSPOSIXErrorDomain, ns.code == Int(ENOSPC) { return true }
        if let underlying = ns.userInfo[NSUnderlyingErrorKey] as? NSError { return isDiskFull(underlying) }
        return false
    }
}
