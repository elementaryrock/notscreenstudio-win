import SwiftUI

/// Shared design tokens for the editor: one dark theme, one set of radii, spacing and type styles.
/// Every editor view (top bar, inspector, timeline, export sheet) reads from here so nothing drifts.
public enum EditorTheme {
    // MARK: Colours (dark theme, system-font UI)

    /// Window background behind everything.
    public static let canvas = Color(white: 0.085)
    /// Side panels (inspector) and the timeline area.
    public static let panel = Color(white: 0.125)
    /// Raised surfaces inside panels: section cards, field backgrounds.
    public static let surface = Color(white: 0.17)
    /// Hovered or pressed surfaces.
    public static let surfaceHover = Color(white: 0.22)
    /// Hairline separators and card outlines.
    public static let stroke = Color.white.opacity(0.08)
    public static let textPrimary = Color.white.opacity(0.92)
    public static let textSecondary = Color.white.opacity(0.62)
    public static let textTertiary = Color.white.opacity(0.38)
    public static let accent = Color.accentColor
    public static let playhead = Color(red: 1.0, green: 0.27, blue: 0.23)
    public static let trim = Color(red: 1.0, green: 0.8, blue: 0.0)
    public static let destructive = Color(red: 1.0, green: 0.35, blue: 0.3)

    // MARK: Corner radii

    public static let radiusSmall: CGFloat = 4
    public static let radiusMedium: CGFloat = 8
    public static let radiusLarge: CGFloat = 12

    // MARK: Spacing and padding

    public static let spacingTight: CGFloat = 4
    public static let spacing: CGFloat = 8
    public static let spacingLoose: CGFloat = 12
    /// Padding inside a panel or section.
    public static let padding: CGFloat = 12
    /// Fixed inspector width.
    public static let inspectorWidth: CGFloat = 300
    public static let topBarHeight: CGFloat = 44
    /// Height of one inspector row's control (slider, picker, toggle).
    public static let rowControlHeight: CGFloat = 22
}

// MARK: - Label styles

/// The three text styles used across the editor. System font only.
public enum EditorLabelStyle {
    /// A row title ("Padding", "Size").
    case row
    /// A section header ("Background").
    case header
    /// Helper text and hints under controls.
    case caption
    /// A value read-out next to a title (monospaced digits).
    case value
}

extension View {
    public func editorLabel(_ style: EditorLabelStyle) -> some View {
        modifier(EditorLabelModifier(style: style))
    }
}

private struct EditorLabelModifier: ViewModifier {
    let style: EditorLabelStyle
    func body(content: Content) -> some View {
        switch style {
        case .row: content.font(.system(size: 12)).foregroundStyle(EditorTheme.textPrimary)
        case .header: content.font(.system(size: 12, weight: .semibold)).foregroundStyle(EditorTheme.textPrimary)
        case .caption: content.font(.system(size: 11)).foregroundStyle(EditorTheme.textSecondary)
        case .value: content.font(.system(size: 11).monospacedDigit()).foregroundStyle(EditorTheme.textSecondary)
        }
    }
}

// MARK: - Button styles

/// Prominent accent button (Export, Done).
public struct EditorPrimaryButtonStyle: ButtonStyle {
    public init() {}
    public func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: 12, weight: .semibold))
            .foregroundStyle(.white)
            .padding(.horizontal, 14).frame(height: 28)
            .background(EditorTheme.accent.opacity(configuration.isPressed ? 0.75 : 1), in: RoundedRectangle(cornerRadius: EditorTheme.radiusMedium))
            .opacity(isEnabledOpacity)
    }
    @Environment(\.isEnabled) private var isEnabled
    private var isEnabledOpacity: Double { isEnabled ? 1 : 0.4 }
}

/// Quiet bordered button (Choose…, Add zoom, Cancel).
public struct EditorSecondaryButtonStyle: ButtonStyle {
    public var destructive = false
    public init(destructive: Bool = false) { self.destructive = destructive }
    @Environment(\.isEnabled) private var isEnabled
    public func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: 12))
            .foregroundStyle(destructive ? EditorTheme.destructive : EditorTheme.textPrimary)
            .padding(.horizontal, 10).frame(height: 26)
            .background(configuration.isPressed ? EditorTheme.surfaceHover : EditorTheme.surface, in: RoundedRectangle(cornerRadius: EditorTheme.radiusMedium))
            .overlay(RoundedRectangle(cornerRadius: EditorTheme.radiusMedium).strokeBorder(EditorTheme.stroke))
            .opacity(isEnabled ? 1 : 0.4)
    }
}

/// Icon-only toolbar button (undo, redo, transport).
public struct EditorIconButtonStyle: ButtonStyle {
    public init() {}
    @Environment(\.isEnabled) private var isEnabled
    public func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: 13, weight: .medium))
            .foregroundStyle(EditorTheme.textPrimary)
            .frame(width: 28, height: 28)
            .background(configuration.isPressed ? EditorTheme.surfaceHover : Color.clear, in: RoundedRectangle(cornerRadius: EditorTheme.radiusMedium))
            .contentShape(RoundedRectangle(cornerRadius: EditorTheme.radiusMedium))
            .opacity(isEnabled ? 1 : 0.35)
    }
}

extension ButtonStyle where Self == EditorPrimaryButtonStyle {
    public static var editorPrimary: EditorPrimaryButtonStyle { EditorPrimaryButtonStyle() }
}
extension ButtonStyle where Self == EditorSecondaryButtonStyle {
    public static var editorSecondary: EditorSecondaryButtonStyle { EditorSecondaryButtonStyle() }
    public static var editorDestructive: EditorSecondaryButtonStyle { EditorSecondaryButtonStyle(destructive: true) }
}
extension ButtonStyle where Self == EditorIconButtonStyle {
    public static var editorIcon: EditorIconButtonStyle { EditorIconButtonStyle() }
}
