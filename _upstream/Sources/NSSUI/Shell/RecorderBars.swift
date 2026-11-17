import AppKit
import NSSCore
import Observation
import SwiftUI

/// What the floating recording bar shows and does. Owned by `ShellModel`, which keeps `elapsed` current.
@MainActor
@Observable
public final class RecordingControls {
    /// Recorded seconds. Stops advancing while paused.
    public internal(set) var elapsed: TimeInterval = 0
    public internal(set) var isPaused = false
    public internal(set) var canPause = true
    @ObservationIgnored var onStop: @MainActor () -> Void = {}
    @ObservationIgnored var onTogglePause: @MainActor () -> Void = {}
    @ObservationIgnored var onDiscard: @MainActor () -> Void = {}
    public init() {}
}

// MARK: - Shared controls

private struct IconButton: View {
    let symbol: String
    let label: String
    let help: String
    var glyphColor: Color = .primary
    var fill: Color = .primary.opacity(0.08)
    let action: () -> Void
    var body: some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: 14, weight: .semibold))
                .foregroundStyle(glyphColor)
                .frame(width: BarStyle.control, height: BarStyle.control)
                .contentShape(Circle())
        }
        .buttonStyle(BarButtonStyle(fill: fill))
        .help(help)
        .accessibilityLabel(label)
    }
}

// MARK: - Setup bar (before recording)

extension ShellModel {
    /// SF Symbol for the picked source.
    public var sourceSymbol: String {
        switch Self.resolve(settings.source, in: listing, fallbackDisplay: CGMainDisplayID()).source {
        case .display: return "display"
        case .window: return "macwindow"
        case .area: return "rectangle.dashed"
        }
    }
}

@MainActor
private final class MenuActionTarget: NSObject {
    var actions: [ObjectIdentifier: () -> Void] = [:]
    @objc func fire(_ sender: NSMenuItem) { actions[ObjectIdentifier(sender)]?() }
}

/// Camera or microphone: a toggle pill with a small chevron menu for the device.
private struct DeviceToggle: View {
    let symbolOn: String
    let symbolOff: String
    let label: String
    let enabled: Bool
    let devices: [DeviceItem]
    let selection: String?
    let onToggle: (Bool) -> Void
    let onSelect: (String?) -> Void

    private var selectedName: String {
        guard let id = ShellModel.pickDevice(preferred: selection, from: devices) else { return "None found" }
        return devices.first { $0.id == id }?.name ?? "None found"
    }

    /// Pops up the device list as a native menu. The chevron itself is plain SwiftUI so it also renders in previews.
    private func showMenu() {
        let menu = NSMenu()
        if devices.isEmpty {
            let item = NSMenuItem(title: "No \(label) found", action: nil, keyEquivalent: "")
            item.isEnabled = false
            menu.addItem(item)
        }
        let current = ShellModel.pickDevice(preferred: selection, from: devices)
        let target = MenuActionTarget()
        for device in devices {
            let item = NSMenuItem(title: device.name, action: #selector(MenuActionTarget.fire(_:)), keyEquivalent: "")
            item.target = target
            item.state = device.id == current ? .on : .off
            let id = device.id
            let wasEnabled = enabled
            let select = onSelect, toggle = onToggle
            target.actions[ObjectIdentifier(item)] = {
                select(id)
                if !wasEnabled { toggle(true) }
            }
            menu.addItem(item)
        }
        // popUp runs its own loop and returns after the choice, so `target` stays alive for the action.
        menu.popUp(positioning: nil, at: NSEvent.mouseLocation, in: nil)
    }

    var body: some View {
        HStack(spacing: 0) {
            Button { onToggle(!enabled) } label: {
                Image(systemName: enabled ? symbolOn : symbolOff)
                    .font(.system(size: 14, weight: .medium))
                    .foregroundStyle(enabled ? Color.white : Color.secondary)
                    .frame(width: BarStyle.control, height: BarStyle.control)
                    .contentShape(Capsule())
            }
            .buttonStyle(BarButtonStyle(fill: enabled ? Color.accentColor : .primary.opacity(0.08)))
            .help(enabled ? "Turn \(label) off" : "Turn \(label) on")
            .accessibilityLabel(label.capitalized)
            .accessibilityValue(enabled ? "On" : "Off")

            Button(action: showMenu) {
                Image(systemName: "chevron.down")
                    .font(.system(size: 9, weight: .bold))
                    .foregroundStyle(.secondary)
                    .frame(width: 18, height: BarStyle.control)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help("\(label.capitalized): \(selectedName)")
            .accessibilityLabel("\(label.capitalized) device")
            .accessibilityValue(selectedName)
        }
    }
}

/// macOS-style record button: a ring with a red disc that shrinks a little while pressed.
private struct RecordButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        Inner(configuration: configuration)
    }
    private struct Inner: View {
        let configuration: Configuration
        @State private var hovering = false
        @Environment(\.isFocused) private var focused
        var body: some View {
            let pressed = configuration.isPressed
            ZStack {
                Circle().strokeBorder(Color.primary.opacity(hovering ? 0.9 : 0.7), lineWidth: 2.5)
                Circle().fill(BarStyle.recordRed.opacity(pressed ? 0.85 : 1)).padding(pressed ? 8 : 5)
            }
            .frame(width: BarStyle.recordButton, height: BarStyle.recordButton)
            .contentShape(Circle())
            .overlay(Circle().strokeBorder(Color.accentColor, lineWidth: 2).padding(-3).opacity(focused ? 1 : 0))
            .animation(.spring(duration: 0.2, bounce: 0.2), value: pressed)
            .animation(.easeOut(duration: 0.12), value: hovering)
            .onHover { hovering = $0 }
        }
    }
}

/// The control bar shown after a source is picked and before recording starts.
struct RecorderSetupBar: View {
    var model: ShellModel

    var body: some View {
        SetupBarContent(
            sourceSymbol: model.sourceSymbol, sourceTitle: model.sourceTitle,
            cameraEnabled: model.settings.cameraEnabled, cameras: model.cameras, cameraID: model.settings.cameraDeviceID,
            micEnabled: model.settings.micEnabled, microphones: model.microphones, micID: model.settings.micDeviceID,
            onClose: { model.cancelSetup() },
            onCamera: { model.setCameraEnabled($0) }, onSelectCamera: { model.selectCamera($0) },
            onMic: { model.setMicEnabled($0) }, onSelectMic: { model.selectMicrophone($0) },
            onRecord: { Task { await model.confirmRecord() } })
        .barAppear()
        .task { await model.watchScreenRecordingPermission() }
    }
}

/// Pure presentation of the setup bar, so previews can render it without a model.
struct SetupBarContent: View {
    var sourceSymbol: String
    var sourceTitle: String
    var cameraEnabled: Bool
    var cameras: [DeviceItem]
    var cameraID: String?
    var micEnabled: Bool
    var microphones: [DeviceItem]
    var micID: String?
    var onClose: () -> Void = {}
    var onCamera: (Bool) -> Void = { _ in }
    var onSelectCamera: (String?) -> Void = { _ in }
    var onMic: (Bool) -> Void = { _ in }
    var onSelectMic: (String?) -> Void = { _ in }
    var onRecord: () -> Void = {}

    var body: some View {
        HStack(spacing: BarStyle.unit) {
            IconButton(symbol: "xmark", label: "Close", help: "Close (Esc)", action: onClose)
                .keyboardShortcut(.cancelAction)

            HStack(spacing: BarStyle.unit) {
                Image(systemName: sourceSymbol).foregroundStyle(.secondary)
                Text(sourceTitle)
                    .font(.system(size: 13, weight: .medium))
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .frame(maxWidth: 190, alignment: .leading)
            }
            .padding(.horizontal, 12)
            .frame(height: BarStyle.control)
            .background(Color.primary.opacity(0.06), in: Capsule())
            .accessibilityElement(children: .combine)
            .accessibilityLabel("Source: \(sourceTitle)")

            DeviceToggle(
                symbolOn: "video.fill", symbolOff: "video.slash.fill", label: "camera",
                enabled: cameraEnabled, devices: cameras, selection: cameraID,
                onToggle: onCamera, onSelect: onSelectCamera)
            DeviceToggle(
                symbolOn: "mic.fill", symbolOff: "mic.slash.fill", label: "microphone",
                enabled: micEnabled, devices: microphones, selection: micID,
                onToggle: onMic, onSelect: onSelectMic)

            Button(action: onRecord) { EmptyView() }
                .buttonStyle(RecordButtonStyle())
                .keyboardShortcut(.defaultAction)
                .help("Start recording (Return)")
                .accessibilityLabel("Start recording")
        }
        .padding(BarStyle.unit)
        .barSurface()
    }
}

// MARK: - Recording bar

/// The small bar shown while recording: red dot, elapsed time, pause or resume, stop.
struct RecordingBar: View {
    var controls: RecordingControls

    var body: some View {
        RecordingBarContent(
            elapsed: controls.elapsed, isPaused: controls.isPaused, canPause: controls.canPause,
            onTogglePause: { controls.onTogglePause() }, onStop: { controls.onStop() })
        .barAppear()
    }
}

struct RecordingBarContent: View {
    var elapsed: TimeInterval
    var isPaused: Bool
    var canPause: Bool
    var onTogglePause: () -> Void = {}
    var onStop: () -> Void = {}
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        HStack(spacing: BarStyle.unit) {
            dot.padding(.leading, 12)
            VStack(alignment: .leading, spacing: 0) {
                Text(ElapsedFormat.string(elapsed))
                    .font(.system(size: 15, weight: .semibold).monospacedDigit())
                    .foregroundStyle(isPaused ? Color.secondary : Color.primary)
                if isPaused {
                    Text("Paused").font(.system(size: 10, weight: .medium)).foregroundStyle(.secondary)
                }
            }
            .frame(minWidth: 52, alignment: .leading)
            .accessibilityElement(children: .combine)
            .accessibilityLabel(isPaused ? "Paused at \(ElapsedFormat.string(elapsed))" : "Recording, \(ElapsedFormat.string(elapsed))")
            if canPause {
                IconButton(
                    symbol: isPaused ? "play.fill" : "pause.fill",
                    label: isPaused ? "Resume recording" : "Pause recording",
                    help: isPaused ? "Resume recording" : "Pause recording", action: onTogglePause)
            }
            IconButton(
                symbol: "stop.fill", label: "Stop recording", help: "Stop recording (\(StopShortcut.display))",
                glyphColor: BarStyle.recordRed, fill: BarStyle.recordRed.opacity(0.16), action: onStop)
        }
        .padding(BarStyle.unit)
        .barSurface()
    }

    @ViewBuilder private var dot: some View {
        let base = Circle().frame(width: 10, height: 10)
        if isPaused {
            base.foregroundStyle(BarStyle.pausedAmber.opacity(0.55))
        } else if reduceMotion {
            base.foregroundStyle(BarStyle.recordRed)
        } else {
            base.foregroundStyle(BarStyle.recordRed)
                .phaseAnimator([1.0, 0.4]) { v, o in v.opacity(o) } animation: { _ in .easeInOut(duration: 0.9) }
        }
    }
}

// MARK: - Panels

/// Borderless panel that can take keys (Return, Esc) without activating the app being recorded.
private final class BarPanel: NSPanel {
    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { false }
}

/// Hosts one bar at the bottom centre of the screen that holds the picked source. Above every window, on every
/// Space, and kept out of the recording.
@MainActor
final class FloatingBarWindow {
    private var panel: NSPanel?

    var isShown: Bool { panel != nil }

    func show<Content: View>(_ content: Content, over source: CGRect?, makeKey: Bool) {
        hide()
        let host = NSHostingView(rootView: content)
        let size = host.fittingSize
        let panel = BarPanel(
            contentRect: CGRect(origin: .zero, size: size),
            styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        panel.contentView = host
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = false // the bar draws its own layered shadow
        panel.level = .statusBar
        panel.isMovableByWindowBackground = true
        panel.hidesOnDeactivate = false
        panel.isReleasedWhenClosed = false
        panel.excludeFromCapture()
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary]
        let screens = NSScreen.screens
        if !screens.isEmpty {
            let i = BarPlacement.screenIndex(for: source, screens: screens.map(\.frame))
            var frame = BarPlacement.frame(size: size, in: screens[i].visibleFrame)
            // The panel carries `shadowInset` of transparent padding; keep the visible bar where it sat before (10 pt).
            frame.origin.y -= max(0, BarStyle.shadowInset - 10)
            panel.setFrame(frame, display: false)
        }
        if makeKey { panel.makeKeyAndOrderFront(nil) } else { panel.orderFrontRegardless() }
        self.panel = panel
    }

    func hide() {
        panel?.orderOut(nil)
        panel = nil
    }
}
