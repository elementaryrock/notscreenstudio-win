import SwiftUI

/// The inspector sections, in display order.
public enum InspectorSectionID: String, CaseIterable, Identifiable, Sendable {
    case background, frame, cursor, zoom, camera, audio
    public var id: String { rawValue }

    public var title: String {
        switch self {
        case .background: "Background"
        case .frame: "Frame"
        case .cursor: "Cursor"
        case .zoom: "Zoom"
        case .camera: "Camera"
        case .audio: "Audio"
        }
    }

    public var symbol: String {
        switch self {
        case .background: "photo.fill"
        case .frame: "rectangle.inset.filled"
        case .cursor: "cursorarrow"
        case .zoom: "plus.magnifyingglass"
        case .camera: "video.fill"
        case .audio: "waveform"
        }
    }

    /// Sections that start open. The rest start collapsed so the list fits without scrolling.
    public var startsExpanded: Bool { self == .background || self == .frame }
}

/// A collapsible inspector section. Same header, padding and card style everywhere; the open/closed
/// state is remembered between launches.
public struct InspectorSection<Content: View>: View {
    private let id: InspectorSectionID
    private let content: Content
    @AppStorage private var expanded: Bool

    public init(_ id: InspectorSectionID, @ViewBuilder content: () -> Content) {
        self.id = id
        self.content = content()
        _expanded = AppStorage(wrappedValue: id.startsExpanded, "editor.inspector.\(id.rawValue).expanded")
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Button {
                withAnimation(.easeInOut(duration: 0.15)) { expanded.toggle() }
            } label: {
                HStack(spacing: EditorTheme.spacing) {
                    Image(systemName: id.symbol)
                        .font(.system(size: 12))
                        .foregroundStyle(EditorTheme.textSecondary)
                        .frame(width: 16)
                    Text(id.title).editorLabel(.header)
                    Spacer()
                    Image(systemName: "chevron.right")
                        .font(.system(size: 10, weight: .semibold))
                        .foregroundStyle(EditorTheme.textTertiary)
                        .rotationEffect(.degrees(expanded ? 90 : 0))
                }
                .padding(.horizontal, EditorTheme.padding)
                .frame(height: 34)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(id.title)
            .accessibilityValue(expanded ? "expanded" : "collapsed")

            if expanded {
                Divider().overlay(EditorTheme.stroke)
                VStack(alignment: .leading, spacing: EditorTheme.spacingLoose) {
                    content
                }
                .padding(EditorTheme.padding)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .background(EditorTheme.surface.opacity(0.55), in: RoundedRectangle(cornerRadius: EditorTheme.radiusLarge))
        .overlay(RoundedRectangle(cornerRadius: EditorTheme.radiusLarge).strokeBorder(EditorTheme.stroke))
        .clipShape(RoundedRectangle(cornerRadius: EditorTheme.radiusLarge))
    }
}

/// Marks where another worker's content plugs into a section. Shows the current controls until the
/// dedicated view lands; the comment on each call site names who owns the slot.
public struct InspectorSlot<Content: View>: View {
    private let content: Content
    public init(@ViewBuilder _ content: () -> Content) { self.content = content() }
    public var body: some View {
        VStack(alignment: .leading, spacing: EditorTheme.spacingLoose) { content }
    }
}
