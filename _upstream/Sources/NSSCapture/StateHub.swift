import Foundation
import NSSCore

/// Lock-protected fan-out of `RecordingState`. Subscribers register synchronously.
final class StateHub: @unchecked Sendable {
    private let lock = NSLock()
    private var state: RecordingState = .idle
    private var continuations: [UUID: AsyncStream<RecordingState>.Continuation] = [:]

    var current: RecordingState { lock.withLock { state } }

    func stream() -> AsyncStream<RecordingState> {
        AsyncStream { c in
            let id = UUID()
            lock.withLock {
                continuations[id] = c
                c.yield(state)
            }
            c.onTermination = { [weak self] _ in self?.lock.withLock { self?.continuations[id] = nil } }
        }
    }

    func set(_ s: RecordingState) {
        lock.withLock {
            state = s
            for c in continuations.values { c.yield(s) }
        }
    }
}

/// Something that went wrong while recording was running.
public struct CaptureEvent: Sendable, Equatable {
    public enum Component: Sendable, Equatable { case screen, microphone, camera, writer }
    public let component: Component
    public let error: NSSError
    /// True if the screen recording itself ended. Mic/camera losses are non-fatal: their files are finalized and recording continues.
    public let isFatal: Bool

    public init(component: Component, error: NSSError, isFatal: Bool) {
        self.component = component
        self.error = error
        self.isFatal = isFatal
    }
}

final class EventHub: @unchecked Sendable {
    private let lock = NSLock()
    private var continuations: [UUID: AsyncStream<CaptureEvent>.Continuation] = [:]

    func stream() -> AsyncStream<CaptureEvent> {
        AsyncStream { c in
            let id = UUID()
            lock.withLock { continuations[id] = c }
            c.onTermination = { [weak self] _ in self?.lock.withLock { self?.continuations[id] = nil } }
        }
    }
    func send(_ e: CaptureEvent) { lock.withLock { for c in continuations.values { c.yield(e) } } }
}
