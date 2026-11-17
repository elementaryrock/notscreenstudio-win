import AppKit
import SwiftUI
import NSSCore

/// Slider row that groups a whole drag into a single undo step.
struct EditSlider: View {
    let model: EditorModel
    let title: String
    let value: Binding<Double>
    let range: ClosedRange<Double>
    var format: (Double) -> String = InspectorFormat.percent

    var body: some View {
        InspectorSliderRow(title, value: value, range: range, format: format) { editing in
            if editing { model.beginGesture(title) } else { model.endGesture() }
        }
    }
}

/// The inspector: collapsible sections in a fixed order (Background, Frame, Cursor, Zoom, Camera, Audio).
///
/// Worker slots: the Background and Zoom sections each contain one clearly marked slot. The Background slot
/// holds `BackgroundPicker(model:)`; worker C replaces `zoomSlot` with `ZoomInspector(model:)`.
struct InspectorView: View {
    let model: EditorModel

    var body: some View {
        ScrollView {
            VStack(spacing: EditorTheme.spacing) {
                InspectorSection(.background) { backgroundSlot }
                InspectorSection(.frame) { frameContent }
                InspectorSection(.cursor) { cursorContent }
                InspectorSection(.zoom) { zoomSlot }
                InspectorSection(.camera) { cameraContent }
                InspectorSection(.audio) { audioContent }
            }
            .padding(EditorTheme.padding)
        }
        .frame(width: EditorTheme.inspectorWidth)
        .background(EditorTheme.panel)
    }

    // MARK: SLOT (worker C): Zoom. Replace the body of `zoomSlot` with `ZoomInspector(model: model)`.

    private var zoomSlot: some View {
        ZoomInspector(model: model)
    }

    // MARK: Background (worker B: BackgroundPicker in Editor/Inspector/)

    private var backgroundSlot: some View {
        BackgroundPicker(model: model)
    }

    // MARK: Frame

    private var frameContent: some View {
        InspectorSlot {
            EditSlider(model: model, title: "Padding", value: model.styleBinding(\.padding, name: "Padding"), range: 0...0.2)
            EditSlider(model: model, title: "Corner radius", value: model.styleBinding(\.cornerRadius, name: "Corner Radius"), range: 0...0.08)
            EditSlider(model: model, title: "Shadow blur", value: model.styleBinding(\.shadow.radius, name: "Shadow"), range: 0...0.1)
            EditSlider(model: model, title: "Shadow opacity", value: model.styleBinding(\.shadow.opacity, name: "Shadow"), range: 0...1)
        }
    }

    // MARK: Cursor

    private var cursorContent: some View {
        InspectorSlot {
            EditSlider(model: model, title: "Size", value: model.styleBinding(\.cursor.size, name: "Cursor Size"), range: 0.5...4, format: InspectorFormat.multiplier)
            EditSlider(model: model, title: "Smoothing", value: model.styleBinding(\.cursor.smoothing, name: "Cursor Smoothing"), range: 0...1)
            EditSlider(model: model, title: "Motion blur", value: model.styleBinding(\.cursor.motionBlur, name: "Motion Blur"), range: 0...1)
            InspectorToggleRow("Click ripples", isOn: model.styleBinding(\.cursor.clickRipples, name: "Click Ripples"))
            InspectorToggleRow("Hide when idle", isOn: model.styleBinding(\.cursor.hideWhenIdle, name: "Hide Cursor When Idle"))
        }
    }

    // MARK: Camera

    @ViewBuilder private var cameraContent: some View {
        if model.project.assets.cameraFile == nil {
            InspectorHint("This recording has no camera. Turn the camera on before recording to add a bubble.")
        } else {
            let cam = model.project.style.camera
            InspectorSlot {
                InspectorToggleRow("Visible", isOn: model.styleBinding(\.camera.visible, name: "Camera Visibility"))
                InspectorSegmentedRow("Shape", selection: Binding(
                    get: { InspectorBindings.kind(of: cam.shape) },
                    set: { model.setStyle(\.camera.shape, InspectorBindings.shape($0), name: "Camera Shape") }
                ), options: InspectorBindings.ShapeKind.allCases.map { ($0, $0.rawValue) })
                EditSlider(model: model, title: "Roundness", value: model.styleBinding(\.camera.roundness, name: "Camera Roundness"), range: 0...1)
                    .disabled(cam.shape == .circle)
                    .opacity(cam.shape == .circle ? 0.4 : 1)
                EditSlider(model: model, title: "Size", value: model.styleBinding(\.camera.size, name: "Camera Size"), range: 0.08...0.4)
                EditSlider(model: model, title: "Shrink when zoomed", value: model.styleBinding(\.camera.zoomedScale, name: "Camera Zoomed Size"),
                           range: InspectorBindings.zoomedScaleRange)
                InspectorToggleRow("Border", isOn: Binding(
                    get: { cam.border != nil },
                    set: { model.setStyle(\.camera.border, $0 ? CameraBorder() : nil, name: "Camera Border") }
                ))
                if let b = cam.border {
                    EditSlider(model: model, title: "Border width", value: Binding(
                        get: { b.width },
                        set: { model.setStyle(\.camera.border, CameraBorder(width: $0, color: b.color), name: "Camera Border") }
                    ), range: 0.001...0.015, format: InspectorFormat.tenthsPercent)
                    InspectorColorRow("Border color", color: Binding(
                        get: { InspectorBindings.color(b.color) },
                        set: { model.setStyle(\.camera.border, CameraBorder(width: b.width, color: InspectorBindings.rgba($0)), name: "Camera Border", coalescingKey: "bordercolor") }
                    ))
                }
                InspectorToggleRow("Mirror", isOn: model.styleBinding(\.camera.mirror, name: "Camera Mirror"))
                InspectorHint("Drag the bubble in the preview to move it.")
            }
        }
    }

    // MARK: Audio

    private var audioContent: some View {
        let a = model.project.assets
        return InspectorSlot {
            audioStatus("Microphone", on: a.micFile != nil, symbol: "mic.fill")
            InspectorHint("Audio from the recording plays in the preview and is included in the export. Trim changes apply to the sound too.")
        }
    }

    private func audioStatus(_ title: String, on: Bool, symbol: String) -> some View {
        HStack {
            Label(title, systemImage: symbol).editorLabel(.row)
            Spacer()
            Text(on ? "Recorded" : "Not recorded").editorLabel(.value)
        }
    }
}
