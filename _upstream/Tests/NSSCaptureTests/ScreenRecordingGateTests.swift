import Foundation
import os
import Testing
@testable import NSSCapture

/// Fake world for the gate: mutable preflight, signing identity and stored flag, with call counters.
private final class FakeWorld: @unchecked Sendable {
    private let lock = OSAllocatedUnfairLock(initialState: S())
    struct S { var preflight = false; var identity: String? = "id-A"; var stored: String?; var requests = 0 }
    var preflight: Bool { get { lock.withLock { $0.preflight } } set { lock.withLock { $0.preflight = newValue } } }
    var identity: String? { get { lock.withLock { $0.identity } } set { lock.withLock { $0.identity = newValue } } }
    var stored: String? { lock.withLock { $0.stored } }
    var requests: Int { lock.withLock { $0.requests } }
    func setStored(_ v: String?) { lock.withLock { $0.stored = v } }
    func gate() -> ScreenRecordingGate {
        ScreenRecordingGate(
            preflight: { self.preflight }, identity: { self.identity },
            loadAskedIdentity: { self.stored }, saveAskedIdentity: { v in self.lock.withLock { $0.stored = v } },
            request: { self.lock.withLock { $0.requests += 1 }; return false })
    }
}

@Suite struct ScreenRecordingGateTests {
    @Test func persistedFlagAloneNeverAllowsAProbe() {
        let w = FakeWorld()
        w.setStored("id-A") // asked before, same identity
        let g = w.gate()
        #expect(g.asked)
        #expect(!g.authorizeProbe(.automatic))
        #expect(!g.mayTouchScreenCaptureKit)
    }

    @Test func automaticProbeNeedsPreflightAndRunsOncePerActivation() {
        let w = FakeWorld()
        let g = w.gate()
        #expect(!g.authorizeProbe(.automatic))
        w.preflight = true
        #expect(g.authorizeProbe(.automatic))   // launch
        #expect(!g.authorizeProbe(.automatic))  // same activation again
        #expect(!g.authorizeProbe(.automatic))
        g.noteActivation()
        #expect(g.authorizeProbe(.automatic))   // new activation
        #expect(!g.authorizeProbe(.automatic))
    }

    @Test func refusedAutomaticProbeDoesNotUseUpTheAllowance() {
        let w = FakeWorld()
        let g = w.gate()
        #expect(!g.authorizeProbe(.automatic))
        w.preflight = true
        #expect(g.authorizeProbe(.automatic))
    }

    @Test func userClickAuthorizesOnlyItsOwnProbe() {
        let w = FakeWorld()
        let g = w.gate()
        #expect(g.authorizeProbe(.user))
        #expect(g.requestFromUserAction())
        g.noteActivation()
        #expect(!g.authorizeProbe(.automatic)) // dismissing the dialog re-activates the app
        #expect(!g.mayTouchScreenCaptureKit)
        g.recordProbeResult(granted: false)
        g.noteActivation()
        #expect(!g.authorizeProbe(.automatic))
        #expect(!g.mayTouchScreenCaptureKit)
    }

    @Test func successfulProbeAllowsLaterAutomaticOnes() {
        let w = FakeWorld()
        let g = w.gate()
        g.recordProbeResult(granted: true)
        #expect(g.mayTouchScreenCaptureKit)
        #expect(g.authorizeProbe(.automatic))
    }

    @Test func flagIsIgnoredAfterSigningIdentityChanges() {
        let w = FakeWorld()
        let g = w.gate()
        g.markAsked()
        #expect(g.asked)
        w.identity = "id-B" // rebuilt with a different signature
        #expect(!g.asked)
        w.identity = nil // unreadable identity never counts as asked
        #expect(!g.asked)
    }

    @Test func explicitAllowRequestsExactlyOnce() {
        let w = FakeWorld()
        let g = w.gate()
        #expect(g.requestFromUserAction())
        #expect(w.requests == 1)
        #expect(g.asked)
        #expect(!g.requestFromUserAction()) // second click: no second prompt
        #expect(w.requests == 1)
        w.identity = "id-B"
        #expect(g.requestFromUserAction()) // new identity may ask again
        #expect(w.requests == 2)
    }

    @Test func nothingRequestsAccessAutomatically() {
        let w = FakeWorld()
        w.preflight = true
        let g = w.gate()
        _ = g.authorizeProbe(.automatic); g.noteActivation(); _ = g.authorizeProbe(.automatic)
        _ = g.asked; _ = g.mayTouchScreenCaptureKit
        #expect(w.requests == 0)
    }
}
