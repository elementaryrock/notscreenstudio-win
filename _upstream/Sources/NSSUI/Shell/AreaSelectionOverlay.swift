import AppKit
import NSSCore

private final class OverlayWindow: NSWindow {
    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { true }
}

/// Dims one screen and lets the user drag a rectangle. The view is flipped, so its coordinates are display points, origin top-left.
private final class AreaSelectionView: NSView {
    let displayID: UInt32
    var onFinish: ((AreaSelection?) -> Void)?
    private var start: CGPoint?
    private var current: CGPoint?

    init(frame: CGRect, displayID: UInt32) {
        self.displayID = displayID
        super.init(frame: frame)
    }
    @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }

    override var isFlipped: Bool { true }
    override var acceptsFirstResponder: Bool { true }
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }
    override func resetCursorRects() { addCursorRect(bounds, cursor: .crosshair) }

    private var selection: CGRect? {
        guard let start, let current else { return nil }
        return CGRect(x: min(start.x, current.x), y: min(start.y, current.y),
                      width: abs(start.x - current.x), height: abs(start.y - current.y))
    }

    override func draw(_ dirtyRect: NSRect) {
        let dim = NSBezierPath(rect: bounds)
        if let selection { dim.append(NSBezierPath(rect: selection)); dim.windingRule = .evenOdd }
        NSColor.black.withAlphaComponent(0.45).setFill()
        dim.fill()
        if let selection {
            NSColor.white.setStroke()
            let border = NSBezierPath(rect: selection)
            border.lineWidth = 1.5
            border.stroke()
            let label = "\(Int(selection.width)) × \(Int(selection.height))" as NSString
            label.draw(at: CGPoint(x: selection.minX + 6, y: max(selection.minY - 22, 4)), withAttributes: [
                .font: NSFont.monospacedDigitSystemFont(ofSize: 12, weight: .medium),
                .foregroundColor: NSColor.white,
            ])
        } else {
            let hint = "Drag to choose an area. Esc to cancel." as NSString
            let attrs: [NSAttributedString.Key: Any] = [.font: NSFont.systemFont(ofSize: 15, weight: .medium), .foregroundColor: NSColor.white]
            let size = hint.size(withAttributes: attrs)
            hint.draw(at: CGPoint(x: bounds.midX - size.width / 2, y: bounds.midY - size.height / 2), withAttributes: attrs)
        }
    }

    override func mouseDown(with event: NSEvent) {
        start = convert(event.locationInWindow, from: nil)
        current = start
        needsDisplay = true
    }

    override func mouseDragged(with event: NSEvent) {
        current = convert(event.locationInWindow, from: nil)
        needsDisplay = true
    }

    override func mouseUp(with event: NSEvent) {
        current = convert(event.locationInWindow, from: nil)
        if let start, let current, let rect = AreaSelectionMath.rect(from: start, to: current, in: bounds) {
            onFinish?(AreaSelection(displayID: displayID, rect: rect))
        } else {
            // Too small: let the user try again.
            start = nil
            current = nil
            needsDisplay = true
        }
    }

    override func keyDown(with event: NSEvent) {
        if event.keyCode == 53 { onFinish?(nil) } else { super.keyDown(with: event) }
    }
}

/// Full-screen transparent overlay on every display. Resolves with the dragged area, or nil on Esc.
@MainActor
final class AreaSelectionOverlay {
    private var windows: [NSWindow] = []
    private var continuation: CheckedContinuation<AreaSelection?, Never>?

    func run() async -> AreaSelection? {
        guard continuation == nil else { return nil }
        return await withCheckedContinuation { cont in
            continuation = cont
            for screen in NSScreen.screens {
                guard let number = screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber else { continue }
                let window = OverlayWindow(contentRect: screen.frame, styleMask: .borderless, backing: .buffered, defer: false)
                window.isOpaque = false
                window.backgroundColor = .clear
                window.level = .screenSaver
                window.hasShadow = false
                window.isReleasedWhenClosed = false
                window.excludeFromCapture()
                window.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
                let view = AreaSelectionView(frame: CGRect(origin: .zero, size: screen.frame.size), displayID: number.uint32Value)
                view.onFinish = { [weak self] result in self?.finish(result) }
                window.contentView = view
                window.setFrame(screen.frame, display: false)
                window.makeKeyAndOrderFront(nil)
                window.makeFirstResponder(view)
                windows.append(window)
            }
            if windows.isEmpty { finish(nil) } else { NSApp.activate() }
        }
    }

    private func finish(_ result: AreaSelection?) {
        windows.forEach { $0.orderOut(nil) }
        windows.removeAll()
        let c = continuation
        continuation = nil
        c?.resume(returning: result)
    }
}
