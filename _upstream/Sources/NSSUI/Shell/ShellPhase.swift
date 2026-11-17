import Foundation
import NSSCore

/// Where the app is in the record → save → edit cycle.
public enum ShellPhase: Equatable, Sendable {
    case idle
    /// `RecordingSession.start` is in flight.
    case preparing
    case recording(startedAt: Date)
    /// Recording stopped; files are being finalized and the project saved.
    case saving
    /// The project is saved and the editor is open on it.
    case editing(ProjectReference)
    case failed(String)

    public var isRecording: Bool { if case .recording = self { return true } else { return false } }
    public var isBusy: Bool {
        switch self {
        case .preparing, .recording, .saving: return true
        case .idle, .editing, .failed: return false
        }
    }
    /// A new recording may start (also from `.editing` and `.failed`, so one click always works).
    public var canStart: Bool { !isBusy }
}

public enum ShellEvent: Equatable, Sendable {
    case startRequested
    case started(Date)
    case startFailed(String)
    /// The user cancelled during the countdown. Not an error.
    case startCancelled
    case stopRequested
    case discardRequested
    case saved(ProjectReference)
    case saveFailed(String)
    /// The screen stream died while recording.
    case captureFailed(String)
    case dismissError
}

/// Pure state machine. `reduce` returns nil for an event that is not valid in the current phase.
public enum ShellStateMachine {
    public static func reduce(_ phase: ShellPhase, _ event: ShellEvent) -> ShellPhase? {
        switch (phase, event) {
        case (.idle, .startRequested), (.editing, .startRequested), (.failed, .startRequested):
            return .preparing
        case (.preparing, .started(let date)):
            return .recording(startedAt: date)
        case (.preparing, .startFailed(let message)):
            return .failed(message)
        case (.preparing, .startCancelled):
            return .idle
        case (.recording, .stopRequested):
            return .saving
        case (.recording, .discardRequested):
            return .idle
        case (.recording, .captureFailed(let message)):
            return .failed(message)
        case (.saving, .saved(let ref)):
            return .editing(ref)
        case (.saving, .saveFailed(let message)):
            return .failed(message)
        case (.failed, .dismissError):
            return .idle
        default:
            return nil
        }
    }
}
