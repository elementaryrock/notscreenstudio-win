import AppKit
import NSSCapture
import NSSCore
import NSSExport
import NSSInput
import NSSMotion
import NSSProject
import NSSRender
import NSSUI
import Observation

/// The app's dependency container. Every module is created here and handed out as its NSSCore protocol,
/// so replacing a stub with the real module is a one-line change at its `let`.
@MainActor
@Observable
final class AppModel {
    // Swap implementations here. Each line is the only place its module is named.
    /// The capture excludes this whole app by process. The window numbers are a fallback for windows whose owner
    /// ScreenCaptureKit does not report.
    @ObservationIgnored let recordingSession: any RecordingSession = ScreenCaptureRecordingSession(ownWindowIDs: {
        await MainActor.run { NSApp.windows.map { $0.windowNumber }.filter { $0 > 0 }.map { UInt32($0) } }
    })
    @ObservationIgnored let inputLogger: any InputLogger = SystemInputLogger()
    @ObservationIgnored let projectStore: any ProjectStore
    @ObservationIgnored let motionFactory: MotionModelFactory = SpringMotionModel.factory
    @ObservationIgnored let renderer: any FrameRenderer
    /// Non-nil when the GPU renderer could not start and the CPU passthrough is in use.
    @ObservationIgnored private(set) var rendererError: String?
    @ObservationIgnored let exporter: any Exporter

    let shell: ShellModel

    /// What the editor window needs, built from the services above.
    var editorDependencies: EditorDependencies {
        EditorDependencies(
            store: projectStore, motionFactory: motionFactory, renderer: renderer, exporter: exporter,
            motionFactoryForAspect: { SpringMotionModel.factory(sourceAspect: $0) })
    }

    init() {
        NSApplication.shared.setActivationPolicy(.accessory)
        let store = FileProjectStore()
        store.installTerminationFlush()
        projectStore = store
        // Metal renderer (falls back to the CPU passthrough only when there is no Metal device). Any other
        // setup failure is shown to the user instead of silently producing flat frames.
        do {
            renderer = try MetalRenderer.makeDefault()
        } catch {
            renderer = PassthroughRenderer()
            rendererError = "The GPU renderer failed to start (\(error)). Previews and exports will show only the background."
        }
        // Preview and export both build motion from the recording's real source aspect.
        exporter = MP4Exporter(
            motionFactory: motionFactory, renderer: renderer,
            motionFactoryForAspect: { SpringMotionModel.factory(sourceAspect: $0) })
        shell = ShellModel(
            services: ShellServices(
                session: recordingSession,
                inputLogger: inputLogger,
                store: projectStore,
                sources: SystemSourceProvider(),
                permissions: SystemPermissions()),
            effects: AppKitShellEffects())
        if let rendererError { shell.showNotice(rendererError) }
    }
}
