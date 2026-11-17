import AppKit

/// Relaunches this app bundle as a new process, then quits the current one.
@MainActor
public enum AppRelauncher {
    /// True when running from a real `.app` bundle (not `swift run`).
    public static var canRelaunch: Bool { Bundle.main.bundleURL.pathExtension == "app" }

    public static func relaunch() {
        guard canRelaunch else {
            let alert = NSAlert()
            alert.messageText = "notscreenstudio cannot relaunch itself here"
            alert.informativeText = "It is not running from an app bundle. Quit it and start it again."
            alert.runModal()
            return
        }
        let config = NSWorkspace.OpenConfiguration()
        config.createsNewApplicationInstance = true
        NSWorkspace.shared.openApplication(at: Bundle.main.bundleURL, configuration: config) { _, error in
            Task { @MainActor in
                guard let error else { NSApplication.shared.terminate(nil); return }
                let alert = NSAlert()
                alert.messageText = "notscreenstudio could not relaunch"
                alert.informativeText = "\(error.localizedDescription)\nQuit the app from the menu bar icon and open it again."
                alert.runModal()
            }
        }
    }
}
