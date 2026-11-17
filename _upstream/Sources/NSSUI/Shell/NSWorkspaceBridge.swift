import AppKit

/// Opens a local `x-apple.systempreferences:` URL. Nothing here touches the network.
@MainActor
enum NSWorkspaceBridge {
    static func open(_ url: URL) { NSWorkspace.shared.open(url) }
}
