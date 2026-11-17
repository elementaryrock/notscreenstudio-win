import AVFoundation
import CoreGraphics
import Foundation
import Testing
import NSSCore
import NSSExport
import NSSMotion
import NSSRender

/// Claim 1: auto-zoom never shows a gap at the edges. A synthetic recording clicks at the four corners and the
/// middle of the four edges (the clicks sit at exactly 0 and 1). It is exported in all three aspect ratios, and every
/// decoded frame is checked against what the motion model says the camera shows.
@Suite(.serialized) struct EdgeGapTests {
    static let positions: [(Double, Double)] = [
        (0, 0), (1, 0), (1, 1), (0, 1), (0.5, 0), (1, 0.5), (0.5, 1), (0, 0.5),
    ]
    static let spacing = 1.4

    static var spec: VerifySpec {
        var s = VerifySpec()
        s.camera = false
        s.mic = false
        s.trim = TrimRange()
        s.duration = 0.6 + Double(positions.count) * spacing + 0.6
        s.clicks = positions.enumerated().map { (0.6 + Double($0.offset) * spacing, $0.element.0, $0.element.1) }
        return s
    }

    @Test(.enabled(if: e2eAvailable), arguments: AspectRatio.allCases)
    func noGapsOrSmearAtAnyEdge(aspect: AspectRatio) async throws {
        var spec = Self.spec
        spec.style.aspectRatio = aspect
        let bundle = try await makeVerifyBundle(spec)
        defer { try? FileManager.default.removeItem(at: bundle.store.libraryURL) }
        let project = bundle.project
        let out = bundle.dir.appendingPathComponent("edges.mp4")
        let preset = ExportPreset(resolution: .p1080, fps: 60, codec: .h264)
        try await EndToEndExportTests.exporter(renderer: try MetalRenderer.makeDefault()).export(
            project: project, projectURL: bundle.dir, track: bundle.track, preset: preset, to: out, progress: { _ in })

        let size = preset.outputSize(for: aspect)
        let motion = SpringMotionModel.factory(sourceAspect: Synthetic.size.aspect)(
            bundle.track, project.timeline, project.style.cursor, spec.duration)

        // The content rectangle, computed here from the plain rule (fit the source inside the output minus padding).
        let pad = project.style.padding * Double(size.height)
        let availW = Double(size.width) - 2 * pad, availH = Double(size.height) - 2 * pad
        let fit = min(availW / Double(Synthetic.size.width), availH / Double(Synthetic.size.height))
        let cw = Double(Synthetic.size.width) * fit, ch = Double(Synthetic.size.height) * fit
        let cx0 = (Double(size.width) - cw) / 2, cy0 = (Double(size.height) - ch) / 2
        let radius = project.style.cornerRadius * Double(size.height)

        var frames = 0, samples = 0, worst = 0.0, skipped = 0
        var touchedEdges = Set<String>()
        var failures: [String] = []
        var peak = 1.0
        try await forEachFrame(out) { i, _, pb in
            let t = Double(i) / 60
            let state = motion.frameState(at: t)
            let vp = state.camera.sourceViewport
            // The motion model guarantee, exactly.
            if !vp.isInsideSource { failures.append("frame \(i): viewport \(vp) leaves the source") }
            peak = max(peak, state.camera.scale)
            if vp.minX == 0 { touchedEdges.insert("left") }
            if vp.maxX == 1 { touchedEdges.insert("right") }
            if vp.minY == 0 { touchedEdges.insert("top") }
            if vp.maxY == 1 { touchedEdges.insert("bottom") }

            let frame = Frame(pb)
            let layout = RenderLayout(outputSize: size, sourceSize: Synthetic.size, style: project.style, camera: state.camera)
            #expect(abs(layout.contentRect.minX - cx0) < 0.5 && abs(layout.contentRect.width - cw) < 0.5, "content rect differs from the plain fit rule")
            // Where the cursor and ripples are drawn is not "screen content": skip samples near them.
            var avoid: [(CGPoint, Double)] = []
            if state.cursor.visible { avoid.append((layout.outputPoint(forSource: state.cursor.position), 0.1 * Double(size.height))) }
            for r in state.cursor.ripples {
                avoid.append((layout.outputPoint(forSource: r.position), layout.rippleRadius(progress: r.progress) * 1.5 + 30))
            }
            func check(_ ox: Double, _ oy: Double, _ what: String) {
                for (c, rad) in avoid where hypot(ox - Double(c.x), oy - Double(c.y)) < rad { skipped += 1; return }
                let u = (ox + 0.5 - cx0) / cw, v = (oy + 0.5 - cy0) / ch
                let wantR = (vp.minX + u * vp.width) * 255, wantG = (vp.minY + v * vp.height) * 255
                let got = frame.mean(Int(ox), Int(oy))
                let err = max(abs(got.r - wantR), abs(got.g - wantG))
                samples += 1; worst = max(worst, err)
                // Background is (26, 128, 230): blue 230 never appears in the ramps (blue 90, or 230 only in the thin band).
                if err > 14 { failures.append("frame \(i) \(what) at (\(Int(ox)),\(Int(oy))): got r\(Int(got.r)) g\(Int(got.g)), the camera shows r\(Int(wantR)) g\(Int(wantG))") }
            }
            let inset = max(radius, 0) + 4
            // Along each edge, 4 px inside the screen area (and clear of the rounded corners)...
            for f in [0.25, 0.5, 0.75] {
                check(cx0 + 4, cy0 + f * ch, "left edge")
                check(cx0 + cw - 5, cy0 + f * ch, "right edge")
                check(cx0 + f * cw, cy0 + 4, "top edge")
                check(cx0 + f * cw, cy0 + ch - 5, "bottom edge")
            }
            // ...just inside the rounded corners...
            for (fx, fy) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
                check(cx0 + (fx == 0 ? inset : cw - inset - 1), cy0 + (fy == 0 ? inset : ch - inset - 1), "corner")
            }
            // ...and a coarse grid over the whole area.
            for gx in 1...5 { for gy in 1...5 { check(cx0 + Double(gx) / 6 * cw, cy0 + Double(gy) / 6 * ch, "interior") } }
            frames += 1
        }
        print("edge gaps \(aspect.rawValue): \(frames) frames, \(samples) samples checked (\(skipped) near cursor/ripples skipped), worst channel error \(String(format: "%.1f", worst))/255, peak zoom \(String(format: "%.2f", peak))x, viewport touched \(touchedEdges.sorted())")
        #expect(failures.isEmpty, "\(failures.count) failures, first: \(failures.prefix(3))")
        #expect(frames == Int((spec.duration * 60).rounded()))
        #expect(peak > 1.3, "auto-zoom never engaged")
        #expect(touchedEdges == ["left", "right", "top", "bottom"], "the clicks never pushed the viewport to every edge: \(touchedEdges)")
        #expect(samples > frames * 20)
    }
}
