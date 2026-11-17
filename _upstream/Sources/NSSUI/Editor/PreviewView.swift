import AppKit
import SwiftUI
import NSSCore

/// Hosts the rendered IOSurface as layer contents and ticks the engine from a display link.
@MainActor
final class PreviewNSView: NSView {
    var engine: PreviewEngine? {
        didSet { engine?.present = { [weak self] pb in self?.show(pb) } }
    }
    var model: EditorModel?
    private let contentLayer = CALayer()
    private var link: CADisplayLink?
    private var draggingBubble = false
    private var aspect: Double = 16.0 / 9.0

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.backgroundColor = NSColor(white: 0.08, alpha: 1).cgColor
        contentLayer.contentsGravity = .resize
        contentLayer.magnificationFilter = .linear
        contentLayer.minificationFilter = .linear
        layer?.addSublayer(contentLayer)
    }
    required init?(coder: NSCoder) { fatalError("not used") }

    override var acceptsFirstResponder: Bool { true }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        link?.invalidate()
        link = nil
        guard window != nil else { return }
        let l = displayLink(target: self, selector: #selector(onTick(_:)))
        l.add(to: .main, forMode: .common)
        link = l
    }

    func stop() {
        link?.invalidate()
        link = nil
    }

    @objc private func onTick(_ sender: CADisplayLink) {
        let dt = sender.targetTimestamp - sender.timestamp
        MainActor.assumeIsolated { engine?.tick(dt: dt > 0 ? dt : 1.0 / 60.0) }
    }

    private func show(_ pb: CVPixelBuffer) {
        guard let surface = CVPixelBufferGetIOSurface(pb)?.takeUnretainedValue() else { return }
        let a = Double(CVPixelBufferGetWidth(pb)) / Double(max(CVPixelBufferGetHeight(pb), 1))
        if abs(a - aspect) > 1e-3 { aspect = a; needsLayout = true }
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        contentLayer.contents = surface
        CATransaction.commit()
    }

    private var fitRect: CGRect {
        let b = bounds
        guard b.width > 0, b.height > 0 else { return .zero }
        var w = b.width, h = w / aspect
        if h > b.height { h = b.height; w = h * aspect }
        return CGRect(x: (b.width - w) / 2, y: (b.height - h) / 2, width: w, height: h)
    }

    override func layout() {
        super.layout()
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        contentLayer.frame = fitRect
        contentLayer.contentsScale = window?.backingScaleFactor ?? 2
        CATransaction.commit()
    }

    /// View point -> output-normalized (origin top-left).
    private func outputPoint(_ event: NSEvent) -> NormalizedPoint? {
        let r = fitRect
        guard r.width > 0 else { return nil }
        let p = convert(event.locationInWindow, from: nil)
        return NormalizedPoint(x: (p.x - r.minX) / r.width, y: 1 - (p.y - r.minY) / r.height)
    }

    override func mouseDown(with event: NSEvent) {
        guard let model, let p = outputPoint(event) else { return }
        if model.pickingFocusFor != nil {
            _ = model.pickFocus(atOutput: p, camera: engine?.displayedCamera ?? .identity)
            return
        }
        let style = model.project.style
        if PreviewGeometry.hitsCameraBubble(p, bubble: style.camera, outputAspect: style.aspectRatio.ratio) {
            draggingBubble = true
            model.beginGesture("Move Camera")
        }
    }

    override func mouseDragged(with event: NSEvent) {
        guard draggingBubble, let model, let p = outputPoint(event) else { return }
        model.moveCameraBubble(toOutput: p)
    }

    override func mouseUp(with event: NSEvent) {
        guard draggingBubble else { return }
        draggingBubble = false
        model?.endGesture()
    }
}

struct PreviewView: NSViewRepresentable {
    let model: EditorModel
    let engine: PreviewEngine

    func makeNSView(context: Context) -> PreviewNSView {
        let v = PreviewNSView(frame: .zero)
        v.model = model
        v.engine = engine
        return v
    }

    func updateNSView(_ nsView: PreviewNSView, context: Context) {
        if nsView.engine !== engine {
            nsView.engine?.present = nil
            nsView.model = model
            nsView.engine = engine
        }
    }

    static func dismantleNSView(_ nsView: PreviewNSView, coordinator: ()) {
        nsView.stop()
    }
}
