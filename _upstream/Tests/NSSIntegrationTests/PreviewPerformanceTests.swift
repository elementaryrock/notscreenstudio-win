import CoreVideo
import Foundation
import Testing
import NSSCore
import NSSExport
import NSSMotion
import NSSRender
@testable import NSSUI

/// Renderer wrapper that records how long each `render` call takes (on the preview's render queue).
final class TimingRenderer: FrameRenderer, @unchecked Sendable {
    private let inner: any FrameRenderer
    private let lock = NSLock()
    private var _times: [Double] = []
    init(_ inner: any FrameRenderer) { self.inner = inner }
    var times: [Double] { lock.lock(); defer { lock.unlock() }; return _times }
    func render(screen: CVPixelBuffer, camera: CVPixelBuffer?, state: FrameState, style: Style, into output: CVPixelBuffer) throws {
        let t0 = ContinuousClock.now
        try inner.render(screen: screen, camera: camera, state: state, style: style, into: output)
        let d = (ContinuousClock.now - t0).seconds
        lock.lock(); _times.append(d); lock.unlock()
    }
}

extension Duration {
    var seconds: Double { Double(components.seconds) + Double(components.attoseconds) / 1e18 }
}

func percentile(_ values: [Double], _ p: Double) -> Double {
    guard !values.isEmpty else { return .nan }
    let s = values.sorted()
    return s[min(s.count - 1, Int((Double(s.count - 1) * p).rounded()))]
}

/// Drives the real `PreviewEngine` (AVPlayer decode + real motion model + Metal at preview size) without a window,
/// ticking at 60 Hz like the display link. Budgets are asserted only with `NSS_BENCH=1`.
@MainActor
@Suite(.serialized) struct PreviewPerformanceTests {
    /// 8 s of worst-case content for the renderer: a click every 0.8 s across the screen (ripples overlap, auto-zoom
    /// keeps moving), a fast cursor (motion blur on), a camera bubble, and a new cursor shape every 0.6 s.
    nonisolated static var busySpec: VerifySpec {
        var s = VerifySpec()
        s.duration = 8
        s.trim = TrimRange()
        s.mic = false
        var clicks: [(t: Double, x: Double, y: Double)] = []
        for k in 0..<9 {
            let x: Double = k % 2 == 0 ? 0.15 : 0.85
            clicks.append((t: 0.5 + Double(k) * 0.8, x: x, y: 0.2 + 0.075 * Double(k)))
        }
        s.clicks = clicks
        s.style.cursor = CursorStyle(size: 1.5, smoothing: 0.5, motionBlur: 1.0, clickRipples: true, hideWhenIdle: false)
        return s
    }
    nonisolated static let busyShapes: [CursorShapeChange] = (0..<14).map {
        CursorShapeChange(t: Double($0) * 0.6, shape: [.arrow, .iBeam, .pointingHand, .resizeLeftRight, .crosshair, .closedHand, .openHand][$0 % 7])
    }

    @Test(.enabled(if: e2eAvailable))
    func previewFrameTimes() async throws {
        let bundle = try await makeVerifyBundle(Self.busySpec, shapes: Self.busyShapes)
        defer { try? FileManager.default.removeItem(at: bundle.store.libraryURL) }
        let timing = TimingRenderer(try MetalRenderer.makeDefault())
        let deps = EditorDependencies(
            store: bundle.store, motionFactory: SpringMotionModel.factory, renderer: timing,
            exporter: MP4Exporter(motionFactory: SpringMotionModel.factory, renderer: timing),
            motionFactoryForAspect: { SpringMotionModel.factory(sourceAspect: $0) })
        let model = EditorModel(project: bundle.project, reference: bundle.ref, track: bundle.track, dependencies: deps)
        let engine = PreviewEngine(model: model)
        model.changeHandler = { [engine] in engine.modelChanged($0) }
        var presentTimes: [ContinuousClock.Instant] = []
        engine.present = { _ in presentTimes.append(.now) }

        // Warm up: first frame, motion model built, pipelines compiled.
        for _ in 0..<300 where presentTimes.isEmpty {
            engine.tick()
            try await Task.sleep(for: .milliseconds(10))
        }
        // Whether a frame arrived within the warm-up window depends on machine load: asserted only with NSS_BENCH=1.
        if runBench { #expect(!presentTimes.isEmpty, "the preview never produced a frame") }
        try await Task.sleep(for: .milliseconds(300))
        engine.tick()

        let warmRenders = timing.times.count
        presentTimes.removeAll()
        engine.play()
        var tickTimes: [Double] = []
        let frames = 360
        let interval = Duration.nanoseconds(16_666_667)
        var deadline = ContinuousClock.now + interval
        for _ in 0..<frames {
            let t0 = ContinuousClock.now
            engine.tick()
            tickTimes.append((ContinuousClock.now - t0).seconds)
            try await Task.sleep(until: deadline)
            deadline += interval
        }
        let playhead = model.playhead
        engine.pause()
        engine.teardown()

        let renders = Array(timing.times.dropFirst(warmRenders))
        let gaps = zip(presentTimes, presentTimes.dropFirst()).map { ($1 - $0).seconds }
        let ms = { (v: Double) in String(format: "%.2f", v * 1000) }
        print("""
        preview bench (\(PreviewEngine.previewSize(for: .r16x9).width)x\(PreviewEngine.previewSize(for: .r16x9).height), \(frames) ticks at 60 Hz, playhead reached \(String(format: "%.1f", playhead)) s):
          Metal render per frame   p50 \(ms(percentile(renders, 0.5))) ms   p95 \(ms(percentile(renders, 0.95))) ms   max \(ms(renders.max() ?? .nan)) ms   (\(renders.count) frames)
          main-thread tick()       p50 \(ms(percentile(tickTimes, 0.5))) ms   p95 \(ms(percentile(tickTimes, 0.95))) ms   max \(ms(tickTimes.max() ?? .nan)) ms
          interval between frames  p50 \(ms(percentile(gaps, 0.5))) ms   p95 \(ms(percentile(gaps, 0.95))) ms   dropped by engine: \(engine.droppedFrames), gaps over 25 ms: \(gaps.filter { $0 > 0.025 }.count)/\(gaps.count)
        """)
        // Everything below depends on wall-clock pacing, so it is reported above and asserted only with NSS_BENCH=1.
        if runBench {
            #expect(playhead > 4, "playback did not advance (playhead \(playhead))")
            #expect(renders.count > frames / 2)
            #expect(percentile(renders, 0.95) < 0.0166, "render p95 over the 16.6 ms budget")
            #expect(percentile(tickTimes, 0.95) < 0.0166, "main-thread tick p95 over the 16.6 ms budget")
        }
    }
}

@Suite(.serialized) struct RendererHeadroomTests {
    /// The renderer alone at 1080p (a full-size preview) and 4K output with the busy content: GPU headroom.
    @Test(.enabled(if: e2eAvailable), arguments: [PixelSize(width: 1920, height: 1080), PixelSize(width: 3840, height: 2160)])
    func rendererFrameTimes(size: PixelSize) async throws {
        let spec = PreviewPerformanceTests.busySpec
        let bundle = try await makeVerifyBundle(spec, shapes: PreviewPerformanceTests.busyShapes)
        defer { try? FileManager.default.removeItem(at: bundle.store.libraryURL) }
        let renderer = try MetalRenderer.makeDefault()
        let source = try await AssetFrameSource.make(
            screenURL: bundle.dir.appendingPathComponent("screen.mov"), cameraURL: bundle.dir.appendingPathComponent("camera.mov"))
        let motion = SpringMotionModel.factory(sourceAspect: Synthetic.size.aspect)(bundle.track, bundle.project.timeline, bundle.project.style.cursor, spec.duration)
        let out = try PixelBuffers.make(width: size.width, height: size.height)
        let screen = try #require(await source.screenFrame(at: 0.5)), camera = await source.cameraFrame(at: 0.5)
        var times: [Double] = [], motionTimes: [Double] = []
        for i in 0..<480 {
            let t = Double(i) / 60
            let m0 = ContinuousClock.now
            let state = motion.frameState(at: t)
            motionTimes.append((ContinuousClock.now - m0).seconds)
            let r0 = ContinuousClock.now
            try renderer.render(screen: screen, camera: camera, state: state, style: bundle.project.style, into: out)
            if i >= 30 { times.append((ContinuousClock.now - r0).seconds) }   // the first frames compile and upload
        }
        let ms = { (v: Double) in String(format: "%.2f", v * 1000) }
        let over = times.filter { $0 > 1.0 / 60 }.count
        print("renderer \(size.width)x\(size.height): p50 \(ms(percentile(times, 0.5))) ms  p95 \(ms(percentile(times, 0.95))) ms  max \(ms(times.max() ?? 0)) ms, frames over 16.7 ms: \(over)/\(times.count); motion lookup p95 \(ms(percentile(motionTimes, 0.95))) ms")
        if runBench { #expect(percentile(times, 0.95) < 1.0 / 60) }
    }
}
