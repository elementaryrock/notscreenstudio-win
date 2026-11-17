import AVFoundation
import CoreGraphics
import Foundation
import Testing
import NSSCore
import NSSExport
import NSSMotion
import NSSProject
import NSSRender

/// Synthetic bundle -> `FileProjectStore` -> real motion model -> `MetalRenderer` -> `MP4Exporter`,
/// then the MP4 is decoded and spot-checked. Skips without a GPU or an H.264 encoder.
@Suite(.serialized) struct EndToEndExportTests {
    static func exporter(renderer: any FrameRenderer) -> MP4Exporter {
        // The same wiring as `AppModel`.
        MP4Exporter(
            motionFactory: SpringMotionModel.factory, renderer: renderer,
            motionFactoryForAspect: { SpringMotionModel.factory(sourceAspect: $0) })
    }

    @Test(.enabled(if: e2eAvailable), arguments: AspectRatio.allCases)
    func exports1080p(aspect: AspectRatio) async throws {
        try await runExport(aspect: aspect, resolution: .p1080)
    }

    @Test(.enabled(if: e2eAvailable && run4K), arguments: AspectRatio.allCases)
    func exports4K(aspect: AspectRatio) async throws {
        try await runExport(aspect: aspect, resolution: .p2160)
    }

    func runExport(aspect: AspectRatio, resolution: ExportResolution) async throws {
        var bundle = try await makeE2EBundle()
        defer { try? FileManager.default.removeItem(at: bundle.store.libraryURL) }
        bundle.project.style.aspectRatio = aspect
        let project = bundle.project
        let out = bundle.dir.appendingPathComponent("out-\(aspect.rawValue)-\(resolution.rawValue).mp4")
        let preset = ExportPreset(resolution: resolution, fps: 60, codec: .h264)
        let renderer = try MetalRenderer.makeDefault()

        let started = Date()
        try await Self.exporter(renderer: renderer).export(
            project: project, projectURL: bundle.dir, track: bundle.track, preset: preset, to: out, progress: { _ in })
        let seconds = Date().timeIntervalSince(started)

        // --- container facts
        let size = preset.outputSize(for: aspect)
        let p = try await probe(out)
        let trimLength = Synthetic.trim.resolvedEnd(duration: Synthetic.duration) - Synthetic.trim.start
        #expect(p.width == size.width && p.height == size.height)
        #expect(abs(Double(p.nominalFPS) - 60) < 0.5)
        #expect(p.frameTimes.count == Int((trimLength * 60).rounded()))
        #expect(abs(p.duration - trimLength) < 0.03)
        let gaps = zip(p.frameTimes, p.frameTimes.dropFirst()).map { $1 - $0 }
        #expect(gaps.allSatisfy { abs($0 - 1.0 / 60) < 1e-4 })
        #expect(p.audioTracks == 1)
        #expect(abs(p.audioDuration - trimLength) < 0.15)

        // --- background at the four corners
        let first = try await decodeFrame(out, at: 0)
        let bg = Synthetic.background
        for (x, y) in [(2, 2), (size.width - 3, 2), (2, size.height - 3), (size.width - 3, size.height - 3)] {
            let px = meanPixel(first, x, y, radius: 1)
            #expect(near(px, bg.r, bg.g, bg.b, tol: 12), "corner (\(x),\(y)) = \(px)")
        }

        // --- camera bubble: its centre shows the camera colour (at frame 0's zoom, where the bubble may have shrunk)
        let motion0 = SpringMotionModel.factory(sourceAspect: Synthetic.size.aspect)(
            bundle.track, project.timeline, project.style.cursor, Synthetic.duration)
        let layout0 = RenderLayout(outputSize: size, sourceSize: Synthetic.size, style: project.style,
                                   camera: motion0.frameState(at: Synthetic.trim.start).camera)
        let bubble = layout0.cameraBubbleRect(project.style.camera, cameraSize: Synthetic.cameraSize)
        let c = Synthetic.cameraColor
        let bubblePx = meanPixel(first, Int(bubble.midX), Int(bubble.midY))
        #expect(near(bubblePx, c.r, c.g, c.b, tol: 16), "bubble centre = \(bubblePx)")
        // When frame 0 is zoomed, the bubble has shrunk toward its anchor: a point inside the full-size
        // bubble but well outside the shrunk one must not show the camera.
        let fullBubble = layout0.cameraBubbleFullRect(project.style.camera)
        if bubble.width < fullBubble.width * 0.9 {
            let ax = RenderLayout.bubbleAnchor(project.style.camera.position.x)
            let ay = RenderLayout.bubbleAnchor(project.style.camera.position.y)
            // The full rect's point farthest from the anchor, 10% in from its edges.
            let ox = fullBubble.minX + (ax < 0.5 ? 0.9 : 0.1) * fullBubble.width
            let oy = fullBubble.minY + (ay < 0.5 ? 0.9 : 0.1) * fullBubble.height
            #expect(!bubble.contains(CGPoint(x: ox, y: oy)))
            let outside = meanPixel(first, Int(ox), Int(oy))
            #expect(!near(outside, c.r, c.g, c.b, tol: 40), "full-size bubble area at frame 0 = \(outside), bubble did not shrink")
        }

        // --- zoomed in on content shortly after a click
        let motion = SpringMotionModel.factory(sourceAspect: Synthetic.size.aspect)(
            bundle.track, project.timeline, project.style.cursor, Synthetic.duration)
        var tStar = Synthetic.trim.start
        var best = 0.0
        var t = Synthetic.trim.start
        while t < Synthetic.trim.resolvedEnd(duration: Synthetic.duration) - 0.05 {
            let s = motion.frameState(at: t).camera.scale
            if s > best { best = s; tStar = t }
            t += 1.0 / 60
        }
        #expect(best > 1.3, "auto-zoom peaked at \(best)x")
        // Snap to an exact output frame.
        let frameIndex = Int(((tStar - Synthetic.trim.start) * 60).rounded())
        let tSource = Synthetic.trim.start + Double(frameIndex) / 60
        let state = motion.frameState(at: tSource)
        let zoomed = RenderLayout(outputSize: size, sourceSize: Synthetic.size, style: project.style, camera: state.camera)
        let flat = RenderLayout(outputSize: size, sourceSize: Synthetic.size, style: project.style, camera: CameraState())
        let content = zoomed.contentRect
        let cursorPx = zoomed.outputPoint(forSource: state.cursor.position)
        let bubbleNow = zoomed.cameraBubbleRect(project.style.camera, cameraSize: Synthetic.cameraSize).insetBy(dx: -30, dy: -30)
        // Pick an output point with a clear view: not near the cursor or bubble, and where zoom changes the ramp most.
        var choice: (pt: CGPoint, red: Double, flatRed: Double)?
        for (fx, fy) in [(0.2, 0.25), (0.8, 0.25), (0.2, 0.75), (0.8, 0.75), (0.35, 0.5), (0.65, 0.5)] {
            let pt = CGPoint(x: content.minX + fx * content.width, y: content.minY + fy * content.height)
            guard hypot(pt.x - cursorPx.x, pt.y - cursorPx.y) > 60, !bubbleNow.contains(pt) else { continue }
            let sx = zoomed.sourcePoint(forOutput: pt).x
            guard sx > 0.03, sx < 0.97 else { continue }
            let red = sx * 255, flatRed = flat.sourcePoint(forOutput: pt).x * 255
            if choice == nil || abs(red - flatRed) > abs(choice!.red - choice!.flatRed) { choice = (pt, red, flatRed) }
        }
        let pick = try #require(choice, "no clear output point to sample")
        let frame = try await decodeFrame(out, at: Double(frameIndex) / 60 + 1e-4)
        let got = meanPixel(frame, Int(pick.pt.x), Int(pick.pt.y), radius: 2)
        #expect(abs(Double(got.r) - pick.red) < 16, "zoomed red \(got.r), expected \(pick.red), un-zoomed would be \(pick.flatRed)")
        #expect(abs(pick.red - pick.flatRed) > 16, "sample cannot tell zoom from no zoom")
        print("e2e \(aspect.rawValue) \(resolution.rawValue): \(size.width)x\(size.height), \(p.frameTimes.count) frames in \(String(format: "%.1f", seconds)) s, peak zoom \(String(format: "%.2f", best))x")
    }

    /// The exporter must frame auto-zoom for the real source aspect, not the 16:9 default.
    @Test(.enabled(if: e2eAvailable))
    func exporterUsesTheRealSourceAspect() async throws {
        final class Seen: @unchecked Sendable {
            let lock = NSLock(); private var v: [Double] = []
            func add(_ a: Double) { lock.lock(); v.append(a); lock.unlock() }
            var values: [Double] { lock.lock(); defer { lock.unlock() }; return v }
        }
        let bundle = try await makeE2EBundle(mic: false, camera: false, duration: 1, trim: TrimRange(start: 0, end: 0.1))
        defer { try? FileManager.default.removeItem(at: bundle.store.libraryURL) }
        let seen = Seen()
        let exporter = MP4Exporter(
            motionFactory: SpringMotionModel.factory, renderer: try MetalRenderer.makeDefault(),
            motionFactoryForAspect: { aspect in seen.add(aspect); return SpringMotionModel.factory(sourceAspect: aspect) })
        try await exporter.export(
            project: bundle.project, projectURL: bundle.dir, track: bundle.track, preset: ExportPreset(),
            to: bundle.dir.appendingPathComponent("o.mp4"), progress: { _ in })
        #expect(seen.values == [960.0 / 540.0])
    }
}
