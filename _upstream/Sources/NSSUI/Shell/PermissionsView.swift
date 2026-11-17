import AppKit
import SwiftUI

/// First-run onboarding and the fix-it screen: explains each permission, asks for it, or opens the right System Settings pane.
@MainActor
public struct PermissionsView: View {
    @Bindable private var model: ShellModel

    public init(model: ShellModel) { self.model = model }

    public var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Permissions").font(.title2.bold())
            Text("notscreenstudio needs your permission before macOS lets it record. Everything stays on your Mac.")
                .foregroundStyle(.secondary)
            ForEach(PermissionKind.allCases) { kind in
                PermissionRow(kind: kind, state: model.permissionStates[kind] ?? .notDetermined,
                              request: { Task { await model.requestPermission(kind) } },
                              openSettings: { model.openSystemSettings(for: kind) },
                              relaunch: AppRelauncher.canRelaunch ? { model.relaunchApp() } : nil)
            }
            HStack {
                Spacer()
                Button("Check Again") { Task { await model.checkAgain() } }
            }
        }
        .padding(24)
        .frame(width: 480)
        .task { await model.watchScreenRecordingPermission() }
    }
}

private struct PermissionRow: View {
    let kind: PermissionKind
    let state: PermissionState
    let request: () -> Void
    let openSettings: () -> Void
    var relaunch: (() -> Void)?

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: icon).foregroundStyle(color).font(.title2).frame(width: 28)
            VStack(alignment: .leading, spacing: 4) {
                Text(PermissionInfo.title(kind)).font(.headline)
                Text(PermissionInfo.explanation(kind)).font(.callout).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                if state == .denied {
                    Text("Turn it on in System Settings > Privacy & Security > \(PermissionInfo.title(kind)).")
                        .font(.caption).foregroundStyle(.orange)
                }
                if kind == .screenRecording, state != .granted, let relaunch {
                    Text("Already turned on? macOS applies the change only to a new launch.")
                        .font(.caption).foregroundStyle(.secondary)
                    Button("Relaunch notscreenstudio", action: relaunch)
                }
            }
            Spacer()
            switch state {
            case .granted: Text("Allowed").foregroundStyle(.green)
            case .notDetermined: Button("Allow", action: request)
            case .denied: Button("Open Settings", action: openSettings)
            }
        }
    }

    private var icon: String {
        switch state {
        case .granted: return "checkmark.circle.fill"
        case .denied: return "xmark.circle.fill"
        case .notDetermined: return "questionmark.circle"
        }
    }
    private var color: Color {
        switch state {
        case .granted: return .green
        case .denied: return .red
        case .notDetermined: return .secondary
        }
    }
}
