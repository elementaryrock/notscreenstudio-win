import AVFoundation
import CoreVideo
import Foundation
import Testing
import NSSCore
import NSSRender
@testable import NSSUI

/// Writes a short solid-colour .mov so the real AVPlayer / video-output path can be exercised without a GPU.
func writeTestMovie(to url: URL, seconds: Double = 1, fps: Int32 = 30, size: Int = 64) async throws {
    let writer = try AVAssetWriter(outputURL: url, fileType: .mov)
    let input = AVAssetWriterInput(mediaType: .video, outputSettings: [
        AVVideoCodecKey: AVVideoCodecType.h264, AVVideoWidthKey: size, AVVideoHeightKey: size,
    ])
    let adaptor = AVAssetWriterInputPixelBufferAdaptor(assetWriterInput: input, sourcePixelBufferAttributes: [
        kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
        kCVPixelBufferWidthKey as String: size, kCVPixelBufferHeightKey as String: size,
    ])
    writer.add(input)
    writer.startWriting()
    writer.startSession(atSourceTime: .zero)
    let frames = Int(seconds * Double(fps))
    for i in 0..<frames {
        while !input.isReadyForMoreMediaData { try await Task.sleep(for: .milliseconds(2)) }
        let pb = try PixelBuffers.make(width: size, height: size)
        CVPixelBufferLockBaseAddress(pb, [])
        if let base = CVPixelBufferGetBaseAddress(pb) { memset(base, Int32(i * 255 / max(frames, 1)), CVPixelBufferGetBytesPerRow(pb) * size) }
        CVPixelBufferUnlockBaseAddress(pb, [])
        adaptor.append(pb, withPresentationTime: CMTime(value: CMTimeValue(i), timescale: fps))
    }
    input.markAsFinished()
    await writer.finishWriting()
    if writer.status != .completed { throw writer.error ?? NSSError.io("writer failed") }
}

/// True when this machine can encode the H.264 test movie (CI runners without a hardware encoder skip).
let canEncodeTestMovie: Bool = {
    let settings: [String: Any] = [AVVideoCodecKey: AVVideoCodecType.h264, AVVideoWidthKey: 64, AVVideoHeightKey: 64]
    guard let w = try? AVAssetWriter(outputURL: FileManager.default.temporaryDirectory.appendingPathComponent("probe-\(UUID().uuidString).mov"), fileType: .mov) else { return false }
    return w.canApply(outputSettings: settings, forMediaType: .video)
}()

final class CallCounter: @unchecked Sendable {
    private let lock = NSLock()
    private var n = 0
    var count: Int { lock.lock(); defer { lock.unlock() }; return n }
    func bump() { lock.lock(); n += 1; lock.unlock() }
}

@MainActor
func makeProjectDir(duration: Double, factory: @escaping MotionModelFactory = { _, _, _, _ in IdentityMotion() }) async throws -> (EditorModel, URL) {
    let dir = FileManager.default.temporaryDirectory.appendingPathComponent("nss-editor-\(UUID().uuidString).nssproj")
    try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    try await writeTestMovie(to: dir.appendingPathComponent("screen.mov"))
    let assets = RecordingAssets(sourcePixelSize: PixelSize(width: 64, height: 64), frameRate: 30, duration: duration)
    let deps = EditorDependencies(store: RecordingStore(), motionFactory: factory, renderer: PassthroughRenderer(), exporter: ScriptedExporter())
    return (EditorModel(project: Project(name: "t", assets: assets), reference: ProjectReference(url: dir), track: .empty, dependencies: deps), dir)
}

@MainActor
@Suite(.enabled(if: canEncodeTestMovie)) struct PreviewEngineTests {
    @Test func motionRebuildsCoalesceToOneInFlightPlusOnePending() async throws {
        let calls = CallCounter()
        let (model, dir) = try await makeProjectDir(duration: 1, factory: { _, _, _, _ in calls.bump(); Thread.sleep(forTimeInterval: 0.05); return IdentityMotion() })
        defer { try? FileManager.default.removeItem(at: dir) }
        let engine = PreviewEngine(model: model)
        model.changeHandler = { [engine] in engine.modelChanged($0) }
        for i in 0..<30 { model.setStyle(\.cursor.size, 1 + Double(i) * 0.1) }   // a slider drag
        try await Task.sleep(for: .milliseconds(500))
        #expect(calls.count <= 3)   // initial build + at most one in flight + one pending
        #expect(engine.motionBuildCount <= 3)
        engine.teardown()
    }

    @Test func trimAndUndoSeekThePlayers() async throws {
        let (model, dir) = try await makeProjectDir(duration: 1)
        defer { try? FileManager.default.removeItem(at: dir) }
        let engine = PreviewEngine(model: model)
        model.changeHandler = { [engine] in engine.modelChanged($0) }
        engine.seek(to: 0.2)
        model.setTrimStart(0.4)
        #expect(model.playhead == 0.4)
        #expect(engine.lastSeekTarget == 0.4)
        model.undo()   // trim back to 0: playhead stays valid and the players match it
        #expect(engine.lastSeekTarget == model.playhead)
        model.redo()
        #expect(model.playhead >= 0.4)
        #expect(engine.lastSeekTarget == model.playhead)
        engine.teardown()
    }


    @Test func seekDecodesAndPresentsFramesWithoutBlockingMain() async throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("nss-editor-\(UUID().uuidString).nssproj")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        try await writeTestMovie(to: dir.appendingPathComponent("screen.mov"))

        let assets = RecordingAssets(sourcePixelSize: PixelSize(width: 64, height: 64), frameRate: 30, duration: 1)
        let deps = EditorDependencies(store: RecordingStore(), motionFactory: { _, _, _, _ in IdentityMotion() }, renderer: PassthroughRenderer(), exporter: ScriptedExporter())
        let model = EditorModel(project: Project(name: "t", assets: assets), reference: ProjectReference(url: dir), track: .empty, dependencies: deps)
        let engine = PreviewEngine(model: model)
        model.changeHandler = { [engine] in engine.modelChanged($0) }
        var presented = 0
        var size = PixelSize(width: 0, height: 0)
        engine.present = { pb in
            presented += 1
            size = PixelSize(width: CVPixelBufferGetWidth(pb), height: CVPixelBufferGetHeight(pb))
        }
        engine.seek(to: 0.5)
        // Wait for the event itself (up to 30 s) so a loaded machine cannot fail the functional check.
        for _ in 0..<3000 where presented == 0 {
            engine.tick()
            try await Task.sleep(for: .milliseconds(10))
        }
        #expect(presented > 0)
        #expect(size == PreviewEngine.previewSize(for: .r16x9))
        #expect(model.playhead == 0.5)

        // Style changes re-render a paused preview immediately (aspect change reallocates the pool once).
        let before = presented
        model.setStyle(\.aspectRatio, .r1x1)
        for _ in 0..<3000 where presented == before {
            engine.tick()
            try await Task.sleep(for: .milliseconds(10))
        }
        #expect(presented > before)
        #expect(size == PreviewEngine.previewSize(for: .r1x1))
        engine.teardown()
    }

    @Test func playbackStopsWhenFileEndsBeforeRecordedDuration() async throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("nss-editor-\(UUID().uuidString).nssproj")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        try await writeTestMovie(to: dir.appendingPathComponent("screen.mov"))

        // Recorded duration is longer than the 1 s file.
        let assets = RecordingAssets(sourcePixelSize: PixelSize(width: 64, height: 64), frameRate: 30, duration: 1.2)
        let deps = EditorDependencies(store: RecordingStore(), motionFactory: { _, _, _, _ in IdentityMotion() }, renderer: PassthroughRenderer(), exporter: ScriptedExporter())
        let model = EditorModel(project: Project(name: "t", assets: assets), reference: ProjectReference(url: dir), track: .empty, dependencies: deps)
        let engine = PreviewEngine(model: model)
        for _ in 0..<50 { engine.tick(); try await Task.sleep(for: .milliseconds(10)) }

        engine.play()
        #expect(model.isPlaying)
        for _ in 0..<3000 where model.isPlaying {
            engine.tick()
            try await Task.sleep(for: .milliseconds(10))
        }
        #expect(!model.isPlaying)

        // Pressing play again restarts from the trim start.
        engine.play()
        #expect(model.isPlaying)
        #expect(model.playhead < 0.5)
        engine.teardown()
    }

    @Test func previewSizesKeepAspect() {
        for a in AspectRatio.allCases {
            let s = PreviewEngine.previewSize(for: a)
            #expect(abs(s.aspect - a.ratio) < 0.01)
        }
    }
}
