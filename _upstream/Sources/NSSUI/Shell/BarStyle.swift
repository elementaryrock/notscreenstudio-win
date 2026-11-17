import AppKit
import SwiftUI

/// Real behind-window vibrancy (`.hudWindow`) so the bars blur whatever is behind them. Follows the system appearance.
struct BarVisualEffect: NSViewRepresentable {
    func makeNSView(context: Context) -> NSVisualEffectView {
        let v = NSVisualEffectView()
        v.material = .hudWindow
        v.blendingMode = .behindWindow
        v.state = .active
        return v
    }
    func updateNSView(_ nsView: NSVisualEffectView, context: Context) {}
}

private struct BarUsesSystemBlurKey: EnvironmentKey { static let defaultValue = true }
extension EnvironmentValues {
    /// False in `ImageRenderer` previews, which cannot draw AppKit views; they fall back to a SwiftUI material.
    var barUsesSystemBlur: Bool {
        get { self[BarUsesSystemBlurKey.self] }
        set { self[BarUsesSystemBlurKey.self] = newValue }
    }
}

/// Shared look for the floating bars and the countdown: one control size, one spacing unit, a real material,
/// a hairline and a layered shadow. Adapts to light and dark because the bars float over any content.
enum BarStyle {
    /// Height and width of every round or pill control.
    static let control: CGFloat = 36
    /// The large record button.
    static let recordButton: CGFloat = 44
    /// One spacing unit (8 pt grid).
    static let unit: CGFloat = 8
    /// Room around a bar for its shadow inside the transparent panel.
    static let shadowInset: CGFloat = 24
    static let hairline: CGFloat = 0.5
    /// Bars are capsules, so there is one corner radius by construction.
    static let recordRed = Color(red: 1.0, green: 0.23, blue: 0.19)
    static let pausedAmber = Color(red: 1.0, green: 0.62, blue: 0.04)

    static func hairlineColor(_ scheme: ColorScheme) -> Color {
        scheme == .dark ? .white.opacity(0.15) : .black.opacity(0.10)
    }
}

/// Capsule on a vibrancy material with an inner hairline and a soft two-layer shadow.
struct BarSurface<S: InsettableShape>: ViewModifier {
    let shape: S
    var padded = true
    @Environment(\.colorScheme) private var scheme
    @Environment(\.barUsesSystemBlur) private var systemBlur

    @ViewBuilder private func blur(_ content: Content) -> some View {
        if systemBlur {
            content.background(BarVisualEffect().clipShape(shape))
        } else {
            content.background(.regularMaterial, in: shape)
        }
    }

    func body(content: Content) -> some View {
        blur(content)
            .overlay(shape.strokeBorder(BarStyle.hairlineColor(scheme), lineWidth: BarStyle.hairline))
            .shadow(color: .black.opacity(scheme == .dark ? 0.35 : 0.12), radius: 1, y: 0.5)
            .shadow(color: .black.opacity(scheme == .dark ? 0.32 : 0.14), radius: 16, y: 8)
            .padding(padded ? BarStyle.shadowInset : 0)
            .fixedSize(horizontal: padded, vertical: padded)
    }
}

extension View {
    func barSurface() -> some View { modifier(BarSurface(shape: Capsule(style: .continuous))) }
    func circleSurface() -> some View { modifier(BarSurface(shape: Circle())) }
}

/// Short fade and scale when a bar first appears. Opacity only when Reduce Motion is on.
struct BarAppear: ViewModifier {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var shown = false

    func body(content: Content) -> some View {
        content
            .opacity(shown ? 1 : 0)
            .scaleEffect(shown || reduceMotion ? 1 : 0.94, anchor: .bottom)
            .onAppear {
                withAnimation(reduceMotion ? .easeOut(duration: 0.15) : .spring(duration: 0.38, bounce: 0.22)) { shown = true }
            }
    }
}

extension View {
    func barAppear() -> some View { modifier(BarAppear()) }
}

/// Round or pill control with a clear hover and pressed state and a keyboard focus ring.
struct BarButtonStyle: ButtonStyle {
    var fill: Color = .primary.opacity(0.08)

    func makeBody(configuration: Configuration) -> some View {
        Inner(configuration: configuration, fill: fill)
    }

    private struct Inner: View {
        let configuration: Configuration
        let fill: Color
        @State private var hovering = false
        @Environment(\.isFocused) private var focused
        @Environment(\.isEnabled) private var enabled

        var body: some View {
            let pressed = configuration.isPressed
            let overlayOpacity = pressed ? 0.18 : (hovering ? 0.08 : 0)
            configuration.label
                .background(fill, in: Capsule())
                .overlay(Capsule().fill(Color.primary.opacity(overlayOpacity)))
                .overlay(Capsule().strokeBorder(Color.accentColor, lineWidth: 2).padding(-2).opacity(focused ? 1 : 0))
                .scaleEffect(pressed ? 0.96 : 1)
                .opacity(enabled ? 1 : 0.4)
                .animation(.easeOut(duration: 0.12), value: pressed)
                .animation(.easeOut(duration: 0.12), value: hovering)
                .onHover { hovering = $0 }
        }
    }
}
