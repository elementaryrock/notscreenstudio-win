import CoreGraphics
import Foundation
import os
import Testing
import NSSCore
@testable import NSSUI

@MainActor
@Suite struct ShellModelTests {
    @Test func oneClickRecordUsesMainDisplayAndNoDevicesByDefault() async throws {
        let rig = ShellRig()
        defer { rig.cleanup() }
        await rig.model.startRecording()
        guard case .recording = rig.model.phase else { Issue.record("phase \(rig.model.phase)"); return }
        #expect(rig.session.startedOptions?.source == .display(id: 7))
        #expect(rig.session.startedOptions?.micDeviceID == nil)
        #expect(rig.session.startedOptions?.cameraDeviceID == nil)
        #expect(rig.logger.startedSource == .display(id: 7))
        #expect(rig.effects.hudShown)
        #expect(rig.effects.shortcutRegistered)
        #expect(!rig.effects.bubbleShown)
        let bundle = try #require(rig.session.startedBundle)
        #expect(bundle.pathExtension == "nssproj")
        #expect(FileManager.default.fileExists(atPath: bundle.path))
    }

    @Test func stateMachineRunsIdleRecordingSavingEditing() async throws {
        let rig = ShellRig()
        defer { rig.cleanup() }
        #expect(rig.model.phase == .idle)
        await rig.model.startRecording()
        #expect(rig.model.isRecording)
        await rig.model.stopRecording()
        guard case .editing(let ref) = rig.model.phase else { Issue.record("phase \(rig.model.phase)"); return }
        #expect(!rig.effects.hudShown)
        #expect(!rig.effects.shortcutRegistered)
        #expect(rig.model.editorRequestID == 1)
        // Saved through the ProjectStore, cursor rebased to the session's origin.
        #expect(rig.logger.stopOrigin == 42)
        let loaded = try await rig.store.load(ref)
        #expect(loaded.assets.duration == 3)
        #expect(loaded.style.camera.visible == false)
        let track = try await rig.store.loadCursorTrack(for: ref, assets: loaded.assets)
        #expect(track.samples.count == 1)
        #expect(rig.model.lastSaved?.project.id == loaded.id)
    }

    @Test func rememberedSourceAndDevicesAreUsed() async {
        let initial = ShellSettings(source: .window(id: 55), micEnabled: true, micDeviceID: "mic1", cameraEnabled: true, cameraDeviceID: "cam1")
        let rig = ShellRig(initial: initial)
        defer { rig.cleanup() }
        await rig.model.startRecording()
        #expect(rig.session.startedOptions == RecordingOptions(source: .window(id: 55), micDeviceID: "mic1", cameraDeviceID: "cam1"))
        #expect(rig.effects.bubbleShown)
    }

    @Test func goneWindowFallsBackToMainDisplay() async {
        let rig = ShellRig(initial: ShellSettings(source: .window(id: 999)))
        defer { rig.cleanup() }
        await rig.model.startRecording()
        #expect(rig.session.startedOptions?.source == .display(id: 7))
        #expect(rig.model.notice != nil)
    }

    @Test func missingScreenPermissionOpensOnboardingInsteadOfRecording() async {
        let rig = ShellRig(permissions: [.screenRecording: .denied, .camera: .granted, .microphone: .granted])
        defer { rig.cleanup() }
        await rig.model.startRecording()
        #expect(rig.model.phase == .idle)
        #expect(rig.model.onboardingRequestID == 1)
        #expect(rig.session.startedOptions == nil)
    }

    @Test func deniedMicAndCameraAreSkippedNotFatal() async {
        let rig = ShellRig(
            permissions: [.screenRecording: .granted, .camera: .denied, .microphone: .denied],
            initial: ShellSettings(micEnabled: true, cameraEnabled: true))
        defer { rig.cleanup() }
        await rig.model.startRecording()
        #expect(rig.model.isRecording)
        #expect(rig.session.startedOptions?.micDeviceID == nil)
        #expect(rig.session.startedOptions?.cameraDeviceID == nil)
        #expect(rig.model.notice?.contains("Microphone") == true)
        #expect(rig.model.notice?.contains("Camera") == true)
        #expect(!rig.effects.bubbleShown)
    }

    @Test func undecidedDevicePermissionIsAskedBeforeStartAndDeclinedIsSkipped() async {
        let rig = ShellRig(
            permissions: [.screenRecording: .granted, .camera: .notDetermined, .microphone: .notDetermined],
            requestResults: [.camera: .denied, .microphone: .denied],
            initial: ShellSettings(micEnabled: true, cameraEnabled: true))
        defer { rig.cleanup() }
        await rig.model.startRecording()
        #expect(rig.model.isRecording)
        #expect(rig.session.startedOptions?.micDeviceID == nil)
        #expect(rig.session.startedOptions?.cameraDeviceID == nil)
        #expect(rig.model.notice?.contains("Microphone skipped") == true)
        #expect(rig.model.notice?.contains("Camera skipped") == true)
        #expect(!rig.effects.bubbleShown)
        #expect(rig.model.onboardingRequestID == 0)
    }

    @Test func undecidedDevicePermissionThatIsGrantedIsUsed() async {
        let rig = ShellRig(
            permissions: [.screenRecording: .granted, .camera: .notDetermined, .microphone: .notDetermined],
            requestResults: [.camera: .granted, .microphone: .granted],
            initial: ShellSettings(micEnabled: true, cameraEnabled: true))
        defer { rig.cleanup() }
        await rig.model.startRecording()
        #expect(rig.session.startedOptions?.micDeviceID == "mic1")
        #expect(rig.session.startedOptions?.cameraDeviceID == "cam1")
        #expect(rig.model.notice == nil)
    }

    @Test func projectNameMatchesBundleName() async throws {
        let rig = ShellRig()
        defer { rig.cleanup() }
        await rig.model.startRecording()
        await rig.model.stopRecording()
        let saved = try #require(rig.model.lastSaved)
        #expect(saved.reference.url.deletingPathExtension().lastPathComponent == saved.project.name)
    }

    @Test func failedSessionStopRemovesTheEmptyBundle() async throws {
        let rig = ShellRig()
        defer { rig.cleanup() }
        await rig.model.startRecording()
        let bundle = try #require(rig.session.startedBundle)
        rig.session.failStop = .io("No screen frames were captured.")
        await rig.model.stopRecording()
        #expect(rig.model.phase == .failed("No screen frames were captured."))
        #expect(!FileManager.default.fileExists(atPath: bundle.path))
        #expect(rig.model.lastSaved == nil)
    }

    @Test func failedProjectSaveKeepsTheFinishedRecording() async throws {
        let rig = ShellRig()
        defer { rig.cleanup() }
        await rig.model.startRecording()
        let bundle = try #require(rig.session.startedBundle)
        // A directory where cursor.json belongs makes the save step fail after the session stopped fine.
        try FileManager.default.createDirectory(at: bundle.appendingPathComponent("cursor.json/x"), withIntermediateDirectories: true)
        await rig.model.stopRecording()
        guard case .failed = rig.model.phase else { Issue.record("phase \(rig.model.phase)"); return }
        #expect(FileManager.default.fileExists(atPath: bundle.path))
        #expect(rig.logger.stopOrigin == 42) // stopped once with the real origin, not reset afterwards
    }

    @Test func startFailureLeavesFailedPhaseAndNoEmptyBundle() async throws {
        let rig = ShellRig()
        defer { rig.cleanup() }
        rig.session.failStart = .io("disk full")
        await rig.model.startRecording()
        #expect(rig.model.phase == .failed("disk full"))
        let leftovers = (try? FileManager.default.contentsOfDirectory(atPath: rig.library.path)) ?? []
        #expect(leftovers.filter { $0.hasSuffix(".nssproj") }.isEmpty)
        #expect(!rig.effects.hudShown)
        rig.model.dismissError()
        #expect(rig.model.phase == .idle)
        #expect(rig.model.canStart)
    }

    @Test func permissionDeniedFromSessionOpensOnboarding() async {
        let rig = ShellRig()
        defer { rig.cleanup() }
        rig.session.failStart = .permissionDenied("Screen recording permission is required.")
        await rig.model.startRecording()
        #expect(rig.model.onboardingRequestID == 1)
    }

    @Test func discardDeletesTheBundleAndReturnsToIdle() async throws {
        let rig = ShellRig()
        defer { rig.cleanup() }
        await rig.model.startRecording()
        let bundle = try #require(rig.session.startedBundle)
        await rig.model.discardRecording()
        #expect(rig.model.phase == .idle)
        #expect(!FileManager.default.fileExists(atPath: bundle.path))
        #expect(!rig.effects.hudShown)
        #expect(rig.model.lastSaved == nil)
        #expect(rig.model.editorRequestID == 0)
    }

    @Test func secondStartWhileRecordingIsIgnored() async {
        let rig = ShellRig()
        defer { rig.cleanup() }
        await rig.model.startRecording()
        let first = rig.session.startedBundle
        await rig.model.startRecording()
        #expect(rig.session.startedBundle == first)
    }

    @Test func toggleStartsThenStops() async {
        let rig = ShellRig()
        defer { rig.cleanup() }
        await rig.model.toggleRecording()
        #expect(rig.model.isRecording)
        await rig.model.toggleRecording()
        if case .editing = rig.model.phase {} else { Issue.record("phase \(rig.model.phase)") }
    }

    @Test func launchFindsTheNewestProjectSoTheEditorCanReopenIt() async throws {
        let rig = ShellRig()
        defer { rig.cleanup() }
        await rig.model.launch()
        #expect(rig.model.latestProject == nil)
        rig.model.requestEditor()
        #expect(rig.model.editorRequestID == 0)
        await rig.model.startRecording()
        await rig.model.stopRecording()
        let saved = try #require(rig.model.lastSaved)
        #expect(rig.model.latestProject == saved.reference)

        // A fresh model on the same library (an app relaunch) discovers it.
        let again = ShellModel(
            services: ShellServices(
                session: rig.session, inputLogger: rig.logger, store: rig.store,
                sources: StubSourceProvider(), permissions: StubPermissions()),
            effects: rig.effects, settingsStore: ShellSettingsStore(defaults: rig.defaults))
        await again.launch()
        #expect(again.latestProject?.url.standardizedFileURL == saved.reference.url.standardizedFileURL)
        again.requestEditor()
        #expect(again.editorRequestID == 1)
    }

    @Test func launchSkipsNewerBundlesThatCannotLoad() async throws {
        let rig = ShellRig()
        defer { rig.cleanup() }
        await rig.model.startRecording()
        await rig.model.stopRecording()
        let valid = try #require(rig.model.lastSaved).reference
        // A newer, empty bundle, like one left behind by a crash during recording.
        let broken = try await rig.store.createProject(named: "Crashed")
        try FileManager.default.setAttributes([.modificationDate: Date().addingTimeInterval(60)], ofItemAtPath: broken.url.path)

        let again = ShellModel(
            services: ShellServices(
                session: rig.session, inputLogger: rig.logger, store: rig.store,
                sources: StubSourceProvider(), permissions: StubPermissions()),
            effects: rig.effects, settingsStore: ShellSettingsStore(defaults: rig.defaults))
        await again.launch()
        #expect(again.latestProject?.url.standardizedFileURL == valid.url.standardizedFileURL)
    }

    @Test func bubbleGeometryIsSavedIntoProjectAndSettings() async throws {
        let rig = ShellRig(initial: ShellSettings(cameraEnabled: true, cameraShape: .tall))
        defer { rig.cleanup() }
        await rig.model.startRecording()
        // The initial frame comes from the saved defaults, relative to the 1440x900 main display.
        let shown = try #require(rig.effects.shownBubbleFrame)
        #expect(abs(shown.width - 0.18 * 1440) < 1e-6)
        // The user drags and resizes the bubble.
        rig.effects.bubbleFrameValue = CGRect(x: 1000, y: 100, width: 288, height: 288)
        await rig.model.stopRecording()
        guard case .editing(let ref) = rig.model.phase else { Issue.record("phase \(rig.model.phase)"); return }
        let project = try await rig.store.load(ref)
        let cam = project.style.camera
        #expect(cam.visible)
        #expect(cam.shape == .tall)
        // Saved in OUTPUT space: the 2880x1800 source sits inside the padded 16:9 output.
        let expected = BubbleGeometry.toOutputSpace(
            center: NormalizedPoint(x: 1144.0 / 1440, y: (900.0 - 244.0) / 900), size: 0.2,
            outputAspect: AspectRatio.r16x9.ratio, sourceAspect: 2880.0 / 1800, padding: project.style.padding)
        #expect(abs(cam.position.x - expected.center.x) < 1e-9)
        #expect(abs(cam.position.y - expected.center.y) < 1e-9)
        #expect(abs(cam.size - expected.size) < 1e-9)
        #expect(cam.size < 0.2)
        #expect(abs(rig.model.settings.bubbleSize - 0.2) < 1e-9)
    }

    @Test func areaOnSecondDisplayPlacesBubbleOnThatDisplay() async throws {
        let rig = ShellRig(initial: ShellSettings(cameraEnabled: true))
        defer { rig.cleanup() }
        rig.effects.areaResult = AreaSelection(displayID: 8, rect: CGRect(x: 100, y: 100, width: 800, height: 600))
        await rig.model.chooseArea()
        #expect(rig.model.settings.source == .area(displayID: 8, rect: CGRect(x: 100, y: 100, width: 800, height: 600)))
        await rig.model.startRecording()
        let shown = try #require(rig.effects.shownBubbleFrame)
        // Area global rect is (1540,100,800,600) top-left, so AppKit x range is 1540...2340.
        #expect(shown.midX > 1540 && shown.midX < 2340)
    }

    @Test func cancelledAreaSelectionKeepsPreviousSource() async {
        let rig = ShellRig(initial: ShellSettings(source: .display(id: 8)))
        defer { rig.cleanup() }
        rig.effects.areaResult = nil
        await rig.model.chooseArea()
        #expect(rig.model.settings.source == .display(id: 8))
    }

    @Test func settingsPersistAcrossModels() async {
        let rig = ShellRig()
        defer { rig.cleanup() }
        rig.model.selectSource(.display(id: 8))
        rig.model.setMicEnabled(true)
        rig.model.selectMicrophone("mic1")
        rig.model.setCameraEnabled(true)
        rig.model.selectCamera("cam1")
        rig.model.selectShape(.wide)
        let reloaded = ShellSettingsStore(defaults: rig.defaults).load()
        #expect(reloaded == rig.model.settings)
        #expect(reloaded.source == .display(id: 8))
        #expect(reloaded.micEnabled && reloaded.cameraEnabled)
        #expect(reloaded.cameraShape == .wide)
    }

    @Test func launchRequestsOnboardingOnlyWhenScreenRecordingIsMissing() async {
        let denied = ShellRig(permissions: [.screenRecording: .notDetermined])
        defer { denied.cleanup() }
        await denied.model.launch()
        #expect(denied.model.onboardingRequestID == 1)
        let ok = ShellRig()
        defer { ok.cleanup() }
        await ok.model.launch()
        #expect(ok.model.onboardingRequestID == 0)
        #expect(ok.model.listing.displays.count == 2)
        #expect(ok.model.microphones.count == 1)
    }

    @Test func captureFailureWhileRecordingEndsInFailedPhase() async throws {
        let rig = ShellRig()
        defer { rig.cleanup() }
        await rig.model.startRecording()
        rig.session.set(.failed(message: "Window closed"))
        // The watcher runs on the main actor; give it a few turns.
        for _ in 0..<1500 where rig.model.isRecording { try await Task.sleep(for: .milliseconds(20)) }
        #expect(rig.model.phase == .failed("Window closed"))
        #expect(!rig.effects.hudShown)
        #expect(rig.session.stopCalls == 1)
        #expect(rig.logger.stopOrigin == 42)
        // The partial recording is finalized and saved, not left half-written.
        let saved = try #require(rig.model.lastSaved)
        #expect(FileManager.default.fileExists(atPath: saved.reference.url.appendingPathComponent("project.json").path))
    }

    @Test func captureFailureWithNothingRecordedRemovesTheBundle() async throws {
        let rig = ShellRig()
        defer { rig.cleanup() }
        await rig.model.startRecording()
        let bundle = try #require(rig.session.startedBundle)
        rig.session.failStop = .io("No screen frames were captured.")
        rig.session.set(.failed(message: "Display unplugged"))
        for _ in 0..<1500 where rig.model.isRecording { try await Task.sleep(for: .milliseconds(20)) }
        #expect(rig.model.phase == .failed("Display unplugged"))
        #expect(rig.session.stopCalls == 1)
        #expect(!FileManager.default.fileExists(atPath: bundle.path))
    }

    @Test func menuBarSymbolReflectsRecordingState() async {
        let rig = ShellRig()
        defer { rig.cleanup() }
        #expect(rig.model.menuBarSymbol == "record.circle")
        await rig.model.startRecording()
        #expect(rig.model.menuBarSymbol == "record.circle.fill")
    }
}

@MainActor @Suite struct ScreenRecordingProbeStateTests {
    @Test func probeResultIsNotOverwrittenByTheHint() async {
        // Preflight says granted, ScreenCaptureKit says denied: the state must stay denied on every refresh.
        let rig = ShellRig(permissions: [.screenRecording: .granted, .camera: .granted, .microphone: .granted],
                           probeResults: [.screenRecording: .denied])
        defer { rig.cleanup() }
        for _ in 0..<3 {
            await rig.model.refreshPermissionsAuthoritative()
            #expect(rig.model.permissionStates[.screenRecording] == .denied)
            rig.model.refreshPermissions()
            #expect(rig.model.permissionStates[.screenRecording] == .denied)
            await rig.model.refreshSources()
            #expect(rig.model.permissionStates[.screenRecording] == .denied)
        }
    }

    @Test func hintDeniedButProbeGrantedShowsTheBarWithoutOnboarding() async {
        let rig = ShellRig(permissions: [.screenRecording: .denied, .camera: .granted, .microphone: .granted],
                           probeResults: [.screenRecording: .granted])
        defer { rig.cleanup() }
        rig.model.beginSetup()
        await rig.model.setupProbeTask?.value
        #expect(rig.effects.setupBarShown)
        #expect(rig.model.isSettingUp)
        #expect(rig.model.onboardingRequestID == 0)
    }

    @Test func bothNotGrantedRequestsOnboardingOnceEvenOnDoubleClick() async {
        let rig = ShellRig(permissions: [.screenRecording: .denied, .camera: .granted, .microphone: .granted])
        defer { rig.cleanup() }
        rig.model.beginSetup()
        rig.model.beginSetup() // second click while the first probe is in flight
        await rig.model.setupProbeTask?.value
        #expect(!rig.effects.setupBarShown)
        #expect(rig.model.onboardingRequestID == 1)
    }

    @Test func doubleClickWithGrantedProbeOpensOneBar() async {
        let rig = ShellRig(permissions: [.screenRecording: .denied, .camera: .granted, .microphone: .granted],
                           probeResults: [.screenRecording: .granted])
        defer { rig.cleanup() }
        rig.model.beginSetup()
        rig.model.beginSetup()
        await rig.model.setupProbeTask?.value
        #expect(rig.probeCounter.count == 1)
        #expect(rig.effects.setupBarShowCount == 1)
        #expect(rig.model.onboardingRequestID == 0)
    }

    @Test func pickingAnotherSourceWhileTheBarIsOpenMovesTheBar() async {
        let rig = ShellRig()
        defer { rig.cleanup() }
        await rig.model.refreshSources()
        rig.model.pickSource(.display(id: 7))
        let first = rig.effects.setupBarSourceRect
        #expect(rig.effects.setupBarShowCount == 1)
        rig.model.pickSource(.window(id: 55))
        #expect(rig.effects.setupBarShowCount == 2)
        #expect(rig.effects.setupBarSourceRect != nil)
        #expect(rig.effects.setupBarSourceRect != first)
    }
}

@MainActor @Suite struct ScreenRecordingPromptTests {
    /// Poll with a fake clock: sleeping only counts, and the third sleep cancels the watcher.
    @Test func pollingNeverProbes() async {
        let rig = ShellRig(permissions: [.screenRecording: .denied, .camera: .granted, .microphone: .granted])
        defer { rig.cleanup() }
        let sleeps = OSAllocatedUnfairLock(initialState: 0)
        let task = Task { @MainActor in
            rig.model.pollSleep = { _ in sleeps.withLock { $0 += 1 }; await Task.yield() }
            await rig.model.watchScreenRecordingPermission()
        }
        while sleeps.withLock({ $0 }) < 50 { await Task.yield() }
        task.cancel()
        await task.value
        #expect(rig.probeCounter.count == 0)
    }

    @Test func pollProbesOnceWhenPreflightTurnsTrue() async {
        let rig = ShellRig(permissions: [.screenRecording: .granted, .camera: .granted, .microphone: .granted])
        defer { rig.cleanup() }
        await rig.model.watchScreenRecordingPermission()
        #expect(rig.probeCounter.count == 1)
        #expect(rig.probeCounter.userCount == 0)
    }

    @Test func allowClickDoesNotProbeAndCheckAgainDoes() async {
        let rig = ShellRig(permissions: [.screenRecording: .denied, .camera: .granted, .microphone: .granted])
        defer { rig.cleanup() }
        await rig.model.requestPermission(.screenRecording)
        #expect(rig.probeCounter.count == 0)
        await rig.model.checkAgain()
        #expect(rig.probeCounter.count == 1)
        #expect(rig.probeCounter.userCount == 1)
    }

    @Test func activationProbesAutomaticallyNotAsUser() async {
        let rig = ShellRig()
        defer { rig.cleanup() }
        await rig.model.handleActivation()
        #expect(rig.probeCounter.count == 1)
        #expect(rig.probeCounter.userCount == 0)
    }
}
