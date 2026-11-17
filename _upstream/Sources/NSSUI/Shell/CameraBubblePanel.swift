import AppKit
@preconcurrency import AVFoundation
import NSSCore

/// Owns the preview `AVCaptureSession`. Start and stop block, so they run on a private queue.
final class CameraPreviewSession: @unchecked Sendable {
    let session = AVCaptureSession()
    private let queue = DispatchQueue(label: "notscreenstudio.shell.camera-preview")
    /// Called on the main queue after the input has been added.
    var onConfigured: (() -> Void)?

    func start(deviceID: String?) {
        queue.async { [self] in
            session.beginConfiguration()
            session.inputs.forEach { session.removeInput($0) }
            let device = deviceID.flatMap { AVCaptureDevice(uniqueID: $0) } ?? AVCaptureDevice.default(for: .video)
            if let device, let input = try? AVCaptureDeviceInput(device: device), session.canAddInput(input) {
                session.addInput(input)
            }
            session.commitConfiguration()
            DispatchQueue.main.async { self.onConfigured?() }
            if !session.isRunning { session.startRunning() }
        }
    }

    func stop() {
        queue.async { [self] in
            if session.isRunning { session.stopRunning() }
            session.inputs.forEach { session.removeInput($0) }
        }
    }
}

/// Live webcam preview, clipped to the bubble shape, mirrored like the final render.
final class CameraBubbleView: NSView {
    let previewLayer: AVCaptureVideoPreviewLayer
    var shape: CameraShape = .circle { didSet { needsLayout = true } }
    var roundness: Double = CameraBubble().roundness { didSet { needsLayout = true } }

    init(session: AVCaptureSession) {
        previewLayer = AVCaptureVideoPreviewLayer(session: session)
        super.init(frame: .zero)
        wantsLayer = true
        layer?.masksToBounds = true
        layer?.borderWidth = 2
        layer?.borderColor = NSColor.white.withAlphaComponent(0.85).cgColor
        layer?.backgroundColor = NSColor.black.withAlphaComponent(0.4).cgColor
        previewLayer.videoGravity = .resizeAspectFill
        layer?.addSublayer(previewLayer)
    }

    /// The preview layer only gets its connection once the session has an input, so this runs after the input is added.
    @MainActor
    func applyMirroring() {
        if let connection = previewLayer.connection, connection.isVideoMirroringSupported {
            connection.automaticallyAdjustsVideoMirroring = false
            connection.isVideoMirrored = true
        }
    }

    @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }

    override func layout() {
        super.layout()
        previewLayer.frame = bounds
        layer?.cornerRadius = BubbleGeometry.cornerRadius(for: shape, roundness: roundness, size: bounds.size)
        layer?.cornerCurve = .circular
    }

    override func scrollWheel(with event: NSEvent) {
        (window as? CameraBubblePanel)?.resize(byWidth: event.scrollingDeltaY * 2)
    }
}

/// Small grab handle in the bubble's bottom-right corner that resizes it, keeping the aspect ratio.
final class ResizeHandle: NSView {
    private var startMouse: CGPoint = .zero
    private var startWidth: CGFloat = 0

    override var mouseDownCanMoveWindow: Bool { false }

    override func draw(_ dirtyRect: NSRect) {
        NSColor.white.withAlphaComponent(0.9).setFill()
        NSBezierPath(ovalIn: bounds.insetBy(dx: 3, dy: 3)).fill()
        NSColor.black.withAlphaComponent(0.35).setStroke()
        NSBezierPath(ovalIn: bounds.insetBy(dx: 3, dy: 3)).stroke()
    }

    override func mouseDown(with event: NSEvent) {
        startMouse = NSEvent.mouseLocation
        startWidth = window?.frame.width ?? 0
    }

    override func mouseDragged(with event: NSEvent) {
        guard let panel = window as? CameraBubblePanel else { return }
        let dx = NSEvent.mouseLocation.x - startMouse.x
        panel.resize(toWidth: startWidth + dx)
    }

    override func resetCursorRects() { addCursorRect(bounds, cursor: .crosshair) }
}

/// Borderless, always-on-top, non-activating panel with the live webcam. Drag it anywhere; resize with the corner handle or scroll.
final class CameraBubblePanel: NSPanel {
    private let preview = CameraPreviewSession()
    private let bubbleView: CameraBubbleView
    private let handle = ResizeHandle()
    private var shape: CameraShape
    private var roundness: Double
    private var sourceWidth: CGFloat

    @MainActor
    init(deviceID: String?, shape: CameraShape, roundness: Double, frame: CGRect, sourceRect: CGRect) {
        self.shape = shape
        self.roundness = roundness
        self.sourceWidth = sourceRect.width
        self.bubbleView = CameraBubbleView(session: preview.session)
        super.init(contentRect: frame, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        isOpaque = false
        backgroundColor = .clear
        hasShadow = true
        level = .statusBar
        isMovableByWindowBackground = true
        hidesOnDeactivate = false
        isReleasedWhenClosed = false
        excludeFromCapture()
        collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary]

        let root = NSView(frame: CGRect(origin: .zero, size: frame.size))
        root.autoresizesSubviews = true
        bubbleView.frame = root.bounds
        bubbleView.autoresizingMask = [.width, .height]
        bubbleView.shape = shape
        bubbleView.roundness = roundness
        root.addSubview(bubbleView)
        handle.frame = CGRect(x: root.bounds.maxX - 22, y: 0, width: 22, height: 22)
        handle.autoresizingMask = [.minXMargin, .maxYMargin]
        root.addSubview(handle)
        contentView = root
        preview.onConfigured = { [weak bubbleView] in MainActor.assumeIsolated { bubbleView?.applyMirroring() } }
        preview.start(deviceID: deviceID)
    }

    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }

    /// Resizes to `width` (clamped), keeping the top-left corner fixed and the shape's aspect.
    @MainActor
    func resize(toWidth width: CGFloat) {
        let w = BubbleGeometry.clampedWidth(width, sourceWidth: sourceWidth)
        let h = w / CGFloat(BubbleGeometry.aspect(for: shape))
        let f = frame
        setFrame(CGRect(x: f.minX, y: f.maxY - h, width: w, height: h), display: true)
    }

    @MainActor
    func resize(byWidth delta: CGFloat) { resize(toWidth: frame.width + delta) }

    @MainActor
    func apply(shape: CameraShape, roundness: Double) {
        self.shape = shape
        self.roundness = roundness
        bubbleView.shape = shape
        bubbleView.roundness = roundness
        resize(toWidth: frame.width)
    }

    @MainActor
    func close(stopping: Bool) {
        if stopping { preview.stop() }
        orderOut(nil)
    }
}
