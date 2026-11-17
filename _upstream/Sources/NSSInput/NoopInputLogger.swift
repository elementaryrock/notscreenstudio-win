import Foundation
import NSSCore

/// STUB (Input worker replaces): records nothing.
public actor NoopInputLogger: InputLogger {
    public init() {}
    public func start(source: CaptureSource) async throws {}
    public func stop(recordingStartHostTime: Double) async -> CursorTrack { .empty }
}
