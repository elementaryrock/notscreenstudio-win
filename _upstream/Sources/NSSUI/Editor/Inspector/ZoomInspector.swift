import SwiftUI
import NSSCore

/// Zoom section content. With a zoom selected: zoom level, follow cursor or fixed point (picked in the
/// preview or with x/y sliders) and the ease-in / ease-out durations. Each slider drag is one undo step.
/// Always: add a zoom, and the auto-zoom actions (also on the Zoom track label).
struct ZoomInspector: View {
    let model: EditorModel

    var body: some View {
        VStack(alignment: .leading, spacing: EditorTheme.spacingLoose) {
            if let z = model.selectedZoom {
                editor(z)
                Divider().overlay(EditorTheme.stroke)
            } else {
                InspectorHint("Select a zoom on the Zoom track to edit it, or double-click the track to add one.")
            }
            InspectorButtonRow(button: "Add zoom at playhead", symbol: "plus") { model.addZoomBlock() }
            autoActions
        }
    }

    private var autoActions: some View {
        let autoCount = model.autoZoomCount
        let hasClicks = model.track.clicks.contains { $0.kind == .down }
        return VStack(alignment: .leading, spacing: EditorTheme.spacing) {
            InspectorButtonRow(button: "Regenerate auto zooms from clicks", symbol: "wand.and.stars") { model.regenerateAutoZooms() }
                .disabled(!hasClicks)
            InspectorButtonRow(button: autoCount > 0 ? "Remove all auto zooms (\(autoCount))" : "Remove all auto zooms",
                               symbol: "trash", destructive: true) { model.removeAllAutoZooms() }
                .disabled(autoCount == 0)
            InspectorHint("Auto zooms are made from your clicks and marked with a sparkle. Your own zooms, and auto zooms you edit, are kept.")
        }
    }

    private func editor(_ z: ZoomBlock) -> some View {
        VStack(alignment: .leading, spacing: EditorTheme.spacingLoose) {
            HStack(spacing: EditorTheme.spacingTight) {
                Text(TimelineGeometry.label(for: z.start) + " – " + TimelineGeometry.label(for: z.end)).editorLabel(.value)
                Spacer()
                if z.isAuto {
                    Label("Auto", systemImage: "sparkle").editorLabel(.caption)
                        .help("Made from clicks. Editing it makes it yours, so Regenerate keeps it.")
                }
            }

            EditSlider(model: model, title: "Zoom level", value: Binding(get: { z.scale }, set: { model.setZoomScale(z.id, $0) }),
                       range: ZoomBlockEditing.scaleRange, format: InspectorFormat.multiplier)

            InspectorSegmentedRow("Focus", selection: Binding(
                get: { ZoomFocusMode(z.focus) },
                set: { mode in
                    switch mode {
                    case .follow: model.setZoomFocus(z.id, .auto)
                    case .fixed: model.setZoomFocus(z.id, .manual(Self.defaultFixedPoint(for: z, model: model)))
                    }
                }
            ), options: [(.follow, "Follow cursor"), (.fixed, "Fixed point")])

            if case .manual(let p) = z.focus {
                EditSlider(model: model, title: "Focus X", value: Binding(
                    get: { p.x }, set: { model.setZoomFocus(z.id, .manual(NormalizedPoint(x: $0, y: p.y))) }
                ), range: 0...1)
                EditSlider(model: model, title: "Focus Y", value: Binding(
                    get: { p.y }, set: { model.setZoomFocus(z.id, .manual(NormalizedPoint(x: p.x, y: $0))) }
                ), range: 0...1)
                InspectorButtonRow(button: model.pickingFocusFor == z.id ? "Click in the preview…" : "Pick point in preview", symbol: "scope") {
                    model.pickingFocusFor = model.pickingFocusFor == z.id ? nil : z.id
                }
            } else {
                InspectorHint("The view pans to keep the cursor in frame.")
            }

            EditSlider(model: model, title: "Ease in", value: Binding(
                get: { z.easeIn }, set: { model.setZoomEase(z.id, easeIn: $0) }
            ), range: ZoomBlockEditing.easeRange, format: Self.seconds)
            EditSlider(model: model, title: "Ease out", value: Binding(
                get: { z.easeOut }, set: { model.setZoomEase(z.id, easeOut: $0) }
            ), range: ZoomBlockEditing.easeRange, format: Self.seconds)

            InspectorButtonRow(button: "Delete zoom", symbol: "trash", destructive: true) { model.deleteZoomBlock(z.id) }
                .help("Delete or Backspace on the timeline does the same")
        }
    }

    /// Switching to a fixed point starts where the cursor is at the block's start, so the view
    /// doesn't jump somewhere unrelated.
    static func defaultFixedPoint(for z: ZoomBlock, model: EditorModel) -> NormalizedPoint {
        model.track.rawPosition(at: z.start)?.clamped ?? .center
    }

    static let seconds: (Double) -> String = { String(format: "%.2f s", $0) }
}

enum ZoomFocusMode: Hashable {
    case follow, fixed
    init(_ f: ZoomFocus) {
        if case .manual = f { self = .fixed } else { self = .follow }
    }
}
