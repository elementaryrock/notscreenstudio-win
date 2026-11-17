import AppKit
import Foundation
import NSSCore
import NSSExport
import Observation

/// State of the export sheet. The `Exporter` is injected; progress arrives on arbitrary threads and is hopped to main.
@MainActor
@Observable
public final class ExportModel {
    public var resolution: ExportResolution = .p1080
    public var codec: VideoCodec = .h264
    public var outputURL: URL
    public private(set) var progress: ExportProgress = .idle

    @ObservationIgnored private let editor: EditorModel
    @ObservationIgnored private var task: Task<Void, Never>?

    public init(editor: EditorModel) {
        self.editor = editor
        outputURL = Self.defaultOutputURL(projectName: editor.project.name)
    }

    public static func defaultOutputURL(projectName: String, in dir: URL? = nil) -> URL {
        let folder = dir ?? FileManager.default.urls(for: .moviesDirectory, in: .userDomainMask)[0]
        let safe = projectName.replacingOccurrences(of: "/", with: "-").trimmingCharacters(in: .whitespaces)
        return folder.appendingPathComponent(safe.isEmpty ? "Recording" : safe).appendingPathExtension("mp4")
    }

    public var aspect: AspectRatio { editor.project.style.aspectRatio }
    public var preset: ExportPreset { ExportPreset(resolution: resolution, fps: 60, codec: codec) }
    public var outputSize: PixelSize { preset.outputSize(for: aspect) }
    public var outputDescription: String {
        "\(InspectorBindings.label(for: aspect)) · \(outputSize.width)×\(outputSize.height)"
    }

    public var isRunning: Bool { if case .running = progress { return true } else { return false } }
    public var fraction: Double { if case .running(let f, _, _) = progress { return f } else { return 0 } }
    public var finishedURL: URL? { if case .finished(let u) = progress { return u } else { return nil } }
    public var failureMessage: String? { if case .failed(let m) = progress { return m } else { return nil } }

    public func start() {
        guard !isRunning else { return }
        let project = editor.project
        let url = editor.reference.url
        let track = editor.track
        let preset = preset
        let out = outputURL
        let exporter = editor.dependencies.exporter
        progress = .running(fraction: 0, framesWritten: 0, totalFrames: 0)
        let sink: @Sendable (ExportProgress) -> Void = { [weak self] p in
            Task { @MainActor in self?.receive(p) }
        }
        task = Task { [weak self] in
            var failure: Error?
            do {
                try await exporter.export(project: project, projectURL: url, track: track, preset: preset, to: out, progress: sink)
            } catch {
                failure = error
            }
            self?.finish(output: out, error: failure)
        }
    }

    private func finish(output: URL, error: Error?) {
        if let error {
            if error is CancellationError { progress = .cancelled } else if !isTerminal { progress = .failed(message: error.localizedDescription) }
        } else if case .running = progress {
            progress = .finished(output)
        }
    }

    private var isTerminal: Bool {
        switch progress {
        case .finished, .failed, .cancelled: true
        default: false
        }
    }

    private func receive(_ p: ExportProgress) {
        // Ignore late updates after cancel/finish.
        if case .cancelled = progress { return }
        if case .running = progress { progress = p } else if case .idle = progress { progress = p }
    }

    public func cancel() {
        task?.cancel()
        progress = .cancelled
    }

    public func reset() { progress = .idle }

    public func revealInFinder() {
        guard let u = finishedURL else { return }
        NSWorkspace.shared.activateFileViewerSelecting([u])
    }
}
