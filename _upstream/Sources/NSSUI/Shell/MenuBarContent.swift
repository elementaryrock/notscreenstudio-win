import AppKit
import SwiftUI
import NSSCore

/// Camera shapes offered in the menu, with the default radius for the rounded one.
public enum ShapeKind: String, CaseIterable, Identifiable, Sendable {
    case circle, square, tall, wide
    public var id: String { rawValue }

    public var title: String {
        switch self {
        case .circle: return "Circle"
        case .square: return "Square"
        case .tall: return "Tall"
        case .wide: return "Wide"
        }
    }

    public var shape: CameraShape {
        switch self {
        case .circle: return .circle
        case .square: return .square
        case .tall: return .tall
        case .wide: return .wide
        }
    }

    public init(_ shape: CameraShape) {
        switch shape {
        case .circle: self = .circle
        case .square: self = .square
        case .tall: self = .tall
        case .wide: self = .wide
        }
    }
}

extension ShellModel {
    /// Short description of what "Record" will capture.
    public var sourceTitle: String {
        let (source, _) = Self.resolve(settings.source, in: listing, fallbackDisplay: CGMainDisplayID())
        switch source {
        case .display(let id): return listing.displays.first { $0.id == id }?.name ?? "Main display"
        case .window(let id): return listing.windows.first { $0.id == id }?.displayName ?? "Window"
        case .area(_, let rect): return "Area \(Int(rect.width)) × \(Int(rect.height))"
        }
    }

    public func isSelected(display: DisplayItem) -> Bool {
        let (source, _) = Self.resolve(settings.source, in: listing, fallbackDisplay: CGMainDisplayID())
        return source == .display(id: display.id)
    }
}

/// The menu bar popover: one big Record button, then source, microphone and camera choices.
@MainActor
public struct MenuBarContent: View {
    @Bindable private var model: ShellModel
    private var openEditor: () -> Void
    private var openPermissions: () -> Void
    @State private var menuWindow = MenuWindowBox()

    public init(
        model: ShellModel,
        openEditor: @escaping () -> Void = {},
        openPermissions: @escaping () -> Void = {}
    ) {
        self.model = model
        self.openEditor = openEditor
        self.openPermissions = openPermissions
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            recordButton
            recordingControls
            banners
            Divider()
            sourceSection
            Divider()
            micSection
            cameraSection
            Divider()
            countdownSection
            Divider()
            footer
        }
        .padding(12)
        .frame(width: 300)
        .excludedFromCapture(when: CapturePrivacyPolicy.shouldExclude(.menu, preparingOrRecording: model.isSettingUp || model.isCountingDown || model.isRecording))
        .background(MenuWindowReader(box: menuWindow).frame(width: 0, height: 0))
        .task { await model.refreshSources() }
    }

    // MARK: Record

    private var recordButton: some View {
        Button {
            if model.isCountingDown { model.cancelCountdown() }
            else if model.isRecording { Task { await model.stopRecording() } }
            else if model.isSettingUp { model.cancelSetup() }
            else { model.beginSetup(); menuWindow.close() }
        } label: {
            HStack {
                Image(systemName: model.isRecording ? "stop.fill" : (model.isCountingDown || model.isSettingUp) ? "xmark" : "record.circle.fill")
                Text(recordTitle).fontWeight(.semibold)
                Spacer()
                Text(model.isRecording ? StopShortcut.display : model.sourceTitle)
                    .font(.caption).opacity(0.8).lineLimit(1)
            }
            .padding(.vertical, 6).padding(.horizontal, 8)
            .frame(maxWidth: .infinity)
        }
        .buttonStyle(.borderedProminent)
        .tint(.red)
        .disabled(!model.canStart && !model.isRecording && !model.isCountingDown && !model.isSettingUp)
    }

    @ViewBuilder private var recordingControls: some View {
        if model.isRecording {
            HStack(spacing: 8) {
                if model.canPause {
                    Button {
                        Task { await model.togglePause() }
                    } label: {
                        Label(model.isPaused ? "Resume" : "Pause", systemImage: model.isPaused ? "play.fill" : "pause.fill")
                            .frame(maxWidth: .infinity)
                    }
                }
                Button(role: .destructive) {
                    Task { await model.discardRecording() }
                } label: {
                    Label("Discard", systemImage: "trash").frame(maxWidth: .infinity)
                }
            }
            .controlSize(.regular)
        }
    }

    private var recordTitle: String {
        switch model.phase {
        case .recording: return model.isPaused ? "Stop Recording (paused \(ElapsedFormat.string(model.elapsed)))" : "Stop Recording \(ElapsedFormat.string(model.elapsed))"
        case .preparing: return model.isCountingDown ? "Cancel Countdown" : "Starting…"
        case .saving: return "Saving…"
        default: return model.isSettingUp ? "Close Recorder" : "Record"
        }
    }

    @ViewBuilder private var banners: some View {
        if case .failed(let message) = model.phase {
            Banner(text: message, systemImage: "exclamationmark.triangle.fill", tint: .orange) { model.dismissError() }
        }
        if let notice = model.notice {
            Banner(text: notice, systemImage: "info.circle.fill", tint: .blue) { model.dismissNotice() }
        }
        if !model.missingRequiredPermissions.isEmpty {
            Button("Screen recording is not allowed yet. Fix…", action: openPermissions)
                .buttonStyle(.link).font(.caption)
        }
    }

    // MARK: Source

    private var sourceSection: some View {
        VStack(alignment: .leading, spacing: 4) {
            SectionTitle("Source")
            ForEach(model.listing.displays) { display in
                SourceRow(title: display.name, systemImage: "display", selected: model.isSelected(display: display)) {
                    model.pickSource(display.isMain ? nil : .display(id: display.id))
                    menuWindow.close()
                }
            }
            Menu {
                if model.listing.windows.isEmpty { Text("No windows found") }
                ForEach(model.listing.windows) { window in
                    Button(window.displayName) {
                        model.pickSource(.window(id: window.id))
                        menuWindow.close()
                    }
                }
            } label: {
                Label(selectedWindowTitle ?? "Window…", systemImage: "macwindow")
            }
            .menuStyle(.borderlessButton)
            Button {
                menuWindow.close()
                Task { await model.chooseArea() }
            } label: {
                Label(selectedAreaTitle ?? "Select Area…", systemImage: "rectangle.dashed")
            }
            .buttonStyle(.plain)
            if let error = model.sourceError {
                Text(error).font(.caption).foregroundStyle(.secondary)
            }
        }
    }

    private var selectedWindowTitle: String? {
        if case .window = model.settings.source { return model.sourceTitle }
        return nil
    }

    private var selectedAreaTitle: String? {
        if case .area = model.settings.source { return model.sourceTitle }
        return nil
    }

    // MARK: Devices

    private var micSection: some View {
        VStack(alignment: .leading, spacing: 4) {
            Toggle("Microphone", isOn: Binding(get: { model.settings.micEnabled }, set: { model.setMicEnabled($0) }))
            if model.settings.micEnabled {
                DevicePicker(
                    devices: model.microphones,
                    selection: Binding(get: { model.settings.micDeviceID }, set: { model.selectMicrophone($0) }))
            }
        }
    }

    private var cameraSection: some View {
        VStack(alignment: .leading, spacing: 4) {
            Toggle("Camera", isOn: Binding(get: { model.settings.cameraEnabled }, set: { model.setCameraEnabled($0) }))
            if model.settings.cameraEnabled {
                DevicePicker(
                    devices: model.cameras,
                    selection: Binding(get: { model.settings.cameraDeviceID }, set: { model.selectCamera($0) }))
                Picker("Shape", selection: Binding(
                    get: { ShapeKind(model.settings.cameraShape) }, set: { model.selectShape($0.shape) })
                ) {
                    ForEach(ShapeKind.allCases) { Text($0.title).tag($0) }
                }
                .pickerStyle(.segmented)
                .labelsHidden()
                if model.settings.cameraShape != .circle {
                    HStack {
                        Text("Corners").font(.caption).foregroundStyle(.secondary)
                        Slider(value: Binding(get: { model.settings.bubbleRoundness }, set: { model.selectRoundness($0) }), in: 0...1)
                    }
                }
            }
        }
    }

    private var countdownSection: some View {
        Picker("Countdown", selection: Binding(get: { model.settings.countdownSeconds }, set: { model.setCountdown($0) })) {
            ForEach(ShellSettings.countdownChoices, id: \.self) { Text($0 == 0 ? "Off" : "\($0) s").tag($0) }
        }
        .pickerStyle(.segmented)
    }

    // MARK: Footer

    private var footer: some View {
        VStack(alignment: .leading, spacing: 4) {
            if model.lastSaved != nil || model.latestProject != nil { Button("Open Last Recording", action: openEditor).buttonStyle(.plain) }
            Button("Permissions…", action: openPermissions).buttonStyle(.plain)
            Button("Quit notscreenstudio") { NSApplication.shared.terminate(nil) }.buttonStyle(.plain)
        }
    }
}

private struct SectionTitle: View {
    let text: String
    init(_ text: String) { self.text = text }
    var body: some View { Text(text.uppercased()).font(.caption2).foregroundStyle(.secondary) }
}

private struct SourceRow: View {
    let title: String
    let systemImage: String
    let selected: Bool
    let action: () -> Void
    var body: some View {
        Button(action: action) {
            HStack {
                Label(title, systemImage: systemImage)
                Spacer()
                if selected { Image(systemName: "checkmark") }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

private struct DevicePicker: View {
    let devices: [DeviceItem]
    @Binding var selection: String?
    var body: some View {
        Picker("Device", selection: Binding(
            get: { ShellModel.pickDevice(preferred: selection, from: devices) ?? "" },
            set: { selection = $0.isEmpty ? nil : $0 })
        ) {
            if devices.isEmpty { Text("None found").tag("") }
            ForEach(devices) { Text($0.name).tag($0.id) }
        }
        .labelsHidden()
        .disabled(devices.isEmpty)
    }
}

private struct Banner: View {
    let text: String
    let systemImage: String
    let tint: Color
    let dismiss: () -> Void
    var body: some View {
        HStack(alignment: .top, spacing: 6) {
            Image(systemName: systemImage).foregroundStyle(tint)
            Text(text).font(.caption).fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 0)
            Button(action: dismiss) { Image(systemName: "xmark") }.buttonStyle(.plain)
        }
        .padding(8)
        .background(tint.opacity(0.12), in: RoundedRectangle(cornerRadius: 8))
    }
}

/// Finds the menu bar popover's window so picking a source can close it.
@MainActor
final class MenuWindowBox {
    weak var window: NSWindow?
    func close() { window?.orderOut(nil) }
}

private struct MenuWindowReader: NSViewRepresentable {
    let box: MenuWindowBox
    final class Reader: NSView {
        var box: MenuWindowBox?
        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            let w = window
            MainActor.assumeIsolated { box?.window = w }
        }
    }
    func makeNSView(context: Context) -> Reader { let r = Reader(); r.box = box; return r }
    func updateNSView(_ nsView: Reader, context: Context) { nsView.box = box }
}
