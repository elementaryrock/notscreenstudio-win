import AppKit
import NSSCore

/// The real `ShellEffects`: HUD panel, camera bubble panel, global stop shortcut and area overlay.
@MainActor
public final class AppKitShellEffects: ShellEffects {
    private let hud = FloatingBarWindow()
    private let setupBar = FloatingBarWindow()
    private let hotKey = GlobalHotKey()
    private let overlay = AreaSelectionOverlay()
    private let countdown = CountdownOverlay()
    private var bubble: CameraBubblePanel?

    public init() {}

    public var primaryScreenHeight: CGFloat { NSScreen.screens.first?.frame.height ?? 0 }

    public func showSetupBar(model: ShellModel, sourceRect: CGRect?) {
        setupBar.show(RecorderSetupBar(model: model), over: sourceRect, makeKey: true)
    }

    public func hideSetupBar() { setupBar.hide() }

    public func showHUD(controls: RecordingControls, sourceRect: CGRect?) {
        hud.show(RecordingBar(controls: controls), over: sourceRect, makeKey: false)
    }

    public func hideHUD() { hud.hide() }

    public func showBubble(deviceID: String?, shape: CameraShape, roundness: Double, frame: CGRect, sourceRect: CGRect) {
        hideBubble()
        let screens = NSScreen.screens.map(\.frame)
        let union = screens.dropFirst().reduce(screens.first ?? sourceRect) { $0.union($1) }
        let placed = BubbleGeometry.keepReachable(frame, in: union)
        let panel = CameraBubblePanel(deviceID: deviceID, shape: shape, roundness: roundness, frame: placed, sourceRect: sourceRect)
        panel.orderFrontRegardless()
        bubble = panel
    }

    public func bubbleFrame() -> CGRect? { bubble?.frame }
    public func updateBubble(shape: CameraShape, roundness: Double) { bubble?.apply(shape: shape, roundness: roundness) }

    public func hideBubble() {
        bubble?.close(stopping: true)
        bubble = nil
    }

    public func registerStopShortcut(_ handler: @escaping @MainActor () -> Void) {
        hotKey.register(keyCode: StopShortcut.keyCode, modifiers: StopShortcut.modifiers, handler: handler)
    }

    public func unregisterStopShortcut() { hotKey.unregister() }

    public func runCountdown(seconds: Int, over rect: CGRect) async -> Bool {
        await countdown.run(seconds: seconds, over: rect)
    }

    public func cancelCountdown() { countdown.cancel() }

    public func selectArea() async -> AreaSelection? { await overlay.run() }
}
