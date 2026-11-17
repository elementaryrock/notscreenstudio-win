import AppKit
import SwiftUI

extension NSWindow {
    /// Keeps this window out of screen recordings and screenshots. Only for the recorder's own overlay windows,
    /// which exist only while preparing or recording. The capture filter also excludes the app by process.
    @MainActor
    public func excludeFromCapture() { sharingType = .none }
}

/// Which windows are hidden from screenshots and recordings. Only the recorder's own overlay windows (setup bar,
/// countdown, recording bar, camera bubble, area selection) are, and the menu while a recording is prepared or
/// running. The Permissions window, editor and home stay capturable so users can screenshot them.
public enum CapturePrivacyPolicy {
    public enum WindowKind: Sendable, CaseIterable {
        case setupBar, countdown, recordingBar, cameraBubble, areaSelection
        case menu, permissions, editor, home
    }

    public static func shouldExclude(_ kind: WindowKind, preparingOrRecording: Bool) -> Bool {
        switch kind {
        case .setupBar, .countdown, .recordingBar, .cameraBubble, .areaSelection:
            return true // these windows only exist while preparing or recording
        case .menu:
            return preparingOrRecording
        case .permissions, .editor, .home:
            return false
        }
    }
}

extension NSWindow {
    /// Applies the policy: `.none` when excluded, normal sharing otherwise.
    @MainActor
    public func applyCapturePrivacy(_ exclude: Bool) { sharingType = exclude ? .none : .readOnly }
}

/// Sets the hosting window's sharing type as soon as the view lands in it, and when `exclude` changes.
private struct CaptureExclusionMarker: NSViewRepresentable {
    var exclude: Bool
    final class MarkerView: NSView {
        var exclude = true
        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            window?.applyCapturePrivacy(exclude)
        }
    }
    func makeNSView(context: Context) -> MarkerView {
        let v = MarkerView()
        v.exclude = exclude
        return v
    }
    func updateNSView(_ nsView: MarkerView, context: Context) {
        nsView.exclude = exclude
        nsView.window?.applyCapturePrivacy(exclude)
    }
}

extension View {
    /// Keeps the window that shows this view out of recordings only while `exclude` is true.
    public func excludedFromCapture(when exclude: Bool) -> some View {
        background(CaptureExclusionMarker(exclude: exclude).frame(width: 0, height: 0))
    }
}
