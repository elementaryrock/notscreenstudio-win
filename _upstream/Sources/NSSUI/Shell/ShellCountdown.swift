import AppKit
import Carbon.HIToolbox
import Observation
import SwiftUI

/// Pure layout and timing for the countdown.
public enum CountdownLayout {
    public static let preferredSide: CGFloat = 240

    /// A square panel centred on `rect`, shrunk if `rect` is smaller than the preferred size.
    public static func panelFrame(centeredOn rect: CGRect, side: CGFloat = preferredSide) -> CGRect {
        let s = max(40, min(side, min(rect.width, rect.height)))
        return CGRect(x: rect.midX - s / 2, y: rect.midY - s / 2, width: s, height: s)
    }

    /// The numbers shown, in order: `seconds, …, 1`.
    public static func numbers(seconds: Int) -> [Int] { seconds > 0 ? Array((1...seconds).reversed()) : [] }
}

@MainActor
@Observable
final class CountdownState {
    var number = 0
}

struct CountdownView: View {
    var state: CountdownState
    var onCancel: () -> Void

    var body: some View {
        CountdownFace(number: state.number)
            .contentShape(Circle())
            .onTapGesture(perform: onCancel)
            .help("Click or press Esc to cancel")
            .accessibilityLabel("Countdown \(state.number)")
            .accessibilityHint("Click to cancel")
            .accessibilityAddTraits(.isButton)
    }
}

/// Large rounded digits on a blurred material circle, in the same style as the bars.
struct CountdownFace: View {
    var number: Int
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        // Sized from the panel, so small sources still show the whole circle and digit.
        GeometryReader { geo in
            let side = min(geo.size.width, geo.size.height)
            let inset = min(BarStyle.shadowInset, side * 0.1)
            let d = max(16, min(168, side - 2 * inset))
            ZStack {
                Text("\(number)")
                    .font(.system(size: d * 0.62, weight: .semibold, design: .rounded))
                    .monospacedDigit()
                    .foregroundStyle(.primary)
                    .id(number)
                    .transition(reduceMotion ? .opacity : .asymmetric(
                        insertion: .scale(scale: 1.2).combined(with: .opacity),
                        removal: .scale(scale: 0.85).combined(with: .opacity)))
            }
            .frame(width: d, height: d)
            .modifier(BarSurface(shape: Circle(), padded: false))
            .position(x: geo.size.width / 2, y: geo.size.height / 2)
        }
        .animation(reduceMotion ? .easeOut(duration: 0.2) : .spring(duration: 0.4, bounce: 0.15), value: number)
    }
}

/// A panel that only accepts clicks (to cancel). It never takes focus, so the app being recorded stays in front.
private final class CountdownPanel: NSPanel {
    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
}

/// Large 3-2-1 centred over the capture source. Esc (a system hot key, so no extra permission) or a click cancels.
@MainActor
final class CountdownOverlay {
    private var panel: NSPanel?
    private var task: Task<Bool, Never>?
    private let hotKey = GlobalHotKey()
    private let state = CountdownState()
    private var cancelled = false

    var isRunning: Bool { task != nil }

    func run(seconds: Int, over rect: CGRect) async -> Bool {
        guard task == nil, seconds > 0 else { return seconds <= 0 }
        cancelled = false
        let numbers = CountdownLayout.numbers(seconds: seconds)
        state.number = numbers[0]

        let host = NSHostingView(rootView: CountdownView(state: state, onCancel: { [weak self] in self?.cancel() }))
        let frame = CountdownLayout.panelFrame(centeredOn: rect)
        let panel = CountdownPanel(contentRect: frame, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        panel.contentView = host
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = false
        panel.level = .screenSaver
        panel.hidesOnDeactivate = false
        panel.isReleasedWhenClosed = false
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary]
        panel.excludeFromCapture()
        panel.setFrame(frame, display: true)
        panel.orderFrontRegardless()
        self.panel = panel
        hotKey.register(keyCode: UInt32(kVK_Escape), modifiers: 0) { [weak self] in self?.cancel() }

        let state = self.state
        let t = Task { @MainActor () -> Bool in
            let clock = ContinuousClock()
            let start = clock.now
            for (i, n) in numbers.enumerated() {
                state.number = n
                do { try await clock.sleep(until: start + .seconds(i + 1)) } catch { return false }
            }
            return true
        }
        task = t
        let completed = await t.value
        hotKey.unregister()
        panel.orderOut(nil)
        self.panel = nil
        task = nil
        return completed && !cancelled
    }

    func cancel() {
        cancelled = true
        task?.cancel()
    }
}
