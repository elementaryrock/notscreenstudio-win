import Foundation
import NSSCore
import NSSExport
import NSSMotion
import NSSRender
@testable import NSSUI

final class RecordingStore: ProjectStore, @unchecked Sendable {
    private let lock = NSLock()
    private var _saves: [Project] = []
    var saves: [Project] { lock.lock(); defer { lock.unlock() }; return _saves }
    var libraryURL: URL { URL(fileURLWithPath: "/tmp") }
    func createProject(named name: String) async throws -> ProjectReference { ProjectReference(url: libraryURL) }
    func load(_ ref: ProjectReference) async throws -> Project { throw NSSError.notImplemented("load") }
    func save(_ project: Project, to ref: ProjectReference) async throws {}
    func loadCursorTrack(for ref: ProjectReference, assets: RecordingAssets) async throws -> CursorTrack { .empty }
    func saveCursorTrack(_ track: CursorTrack, for ref: ProjectReference, assets: RecordingAssets) async throws {}
    func scheduleAutosave(_ project: Project, to ref: ProjectReference) { lock.lock(); _saves.append(project); lock.unlock() }
    func listProjects() async throws -> [ProjectReference] { [] }
}

struct ScriptedExporter: Exporter {
    var fail = false
    func export(project: Project, projectURL: URL, track: CursorTrack, preset: ExportPreset, to outputURL: URL, progress: @escaping @Sendable (ExportProgress) -> Void) async throws {
        progress(.running(fraction: 0.5, framesWritten: 5, totalFrames: 10))
        if fail { throw NSSError.io("disk full") }
        progress(.finished(outputURL))
    }
}

@MainActor
func makeModel(duration: MediaTime = 20, store: RecordingStore = RecordingStore(), exporter: any Exporter = ScriptedExporter()) -> EditorModel {
    let assets = RecordingAssets(sourcePixelSize: PixelSize(width: 1920, height: 1080), frameRate: 60, duration: duration)
    let deps = EditorDependencies(store: store, motionFactory: LinearMotionModel.factory, renderer: PassthroughRenderer(), exporter: exporter)
    return EditorModel(
        project: Project(name: "Demo", assets: assets),
        reference: ProjectReference(url: URL(fileURLWithPath: "/tmp/demo.nssproj")),
        track: CursorTrack(clicks: [ClickEvent(t: 1, x: 0.5, y: 0.5, kind: .down)]),
        dependencies: deps
    )
}
