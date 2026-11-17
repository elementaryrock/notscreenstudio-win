import AVFoundation
import Foundation
import Testing
import NSSCore
import NSSRender
@testable import NSSExport

private func near(_ a: Double, _ b: Double, _ tol: Double) -> Bool { abs(a - b) <= tol }

@Suite(.serialized) struct NSSExportTests {
    @Test(.enabled(if: h264Available), arguments: AspectRatio.allCases)
    func exports1080pForEveryAspect(aspect: AspectRatio) async throws {
        let b = try await makeBundle(trim: TrimRange(start: 0, end: 0.1), aspect: aspect)
        defer { try? FileManager.default.removeItem(at: b.dir) }
        let out = b.dir.appendingPathComponent("out.mp4")
        let renderer = RecordingRenderer()
        let events = Collector()
        try await MP4Exporter(renderer: renderer).export(
            project: b.project, projectURL: b.dir, track: .empty, preset: ExportPreset(resolution: .p1080), to: out,
            progress: { events.add($0) })

        let expected = ExportPreset(resolution: .p1080).outputSize(for: aspect)
        let p = try await probe(out)
        #expect(p.width == expected.width && p.height == expected.height)
        #expect(near(p.duration, 0.1, 0.03))
        #expect(p.videoFrames == 6)
        #expect(near(p.firstPTS, 0, 1e-6))
        #expect(near(Double(p.nominalFPS), 60, 0.5))
        #expect(p.audioTracks == 0)
        #expect(renderer.calls.count == 6)
        #expect(renderer.calls.allSatisfy { $0.width == expected.width && $0.height == expected.height && $0.ioSurface })
        #expect(events.events.last == .finished(out))
        #expect(events.events.contains { if case .running(let f, _, _) = $0 { f == 1 } else { false } })
    }

    @Test(.enabled(if: h264Available))
    func exports4K() async throws {
        let b = try await makeBundle(trim: TrimRange(start: 0, end: 0.1))
        defer { try? FileManager.default.removeItem(at: b.dir) }
        let out = b.dir.appendingPathComponent("out4k.mp4")
        try await MP4Exporter().export(project: b.project, projectURL: b.dir, track: .empty,
                                       preset: ExportPreset(resolution: .p2160), to: out, progress: { _ in })
        let p = try await probe(out)
        #expect(p.width == 3840 && p.height == 2160)
        #expect(p.videoFrames == 6)
    }

    @Test(.enabled(if: h264Available))
    func square4KAndVertical4KSizes() async throws {
        for (aspect, w, h) in [(AspectRatio.r1x1, 2160, 2160), (.r9x16, 2160, 3840)] {
            let b = try await makeBundle(trim: TrimRange(start: 0, end: 0.05), aspect: aspect)
            defer { try? FileManager.default.removeItem(at: b.dir) }
            let out = b.dir.appendingPathComponent("o.mp4")
            try await MP4Exporter().export(project: b.project, projectURL: b.dir, track: .empty,
                                           preset: ExportPreset(resolution: .p2160), to: out, progress: { _ in })
            let p = try await probe(out)
            #expect(p.width == w && p.height == h)
        }
    }

    @Test(.enabled(if: hevcAvailable))
    func exportsHEVC() async throws {
        let b = try await makeBundle(trim: TrimRange(start: 0, end: 0.2))
        defer { try? FileManager.default.removeItem(at: b.dir) }
        let out = b.dir.appendingPathComponent("hevc.mp4")
        try await MP4Exporter().export(project: b.project, projectURL: b.dir, track: .empty,
                                       preset: ExportPreset(codec: .hevc), to: out, progress: { _ in })
        let p = try await probe(out)
        #expect(p.videoCodec == kCMVideoCodecType_HEVC)
        #expect(p.videoFrames == 12)
    }

    @Test(.enabled(if: h264Available))
    func trimIsRespectedAndFramesMapAtOrBefore() async throws {
        // Source frame k covers [k/30, (k+1)/30). Trim 0.2...0.4 = source frames 6..11.
        let b = try await makeBundle(trim: TrimRange(start: 0.2, end: 0.4))
        defer { try? FileManager.default.removeItem(at: b.dir) }
        let out = b.dir.appendingPathComponent("trim.mp4")
        let renderer = RecordingRenderer()
        try await MP4Exporter(renderer: renderer).export(project: b.project, projectURL: b.dir, track: .empty,
                                                         preset: ExportPreset(), to: out, progress: { _ in })
        let p = try await probe(out)
        #expect(near(p.duration, 0.2, 0.03))
        #expect(p.videoFrames == 12)
        let calls = renderer.calls
        #expect(calls.count == 12)
        #expect(near(calls[0].time, 0.2, 1e-9))
        #expect(near(calls[11].time, 0.2 + 11.0 / 60, 1e-9))
        let indexes = calls.map { $0.gray / 16 }
        #expect(indexes == [6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11])
    }

    @Test(.enabled(if: h264Available))
    func slowSourceHoldsFrames() async throws {
        // A 10 fps source exported at 60 fps holds each frame for 6 outputs.
        let b = try await makeBundle(frames: 2, fps: 10)
        defer { try? FileManager.default.removeItem(at: b.dir) }
        let renderer = RecordingRenderer()
        try await MP4Exporter(renderer: renderer).export(project: b.project, projectURL: b.dir, track: .empty, preset: ExportPreset(),
                                                         to: b.dir.appendingPathComponent("o.mp4"), progress: { _ in })
        let indexes = renderer.calls.map { $0.gray / 16 }
        #expect(indexes.count == 12)
        #expect(indexes == (0..<12).map { $0 / 6 })
    }

    @Test(.enabled(if: h264Available))
    func mixesAudioAsAAC48kStereo() async throws {
        let b = try await makeBundle(mic: true, trim: TrimRange(start: 0.1, end: 0.4))
        defer { try? FileManager.default.removeItem(at: b.dir) }
        let out = b.dir.appendingPathComponent("audio.mp4")
        try await MP4Exporter().export(project: b.project, projectURL: b.dir, track: .empty, preset: ExportPreset(), to: out, progress: { _ in })
        let p = try await probe(out)
        #expect(p.audioTracks == 1)
        #expect(p.audioSampleRate == 48_000)
        #expect(p.audioChannels == 2)
        #expect(near(p.audioDuration, 0.3, 0.06))
        #expect(near(p.duration, 0.3, 0.06))
    }

    @Test(.enabled(if: h264Available))
    func cancellationLeavesNoFiles() async throws {
        let b = try await makeBundle()
        defer { try? FileManager.default.removeItem(at: b.dir) }
        let outDir = b.dir.appendingPathComponent("exports")
        let out = outDir.appendingPathComponent("cancel.mp4")
        let renderer = RecordingRenderer()
        let events = Collector()
        let box = TaskBox()
        renderer.onRender = { n in if n == 3 { box.task?.cancel() } }
        let project = b.project, dir = b.dir
        let task = Task {
            try await MP4Exporter(renderer: renderer).export(project: project, projectURL: dir, track: .empty, preset: ExportPreset(),
                                                             to: out, progress: { events.add($0) })
        }
        box.task = task
        await #expect(throws: CancellationError.self) { try await task.value }
        #expect(!FileManager.default.fileExists(atPath: out.path))
        let leftovers = (try? FileManager.default.contentsOfDirectory(atPath: outDir.path)) ?? []
        #expect(leftovers.isEmpty)
        #expect(events.events.last == .cancelled)
        #expect(renderer.calls.count < 30)
    }

    @Test func missingScreenFileFailsClearlyAndKeepsExistingOutput() async throws {
        let dir = try makeTempDir()
        defer { try? FileManager.default.removeItem(at: dir) }
        let out = dir.appendingPathComponent("keep.mp4")
        try Data("old".utf8).write(to: out)
        let project = Project(name: "t", assets: RecordingAssets(sourcePixelSize: PixelSize(width: 10, height: 10), frameRate: 60, duration: 1))
        let events = Collector()
        await #expect(throws: NSSError.self) {
            try await MP4Exporter().export(project: project, projectURL: dir, track: .empty, preset: ExportPreset(), to: out, progress: { events.add($0) })
        }
        #expect(try Data(contentsOf: out) == Data("old".utf8))
        if case .failed(let message)? = events.events.last { #expect(message.contains("screen.mov")) } else { Issue.record("expected .failed") }
    }

    @Test(.enabled(if: h264Available))
    func emptyTrimFails() async throws {
        let b = try await makeBundle(trim: TrimRange(start: 0.3, end: 0.3))
        defer { try? FileManager.default.removeItem(at: b.dir) }
        await #expect(throws: NSSError.self) {
            try await MP4Exporter().export(project: b.project, projectURL: b.dir, track: .empty, preset: ExportPreset(),
                                           to: b.dir.appendingPathComponent("o.mp4"), progress: { _ in })
        }
    }

    @Test(.enabled(if: h264Available))
    func overwritesExistingOutput() async throws {
        let b = try await makeBundle(trim: TrimRange(start: 0, end: 0.1))
        defer { try? FileManager.default.removeItem(at: b.dir) }
        let out = b.dir.appendingPathComponent("o.mp4")
        try Data("old".utf8).write(to: out)
        try await MP4Exporter().export(project: b.project, projectURL: b.dir, track: .empty, preset: ExportPreset(), to: out, progress: { _ in })
        #expect(try await probe(out).videoFrames == 6)
    }

    @Test(.enabled(if: h264Available))
    func assetFrameSourceSelectsAtOrBeforeAndSeeksBackward() async throws {
        let b = try await makeBundle()
        defer { try? FileManager.default.removeItem(at: b.dir) }
        let source = try await AssetFrameSource.make(screenURL: b.dir.appendingPathComponent("screen.mov"))
        func index(_ t: Double) async -> Int? {
            guard let pb = await source.screenFrame(at: t) else { return nil }
            CVPixelBufferLockBaseAddress(pb, .readOnly); defer { CVPixelBufferUnlockBaseAddress(pb, .readOnly) }
            let base = CVPixelBufferGetBaseAddress(pb)!.assumingMemoryBound(to: UInt8.self)
            let mid = CVPixelBufferGetBytesPerRow(pb) * (CVPixelBufferGetHeight(pb) / 2) + (CVPixelBufferGetWidth(pb) / 2) * 4
            return Int(base[mid]) / 16
        }
        #expect(await index(0) == 0)
        #expect(await index(0.05) == 1)   // 1/30 = 0.0333 <= 0.05 < 0.0667
        #expect(await index(0.2) == 6)
        #expect(await index(0.1) == 3)    // backward seek
        #expect(await index(99) == 14)    // past the end holds the last frame
        #expect(await source.cameraFrame(at: 0) == nil)
        #expect(source.screenError == nil)
    }

    @Test(.enabled(if: h264Available))
    func sparseSourceReturnsAtOrBeforeFrameBeyondLeadIn() async throws {
        // Frames at 0 s, 0.5 s and 3 s (variable frame rate). At t = 2.5 the at-or-before frame is the
        // 0.5 s one, 2 s behind the request, which is more than the 1 s restart lead-in.
        let dir = try makeTempDir()
        defer { try? FileManager.default.removeItem(at: dir) }
        let url = dir.appendingPathComponent("screen.mov")
        try await writeSparseVideo(to: url, pts: [0, 0.5, 3.0], endTime: 3.2)
        let source = try await AssetFrameSource.make(screenURL: url)
        #expect(await source.screenFrame(at: 2.5).map(grayIndex) == 1)
        #expect(await source.screenFrame(at: 2.9).map(grayIndex) == 1)
        #expect(await source.screenFrame(at: 3.0).map(grayIndex) == 2)
        #expect(await source.screenFrame(at: 0.2).map(grayIndex) == 0)   // backward
        #expect(await source.screenFrame(at: 2.5).map(grayIndex) == 1)   // forward jump past the lead-in
        #expect(source.screenError == nil)
    }

    @Test(.enabled(if: h264Available))
    func exportWithSparseSourceAndLateTrimStart() async throws {
        let dir = try makeTempDir()
        defer { try? FileManager.default.removeItem(at: dir) }
        try await writeSparseVideo(to: dir.appendingPathComponent("screen.mov"), pts: [0, 0.5, 3.0], endTime: 3.2)
        var project = Project(name: "t", assets: RecordingAssets(sourcePixelSize: PixelSize(width: 320, height: 180), frameRate: 30, duration: 3.2),
                              timeline: Timeline(trim: TrimRange(start: 2.95, end: 3.05)))
        project.style.aspectRatio = .r16x9
        let renderer = RecordingRenderer()
        try await MP4Exporter(renderer: renderer).export(project: project, projectURL: dir, track: .empty, preset: ExportPreset(),
                                                         to: dir.appendingPathComponent("o.mp4"), progress: { _ in })
        // 6 frames at 2.95, 2.967, 2.983, 3.0, 3.017, 3.033: the 0.5 s frame, then the 3 s frame. Never a future one.
        #expect(renderer.calls.map { $0.gray / 16 } == [1, 1, 1, 2, 2, 2])
    }

    @Test(.enabled(if: h264Available))
    func holdsFirstFrameBeforeTrackStartWithoutRestarting() async throws {
        let dir = try makeTempDir()
        defer { try? FileManager.default.removeItem(at: dir) }
        let url = dir.appendingPathComponent("screen.mov")
        try await writeSparseVideo(to: url, pts: [1.0, 1.5], endTime: 2.0)
        let source = try await AssetFrameSource.make(screenURL: url)
        #expect(await source.screenFrame(at: 0.2).map(grayIndex) == 0)
        #expect(await source.screenFrame(at: 0.3).map(grayIndex) == 0)
        #expect(await source.screenFrame(at: 1.2).map(grayIndex) == 0)
        #expect(await source.screenFrame(at: 1.6).map(grayIndex) == 1)
    }

    @Test(.enabled(if: h264Available))
    func decodeFailureMidExportFailsAndLeavesNoFile() async throws {
        let b = try await makeBundle()
        defer { try? FileManager.default.removeItem(at: b.dir) }
        let out = b.dir.appendingPathComponent("fail.mp4")
        let events = Collector()
        let failing = try FailingFrameSource(failAfter: 3)
        await #expect(throws: NSSError.self) {
            try await MP4Exporter(frameSourceFactory: { _, _ in failing }).export(
                project: b.project, projectURL: b.dir, track: .empty, preset: ExportPreset(), to: out, progress: { events.add($0) })
        }
        #expect(!FileManager.default.fileExists(atPath: out.path))
        let leftovers = (try? FileManager.default.contentsOfDirectory(atPath: b.dir.path)) ?? []
        #expect(!leftovers.contains { $0.contains("partial") })
        if case .failed(let message)? = events.events.last { #expect(message.contains("simulated decode failure")) } else { Issue.record("expected .failed") }
        #expect(!events.events.contains { if case .finished = $0 { true } else { false } })
    }

    @Test func bitratesScaleWithResolution() {
        let hd = MP4Exporter.bitrate(size: PixelSize(width: 1920, height: 1080), fps: 60, codec: .h264)
        let uhd = MP4Exporter.bitrate(size: PixelSize(width: 3840, height: 2160), fps: 60, codec: .h264)
        let hevc = MP4Exporter.bitrate(size: PixelSize(width: 3840, height: 2160), fps: 60, codec: .hevc)
        #expect(hd > 10_000_000 && hd < 25_000_000)
        #expect(uhd > hd && uhd <= 80_000_000)
        #expect(hevc < uhd)
    }
}

final class TaskBox: @unchecked Sendable {
    private let lock = NSLock()
    private var _task: Task<Void, Error>?
    var task: Task<Void, Error>? {
        get { lock.lock(); defer { lock.unlock() }; return _task }
        set { lock.lock(); _task = newValue; lock.unlock() }
    }
}
