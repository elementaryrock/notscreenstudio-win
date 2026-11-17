import AVFoundation
import CoreVideo
import Foundation
import Testing
import NSSCore
import NSSExport
import NSSMotion
import NSSRender
@testable import NSSUI

/// Claim 3: the export equals the preview, with audio.
///
/// Three comparisons per aspect ratio, on one synthetic recording (trim 0.4...2.0 s, two clicks, cursor shape changes,
/// camera bubble, a mic track, and three flash + tone-burst "claps" at 0.2 s, 1.0 s and 1.9 s):
///
/// 1. Preview == exporter inputs, exactly. The real `PreviewEngine` (AVPlayer frames, real motion model, real
///    renderer, at preview size) is compared byte for byte with a direct render that uses what the exporter feeds
///    the renderer (`AssetFrameSource` frames at t, the same motion model). Same renderer and size, so any
///    difference is a different frame, a different state or a different style.
/// 2. Exported MP4 == direct render at export size, for EVERY frame, after decoding. The only difference allowed is
///    H.264 loss, measured as PSNR. Neighbouring frames (t +- 1, 2 frames) are also compared: the exported frame
///    must be closest to its own time, which proves trim, zoom, cursor and ripple timing match frame for frame.
/// 3. Audio: present, trimmed like the video, and each tone burst starts within one video frame of its flash.
@Suite(.serialized) struct ParityTests {
    /// PSNR floor for "encoder differences only". The encoder is H.264 at 0.12 bits per pixel per frame on
    /// content with sharp text-like edges, cursor and ripple. Measured values are printed; the floor sits well
    /// under them but far above the PSNR of a wrong frame (see `neighbourMargin`).
    static let minPSNR = 32.0

    static var spec: VerifySpec {
        var s = VerifySpec()
        s.flashes = [0.2, 1.0, 1.9]
        return s
    }
    static let shapes = [
        CursorShapeChange(t: 0, shape: .arrow), CursorShapeChange(t: 0.9, shape: .iBeam), CursorShapeChange(t: 1.5, shape: .pointingHand),
    ]

    @Test(.enabled(if: e2eAvailable), arguments: AspectRatio.allCases)
    func exportMatchesPreviewWithAudio(aspect: AspectRatio) async throws {
        var spec = Self.spec
        spec.style.aspectRatio = aspect
        let bundle = try await makeVerifyBundle(spec, shapes: Self.shapes)
        defer { try? FileManager.default.removeItem(at: bundle.store.libraryURL) }
        let project = bundle.project
        let trim = spec.trim
        let trimLength = trim.resolvedEnd(duration: spec.duration) - trim.start
        let fps = 60.0
        let out = bundle.dir.appendingPathComponent("parity.mp4")
        let preset = ExportPreset(resolution: .p1080, fps: 60, codec: .h264)
        let renderer = try MetalRenderer.makeDefault()
        try await EndToEndExportTests.exporter(renderer: renderer).export(
            project: project, projectURL: bundle.dir, track: bundle.track, preset: preset, to: out, progress: { _ in })
        let size = preset.outputSize(for: aspect)

        // ---- container: frame count and timestamps (the trim)
        let p = try await probe(out)
        let totalFrames = Int((trimLength * fps).rounded())
        #expect(p.frameTimes.count == totalFrames)
        #expect(zip(p.frameTimes, p.frameTimes.indices).allSatisfy { abs($0 - Double($1) / fps) < 1e-6 }, "frame timestamps are not exactly i/60")
        #expect(p.audioTracks == 1)
        #expect(abs(p.audioDuration - trimLength) < 0.1, "audio is \(p.audioDuration) s, trim is \(trimLength) s")

        // ---- every exported frame against a direct render at export size
        let screenURL = bundle.dir.appendingPathComponent("screen.mov"), cameraURL = bundle.dir.appendingPathComponent("camera.mov")
        let source = try await AssetFrameSource.make(screenURL: screenURL, cameraURL: cameraURL)
        let fileDuration = try await AVURLAsset(url: screenURL).load(.duration).seconds   // what the exporter hands the motion model
        let motion = SpringMotionModel.factory(sourceAspect: Synthetic.size.aspect)(bundle.track, project.timeline, project.style.cursor, fileDuration)
        var coded: [Frame] = [], rendered: [Frame] = []   // 8x subsampled, for the neighbour comparison
        var psnrs: [Double] = [], maxErrors: [Int] = []
        var whiteVideo: [Bool] = [], whiteRender: [Bool] = []
        // The screen area inside the padding (flash probes must sit on it, whatever the aspect ratio).
        let pad = project.style.padding * Double(size.height)
        let fit = min((Double(size.width) - 2 * pad) / Double(Synthetic.size.width), (Double(size.height) - 2 * pad) / Double(Synthetic.size.height))
        let cw = Double(Synthetic.size.width) * fit, ch = Double(Synthetic.size.height) * fit
        let cx0 = (Double(size.width) - cw) / 2, cy0 = (Double(size.height) - ch) / 2
        try await forEachFrame(out) { i, _, pb in
            let t = trim.start + Double(i) / fps
            let a = Frame(pb)
            let b = try await directRender(source: source, renderer: renderer, motion: motion, style: project.style, t: t, size: size)
            let m = psnr(a, b, step: 4)
            psnrs.append(m.psnr); maxErrors.append(m.maxError)
            coded.append(a.subsampled(8)); rendered.append(b.subsampled(8))
            // "Flash on": the content is white at several points clear of the cursor and bubble.
            let probes = [(0.5, 0.12), (0.15, 0.85), (0.85, 0.12)].map { a.px(Int(cx0 + $0.0 * cw), Int(cy0 + $0.1 * ch)) }
            let rp = [(0.5, 0.12), (0.15, 0.85), (0.85, 0.12)].map { b.px(Int(cx0 + $0.0 * cw), Int(cy0 + $0.1 * ch)) }
            whiteRender.append(rp.filter { $0.r > 225 && $0.g > 225 && $0.b > 225 }.count >= 2)
            whiteVideo.append(probes.filter { $0.r > 225 && $0.g > 225 && $0.b > 225 }.count >= 2)
        }
        #expect(psnrs.count == totalFrames)
        #expect(whiteVideo == whiteRender, "flash frames differ: video \(whiteVideo.indices.filter { whiteVideo[$0] }) render \(whiteRender.indices.filter { whiteRender[$0] })")
        let minP = psnrs.min() ?? 0, meanP = psnrs.reduce(0, +) / Double(max(psnrs.count, 1))
        let sortedErr = maxErrors.sorted()
        #expect(minP > Self.minPSNR, "worst frame PSNR \(minP) dB")

        // Frame for frame: each exported frame must match the render of its own time better than the renders of
        // the frames around it, wherever those renders differ visibly from each other.
        var checked = 0, wrong: [Int] = []
        var margins: [Double] = []
        for i in 0..<coded.count {
            let own = psnr(coded[i], rendered[i], step: 1).psnr
            for j in max(0, i - 2)...min(coded.count - 1, i + 2) where j != i {
                let apart = psnr(rendered[i], rendered[j], step: 1).psnr
                guard apart < 45 else { continue }   // too similar to tell apart (an idle moment)
                checked += 1
                let other = psnr(coded[i], rendered[j], step: 1).psnr
                margins.append(own - other)
                if other >= own { wrong.append(i) }
            }
        }
        #expect(wrong.isEmpty, "frames closer to a neighbouring time than to their own: \(Set(wrong).sorted())")
        #expect(checked > totalFrames, "too few distinguishable neighbours (\(checked))")

        // ---- audio: trimmed like the video, and each burst lines up with its flash
        let audio = try await decodeAudio(out)
        let onsets = burstOnsets(audio)
        let expectedBursts = spec.flashes.map { $0 - trim.start }.filter { $0 >= 0 && $0 < trimLength }
        var flashStarts: [Double] = []
        for i in whiteVideo.indices where whiteVideo[i] && (i == 0 || !whiteVideo[i - 1]) { flashStarts.append(Double(i) / fps) }
        #expect(onsets.count == expectedBursts.count, "bursts heard at \(onsets), expected \(expectedBursts) (the 0.2 s burst is trimmed away)")
        #expect(flashStarts.count == expectedBursts.count, "flashes seen at \(flashStarts), expected \(expectedBursts)")
        // The flash appears in the first source frame stamped at or after the flash time. The recording drops frames
        // (like ScreenCaptureKit), so that frame can be later than the flash time: use its real timestamp.
        // The movie file stores timestamps on its own (coarser) time scale, so take the stamps from the file, matching frames by index.
        let sourceTimes = variableTimes(duration: spec.duration)
        let fileTimes = try await probe(screenURL).frameTimes
        #expect(fileTimes.count == sourceTimes.count)
        let flashSourceTimes: [Double] = spec.flashes.compactMap { f in
            sourceTimes.firstIndex { $0 >= f - 1e-9 }.map { fileTimes[$0] - trim.start }
        }.filter { $0 >= 0 && $0 < trimLength }
        var syncOffsets: [Double] = []
        for (a, v) in zip(onsets, flashStarts) { syncOffsets.append(v - a) }
        for (v, f) in zip(flashStarts, flashSourceTimes) {
            #expect(v >= f - 1e-6 && v < f + 1 / fps + 1e-6, "flash shown at \(v), its source frame is at \(f); ")
        }
        // Audio is exact: each burst starts within one output frame of the moment it was recorded.
        for (a, e) in zip(onsets, expectedBursts) { #expect(abs(a - e) < 1 / fps, "burst at \(a), expected \(e)") }
        // Sync: audio and the flash of its own clap differ only by the source's frame timing (<= one dropped source frame + one output frame).
        #expect(syncOffsets.allSatisfy { $0 > -0.004 && $0 < 2 / fps + 0.004 }, "audio/video offsets \(syncOffsets)")

        let ms = { (v: Double) in String(format: "%+.1f", v * 1000) }
        print("""
        parity \(aspect.rawValue) \(size.width)x\(size.height): \(totalFrames) frames
          export vs direct render: PSNR min \(String(format: "%.1f", minP)) / mean \(String(format: "%.1f", meanP)) dB, max channel error p50 \(sortedErr[sortedErr.count / 2]) / max \(sortedErr.last ?? 0) of 255
          frame-for-frame: \(checked) neighbour comparisons, own-time PSNR beats neighbours by min \(String(format: "%.1f", margins.min() ?? 0)) dB, mismatches \(Set(wrong).count)
          audio: \(onsets.count) bursts at \(onsets.map { String(format: "%.3f", $0) }) s (expected \(expectedBursts)), flashes at \(flashStarts.map { String(format: "%.3f", $0) }) s, video-audio offset \(syncOffsets.map(ms)) ms, audio length \(String(format: "%.3f", p.audioDuration)) s vs video \(String(format: "%.3f", p.duration)) s
        """)
    }

    /// The real preview engine against what the exporter would render, byte for byte.
    @MainActor
    @Test(.enabled(if: e2eAvailable), arguments: AspectRatio.allCases)
    func previewEngineFramesAreByteIdenticalToExporterInputs(aspect: AspectRatio) async throws {
        var spec = Self.spec
        spec.style.aspectRatio = aspect
        let bundle = try await makeVerifyBundle(spec, shapes: Self.shapes)
        defer { try? FileManager.default.removeItem(at: bundle.store.libraryURL) }
        let renderer = try MetalRenderer.makeDefault()
        let deps = EditorDependencies(
            store: bundle.store, motionFactory: SpringMotionModel.factory, renderer: renderer,
            exporter: MP4Exporter(motionFactory: SpringMotionModel.factory, renderer: renderer),
            motionFactoryForAspect: { SpringMotionModel.factory(sourceAspect: $0) })
        let model = EditorModel(project: bundle.project, reference: bundle.ref, track: bundle.track, dependencies: deps)
        let engine = PreviewEngine(model: model)
        model.changeHandler = { [engine] in engine.modelChanged($0) }
        defer { engine.teardown() }
        var presented: Frame?
        var presentCount = 0
        engine.present = { pb in presented = Frame(pb); presentCount += 1 }

        let size = PreviewEngine.previewSize(for: aspect)
        let source = try await AssetFrameSource.make(
            screenURL: bundle.dir.appendingPathComponent("screen.mov"), cameraURL: bundle.dir.appendingPathComponent("camera.mov"))
        let motion = SpringMotionModel.factory(sourceAspect: Synthetic.size.aspect)(
            bundle.track, bundle.project.timeline, bundle.project.style.cursor, model.duration)
        let reference = try MetalRenderer.makeDefault()

        // Frame indices (0.4 s + k/60): the start, a click's ripple, both shape changes, a flash onset, and the end.
        let ks = [0, 7, 24, 30, 36, 37, 45, 62, 80, 95]
        var worst = 0, tries: [Int] = []
        for k in ks {
            let t = bundle.project.timeline.trim.start + Double(k) / 60
            let want = try await directRender(source: source, renderer: reference, motion: motion, style: bundle.project.style, t: t, size: size)
            engine.seek(to: t)
            // Wait for the event: the engine presents the frame for this time once the player has decoded it and the
            // motion model is built. Stale presents on the way (the previous frame) are expected and not compared.
            var attempts = 0, matched = false, lastErr = Int.max
            while attempts < 3000 && !matched {
                let before = presentCount
                engine.tick()
                try await Task.sleep(for: .milliseconds(10))
                attempts += 1
                guard presentCount > before, engine.motionIsCurrent, let got = presented else { continue }
                lastErr = psnr(got, want, step: 1).maxError
                matched = lastErr == 0
            }
            tries.append(attempts)
            worst = max(worst, matched ? 0 : lastErr)
            #expect(matched, "preview at t=\(t) (k=\(k)) never became identical to the exporter's inputs; last max error \(lastErr)")
        }
        print("preview vs exporter inputs \(aspect.rawValue) \(size.width)x\(size.height): \(ks.count) frames byte-identical=\(worst == 0), ticks needed \(tries)")
    }
}
