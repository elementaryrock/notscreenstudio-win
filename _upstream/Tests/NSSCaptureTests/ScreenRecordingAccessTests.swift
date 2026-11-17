import Foundation
import os
import Testing
@preconcurrency import ScreenCaptureKit
@testable import NSSCapture

@Suite struct ScreenRecordingAccessTests {
    @Test func preflightFalseButShareableOkIsGranted() async {
        let a = await ScreenRecordingAccess.evaluate(asked: true, preflight: { false }, shareable: { .ok })
        #expect(a.isGranted)
        #expect(a.message == nil)
    }

    @Test func bothFailingIsNotGranted() async {
        let a = await ScreenRecordingAccess.evaluate(
            asked: true, preflight: { false }, shareable: { .failed(domain: SCStreamErrorDomain, code: SCStreamError.Code.userDeclined.rawValue) })
        #expect(!a.isGranted)
        #expect(a.message?.contains("System Settings") == true)
    }

    @Test func preflightTrueButShareableFailedIsNotGranted() {
        let a = ScreenRecordingAccess(preflight: true, shareable: .failed(domain: "X", code: 1))
        #expect(!a.isGranted)
    }

    @Test func withoutProbeThePreflightHintDecides() {
        #expect(ScreenRecordingAccess(preflight: true, shareable: nil).isGranted)
        #expect(!ScreenRecordingAccess(preflight: false, shareable: nil).isGranted)
    }

    @Test func errorCodesMapToClearMessages() {
        let declined = ScreenRecordingAccess.message(domain: SCStreamErrorDomain, code: SCStreamError.Code.userDeclined.rawValue)
        #expect(declined.contains("Screen & System Audio Recording"))
        let conn = ScreenRecordingAccess.message(domain: SCStreamErrorDomain, code: SCStreamError.Code.failedApplicationConnectionInvalid.rawValue)
        #expect(conn.contains("Relaunch"))
        let ents = ScreenRecordingAccess.message(domain: SCStreamErrorDomain, code: SCStreamError.Code.missingEntitlements.rawValue)
        #expect(ents.contains("entitlements"))
        let other = ScreenRecordingAccess.message(domain: "Foo", code: 42)
        #expect(other.contains("Foo") && other.contains("42"))
    }

    @Test func probeIsSkippedBeforeAskingWhenPreflightIsFalse() async {
        let ran = OSAllocatedUnfairLock(initialState: false)
        let a = await ScreenRecordingAccess.evaluate(asked: false, preflight: { false }, shareable: { ran.withLock { $0 = true }; return .ok })
        #expect(!ran.withLock { $0 })
        #expect(a.shareable == nil && !a.isGranted)
    }

    @Test func probeRunsWhenAskedOrPreflightTrue() async {
        let asked = await ScreenRecordingAccess.evaluate(asked: true, preflight: { false }, shareable: { .ok })
        #expect(asked.isGranted)
        let hinted = await ScreenRecordingAccess.evaluate(asked: false, preflight: { true }, shareable: { .ok })
        #expect(hinted.shareable == .ok)
    }

    @Test func refusedProbeKeepsALastGrantedVerdict() {
        // Preflight stays false in a process that can capture; a rate-limited re-probe must not downgrade.
        #expect(ScreenRecordingAccess.refused(preflight: false, lastGranted: true).isGranted)
        #expect(!ScreenRecordingAccess.refused(preflight: false, lastGranted: false).isGranted)
        #expect(!ScreenRecordingAccess.refused(preflight: false, lastGranted: nil).isGranted)
        #expect(ScreenRecordingAccess.refused(preflight: true, lastGranted: nil).isGranted)
    }
}
