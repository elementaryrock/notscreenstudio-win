import CoreMedia
import Foundation
import NSSCore

/// STUB (Recording worker replaces): does nothing, reports state transitions.
public actor NoopRecordingSession: RecordingSession {
    /// Lock-protected subscriber registry. Registration happens synchronously inside `stateUpdates`,
    /// so a stream that terminates immediately can never leave a continuation behind.
    private final class Hub: @unchecked Sendable {
        private let lock = NSLock()
        private var state: RecordingState = .idle
        private var continuations: [UUID: AsyncStream<RecordingState>.Continuation] = [:]

        var current: RecordingState { lock.withLock { state } }

        func add(_ id: UUID, _ c: AsyncStream<RecordingState>.Continuation) {
            lock.withLock {
                continuations[id] = c
                c.yield(state)
            }
        }
        func remove(_ id: UUID) { lock.withLock { continuations[id] = nil } }
        func set(_ s: RecordingState) {
            lock.withLock {
                state = s
                for c in continuations.values { c.yield(s) }
            }
        }
    }

    private let hub = Hub()
    private var pendingSize = PixelSize(width: 1920, height: 1080)
    private var startHostTime: Double = 0

    public init() {}

    public nonisolated var stateUpdates: AsyncStream<RecordingState> {
        let hub = self.hub
        return AsyncStream { continuation in
            let id = UUID()
            hub.add(id, continuation)
            continuation.onTermination = { _ in hub.remove(id) }
        }
    }

    public var state: RecordingState { hub.current }

    public func start(options: RecordingOptions, into bundleURL: URL) async throws {
        startHostTime = CMClockGetTime(CMClockGetHostTimeClock()).seconds
        hub.set(.recording(startedAt: Date()))
    }

    public func stop() async throws -> RecordingAssets {
        hub.set(.finished)
        return RecordingAssets(sourcePixelSize: pendingSize, frameRate: 60, duration: 0, startHostTime: startHostTime)
    }
}
