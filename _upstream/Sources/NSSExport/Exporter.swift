import Foundation
import NSSCore

/// Export entry point contract.
///
/// `export` renders the project's trimmed range at `preset` and writes an MP4 to `outputURL`.
/// It reports progress through `progress`, ending with `.finished`, `.failed` or `.cancelled`.
/// Cancel by cancelling the calling `Task`: the call then throws `CancellationError` and leaves nothing
/// behind. Errors are `NSSError` values.
public protocol Exporter: Sendable {
    func export(
        project: Project,
        projectURL: URL,
        track: CursorTrack,
        preset: ExportPreset,
        to outputURL: URL,
        progress: @escaping @Sendable (ExportProgress) -> Void
    ) async throws
}

/// A `FrameSource` that can fail mid-stream. The `FrameSource` API can only return nil, so the exporter
/// polls this after every frame request and aborts with the error instead of exporting held frames.
public protocol DecodeErrorReporting: Sendable {
    /// First decode error seen on the screen stream, if any.
    var screenError: NSSError? { get }
    /// First decode error seen on the camera stream, if any.
    var cameraError: NSSError? { get }
}
