import Foundation
import Testing
import NSSCore
@testable import NSSMotion

/// Deterministic generator so a failing case can be replayed from its seed.
struct SplitMix64: RandomNumberGenerator {
    var state: UInt64
    init(seed: UInt64) { state = seed }
    mutating func next() -> UInt64 {
        state &+= 0x9E37_79B9_7F4A_7C15
        var z = state
        z = (z ^ (z >> 30)) &* 0xBF58_476D_1CE4_E5B9
        z = (z ^ (z >> 27)) &* 0x94D0_49BB_1331_11EB
        return z ^ (z >> 31)
    }
}

/// Mirror of `RenderLayout` (NSSRender) for the content rect and `sourcePoint(forOutput:)`, so this test
/// target, which depends only on NSSMotion, can check the region the renderer actually samples for each
/// output aspect. Returns the source coordinates of the four corners of the content rect.
func renderedSourceCorners(camera: CameraState, output: (w: Double, h: Double), sourceAspect: Double, padding: Double) -> [NormalizedPoint] {
    let pad = max(0, padding) * output.h
    let availW = max(1, output.w - 2 * pad), availH = max(1, output.h - 2 * pad)
    let (cw, ch) = sourceAspect > availW / availH ? (availW, availW / sourceAspect) : (availH * sourceAspect, availH)
    let rect = (x: (output.w - cw) / 2, y: (output.h - ch) / 2, w: cw, h: ch)
    return [(0.0, 0.0), (1, 0), (0, 1), (1, 1)].map { fx, fy in
        let px = rect.x + fx * rect.w, py = rect.y + fy * rect.h
        let ux = (px - rect.x) / rect.w, uy = (py - rect.y) / rect.h
        return NormalizedPoint(x: camera.center.x + (ux - 0.5) / camera.scale, y: camera.center.y + (uy - 0.5) / camera.scale)
    }
}

let outputSizes: [(name: String, w: Double, h: Double)] = [("16:9", 1920, 1080), ("1:1", 1080, 1080), ("9:16", 1080, 1920)]
let sourceAspects: [Double] = [16.0 / 9.0, 16.0 / 10.0, 4.0 / 3.0, 1, 9.0 / 16.0, 21.0 / 9.0, 32.0 / 9.0]

/// Asserts the viewport contract for one frame. Returns false on the first violation (to stop a trial).
@discardableResult
func checkViewport(_ c: CameraState, sourceAspect: Double, padding: Double = 0.06, context: @autoclosure () -> String) -> Bool {
    let v = c.sourceViewport
    let ok = c.scale.isFinite && c.scale >= 1 && c.center.x.isFinite && c.center.y.isFinite && v.isInsideSource
    #expect(ok, "viewport \(v) scale \(c.scale) left the source: \(context())")
    guard ok else { return false }
    for out in outputSizes {
        for p in renderedSourceCorners(camera: c, output: (out.w, out.h), sourceAspect: sourceAspect, padding: padding) {
            // The mirror goes through pixel space, so allow rounding error only.
            let inside = p.x >= -1e-9 && p.x <= 1 + 1e-9 && p.y >= -1e-9 && p.y <= 1 + 1e-9
            #expect(inside, "\(out.name) output samples source \(p) outside 0...1: \(context())")
            if !inside { return false }
        }
    }
    return true
}

/// Random but adversarial tuning: underdamped springs (overshoot), extreme scales and dead zones.
func randomTuning(_ r: inout SplitMix64) -> MotionTuning {
    var t = MotionTuning()
    t.simulationRate = [60.0, 120, 240, 333][Int.random(in: 0..<4, using: &r)]
    t.zoomSpring = Spring(response: .random(in: 0.05...1.5, using: &r), dampingRatio: .random(in: 0.1...2, using: &r))
    t.panSpring = Spring(response: .random(in: 0.05...1.5, using: &r), dampingRatio: .random(in: 0.1...2, using: &r))
    t.maxZoomScale = .random(in: 1...12, using: &r)
    t.autoZoomScale = .random(in: 0.5...14, using: &r)
    t.autoZoomLeadIn = .random(in: 0...1.5, using: &r)
    t.autoZoomHold = .random(in: 0.1...3, using: &r)
    t.clusterMaxDistance = .random(in: 0.01...1, using: &r)
    t.clusterPanGap = .random(in: 0...3, using: &r)
    t.minZoomOutGap = .random(in: 0...2, using: &r)
    t.deadZone = .random(in: 0...1.5, using: &r)
    t.preAimMaxScale = .random(in: 1...3, using: &r)
    return t
}

/// A position that is often exactly on a corner or edge, sometimes slightly outside (window capture).
func randomPoint(_ r: inout SplitMix64) -> NormalizedPoint {
    func coord(_ r: inout SplitMix64) -> Double {
        switch Int.random(in: 0..<6, using: &r) {
        case 0: return 0
        case 1: return 1
        case 2: return .random(in: -0.2...0, using: &r)
        case 3: return .random(in: 1...1.2, using: &r)
        default: return .random(in: 0...1, using: &r)
        }
    }
    return NormalizedPoint(x: coord(&r), y: coord(&r))
}

@Suite struct ViewportPropertyTests {
    @Test func clampIsExactForAnyInput() {
        var r = SplitMix64(seed: 7)
        let specials: [Double] = [.nan, .infinity, -.infinity, 0, 1, -1, 2, 0.5, 1e-300, 1e300, .ulpOfOne]
        for _ in 0..<20_000 {
            let scale: Double = Bool.random(using: &r) ? specials.randomElement(using: &r)! : .random(in: 0...50, using: &r)
            let cx: Double = Bool.random(using: &r) ? specials.randomElement(using: &r)! : .random(in: -2...3, using: &r)
            let cy: Double = .random(in: -2...3, using: &r)
            let c = CameraState.clampedToSource(scale: scale, center: NormalizedPoint(x: cx, y: cy))
            #expect(c.scale.isFinite && c.scale >= 1 && c.center.x.isFinite && c.center.y.isFinite)
            #expect(c.sourceViewport.isInsideSource, "scale \(scale) center \(cx),\(cy) -> \(c.sourceViewport)")
        }
        // Inside values are untouched.
        let inside = CameraState.clampedToSource(scale: 2, center: NormalizedPoint(x: 0.3, y: 0.7))
        #expect(inside == CameraState(scale: 2, center: NormalizedPoint(x: 0.3, y: 0.7)))
    }

    @Test(arguments: 0..<24)
    func randomClicksTuningsAndAspectsNeverShowEdges(trial: Int) {
        let seed = UInt64(1000 + trial)
        var r = SplitMix64(seed: seed)
        let tuning = randomTuning(&r)
        let sourceAspect = sourceAspects[trial % sourceAspects.count]
        let duration = Double.random(in: 4...16, using: &r)

        // Cursor: random walk with jumps to corners and edges.
        var p = randomPoint(&r)
        let samples = makeSamples(duration: duration) { _ in
            if Int.random(in: 0..<60, using: &r) == 0 { p = randomPoint(&r) }
            p.x += .random(in: -0.01...0.01, using: &r)
            p.y += .random(in: -0.01...0.01, using: &r)
            return p
        }
        // Clicks: bursts (pans inside one segment), isolated clicks, drags, many at the corners.
        var clicks: [ClickEvent] = []
        var t = Double.random(in: 0...1, using: &r)
        while t < duration {
            let at = randomPoint(&r)
            clicks.append(ClickEvent(t: t, x: at.x, y: at.y, kind: .down))
            let hold = Bool.random(using: &r) ? 0.05 : .random(in: 0...1.5, using: &r)
            clicks.append(ClickEvent(t: t + hold, x: at.x, y: at.y, kind: .up))
            t += Bool.random(using: &r) ? .random(in: 0.1...0.8, using: &r) : .random(in: 0.8...4, using: &r)
        }
        // Manual blocks with focus anywhere, including outside the source, and extreme scales.
        var blocks: [ZoomBlock] = []
        for _ in 0..<Int.random(in: 0...3, using: &r) {
            let s = Double.random(in: 0...duration, using: &r)
            let focus: ZoomFocus = Bool.random(using: &r) ? .auto : .manual(randomPoint(&r))
            blocks.append(ZoomBlock(start: s, end: s + .random(in: 0.2...4, using: &r), scale: .random(in: 0.5...20, using: &r), focus: focus))
        }
        let style = CursorStyle(smoothing: .random(in: 0...1, using: &r), hideWhenIdle: Bool.random(using: &r))
        let model = SpringMotionModel(
            track: CursorTrack(samples: samples, clicks: clicks),
            timeline: Timeline(zoomBlocks: blocks, autoZoomEnabled: trial % 5 != 0),
            cursorStyle: style,
            duration: duration,
            sourceAspect: sourceAspect,
            tuning: tuning
        )

        // Every simulated step, plus random times in between (interpolation) and outside the range.
        var times = Array(stride(from: -0.5, through: duration + 0.5, by: 1.0 / 240))
        for _ in 0..<2000 { times.append(.random(in: 0...duration, using: &r)) }
        for time in times {
            let c = model.frameState(at: time).camera
            if !checkViewport(c, sourceAspect: sourceAspect, context: "seed \(seed) t \(time)") { return }
        }
    }

    /// The four corners in turn, with bouncy springs: the camera overshoots both while zooming and
    /// while panning, and must still stop exactly at the source edges.
    @Test func cornerClicksWithOvershootingSpringsStayInside() {
        var tuning = MotionTuning()
        tuning.zoomSpring = Spring(response: 0.6, dampingRatio: 0.25)
        tuning.panSpring = Spring(response: 0.4, dampingRatio: 0.2)
        tuning.autoZoomScale = 3
        let corners = [NormalizedPoint(x: 0, y: 0), NormalizedPoint(x: 1, y: 0), NormalizedPoint(x: 1, y: 1), NormalizedPoint(x: 0, y: 1)]
        // Quick clicks pan between corners within one segment; then the same corners far apart in time.
        var clicks: [ClickEvent] = []
        for (i, c) in corners.enumerated() { clicks.append(ClickEvent(t: 1 + Double(i) * 0.6, x: c.x, y: c.y, kind: .down)) }
        for (i, c) in corners.enumerated() { clicks.append(ClickEvent(t: 8 + Double(i) * 4, x: c.x, y: c.y, kind: .down)) }
        let samples = makeSamples(duration: 26) { t in
            if let last = clicks.last(where: { $0.t <= t }) { return last.point }
            return .center
        }
        for aspect in sourceAspects {
            let m = SpringMotionModel(
                track: CursorTrack(samples: samples, clicks: clicks), timeline: Timeline(),
                cursorStyle: CursorStyle(), duration: 26, sourceAspect: aspect, tuning: tuning
            )
            var peak = 1.0
            var touchedEdge = false
            for t in stride(from: 0.0, through: 26, by: 1.0 / 480) {
                let c = m.frameState(at: t).camera
                guard checkViewport(c, sourceAspect: aspect, context: "aspect \(aspect) t \(t)") else { return }
                peak = max(peak, c.scale)
                let v = c.sourceViewport
                if c.scale > 2, v.minX == 0 || v.maxX == 1, v.minY == 0 || v.maxY == 1 { touchedEdge = true }
            }
            #expect(peak > 3.3, "the zoom spring should overshoot the 3x target (peak \(peak))")
            #expect(touchedEdge, "corner clicks should pin the viewport to the source corners")
        }
    }

    /// When settled on a corner click, the viewport is exactly the corner quarter (at 2x).
    @Test func settledCornerZoomIsPinnedToTheCorner() {
        let m = model(clicks: [click(1, 1, 1)], duration: 6)
        let c = m.frameState(at: 2.5).camera
        #expect(abs(c.scale - 2) < 1e-3)
        let v = c.sourceViewport
        #expect(v.maxX == 1 && v.maxY == 1)
        #expect(abs(v.minX - (1 - 1 / c.scale)) < 1e-12)
    }

    @Test func nonFiniteClicksAndBlocksCannotPoisonTheCamera() {
        let clicks = [click(1, .nan, 0.5), click(1.2, 0.5, .infinity), click(1.4, 0.9, 0.9)]
        let blocks = [ZoomBlock(start: 4, end: 6, scale: .infinity, focus: .manual(NormalizedPoint(x: .nan, y: -.infinity)))]
        let m = model(clicks: clicks, timeline: Timeline(zoomBlocks: blocks), duration: 8)
        for t in stride(from: 0.0, through: 8, by: 1.0 / 120) {
            checkViewport(m.frameState(at: t).camera, sourceAspect: 16.0 / 9.0, context: "t \(t)")
        }
    }
}
