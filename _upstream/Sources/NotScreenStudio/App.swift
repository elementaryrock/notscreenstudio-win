import AppKit
import NSSCore
import NSSProject
import NSSUI
import Observation
import SwiftUI

@main
struct NotScreenStudioApp: App {
    @State private var app: AppModel
    @State private var router = EditorRouter()
    @State private var library: FileProjectLibrary
    @Environment(\.openWindow) private var openWindow

    init() {
        // `--smoke-test`: build every dependency, render one frame, exit. Used by scripts/bundle.sh and CI.
        if CommandLine.arguments.contains("--smoke-test") { SmokeTest.runAndExit() }
        let model = AppModel()
        _app = State(initialValue: model)
        _library = State(initialValue: FileProjectLibrary(libraryURL: model.projectStore.libraryURL))
    }

    var body: some Scene {
        MenuBarExtra {
            MenuBarContent(
                model: app.shell,
                openEditor: { router.reference = app.shell.lastSaved?.reference ?? app.shell.latestProject; show("editor") },
                openPermissions: { show("permissions") })
        } label: {
            MenuBarLabel(
                model: app.shell,
                // A recording that just finished opens straight in the editor.
                openEditor: { router.reference = app.shell.lastSaved?.reference ?? app.shell.latestProject; show("editor") },
                openPermissions: { show("permissions") })
        }
        .menuBarExtraStyle(.window)

        Window("notscreenstudio", id: "editor") {
            EditorWindowContent(model: app.shell, router: router, library: library, dependencies: app.editorDependencies)
        }

        Window("Permissions", id: "permissions") {
            PermissionsView(model: app.shell)
        }
        .windowResizability(.contentSize)
    }

    private func show(_ id: String) {
        openWindow(id: id)
        NSApplication.shared.activate()
    }
}

/// The menu bar icon. It lives as long as the app, so it also launches the shell and opens windows on request.
struct MenuBarLabel: View {
    var model: ShellModel
    var openEditor: () -> Void
    var openPermissions: () -> Void

    var body: some View {
        Group {
            if model.isRecording, let title = model.menuBarTitle {
                // A red record symbol and the running time while recording; a grey pause symbol when paused.
                HStack(spacing: 4) {
                    Image(systemName: model.isPaused ? "pause.circle.fill" : "record.circle.fill")
                        .symbolRenderingMode(.palette)
                        .foregroundStyle(model.isPaused ? Color.orange : Color.red, model.isPaused ? Color.orange : Color.red)
                    Text(title).monospacedDigit()
                }
            } else {
                Image(systemName: model.menuBarSymbol)
            }
        }
            .accessibilityLabel(model.isRecording ? (model.isPaused ? "notscreenstudio, recording paused" : "notscreenstudio, recording") : "notscreenstudio")
            .task { await model.launch() }
            .onChange(of: model.editorRequestID) { openEditor() }
            .onChange(of: model.onboardingRequestID) { openPermissions() }
    }
}

/// Which project the editor window shows. `nil` means the home screen. Lives in the app so it survives the
/// window being closed and reopened.
@MainActor @Observable
final class EditorRouter {
    var reference: ProjectReference?
}

/// Editor window: the home screen when no project is open, the editor otherwise.
struct EditorWindowContent: View {
    var model: ShellModel
    var router: EditorRouter
    var library: FileProjectLibrary
    var dependencies: EditorDependencies

    var body: some View {
        if let reference = router.reference {
            var deps = dependencies
            deps.library = library
            return AnyView(
                EditorView(reference: reference, dependencies: deps, onBack: { router.reference = nil })
                    .id(reference))
        }
        return AnyView(
            HomeView(
                library: library, isRecording: model.isRecording,
                onOpen: { router.reference = $0 },
                onRecord: { model.beginSetup() }))
    }
}
