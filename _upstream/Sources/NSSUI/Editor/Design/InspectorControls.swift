import AppKit
import SwiftUI

/// Standard inspector rows. Every control in every section is one of these, so spacing, label style
/// and control height are identical everywhere.

/// Title on the left, value read-out on the right, slider underneath. `onEditingChanged` lets the caller
/// group a whole drag into one undo step.
public struct InspectorSliderRow: View {
    let title: String
    @Binding var value: Double
    let range: ClosedRange<Double>
    var format: (Double) -> String
    var onEditingChanged: (Bool) -> Void

    public init(
        _ title: String, value: Binding<Double>, range: ClosedRange<Double>,
        format: @escaping (Double) -> String = InspectorFormat.percent,
        onEditingChanged: @escaping (Bool) -> Void = { _ in }
    ) {
        self.title = title
        _value = value
        self.range = range
        self.format = format
        self.onEditingChanged = onEditingChanged
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: EditorTheme.spacingTight) {
            HStack {
                Text(title).editorLabel(.row)
                Spacer()
                Text(format(value)).editorLabel(.value)
            }
            Slider(value: $value, in: range, onEditingChanged: onEditingChanged)
                .controlSize(.small)
                .tint(EditorTheme.accent)
        }
        .accessibilityElement(children: .combine)
    }
}

public struct InspectorToggleRow: View {
    let title: String
    @Binding var isOn: Bool
    public init(_ title: String, isOn: Binding<Bool>) { self.title = title; _isOn = isOn }
    public var body: some View {
        HStack {
            Text(title).editorLabel(.row)
            Spacer()
            Toggle(title, isOn: $isOn).labelsHidden().toggleStyle(.switch).controlSize(.small)
        }
        .frame(minHeight: EditorTheme.rowControlHeight)
    }
}

/// Segmented control with a title above it. Options are `(value, label)` pairs.
public struct InspectorSegmentedRow<Value: Hashable>: View {
    let title: String?
    @Binding var selection: Value
    let options: [(value: Value, label: String)]
    public init(_ title: String? = nil, selection: Binding<Value>, options: [(value: Value, label: String)]) {
        self.title = title
        _selection = selection
        self.options = options
    }
    public var body: some View {
        VStack(alignment: .leading, spacing: EditorTheme.spacingTight) {
            if let title { Text(title).editorLabel(.row) }
            Picker(title ?? "", selection: $selection) {
                ForEach(options.indices, id: \.self) { Text(options[$0].label).tag(options[$0].value) }
            }
            .pickerStyle(.segmented).labelsHidden().controlSize(.small)
        }
    }
}

/// Title on the left, colour well on the right.
public struct InspectorColorRow: View {
    let title: String
    @Binding var color: Color
    var supportsOpacity = false
    public init(_ title: String, color: Binding<Color>, supportsOpacity: Bool = false) {
        self.title = title
        _color = color
        self.supportsOpacity = supportsOpacity
    }
    public var body: some View {
        HStack {
            Text(title).editorLabel(.row)
            Spacer()
            ColorPicker(title, selection: $color, supportsOpacity: supportsOpacity).labelsHidden()
        }
        .frame(minHeight: EditorTheme.rowControlHeight)
    }
}

/// Title on the left, pop-up menu on the right.
public struct InspectorPickerRow<Value: Hashable>: View {
    let title: String
    @Binding var selection: Value
    let options: [(value: Value, label: String)]
    public init(_ title: String, selection: Binding<Value>, options: [(value: Value, label: String)]) {
        self.title = title
        _selection = selection
        self.options = options
    }
    public var body: some View {
        HStack {
            Text(title).editorLabel(.row)
            Spacer()
            Picker(title, selection: $selection) {
                ForEach(options.indices, id: \.self) { Text(options[$0].label).tag(options[$0].value) }
            }
            .labelsHidden().pickerStyle(.menu).fixedSize()
        }
        .frame(minHeight: EditorTheme.rowControlHeight)
    }
}

/// Title on the left, a button on the right.
public struct InspectorButtonRow: View {
    let title: String?
    let buttonTitle: String
    var symbol: String?
    var destructive = false
    let action: () -> Void
    public init(_ title: String? = nil, button: String, symbol: String? = nil, destructive: Bool = false, action: @escaping () -> Void) {
        self.title = title
        buttonTitle = button
        self.symbol = symbol
        self.destructive = destructive
        self.action = action
    }
    public var body: some View {
        HStack {
            if let title { Text(title).editorLabel(.row); Spacer() }
            Button(action: action) {
                if let symbol { Label(buttonTitle, systemImage: symbol) } else { Text(buttonTitle) }
            }
            .buttonStyle(EditorSecondaryButtonStyle(destructive: destructive))
            if title == nil { Spacer(minLength: 0) }
        }
    }
}

/// Helper text under a group of controls.
public struct InspectorHint: View {
    let text: String
    public init(_ text: String) { self.text = text }
    public var body: some View { Text(text).editorLabel(.caption).fixedSize(horizontal: false, vertical: true) }
}

/// Value formatters shared by the rows.
public enum InspectorFormat {
    public static let percent: @Sendable (Double) -> String = { String(format: "%.0f%%", $0 * 100) }
    public static let multiplier: @Sendable (Double) -> String = { String(format: "%.1f×", $0) }
    public static let degrees: @Sendable (Double) -> String = { String(format: "%.0f°", $0) }
    public static let tenthsPercent: @Sendable (Double) -> String = { String(format: "%.1f%%", $0 * 100) }
}
