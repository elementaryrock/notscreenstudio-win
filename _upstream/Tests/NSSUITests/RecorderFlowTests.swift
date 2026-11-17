import CoreGraphics
import Foundation
import NSSCapture
import NSSCore
import Testing
@testable import NSSUI

@Suite("Recorder bars and pause")
struct RecorderFlowTests {
    @Test func elapsedClockStopsWhilePaused() {
        let t0 = Date(timeIntervalSinceReferenceDate: 1000)
        var clock = ElapsedClock(startedAt: t0)
        #expect(clock.elapsed(at: t0.addingTimeInterval(5)) == 5)
        clock.pause(at: t0.addingTimeInterval(5))
        #expect(clock.isPaused)
        #expect(clock.elapsed(at: t0.addingTimeInterval(60)) == 5)
        clock.resume(at: t0.addingTimeInterval(65))
        #expect(clock.elapsed(at: t0.addingTimeInterval(65)) == 5)
        #expect(clock.elapsed(at: t0.addingTimeInterval(68)) == 8)
        clock.resume(at: t0.addingTimeInterval(70)) // not paused: ignored
        #expect(clock.elapsed(at: t0.addingTimeInterval(70)) == 10)
    }

    @Test func barSitsAtBottomCentreOfTheScreenWithTheSource() {
        let screens = [CGRect(x: 0, y: 0, width: 1440, height: 900), CGRect(x: 1440, y: 0, width: 1920, height: 1080)]
        #expect(BarPlacement.screenIndex(for: CGRect(x: 1600, y: 100, width: 400, height: 300), screens: screens) == 1)
        #expect(BarPlacement.screenIndex(for: nil, screens: screens) == 0)
        let f = BarPlacement.frame(size: CGSize(width: 400, height: 80), in: CGRect(x: 0, y: 25, width: 1440, height: 850))
        #expect(f.midX == 720)
        #expect(f.minY == 25 + BarPlacement.bottomMargin)
        let narrow = BarPlacement.frame(size: CGSize(width: 900, height: 80), in: CGRect(x: 0, y: 0, width: 600, height: 400))
        #expect(narrow.width == 600)
    }

    @MainActor @Test func pickingASourceShowsTheBarAndRecordStartsTheCountdown() async {
        let rig = ShellRig(initial: ShellSettings(countdownSeconds: 3))
        defer { rig.cleanup() }
        await rig.model.refreshSources()
        rig.model.pickSource(.window(id: 55))
        #expect(rig.effects.setupBarShown)
        #expect(rig.model.isSettingUp)
        #expect(rig.model.settings.source == .window(id: 55))
        #expect(rig.effects.setupBarSourceRect != nil)
        #expect(rig.session.startedOptions == nil) // nothing records until Record is pressed
        await rig.model.confirmRecord()
        #expect(!rig.effects.setupBarShown)
        #expect(rig.effects.countdownRuns == [3])
        #expect(rig.model.isRecording)
        #expect(rig.effects.hudShown)
        await rig.model.stopRecording()
    }

    @MainActor @Test func closingTheBarCancelsEverything() async {
        let rig = ShellRig()
        defer { rig.cleanup() }
        await rig.model.refreshSources()
        rig.model.beginSetup()
        rig.model.cancelSetup()
        #expect(!rig.effects.setupBarShown)
        #expect(!rig.model.isSettingUp)
        #expect(rig.session.startedOptions == nil)
        await rig.model.confirmRecord() // the bar is gone: ignored
        #expect(!rig.model.isRecording)
    }

    @MainActor @Test func cancellingTheCountdownBringsTheBarBack() async {
        let rig = ShellRig()
        defer { rig.cleanup() }
        rig.effects.countdownResult = false
        await rig.model.refreshSources()
        rig.model.beginSetup()
        await rig.model.confirmRecord()
        #expect(!rig.model.isRecording)
        #expect(rig.effects.setupBarShown)
    }

    @MainActor @Test func barTogglesChangeSettingsAndGoIntoTheRecording() async {
        let rig = ShellRig()
        defer { rig.cleanup() }
        await rig.model.refreshSources()
        rig.model.beginSetup()
        rig.model.setCameraEnabled(true)
        rig.model.setMicEnabled(true)
        await rig.model.confirmRecord()
        #expect(rig.session.startedOptions?.cameraDeviceID == "cam1")
        #expect(rig.session.startedOptions?.micDeviceID == "mic1")
        await rig.model.stopRecording()
    }

    @MainActor @Test func pauseAndResumeReachTheSessionAndFreezeTheTimer() async {
        let clock = TestClock(Date(timeIntervalSinceReferenceDate: 5000))
        let rig = ShellRig(initial: ShellSettings(countdownSeconds: 0), now: { clock.now })
        defer { rig.cleanup() }
        await rig.model.startRecording()
        #expect(rig.model.isRecording)
        #expect(rig.model.canPause)
        #expect(rig.effects.hudControls?.canPause == true)
        clock.now = clock.now.addingTimeInterval(4)
        await rig.model.pauseRecording()
        #expect(rig.model.isPaused)
        #expect(rig.effects.hudControls?.isPaused == true)
        #expect(rig.session.pauseCalls == 1)
        clock.now = clock.now.addingTimeInterval(30)
        await rig.model.pauseRecording() // already paused: ignored
        #expect(rig.session.pauseCalls == 1)
        #expect(rig.model.elapsed == 4)
        await rig.model.resumeRecording()
        #expect(!rig.model.isPaused)
        #expect(rig.session.resumeCalls == 1)
        clock.now = clock.now.addingTimeInterval(2)
        await rig.model.togglePause()
        #expect(rig.model.elapsed == 6)
        await rig.model.stopRecording()
        #expect(rig.model.elapsed == 0)
        #expect(!rig.effects.hudShown)
    }

    @MainActor @Test func stopRemovesPausedTimeFromTheCursorTrackAndOpensTheEditor() async throws {
        let rig = ShellRig(initial: ShellSettings(countdownSeconds: 0))
        defer { rig.cleanup() }
        let before = rig.model.editorRequestID
        await rig.model.startRecording()
        await rig.model.pauseRecording()
        await rig.model.resumeRecording()
        await rig.model.stopRecording()
        #expect(rig.model.editorRequestID == before + 1)
        let saved = try #require(rig.model.lastSaved)
        // The fake pauses host time 43...44 (t = 1...2). The only sample sits at t = 0, so it is kept.
        #expect(saved.cursorTrack.samples.map(\.t) == [0])
    }

    @MainActor @Test func menuBarShowsElapsedTimeOnlyWhileRecording() async {
        let rig = ShellRig(initial: ShellSettings(countdownSeconds: 0))
        defer { rig.cleanup() }
        #expect(rig.model.menuBarTitle == nil)
        await rig.model.startRecording()
        #expect(rig.model.menuBarTitle == "0:00")
        await rig.model.stopRecording()
        #expect(rig.model.menuBarTitle == nil)
    }
}

@MainActor
final class TestClock {
    var now: Date
    init(_ now: Date) { self.now = now }
}
