import CoreGraphics
import Foundation
import Testing
import NSSCapture
import NSSCore
import NSSProject
@testable import NSSUI

@Suite struct ShellCountdownLogicTests {
    @Test func countdownShowsThreeTwoOne() {
        #expect(CountdownLayout.numbers(seconds: 3) == [3, 2, 1])
        #expect(CountdownLayout.numbers(seconds: 5) == [5, 4, 3, 2, 1])
        #expect(CountdownLayout.numbers(seconds: 0).isEmpty)
    }

    @Test func panelIsCentredOnTheSource() {
        let f = CountdownLayout.panelFrame(centeredOn: CGRect(x: 100, y: 50, width: 1000, height: 600))
        #expect(f.midX == 600 && f.midY == 350)
        #expect(f.width == CountdownLayout.preferredSide && f.height == f.width)
    }

    @Test func panelShrinksForTinySources() {
        let f = CountdownLayout.panelFrame(centeredOn: CGRect(x: 0, y: 0, width: 100, height: 100))
        #expect(f.width == 100)
    }

    @Test func settingsDefaultToThreeSeconds() {
        #expect(ShellSettings().countdownSeconds == 3)
        let old = Data(#"{"micEnabled":true}"#.utf8)
        let decoded = try? JSONDecoder().decode(ShellSettings.self, from: old)
        #expect(decoded?.countdownSeconds == 3)
        #expect(decoded?.bubbleRoundness == CameraBubble().roundness)
    }

    @Test func unsupportedCountdownValuesFallBack() {
        let data = Data(#"{"countdownSeconds":7,"bubbleRoundness":4}"#.utf8)
        let s = try? JSONDecoder().decode(ShellSettings.self, from: data)
        #expect(s?.countdownSeconds == 3)
        #expect(s?.bubbleRoundness == 1)
    }

    @Test func cancelledStartReturnsToIdleWithoutError() {
        #expect(ShellStateMachine.reduce(.preparing, .startCancelled) == .idle)
        #expect(ShellStateMachine.reduce(.idle, .startCancelled) == nil)
    }
}

@MainActor
@Suite struct ShellCountdownModelTests {
    @Test func countdownRunsOverTheSourceBeforeRecordingStarts() async throws {
        let rig = ShellRig(initial: ShellSettings(source: .window(id: 55), countdownSeconds: 5))
        defer { rig.cleanup() }
        var sessionStartedDuringCountdown = true
        rig.effects.onCountdown = { sessionStartedDuringCountdown = rig.session.startedOptions != nil }
        await rig.model.startRecording()
        #expect(rig.effects.countdownRuns == [5])
        #expect(sessionStartedDuringCountdown == false) // a plain session starts only after the countdown
        #expect(rig.effects.countdownRect == CGRect(x: 100, y: 900 - 100 - 600, width: 800, height: 600))
        #expect(rig.session.startedOptions?.countdownSeconds == 5)
        #expect(rig.model.isRecording)
        #expect(!rig.model.isCountingDown)
    }

    @Test func zeroSecondsSkipsTheCountdown() async {
        let rig = ShellRig(initial: ShellSettings(countdownSeconds: 0))
        defer { rig.cleanup() }
        await rig.model.startRecording()
        #expect(rig.effects.countdownRuns.isEmpty)
        #expect(rig.model.isRecording)
    }

    @Test func menuSettingIsSavedAndValidated() {
        let rig = ShellRig()
        defer { rig.cleanup() }
        rig.model.setCountdown(5)
        #expect(ShellSettingsStore(defaults: rig.defaults).load().countdownSeconds == 5)
        rig.model.setCountdown(4)
        #expect(rig.model.settings.countdownSeconds == 5)
        rig.model.setCountdown(0)
        #expect(ShellSettingsStore(defaults: rig.defaults).load().countdownSeconds == 0)
    }

    @Test func cancellingTheCountdownStartsNothingAndCleansUp() async throws {
        let rig = ShellRig(initial: ShellSettings(cameraEnabled: true))
        defer { rig.cleanup() }
        rig.effects.countdownResult = false
        await rig.model.startRecording()
        #expect(rig.model.phase == .idle)
        #expect(rig.model.notice == nil)
        #expect(rig.session.startedOptions == nil)
        #expect(!rig.effects.bubbleShown)
        #expect(!rig.effects.hudShown)
        #expect(rig.logger.stopOrigin == 0)
        #expect(try await rig.store.listProjects().isEmpty)
        #expect(rig.model.canStart)
    }

    @Test func warmSessionPreparesDuringCountdownAndBeginsAfter() async throws {
        let rig = ShellRig(warm: true)
        defer { rig.cleanup() }
        var callsDuringCountdown: [String] = []
        rig.effects.onCountdown = { callsDuringCountdown = rig.warm?.calls ?? [] }
        await rig.model.startRecording()
        // `prepare` runs concurrently with the countdown; `begin` only after it ended.
        #expect(!callsDuringCountdown.contains("begin"))
        let calls = try #require(rig.warm?.calls)
        #expect(calls.first == "preflight")
        #expect(calls.firstIndex(of: "prepare")! > 0)
        #expect(calls.firstIndex(of: "begin")! > calls.firstIndex(of: "prepare")!)
        #expect(!calls.contains("start"))
        #expect(rig.model.isRecording)
    }

    @Test func cancelWithWarmSessionDiscardsThePreparedCapture() async {
        let rig = ShellRig(warm: true)
        defer { rig.cleanup() }
        rig.effects.countdownResult = false
        await rig.model.startRecording()
        let calls = rig.warm?.calls ?? []
        #expect(calls.contains("cancelPrepared"))
        #expect(!calls.contains("begin"))
        #expect(rig.model.phase == .idle)
    }

    @Test func prepareFailureCutsTheCountdownShortAndReportsTheError() async {
        let rig = ShellRig(warm: true)
        defer { rig.cleanup() }
        rig.warm?.failPrepare = .io("Window closed.")
        await rig.model.startRecording()
        #expect(rig.effects.cancelCountdownCalls >= 1)
        #expect(rig.model.phase == .failed("Window closed."))
        #expect(!rig.effects.bubbleShown)
    }

    @Test func bubbleWaitsForPermissionPreflight() async {
        let rig = ShellRig(initial: ShellSettings(cameraEnabled: true), warm: true)
        defer { rig.cleanup() }
        rig.warm?.failPreflight = .permissionDenied("Screen recording is off.")
        var bubbleShownEver = false
        rig.effects.onCountdown = { bubbleShownEver = true }
        await rig.model.startRecording()
        #expect(rig.model.phase == .failed("Screen recording is off."))
        #expect(!rig.effects.bubbleShown)
        #expect(rig.effects.bubbleShape == nil) // never shown
        #expect(!bubbleShownEver) // and no countdown either
        #expect(rig.warm?.calls.contains("prepare") == false)
    }

    @Test func streamDyingDuringCountdownReportsTheRealError() async {
        let rig = ShellRig(warm: true)
        defer { rig.cleanup() }
        rig.warm?.fatalDuringPrepare = .io("The display was disconnected.")
        await rig.model.startRecording()
        #expect(rig.effects.cancelCountdownCalls >= 1)
        #expect(rig.model.phase == .failed("The display was disconnected."))
        #expect(rig.warm?.calls.contains("cancelPrepared") == true)
        #expect(rig.warm?.calls.contains("begin") == false)
    }

    @Test func hudAppearsAfterBeginAndExclusionsAreRefreshed() async throws {
        let rig = ShellRig(warm: true)
        defer { rig.cleanup() }
        await rig.model.startRecording()
        try await Task.sleep(for: .milliseconds(50))
        #expect(rig.effects.hudShown)
        #expect(rig.warm?.calls.contains("refresh") == true)
    }
}

@MainActor
@Suite struct ShellBubbleOverlayTests {
    @Test func overlayGetsShapeAndRoundnessFromSettings() async {
        let s = ShellSettings(cameraEnabled: true, cameraShape: .tall, bubbleRoundness: 0.8)
        let rig = ShellRig(initial: s)
        defer { rig.cleanup() }
        await rig.model.startRecording()
        #expect(rig.effects.bubbleShape == .tall)
        #expect(rig.effects.bubbleRoundness == 0.8)
        // Frame uses the shared aspect: tall is 3:4.
        let f = rig.effects.shownBubbleFrame!
        #expect(abs(f.width / f.height - 0.75) < 0.001)
    }

    @Test func bubbleIsShownBeforeCaptureStarts() async {
        let rig = ShellRig(initial: ShellSettings(cameraEnabled: true))
        defer { rig.cleanup() }
        var bubbleDuringCountdown = false
        rig.effects.onCountdown = { bubbleDuringCountdown = rig.effects.bubbleShown }
        await rig.model.startRecording()
        #expect(bubbleDuringCountdown)
    }

    @Test func changingShapeOrRoundnessUpdatesTheLiveBubble() {
        let rig = ShellRig()
        defer { rig.cleanup() }
        rig.model.selectShape(.wide)
        rig.model.selectRoundness(0.5)
        #expect(rig.effects.bubbleShape == .wide)
        #expect(rig.effects.bubbleRoundness == 0.5)
        rig.model.selectRoundness(3)
        #expect(rig.model.settings.bubbleRoundness == 1)
    }

    @Test func overlayCornerRadiusMatchesSharedGeometry() {
        for shape in CameraShape.allCases {
            for r in [0.0, 0.3, 1.0] {
                let size = CGSize(width: 240, height: 240 / CameraBubbleGeometry.aspect(for: shape))
                let overlay = BubbleGeometry.cornerRadius(for: shape, roundness: r, size: size)
                let shared = CameraBubbleGeometry.layout(shape: shape, roundness: r, width: 240).cornerRadius
                #expect(Double(overlay) == shared)
            }
        }
    }

    @Test func savedProjectKeepsOverlayShapeAndRoundness() async throws {
        let s = ShellSettings(cameraEnabled: true, cameraShape: .wide, bubbleRoundness: 0.9)
        let rig = ShellRig(initial: s)
        defer { rig.cleanup() }
        await rig.model.startRecording()
        await rig.model.stopRecording()
        let camera = try #require(rig.model.lastSaved?.project.style.camera)
        #expect(camera.shape == .wide)
        #expect(camera.roundness == 0.9)
    }
}
