import CoreGraphics
import Foundation
import NSSCore
@preconcurrency import ScreenCaptureKit

/// Hardware-free copy of the parts of `SCShareableContent` that filter construction needs.
/// `SCShareableContent` cannot be created in a test, so the decisions are made on this and tested.
struct ContentSnapshot: Sendable, Equatable {
    struct App: Sendable, Equatable {
        var pid: Int32
        var bundleID: String?
    }
    struct Window: Sendable, Equatable {
        var id: UInt32
        var ownerPID: Int32?
        var ownerBundleID: String?
        var title: String = ""
        var layer: Int = 0
        var isOnScreen: Bool = true
    }
    var displayIDs: [UInt32] = []
    var apps: [App] = []
    var windows: [Window] = []
}

/// What to keep out of a display or area capture.
struct ExclusionPlan: Sendable, Equatable {
    /// Processes whose windows (present and future) are excluded. Preferred, because it also covers windows
    /// created after the filter was built.
    var appPIDs: [Int32] = []
    /// Individual windows. Used only when no owning process could be resolved.
    var windowIDs: [UInt32] = []
    var isEmpty: Bool { appPIDs.isEmpty && windowIDs.isEmpty }
}

/// The decision for one capture: what to record and what to exclude.
struct FilterPlan: Sendable, Equatable {
    enum Target: Sendable, Equatable { case display(UInt32), window(UInt32) }
    var target: Target
    /// Sub-rectangle of the display in points (area capture).
    var area: CGRect?
    var isWindow: Bool
    var exclusion: ExclusionPlan
}

enum CaptureFilter {
    /// Which apps and windows belong to this process. Matches by PID and by bundle ID, and adds any window
    /// ID the app reports for itself (`NSWindow.windowNumber`) as a fallback.
    static func exclusion(
        in snapshot: ContentSnapshot, ownPID: Int32, ownBundleID: String?, ownWindowIDs: [UInt32]
    ) -> ExclusionPlan {
        func isOwn(_ pid: Int32?, _ bundle: String?) -> Bool {
            CaptureMath.isOwnApp(pid: pid ?? -1, bundleID: bundle, ownPID: ownPID, ownBundleID: ownBundleID)
        }
        var pids = Set(snapshot.apps.filter { isOwn($0.pid, $0.bundleID) }.map(\.pid))
        var windows = Set<UInt32>()
        for w in snapshot.windows {
            if isOwn(w.ownerPID, w.ownerBundleID) {
                windows.insert(w.id)
                // A menu-bar-only app can be missing from `applications` while its windows still name it.
                if let pid = w.ownerPID { pids.insert(pid) }
            }
        }
        let known = Set(snapshot.windows.map(\.id))
        for id in ownWindowIDs where known.contains(id) { windows.insert(id) }
        return ExclusionPlan(appPIDs: pids.sorted(), windowIDs: windows.sorted())
    }

    static func plan(
        source: CaptureSource, snapshot: ContentSnapshot, ownPID: Int32, ownBundleID: String?, ownWindowIDs: [UInt32]
    ) throws -> FilterPlan {
        switch source {
        case .display(let id), .area(let id, _):
            guard snapshot.displayIDs.contains(id) else {
                throw NSSError.unsupported("The selected display is no longer available.")
            }
            var area: CGRect?
            if case .area(_, let rect) = source { area = rect }
            return FilterPlan(
                target: .display(id), area: area, isWindow: false,
                exclusion: exclusion(in: snapshot, ownPID: ownPID, ownBundleID: ownBundleID, ownWindowIDs: ownWindowIDs))
        case .window(let id):
            guard snapshot.windows.contains(where: { $0.id == id }) else {
                throw NSSError.io("The selected window was closed.")
            }
            // A single-window capture shows only that window, so nothing else can leak in.
            return FilterPlan(target: .window(id), area: nil, isWindow: true, exclusion: ExclusionPlan())
        }
    }

    // MARK: ScreenCaptureKit adapter (thin; the decisions above are tested)

    static func snapshot(of content: SCShareableContent) -> ContentSnapshot {
        ContentSnapshot(
            displayIDs: content.displays.map(\.displayID),
            apps: content.applications.map { .init(pid: $0.processID, bundleID: $0.bundleIdentifier) },
            windows: content.windows.map {
                .init(
                    id: $0.windowID, ownerPID: $0.owningApplication?.processID,
                    ownerBundleID: $0.owningApplication?.bundleIdentifier,
                    title: $0.title ?? "", layer: $0.windowLayer, isOnScreen: $0.isOnScreen)
            })
    }

    static func makeFilter(_ plan: FilterPlan, content: SCShareableContent) throws -> SCContentFilter {
        switch plan.target {
        case .window(let id):
            guard let w = content.windows.first(where: { $0.windowID == id }) else {
                throw NSSError.io("The selected window was closed.")
            }
            return SCContentFilter(desktopIndependentWindow: w)
        case .display(let id):
            guard let d = content.displays.first(where: { $0.displayID == id }) else {
                throw NSSError.unsupported("The selected display is no longer available.")
            }
            // Resolve processes from `applications` and, failing that, from the windows that name them.
            var byPID: [Int32: SCRunningApplication] = [:]
            for a in content.applications { byPID[a.processID] = a }
            for w in content.windows { if let a = w.owningApplication, byPID[a.processID] == nil { byPID[a.processID] = a } }
            let apps = plan.exclusion.appPIDs.compactMap { byPID[$0] }
            if !apps.isEmpty {
                return SCContentFilter(display: d, excludingApplications: apps, exceptingWindows: [])
            }
            let ids = Set(plan.exclusion.windowIDs)
            let windows = content.windows.filter { ids.contains($0.windowID) }
            return SCContentFilter(display: d, excludingWindows: windows)
        }
    }
}

/// `NSS_CAPTURE_DEBUG=1` prints what the capture excluded, to stderr, so it can be checked by eye.
enum CaptureDebug {
    static var isEnabled: Bool { ProcessInfo.processInfo.environment["NSS_CAPTURE_DEBUG"] == "1" }

    static func log(_ text: @autoclosure () -> String) {
        guard isEnabled else { return }
        FileHandle.standardError.write(Data("[NSS capture] \(text())\n".utf8))
    }

    /// One readable line per fact: what was excluded and why.
    static func describe(_ plan: FilterPlan, snapshot: ContentSnapshot, ownPID: Int32, filterBuiltFrom: String) -> String {
        var lines = ["filter (\(filterBuiltFrom)) target=\(plan.target) ownPID=\(ownPID)"]
        let ex = plan.exclusion
        if plan.isWindow {
            lines.append("  single-window capture: nothing else is visible, no exclusion needed")
        } else if ex.isEmpty {
            lines.append("  WARNING: nothing excluded. This app's windows WILL appear in the recording.")
        } else {
            lines.append("  excluded apps (pid): \(ex.appPIDs)")
            lines.append("  own windows seen by ScreenCaptureKit (\(ex.windowIDs.count)): "
                + ex.windowIDs.map { id in
                    let w = snapshot.windows.first { $0.id == id }
                    return "\(id)[\(w?.title ?? "")]"
                }.joined(separator: ", "))
            if ex.appPIDs.isEmpty { lines.append("  no owning process found; excluding by window ID only") }
        }
        return lines.joined(separator: "\n")
    }
}
