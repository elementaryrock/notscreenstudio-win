import AppKit
import CoreGraphics
import Foundation
import NSSCore
@preconcurrency import ScreenCaptureKit

public struct CaptureDisplayInfo: Sendable, Identifiable {
    public let id: UInt32
    public let name: String
    /// Frame in global display points.
    public let frame: CGRect
    public let pixelSize: PixelSize
    public let scale: Double
    public let isMain: Bool
    public var thumbnail: CGImage?
    public var source: CaptureSource { .display(id: id) }
}

public struct CaptureWindowInfo: Sendable, Identifiable {
    public let id: UInt32
    public let title: String
    public let appName: String
    public let bundleID: String?
    public let frame: CGRect
    /// The display holding most of the window, if known.
    public let displayID: UInt32?
    public var thumbnail: CGImage?
    public var source: CaptureSource { .window(id: id) }
}

public struct CaptureSourceList: Sendable {
    public var displays: [CaptureDisplayInfo]
    public var windows: [CaptureWindowInfo]
}

/// Lists displays and windows for the source picker. Needs screen recording permission.
public enum CaptureSourceCatalog {
    public static func load(includeThumbnails: Bool = false, thumbnailMaxSize: CGSize = CGSize(width: 320, height: 200)) async throws -> CaptureSourceList {
        // SCShareableContent can show the system prompt for an app with no decision yet. Do not call it before
        // the preflight says yes or an earlier probe in this process succeeded.
        guard CapturePermissions.screenRecordingGranted || ScreenRecordingGate.system.mayTouchScreenCaptureKit else {
            throw NSSError.permissionDenied(ScreenRecordingAccess.settingsHint)
        }
        let content: SCShareableContent
        do {
            content = try await SCShareableContent.excludingDesktopWindows(true, onScreenWindowsOnly: true)
            ScreenRecordingCache.store(true)
            ScreenRecordingGate.system.recordProbeResult(granted: true)
        } catch {
            let ns = error as NSError
            ScreenRecordingCache.store(false)
            ScreenRecordingGate.system.recordProbeResult(granted: false)
            if PermissionDiagnostics.isEnabled {
                PermissionDiagnostics.log(ScreenRecordingAccess(preflight: CGPreflightScreenCaptureAccess(), shareable: .failed(domain: ns.domain, code: ns.code)))
            }
            throw CaptureErrorMapper.map(error)
        }
        let names = await screenNames()
        let ownPID = ProcessInfo.processInfo.processIdentifier
        let ownBundle = Bundle.main.bundleIdentifier
        let mainID = CGMainDisplayID()

        var displays: [CaptureDisplayInfo] = content.displays.map { d in
            let scale = displayScale(d.displayID, points: CGSize(width: d.width, height: d.height))
            return CaptureDisplayInfo(
                id: d.displayID,
                name: names[d.displayID] ?? "Display \(d.displayID)",
                frame: d.frame,
                pixelSize: CaptureMath.pixelSize(points: CGSize(width: d.width, height: d.height), scale: scale),
                scale: scale,
                isMain: d.displayID == mainID,
                thumbnail: nil)
        }
        displays.sort { ($0.isMain ? 0 : 1, $0.id) < ($1.isMain ? 0 : 1, $1.id) }

        var windows: [CaptureWindowInfo] = []
        for w in content.windows {
            let app = w.owningApplication
            guard CaptureMath.isPickableWindow(layer: w.windowLayer, frame: w.frame, isOnScreen: w.isOnScreen, hasApp: app != nil),
                  let app,
                  !CaptureMath.isOwnApp(pid: app.processID, bundleID: app.bundleIdentifier, ownPID: ownPID, ownBundleID: ownBundle)
            else { continue }
            let display = content.displays.max { a, b in
                a.frame.intersection(w.frame).area < b.frame.intersection(w.frame).area
            }
            windows.append(CaptureWindowInfo(
                id: w.windowID,
                title: w.title ?? "",
                appName: app.applicationName,
                bundleID: app.bundleIdentifier,
                frame: w.frame,
                displayID: display?.displayID,
                thumbnail: nil))
        }
        windows.sort { ($0.appName.lowercased(), $0.title.lowercased()) < ($1.appName.lowercased(), $1.title.lowercased()) }

        if includeThumbnails {
            for i in displays.indices {
                if let d = content.displays.first(where: { $0.displayID == displays[i].id }) {
                    let filter = SCContentFilter(display: d, excludingWindows: [])
                    displays[i].thumbnail = await thumbnail(filter: filter, aspect: displays[i].frame.size, maxSize: thumbnailMaxSize)
                }
            }
            for i in windows.indices {
                if let w = content.windows.first(where: { $0.windowID == windows[i].id }) {
                    let filter = SCContentFilter(desktopIndependentWindow: w)
                    windows[i].thumbnail = await thumbnail(filter: filter, aspect: windows[i].frame.size, maxSize: thumbnailMaxSize)
                }
            }
        }
        return CaptureSourceList(displays: displays, windows: windows)
    }

    /// Fit `aspect` inside `maxSize`, keeping the aspect ratio.
    static func fitted(_ aspect: CGSize, in maxSize: CGSize) -> CGSize {
        guard aspect.width > 0, aspect.height > 0 else { return maxSize }
        let s = min(maxSize.width / aspect.width, maxSize.height / aspect.height)
        return CGSize(width: max(1, (aspect.width * s).rounded()), height: max(1, (aspect.height * s).rounded()))
    }

    private static func thumbnail(filter: SCContentFilter, aspect: CGSize, maxSize: CGSize) async -> CGImage? {
        let size = fitted(aspect, in: maxSize)
        let config = SCStreamConfiguration()
        config.width = Int(size.width)
        config.height = Int(size.height)
        config.showsCursor = false
        return try? await SCScreenshotManager.captureImage(contentFilter: filter, configuration: config)
    }

    private static func displayScale(_ id: CGDirectDisplayID, points: CGSize) -> Double {
        guard points.width > 0, let mode = CGDisplayCopyDisplayMode(id) else { return 1 }
        let s = Double(mode.pixelWidth) / points.width
        return s >= 1 ? s : 1
    }

    @MainActor
    private static func screenNamesMain() -> [UInt32: String] {
        var out: [UInt32: String] = [:]
        for s in NSScreen.screens {
            if let n = s.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber {
                out[n.uint32Value] = s.localizedName
            }
        }
        return out
    }

    private static func screenNames() async -> [UInt32: String] { await screenNamesMain() }
}

private extension CGRect {
    var area: CGFloat { isNull ? 0 : width * height }
}
