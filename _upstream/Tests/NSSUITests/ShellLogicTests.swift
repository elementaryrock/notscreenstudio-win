import CoreGraphics
import Foundation
import Testing
import NSSCore
@testable import NSSUI

@Suite struct ShellStateMachineTests {
    let ref = ProjectReference(url: URL(fileURLWithPath: "/tmp/x.nssproj"))
    let date = Date(timeIntervalSince1970: 100)

    @Test func happyPath() {
        var p = ShellPhase.idle
        p = ShellStateMachine.reduce(p, .startRequested)!
        #expect(p == .preparing)
        p = ShellStateMachine.reduce(p, .started(date))!
        #expect(p == .recording(startedAt: date))
        p = ShellStateMachine.reduce(p, .stopRequested)!
        #expect(p == .saving)
        p = ShellStateMachine.reduce(p, .saved(ref))!
        #expect(p == .editing(ref))
    }

    @Test func canRecordAgainFromEditingAndFailed() {
        #expect(ShellStateMachine.reduce(.editing(ref), .startRequested) == .preparing)
        #expect(ShellStateMachine.reduce(.failed("x"), .startRequested) == .preparing)
    }

    @Test func failuresAndDiscard() {
        #expect(ShellStateMachine.reduce(.preparing, .startFailed("no")) == .failed("no"))
        #expect(ShellStateMachine.reduce(.saving, .saveFailed("disk")) == .failed("disk"))
        #expect(ShellStateMachine.reduce(.recording(startedAt: date), .discardRequested) == .idle)
        #expect(ShellStateMachine.reduce(.recording(startedAt: date), .captureFailed("gone")) == .failed("gone"))
        #expect(ShellStateMachine.reduce(.failed("x"), .dismissError) == .idle)
    }

    @Test func invalidTransitionsAreRejected() {
        #expect(ShellStateMachine.reduce(.idle, .stopRequested) == nil)
        #expect(ShellStateMachine.reduce(.saving, .startRequested) == nil)
        #expect(ShellStateMachine.reduce(.recording(startedAt: date), .startRequested) == nil)
        #expect(ShellStateMachine.reduce(.preparing, .stopRequested) == nil)
        #expect(ShellStateMachine.reduce(.idle, .saved(ref)) == nil)
    }

    @Test func canStartOnlyWhenNotBusy() {
        #expect(ShellPhase.idle.canStart)
        #expect(ShellPhase.editing(ref).canStart)
        #expect(!ShellPhase.preparing.canStart)
        #expect(!ShellPhase.recording(startedAt: date).canStart)
        #expect(!ShellPhase.saving.canStart)
    }
}

@Suite struct ShellSettingsTests {
    @Test func roundTripsThroughUserDefaults() {
        let name = "shell-settings-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: name)!
        defer { defaults.removePersistentDomain(forName: name) }
        let store = ShellSettingsStore(defaults: defaults)
        #expect(store.load() == ShellSettings())
        let s = ShellSettings(
            source: .area(displayID: 3, rect: CGRect(x: 10, y: 20, width: 300, height: 200)),
            micEnabled: true, micDeviceID: "m", cameraEnabled: true, cameraDeviceID: "c",
            cameraShape: .tall, bubbleSize: 0.3, bubbleCenter: NormalizedPoint(x: 0.1, y: 0.2))
        store.save(s)
        #expect(ShellSettingsStore(defaults: defaults).load() == s)
    }

    @Test func missingFieldsDecodeToDefaults() throws {
        let s = try JSONDecoder().decode(ShellSettings.self, from: Data(#"{"micEnabled":true}"#.utf8))
        #expect(s.micEnabled)
        #expect(s.source == nil)
        #expect(s.cameraShape == .circle)
        #expect(s.bubbleSize == ShellSettings.defaultBubbleSize)
    }

    @Test func corruptDataFallsBackToDefaults() {
        let name = "shell-settings-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: name)!
        defer { defaults.removePersistentDomain(forName: name) }
        defaults.set(Data("not json".utf8), forKey: ShellSettingsStore.key)
        #expect(ShellSettingsStore(defaults: defaults).load() == ShellSettings())
    }

    @Test func defaultsAreMainDisplayAndNoDevices() {
        let s = ShellSettings()
        #expect(s.source == nil)
        #expect(!s.micEnabled)
        #expect(!s.cameraEnabled)
    }
}

@MainActor @Suite struct BubbleGeometryTests {
    let source = CGRect(x: 0, y: 0, width: 1440, height: 900)

    @Test func outputSpaceKeepsBubbleOverTheSameContentPixel() {
        // 16:9 source in a 16:9 output with 10% padding: content is 0.8 of the height, so 0.8 wide.
        let r = BubbleGeometry.toOutputSpace(
            center: NormalizedPoint(x: 1, y: 1), size: 0.25, outputAspect: 16.0 / 9, sourceAspect: 16.0 / 9, padding: 0.1)
        #expect(abs(r.center.x - 0.9) < 1e-9 && abs(r.center.y - 0.9) < 1e-9)
        #expect(abs(r.size - 0.2) < 1e-9)
        // Source centre stays at the output centre for any aspect.
        let c = BubbleGeometry.toOutputSpace(
            center: .center, size: 0.1, outputAspect: 9.0 / 16, sourceAspect: 16.0 / 9, padding: 0.06)
        #expect(abs(c.center.x - 0.5) < 1e-9 && abs(c.center.y - 0.5) < 1e-9)
        // A 9:16 output shrinks a 16:9 source to the output width minus padding, so the bubble shrinks too.
        #expect(c.size < 0.1)
    }

    @Test func normalizesCenterWithTopLeftOrigin() throws {
        // A 200x200 panel in the bottom-left corner of the source (AppKit y = 0 is the bottom).
        let panel = CGRect(x: 0, y: 0, width: 200, height: 200)
        let r = try #require(BubbleGeometry.normalize(panelFrame: panel, in: source))
        #expect(abs(r.center.x - 100.0 / 1440) < 1e-9)
        #expect(abs(r.center.y - 800.0 / 900) < 1e-9) // near the bottom in top-left coordinates
        #expect(abs(r.size - 200.0 / 1440) < 1e-9)
    }

    @Test func offsetSourceIsRelative() throws {
        let src = CGRect(x: 1440, y: 100, width: 1000, height: 500)
        let panel = CGRect(x: 1440 + 400, y: 100 + 150, width: 200, height: 200) // centered
        let r = try #require(BubbleGeometry.normalize(panelFrame: panel, in: src))
        #expect(abs(r.center.x - 0.5) < 1e-9)
        #expect(abs(r.center.y - 0.5) < 1e-9)
    }

    @Test func roundTrips() throws {
        for shape in CameraShape.allCases {
            let frame = BubbleGeometry.panelFrame(center: NormalizedPoint(x: 0.7, y: 0.3), size: 0.2, shape: shape, in: source)
            let r = try #require(BubbleGeometry.normalize(panelFrame: frame, in: source))
            #expect(abs(r.center.x - 0.7) < 1e-9)
            #expect(abs(r.center.y - 0.3) < 1e-9)
            #expect(abs(r.size - 0.2) < 1e-9)
        }
    }

    @Test func centerIsClampedWhenDraggedOffSource() throws {
        let panel = CGRect(x: -500, y: 2000, width: 100, height: 100)
        let r = try #require(BubbleGeometry.normalize(panelFrame: panel, in: source))
        #expect(r.center == NormalizedPoint(x: 0, y: 0))
    }

    @Test func emptySourceHasNoResult() {
        #expect(BubbleGeometry.normalize(panelFrame: CGRect(x: 0, y: 0, width: 10, height: 10), in: .zero) == nil)
    }

    @Test func aspectFollowsShape() {
        #expect(BubbleGeometry.aspect(for: .circle) == 1)
        #expect(BubbleGeometry.aspect(for: .square) == 1)
        #expect(abs(BubbleGeometry.aspect(for: .wide) - 16.0 / 9.0) < 1e-9)
        #expect(abs(BubbleGeometry.aspect(for: .tall) - 0.75) < 1e-9)
        let f = BubbleGeometry.panelFrame(center: .center, size: 0.2, shape: .wide, in: source)
        #expect(abs(f.width / f.height - 16.0 / 9.0) < 1e-9)
    }

    @Test func cornerRadiusPerShape() {
        let size = CGSize(width: 200, height: 200)
        #expect(BubbleGeometry.cornerRadius(for: .circle, size: size) == 100)
        #expect(BubbleGeometry.cornerRadius(for: .square, roundness: 0, size: size) == 0)
        #expect(BubbleGeometry.cornerRadius(for: .square, roundness: 0.5, size: size) == 50)
        #expect(BubbleGeometry.cornerRadius(for: .square, roundness: 9, size: size) == 100)
    }

    @Test func flipIsItsOwnInverse() {
        let r = CGRect(x: 10, y: 20, width: 30, height: 40)
        #expect(BubbleGeometry.flip(BubbleGeometry.flip(r, primaryHeight: 900), primaryHeight: 900) == r)
        #expect(BubbleGeometry.flip(r, primaryHeight: 900).minY == 840)
    }

    @Test func keepReachableNeverLeavesPanelFullyOffscreen() {
        let bounds = CGRect(x: 0, y: 0, width: 1000, height: 800)
        let far = CGRect(x: 5000, y: -5000, width: 200, height: 200)
        let f = BubbleGeometry.keepReachable(far, in: bounds)
        #expect(f.intersects(bounds))
        #expect(BubbleGeometry.keepReachable(CGRect(x: 100, y: 100, width: 200, height: 200), in: bounds) == CGRect(x: 100, y: 100, width: 200, height: 200))
    }

    @Test func widthClamp() {
        #expect(BubbleGeometry.clampedWidth(10, sourceWidth: 1440) == 80)
        #expect(BubbleGeometry.clampedWidth(5000, sourceWidth: 1440) == 720)
        #expect(BubbleGeometry.clampedWidth(300, sourceWidth: 1440) == 300)
    }

    @Test func locatesSources() {
        let displays = [ShellRig.mainDisplay, ShellRig.sideDisplay]
        let windows = [WindowItem(id: 5, title: "", appName: "A", frame: CGRect(x: 100, y: 50, width: 400, height: 300))]
        let d = SourceLocator.appKitRect(for: .display(id: 7), displays: displays, windows: windows, primaryHeight: 900)
        #expect(d == CGRect(x: 0, y: 0, width: 1440, height: 900))
        let w = SourceLocator.appKitRect(for: .window(id: 5), displays: displays, windows: windows, primaryHeight: 900)
        #expect(w == CGRect(x: 100, y: 550, width: 400, height: 300))
        let a = SourceLocator.appKitRect(for: .area(displayID: 8, rect: CGRect(x: 10, y: 20, width: 100, height: 100)), displays: displays, windows: windows, primaryHeight: 900)
        #expect(a == CGRect(x: 1450, y: 780, width: 100, height: 100))
        #expect(SourceLocator.appKitRect(for: .display(id: 99), displays: displays, windows: windows, primaryHeight: 900) == nil)
    }
}

@Suite struct AreaSelectionMathTests {
    let bounds = CGRect(x: 0, y: 0, width: 1000, height: 800)

    @Test func dragInAnyDirection() {
        let r = AreaSelectionMath.rect(from: CGPoint(x: 300, y: 400), to: CGPoint(x: 100, y: 100), in: bounds)
        #expect(r == CGRect(x: 100, y: 100, width: 200, height: 300))
    }

    @Test func clampsToBounds() {
        let r = AreaSelectionMath.rect(from: CGPoint(x: 900, y: 700), to: CGPoint(x: 1200, y: 900), in: bounds)
        #expect(r == CGRect(x: 900, y: 700, width: 100, height: 100))
    }

    @Test func tinyDragsAreRejected() {
        #expect(AreaSelectionMath.rect(from: CGPoint(x: 10, y: 10), to: CGPoint(x: 20, y: 200), in: bounds) == nil)
    }
}

@MainActor @Suite struct ShellHelperTests {
    @Test func resolveFallsBackToMainDisplay() {
        let listing = SourceListing(displays: [ShellRig.sideDisplay, ShellRig.mainDisplay], windows: [WindowItem(id: 5, title: "", appName: "A", frame: .zero)])
        #expect(ShellModel.resolve(nil, in: listing, fallbackDisplay: 1) == (.display(id: 7), false))
        #expect(ShellModel.resolve(.display(id: 8), in: listing, fallbackDisplay: 1) == (.display(id: 8), false))
        #expect(ShellModel.resolve(.display(id: 99), in: listing, fallbackDisplay: 1) == (.display(id: 7), true))
        #expect(ShellModel.resolve(.window(id: 5), in: listing, fallbackDisplay: 1) == (.window(id: 5), false))
        #expect(ShellModel.resolve(.window(id: 6), in: listing, fallbackDisplay: 1) == (.display(id: 7), true))
        #expect(ShellModel.resolve(nil, in: SourceListing(), fallbackDisplay: 3) == (.display(id: 3), false))
    }

    @Test func pickDevicePrefersSavedThenDefaultThenFirst() {
        let devices = [DeviceItem(id: "a", name: "A"), DeviceItem(id: "b", name: "B", isDefault: true)]
        #expect(ShellModel.pickDevice(preferred: "a", from: devices) == "a")
        #expect(ShellModel.pickDevice(preferred: "gone", from: devices) == "b")
        #expect(ShellModel.pickDevice(preferred: nil, from: devices) == "b")
        #expect(ShellModel.pickDevice(preferred: nil, from: [DeviceItem(id: "z", name: "Z")]) == "z")
        #expect(ShellModel.pickDevice(preferred: "a", from: []) == nil)
    }

    @Test func elapsedFormat() {
        #expect(ElapsedFormat.string(0) == "0:00")
        #expect(ElapsedFormat.string(7.9) == "0:07")
        #expect(ElapsedFormat.string(723) == "12:03")
        #expect(ElapsedFormat.string(3723) == "1:02:03")
        #expect(ElapsedFormat.string(-4) == "0:00")
    }

    @Test func removeBundleRefusesAnythingOutsideTheLibrary() throws {
        let library = FileManager.default.temporaryDirectory.appendingPathComponent("shell-rm-\(UUID().uuidString)")
        let bundle = library.appendingPathComponent("a.nssproj")
        let other = library.appendingPathComponent("keep")
        try FileManager.default.createDirectory(at: bundle, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: other, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: library) }
        #expect(!ShellModel.removeBundle(ProjectReference(url: other), library: library))
        #expect(!ShellModel.removeBundle(ProjectReference(url: library), library: library))
        #expect(!ShellModel.removeBundle(ProjectReference(url: bundle), library: library.appendingPathComponent("elsewhere")))
        #expect(FileManager.default.fileExists(atPath: other.path))
        #expect(ShellModel.removeBundle(ProjectReference(url: bundle), library: library))
        #expect(!FileManager.default.fileExists(atPath: bundle.path))
    }

    @Test func permissionLinksPointAtPrivacyPanes() {
        for kind in PermissionKind.allCases {
            #expect(PermissionInfo.settingsURL(kind).absoluteString.hasPrefix("x-apple.systempreferences:com.apple.preference.security?Privacy_"))
        }
        #expect(PermissionInfo.isRequired(.screenRecording))
        #expect(!PermissionInfo.isRequired(.camera))
    }

    @Test func shapeKindRoundTrips() {
        for kind in ShapeKind.allCases { #expect(ShapeKind(kind.shape) == kind) }
    }
}

@Suite struct CapturePrivacyPolicyTests {
    @Test func overlaysAreAlwaysExcluded() {
        for kind in [CapturePrivacyPolicy.WindowKind.setupBar, .countdown, .recordingBar, .cameraBubble, .areaSelection] {
            #expect(CapturePrivacyPolicy.shouldExclude(kind, preparingOrRecording: true))
        }
    }

    @Test func permissionsEditorAndHomeStayCapturable() {
        for kind in [CapturePrivacyPolicy.WindowKind.permissions, .editor, .home] {
            #expect(!CapturePrivacyPolicy.shouldExclude(kind, preparingOrRecording: true))
            #expect(!CapturePrivacyPolicy.shouldExclude(kind, preparingOrRecording: false))
        }
    }

    @Test func menuIsExcludedOnlyWhilePreparingOrRecording() {
        #expect(CapturePrivacyPolicy.shouldExclude(.menu, preparingOrRecording: true))
        #expect(!CapturePrivacyPolicy.shouldExclude(.menu, preparingOrRecording: false))
    }
}
