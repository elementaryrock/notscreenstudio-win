import Testing
import NSSCore
import Foundation
@testable import NSSCapture

@Suite struct NSSCaptureTests {
    @Test func noopSessionTransitions() async throws {
        let s = NoopRecordingSession()
        #expect(await s.state == .idle)
        try await s.start(options: RecordingOptions(source: .display(id: 1)), into: URL(fileURLWithPath: NSTemporaryDirectory()))
        if case .recording = await s.state {} else { Issue.record("expected recording") }
        _ = try await s.stop()
        #expect(await s.state == .finished)
    }
}
