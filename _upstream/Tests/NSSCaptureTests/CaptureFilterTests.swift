import CoreGraphics
import Foundation
import Testing
import NSSCore
@testable import NSSCapture

@Suite struct CaptureFilterPlanningTests {
    static let ownPID: Int32 = 500
    static let ownBundle = "dev.notscreenstudio.app"

    static func snapshot(
        apps: [ContentSnapshot.App] = [], windows: [ContentSnapshot.Window] = [], displays: [UInt32] = [1, 2]
    ) -> ContentSnapshot {
        ContentSnapshot(displayIDs: displays, apps: apps, windows: windows)
    }

    static func plan(
        _ snap: ContentSnapshot, source: CaptureSource = .display(id: 1), ownWindowIDs: [UInt32] = []
    ) throws -> FilterPlan {
        try CaptureFilter.plan(source: source, snapshot: snap, ownPID: ownPID, ownBundleID: ownBundle, ownWindowIDs: ownWindowIDs)
    }

    @Test func ownAppListedByScreenCaptureKitIsExcludedByPID() throws {
        let snap = Self.snapshot(apps: [.init(pid: 10, bundleID: "com.apple.finder"), .init(pid: Self.ownPID, bundleID: Self.ownBundle)])
        let p = try Self.plan(snap)
        #expect(p.exclusion.appPIDs == [Self.ownPID])
        #expect(p.target == .display(1))
    }

    @Test func bundleIDMatchesEvenWhenPIDDiffers() throws {
        let snap = Self.snapshot(apps: [.init(pid: 777, bundleID: Self.ownBundle)])
        #expect(try Self.plan(snap).exclusion.appPIDs == [777])
    }

    /// The Round 1 bug: a menu-bar-only app was missing from `applications`, so nothing was excluded.
    @Test func menuBarAppMissingFromApplicationsIsFoundThroughItsWindows() throws {
        let snap = Self.snapshot(
            apps: [.init(pid: 10, bundleID: "com.apple.finder")],
            windows: [
                .init(id: 90, ownerPID: 10, ownerBundleID: "com.apple.finder"),
                .init(id: 91, ownerPID: Self.ownPID, ownerBundleID: Self.ownBundle, title: "HUD", layer: 25),
                .init(id: 92, ownerPID: Self.ownPID, ownerBundleID: Self.ownBundle, title: "Bubble", layer: 25),
            ])
        let p = try Self.plan(snap)
        #expect(p.exclusion.appPIDs == [Self.ownPID])
        #expect(p.exclusion.windowIDs == [91, 92])
        #expect(!p.exclusion.isEmpty)
    }

    @Test func windowIDsFromTheAppAreAFallbackWhenNoOwnerIsReported() throws {
        let snap = Self.snapshot(windows: [
            .init(id: 90, ownerPID: nil, ownerBundleID: nil),
            .init(id: 91, ownerPID: nil, ownerBundleID: nil),
        ])
        let p = try Self.plan(snap, ownWindowIDs: [91, 4242])
        #expect(p.exclusion.appPIDs.isEmpty)
        #expect(p.exclusion.windowIDs == [91]) // 4242 is not known to ScreenCaptureKit, 90 is not ours
    }

    @Test func otherAppsAreNeverExcluded() throws {
        let snap = Self.snapshot(
            apps: [.init(pid: 10, bundleID: "com.apple.finder")],
            windows: [.init(id: 90, ownerPID: 10, ownerBundleID: "com.apple.finder")])
        #expect(try Self.plan(snap).exclusion.isEmpty)
    }

    @Test func emptyBundleIDDoesNotMatch() throws {
        let snap = Self.snapshot(apps: [.init(pid: 11, bundleID: "")])
        let p = try CaptureFilter.plan(source: .display(id: 1), snapshot: snap, ownPID: Self.ownPID, ownBundleID: "", ownWindowIDs: [])
        #expect(p.exclusion.isEmpty)
    }

    @Test func areaCaptureKeepsTheRectAndExcludesOurWindows() throws {
        let rect = CGRect(x: 10, y: 20, width: 300, height: 200)
        let snap = Self.snapshot(apps: [.init(pid: Self.ownPID, bundleID: Self.ownBundle)])
        let p = try Self.plan(snap, source: .area(displayID: 2, rect: rect))
        #expect(p.target == .display(2))
        #expect(p.area == rect)
        #expect(p.isWindow == false)
        #expect(p.exclusion.appPIDs == [Self.ownPID])
    }

    @Test func windowCaptureNeedsNoExclusion() throws {
        let snap = Self.snapshot(windows: [.init(id: 90, ownerPID: 10, ownerBundleID: "x")])
        let p = try Self.plan(snap, source: .window(id: 90))
        #expect(p.target == .window(90))
        #expect(p.isWindow)
        #expect(p.exclusion.isEmpty)
    }

    @Test func missingSourcesThrowFriendlyErrors() {
        let snap = Self.snapshot(displays: [1])
        #expect(throws: NSSError.unsupported("The selected display is no longer available.")) {
            _ = try Self.plan(snap, source: .display(id: 9))
        }
        #expect(throws: NSSError.unsupported("The selected display is no longer available.")) {
            _ = try Self.plan(snap, source: .area(displayID: 9, rect: .zero))
        }
        #expect(throws: NSSError.io("The selected window was closed.")) {
            _ = try Self.plan(snap, source: .window(id: 9))
        }
    }

    @Test func laterRefreshPicksUpWindowsCreatedAfterTheFirstFilter() throws {
        let before = Self.snapshot(apps: [.init(pid: 10, bundleID: "x")])
        #expect(try Self.plan(before).exclusion.isEmpty)
        let after = Self.snapshot(
            apps: [.init(pid: 10, bundleID: "x")],
            windows: [.init(id: 91, ownerPID: Self.ownPID, ownerBundleID: Self.ownBundle, title: "HUD")])
        #expect(try Self.plan(after).exclusion == ExclusionPlan(appPIDs: [Self.ownPID], windowIDs: [91]))
    }

    @Test func debugDescriptionListsWhatWasExcluded() throws {
        let snap = Self.snapshot(windows: [.init(id: 91, ownerPID: Self.ownPID, ownerBundleID: Self.ownBundle, title: "HUD")])
        let p = try Self.plan(snap)
        let text = CaptureDebug.describe(p, snapshot: snap, ownPID: Self.ownPID, filterBuiltFrom: "initial")
        #expect(text.contains("excluded apps (pid): [500]"))
        #expect(text.contains("91[HUD]"))
    }

    @Test func debugDescriptionWarnsWhenNothingIsExcluded() throws {
        let p = try Self.plan(Self.snapshot())
        #expect(CaptureDebug.describe(p, snapshot: Self.snapshot(), ownPID: Self.ownPID, filterBuiltFrom: "initial").contains("WARNING"))
    }
}

@Suite struct WarmSessionStateTests {
    @Test func beginWithoutPrepareThrows() async {
        let s = ScreenCaptureRecordingSession()
        await #expect(throws: NSSError.self) { try await s.beginRecording() }
    }

    @Test func cancelWithoutPrepareIsHarmless() async {
        let s = ScreenCaptureRecordingSession()
        await s.cancelPrepared()
        #expect(await s.state == .idle)
    }

    @Test func refreshWithoutStreamIsHarmless() async {
        let s = ScreenCaptureRecordingSession()
        await s.refreshExclusions()
    }
}

@Suite struct WarmConformanceTests {
    /// The shell warms up capture only if the real session is a `WarmableRecordingSession`.
    @Test func realSessionIsWarmable() {
        #expect((ScreenCaptureRecordingSession() as any RecordingSession) is any WarmableRecordingSession)
    }
}
