import Foundation
import Testing
import NSSCore
@testable import NSSMotion

// MARK: - Fixtures

/// 120 Hz samples from a position function over [0, duration].
func makeSamples(duration: Double, rate: Double = 120, _ f: (Double) -> NormalizedPoint) -> [CursorSample] {
    let n = Int(duration * rate)
    return (0...n).map { i in
        let t = Double(i) / rate
        let p = f(t)
        return CursorSample(t: t, x: p.x, y: p.y)
    }
}

func click(_ t: Double, _ x: Double, _ y: Double) -> ClickEvent {
    ClickEvent(t: t, x: x, y: y, kind: .down)
}

func model(
    samples: [CursorSample] = [],
    clicks: [ClickEvent] = [],
    timeline: Timeline = Timeline(),
    style: CursorStyle = CursorStyle(),
    duration: Double = 20,
    tuning: MotionTuning = .standard
) -> SpringMotionModel {
    SpringMotionModel(
        track: CursorTrack(samples: samples, clicks: clicks),
        timeline: timeline,
        cursorStyle: style,
        duration: duration,
        sourceAspect: 16.0 / 9.0,
        tuning: tuning
    )
}

// MARK: - Spring

@Suite struct SpringTests {
    @Test func criticallyDampedConvergesWithoutOvershoot() {
        let spring = Spring(response: 0.5, dampingRatio: 1)
        let stepper = spring.stepper(dt: 1.0 / 240)
        var x = 0.0, v = 0.0
        var prev = x
        for _ in 0..<(240 * 3) {
            stepper.step(&x, &v, target: 1)
            #expect(x <= 1 + 1e-12)
            #expect(x >= prev - 1e-12) // monotone
            prev = x
        }
        #expect(abs(x - 1) < 1e-6)
        #expect(abs(v) < 1e-4)
    }

    @Test func criticallyDampedReachesTargetWithinAboutOneResponse() {
        let spring = Spring(response: 0.6)
        let (x, _) = spring.evolve(displacement: 1, velocity: 0, dt: 0.6 * 1.2)
        #expect(abs(x) < 0.01)
    }

    @Test func stepperMatchesAnalyticSolution() {
        for zeta in [0.4, 1.0, 1.8] {
            let spring = Spring(response: 0.3, dampingRatio: zeta)
            let stepper = spring.stepper(dt: 1.0 / 240)
            var x = 0.7, v = -2.0
            for _ in 0..<240 { stepper.step(&x, &v, target: 0) }
            let exact = spring.evolve(displacement: 0.7, velocity: -2.0, dt: 1)
            #expect(abs(x - exact.displacement) < 1e-9)
            #expect(abs(v - exact.velocity) < 1e-8)
        }
    }

    @Test func underdampedOvershootsAndOverdampedConverges() {
        let under = Spring(response: 0.5, dampingRatio: 0.5).stepper(dt: 1.0 / 240)
        var x = 0.0, v = 0.0, peak = 0.0
        for _ in 0..<480 {
            under.step(&x, &v, target: 1)
            peak = max(peak, x)
        }
        #expect(peak > 1.05)

        let over = Spring(response: 0.5, dampingRatio: 2).stepper(dt: 1.0 / 240)
        x = 0; v = 0
        for _ in 0..<(240 * 5) {
            over.step(&x, &v, target: 1)
            #expect(x <= 1 + 1e-12)
        }
        #expect(abs(x - 1) < 1e-3)
    }

    @Test func largeStepIsStable() {
        let s = Spring(response: 0.05).stepper(dt: 1) // 20x the response
        var x = 1.0, v = 50.0
        for _ in 0..<10 { s.step(&x, &v, target: 0) }
        #expect(x.isFinite && abs(x) < 1e-6)
    }
}

// MARK: - Model

@Suite struct SpringMotionModelTests {
    let wander = makeSamples(duration: 20) { t in
        NormalizedPoint(x: 0.5 + 0.4 * sin(t * 0.9), y: 0.5 + 0.35 * cos(t * 1.3))
    }

    @Test func deterministicForwardBackwardAndAcrossBuilds() {
        let clicks = [click(2, 0.2, 0.3), click(2.6, 0.25, 0.35), click(9, 0.8, 0.8)]
        let m1 = model(samples: wander, clicks: clicks)
        let m2 = model(samples: wander, clicks: clicks)
        let times = stride(from: 0.0, through: 20, by: 1.0 / 60).map { $0 }
        let forward = times.map { m1.frameState(at: $0) }
        let backward = times.reversed().map { m1.frameState(at: $0) }.reversed()
        #expect(forward == Array(backward))
        var rng = SystemRandomNumberGenerator()
        for _ in 0..<500 {
            let t = Double.random(in: 0...20, using: &rng)
            #expect(m1.frameState(at: t) == m2.frameState(at: t))
        }
    }

    @Test func viewportStaysInsideSource() {
        let corners = makeSamples(duration: 12) { t in
            let k = Int(t / 2) % 4
            return [NormalizedPoint(x: 0, y: 0), NormalizedPoint(x: 1, y: 0), NormalizedPoint(x: 1, y: 1), NormalizedPoint(x: 0, y: 1)][k]
        }
        let clicks = [click(1, 0, 0), click(3, 1, 0), click(5, 1, 1), click(7, 0, 1), click(9.5, 0.01, 0.99)]
        let blocks = [ZoomBlock(start: 10.5, end: 12, scale: 4, focus: .manual(NormalizedPoint(x: 1, y: 1)))]
        let m = model(samples: corners, clicks: clicks, timeline: Timeline(zoomBlocks: blocks), duration: 12)
        var maxScale = 1.0
        for t in stride(from: 0.0, through: 12, by: 1.0 / 240) {
            let c = m.frameState(at: t).camera
            let h = 0.5 / c.scale
            #expect(c.scale >= 1)
            #expect(c.center.x >= h - 1e-9 && c.center.x <= 1 - h + 1e-9)
            #expect(c.center.y >= h - 1e-9 && c.center.y <= 1 - h + 1e-9)
            maxScale = max(maxScale, c.scale)
        }
        #expect(maxScale > 3.5)
    }

    @Test func clusteredClicksMakeOneSegmentWithLeadInAndHold() {
        let t = MotionTuning.standard
        let clicks = [click(2, 0.3, 0.3), click(2.5, 0.32, 0.31), click(3, 0.31, 0.29)]
        let m = model(clicks: clicks)
        #expect(m.zoomSegments.count == 1)
        let seg = m.zoomSegments[0]
        #expect(abs(seg.start - (2 - t.autoZoomLeadIn)) < 1e-9)
        #expect(abs(seg.end - (3 + t.autoZoomHold)) < 1e-9)
        #expect(seg.origin == .auto)
        #expect(seg.scale == t.autoZoomScale)

        #expect(m.frameState(at: 0.5).camera.scale == 1)
        #expect(m.frameState(at: 2).camera.scale > 1.6) // mostly in by the first click
        let held = m.frameState(at: 4).camera
        #expect(abs(held.scale - 2) < 0.02)
        #expect(abs(held.center.x - 0.3) < 0.02 && abs(held.center.y - 0.3) < 0.02)
        #expect(abs(m.frameState(at: 9).camera.scale - 1) < 1e-3) // eased out after idle
    }

    @Test func distantInTimeClicksMakeSeparateSegments() {
        let m = model(clicks: [click(2, 0.3, 0.3), click(10, 0.3, 0.3)])
        #expect(m.zoomSegments.count == 2)
        #expect(abs(m.frameState(at: 7).camera.scale - 1) < 1e-2)
    }

    @Test func farApartClicksSplitButQuickOnesPan() {
        // 2 s apart, far apart in space -> separate clusters, but gap after hold is short -> merged.
        let quick = model(clicks: [click(2, 0.1, 0.1), click(2.5, 0.9, 0.9)])
        #expect(quick.zoomSegments.count == 1)
        // Long enough apart (but within clusterMaxGap) and far -> zoom out between.
        var tuning = MotionTuning()
        tuning.autoZoomHold = 0.5
        tuning.autoZoomLeadIn = 0.3
        let split = model(clicks: [click(2, 0.1, 0.1), click(4.9, 0.9, 0.9)], tuning: tuning)
        #expect(split.zoomSegments.count == 2)
        let near = model(clicks: [click(2, 0.4, 0.4), click(4.9, 0.42, 0.41)], tuning: tuning)
        #expect(near.zoomSegments.count == 1)
    }

    @Test func holdExtendsWhileCursorMoves() {
        let t = MotionTuning.standard
        // Cursor keeps moving until 5 s, then stops.
        let samples = makeSamples(duration: 12) { t in
            NormalizedPoint(x: 0.3 + 0.05 * sin(min(t, 5) * 3), y: 0.3)
        }
        let m = model(samples: samples, clicks: [click(2, 0.3, 0.3)])
        #expect(m.zoomSegments.count == 1)
        let end = m.zoomSegments[0].end
        #expect(end > 5 + t.autoZoomHold - 0.05)
        #expect(end <= 2 + t.autoZoomMaxHold + 1e-9)
    }

    @Test func autoZoomDisabled() {
        let m = model(clicks: [click(2, 0.3, 0.3)], timeline: Timeline(autoZoomEnabled: false))
        #expect(m.zoomSegments.isEmpty)
        for t in stride(from: 0.0, through: 20, by: 0.25) {
            #expect(m.frameState(at: t).camera == .identity)
        }
    }

    @Test func manualBlockOverridesAuto() {
        let id = UUID()
        let block = ZoomBlock(id: id, start: 3, end: 8, scale: 3, focus: .manual(NormalizedPoint(x: 0.75, y: 0.7)))
        let m = model(samples: wander, clicks: [click(2, 0.2, 0.2), click(3.5, 0.25, 0.2)], timeline: Timeline(zoomBlocks: [block]))
        #expect(Array(m.zoomSegments.prefix(2).map(\.origin)) == [.auto, .manual(id)])
        #expect(m.zoomSegments[0].end == 3)
        // No auto segment overlaps the manual block (a remnant may follow it).
        for s in m.zoomSegments where s.origin == .auto {
            #expect(s.end <= 3 || s.start >= 8)
        }
        let s = m.frameState(at: 7.5).camera
        #expect(abs(s.scale - 3) < 0.02)
        #expect(abs(s.center.x - 0.75) < 0.01 && abs(s.center.y - 0.7) < 0.01)

        // A manual scale-1 block suppresses auto zoom entirely.
        let off = ZoomBlock(start: 0, end: 20, scale: 1)
        let m2 = model(clicks: [click(2, 0.2, 0.2)], timeline: Timeline(zoomBlocks: [off]))
        #expect(m2.frameState(at: 3).camera.scale == 1)
    }

    @Test func manualAutoFocusFollowsCursorWithDeadZone() {
        // Cursor parked at 0.3 then jumps to 0.8 at t = 4.
        let samples = makeSamples(duration: 10) { t in
            t < 2.5 ? NormalizedPoint(x: 0.4, y: 0.5) : (t < 4 ? NormalizedPoint(x: 0.43, y: 0.52) : NormalizedPoint(x: 0.8, y: 0.5))
        }
        let block = ZoomBlock(start: 1, end: 10, scale: 2, focus: .auto)
        let m = model(samples: samples, timeline: Timeline(zoomBlocks: [block], autoZoomEnabled: false), duration: 10)
        let a = m.frameState(at: 3.9).camera
        // small move (0.03) inside the dead zone: camera did not pan
        #expect(abs(a.center.x - 0.4) < 1e-3 && abs(a.center.y - 0.5) < 1e-3)
        let b = m.frameState(at: 9).camera
        // big move: pans so the cursor sits at the dead-zone edge (0.8 - 0.125), clamped
        #expect(abs(b.center.x - 0.675) < 0.01)
    }

    @Test func ripplesTiming() {
        let d = MotionTuning.standard.rippleDuration
        let m = model(clicks: [click(1, 0.4, 0.6), ClickEvent(t: 1.1, x: 0.4, y: 0.6, kind: .up)])
        #expect(m.frameState(at: 0.99).cursor.ripples.isEmpty)
        let r0 = m.frameState(at: 1).cursor.ripples
        #expect(r0.count == 1 && r0[0].progress == 0 && r0[0].position == NormalizedPoint(x: 0.4, y: 0.6))
        let rh = m.frameState(at: 1 + d / 2).cursor.ripples
        #expect(rh.count == 1 && abs(rh[0].progress - 0.5) < 1e-9)
        #expect(m.frameState(at: 1 + d).cursor.ripples.isEmpty)

        let two = model(clicks: [click(1, 0.1, 0.1), click(1.1, 0.2, 0.2)])
        #expect(two.frameState(at: 1.2).cursor.ripples.count == 2)

        let off = model(clicks: [click(1, 0.4, 0.6)], style: CursorStyle(clickRipples: false))
        #expect(off.frameState(at: 1.1).cursor.ripples.isEmpty)
    }

    @Test func smoothCursorFollowsAndReportsVelocity() {
        // Constant motion to the right at 0.2 /s, then stops at t = 3.
        let samples = makeSamples(duration: 6) { t in NormalizedPoint(x: 0.2 + 0.2 * min(t, 3), y: 0.5) }
        let m = model(samples: samples, duration: 6)
        let moving = m.frameState(at: 2).cursor
        #expect(abs(moving.velocity.x - 0.2) < 0.01)
        #expect(abs(moving.velocity.y) < 1e-6)
        #expect(moving.position.x < 0.6 && moving.position.x > 0.55) // lags a little behind 0.6
        let settled = m.frameState(at: 5).cursor
        #expect(abs(settled.position.x - 0.8) < 1e-4)
        #expect(abs(settled.velocity.x) < 1e-3)
        #expect(settled.visible && settled.opacity == 1)

        let raw = model(samples: samples, style: CursorStyle(smoothing: 0), duration: 6).frameState(at: 2).cursor
        #expect(abs(raw.position.x - 0.6) < 1e-9)
        #expect(abs(raw.velocity.x - 0.2) < 1e-6)
    }

    @Test func noSamplesHidesCursor() {
        let s = model(clicks: [click(1, 0.5, 0.5)]).frameState(at: 2)
        #expect(!s.cursor.visible)
    }

    @Test func hideWhenIdleFades() {
        let t = MotionTuning.standard
        let samples = makeSamples(duration: 10) { tt in
            NormalizedPoint(x: tt < 1 || tt > 7 ? 0.2 + 0.05 * tt : 0.25, y: 0.5)
        }
        let m = model(samples: samples, style: CursorStyle(hideWhenIdle: true), duration: 10)
        #expect(m.frameState(at: 1).cursor.opacity == 1)
        let fading = m.frameState(at: 1 + t.idleHideDelay + t.idleFadeOutDuration / 2).cursor
        #expect(fading.opacity > 0.2 && fading.opacity < 0.8)
        let hidden = m.frameState(at: 6.5).cursor
        #expect(hidden.opacity == 0 && !hidden.visible)
        let back = m.frameState(at: 7 + t.idleFadeInDuration + 0.05).cursor
        #expect(back.opacity == 1 && back.visible)

        let always = model(samples: samples, duration: 10).frameState(at: 6.5).cursor
        #expect(always.opacity == 1 && always.visible)
    }

    @Test func trimStartIsSettled() {
        // A zoom segment spans the trim start: the camera must already be fully zoomed there.
        let clicks = [click(4, 0.3, 0.3), click(4.3, 0.3, 0.3), click(5, 0.3, 0.3)]
        let m = model(samples: wander, clicks: clicks, timeline: Timeline(trim: TrimRange(start: 4.5, end: 15)))
        let s = m.frameState(at: 4.5)
        #expect(s.cursor.ripples.count == 1) // the 4.3 s click, still rippling at the trim start
        #expect(abs(s.camera.scale - 2) < 1e-6)
        // the cursor is settled on the raw position at the trim start
        let raw = CursorTrack(samples: wander).rawPosition(at: 4.5)!
        #expect(abs(s.cursor.position.x - raw.x) < 1e-6 && abs(s.cursor.position.y - raw.y) < 1e-6)
        // times outside the trim clamp to its edges
        var before = m.frameState(at: 1)
        before.time = 4.5
        #expect(before == s)
    }

    @Test func nestedManualBlockKeepsOuterRemainder() {
        let a = UUID(), b = UUID()
        let outer = ZoomBlock(id: a, start: 0, end: 10, scale: 2, focus: .manual(NormalizedPoint(x: 0.3, y: 0.3)))
        let inner = ZoomBlock(id: b, start: 2, end: 3, scale: 3, focus: .manual(NormalizedPoint(x: 0.6, y: 0.6)))
        let m = model(clicks: [click(6, 0.9, 0.9)], timeline: Timeline(zoomBlocks: [inner, outer]))
        let segs = m.zoomSegments
        #expect(segs.map(\.origin) == [.manual(a), .manual(b), .manual(a)])
        #expect(segs.map(\.start) == [0, 2, 3])
        #expect(segs.map(\.end) == [2, 3, 10])
        // A still owns 3...10 s: zoomed at 2x on its focus, and the auto-zoom click at 6 s is ignored.
        let c = m.frameState(at: 8).camera
        #expect(abs(c.scale - 2) < 0.01)
        #expect(abs(c.center.x - 0.3) < 0.01 && abs(c.center.y - 0.3) < 0.01)

        // Follow-mode remainders are re-anchored on the cursor where they resume.
        let samples = makeSamples(duration: 10) { t in NormalizedPoint(x: t < 3 ? 0.2 : 0.7, y: 0.5) }
        let follow = ZoomBlock(id: a, start: 0, end: 10, scale: 2, focus: .auto)
        let m2 = model(samples: samples, timeline: Timeline(zoomBlocks: [follow, inner], autoZoomEnabled: false), duration: 10)
        #expect(m2.zoomSegments.count == 3)
        #expect(abs(m2.zoomSegments[2].initialFocus.x - 0.7) < 1e-9)
        #expect(m2.zoomSegments[2].followFrom == 0)
    }

    @Test func hugeFiniteDurationsAndTimestampsAreBounded() {
        var tuning = MotionTuning()
        tuning.maxSimulationFrames = 10_000 // keep the debug test fast; same code path as the default cap
        let huge = model(clicks: [click(5, 0.3, 0.3)], duration: 1e15, tuning: tuning)
        #expect(huge.simulatedFrameCount <= 10_000)
        #expect(huge.endTime == tuning.maxDuration)
        _ = huge.frameState(at: 1e15)
        _ = huge.frameState(at: 6)

        // Absolute timestamps (seconds since 1970) by mistake, with the duration derived from them.
        let absolute = SpringMotionModel(
            track: CursorTrack(samples: [CursorSample(t: 1.7e9, x: 0.5, y: 0.5)], clicks: [click(1.7e9 + 1, 0.4, 0.4)]),
            timeline: Timeline(), cursorStyle: CursorStyle(), duration: 0, tuning: tuning
        )
        #expect(absolute.simulatedFrameCount <= 10_000)
        #expect(absolute.frameState(at: 1.7e9).camera.scale >= 1)

        let absurd = model(clicks: [click(1e300, 0.4, 0.4)], timeline: Timeline(trim: TrimRange(start: 1e300, end: .greatestFiniteMagnitude)), duration: .greatestFiniteMagnitude, tuning: tuning)
        #expect(absurd.simulatedFrameCount <= 10_000)

        // Default cap: the table stays at or below maxSimulationFrames (about 36 MB).
        #expect(MotionTuning.standard.maxSimulationFrames * MemoryLayout<SpringMotionModel.SimFrame>.stride <= 36_000_000)
    }

    @Test func zoomOutDoesNotSwingTowardNextSegment() {
        // Segment 1 focuses top-left, segment 2 (8 s later) bottom-right.
        let m = model(clicks: [click(2, 0.2, 0.2), click(10, 0.8, 0.8)])
        #expect(m.zoomSegments.count == 2)
        let end1 = m.zoomSegments[0].end
        for t in stride(from: end1, to: m.zoomSegments[1].start, by: 1.0 / 120) {
            let c = m.frameState(at: t).camera
            guard c.scale > 1.05 else { continue }
            // While zooming out the focus stays at 0.2, so the center sits on the low clamp edge.
            #expect(c.center.x <= max(0.2, 0.5 / c.scale) + 1e-4)
            #expect(c.center.y <= max(0.2, 0.5 / c.scale) + 1e-4)
        }
        // ... and the next zoom-in still lands on its target.
        let c2 = m.frameState(at: 11).camera
        #expect(abs(c2.center.x - 0.75) < 0.02) // 0.8 clamped to 1 - 0.5/2
    }

    @Test func dragHoldsZoomUntilRelease() {
        let t = MotionTuning.standard
        let clicks = [click(2, 0.3, 0.3), ClickEvent(t: 9, x: 0.35, y: 0.3, kind: .up)]
        let m = model(clicks: clicks)
        #expect(m.zoomSegments.count == 1)
        #expect(abs(m.zoomSegments[0].end - (9 + t.autoZoomHold)) < 1e-9)
        #expect(abs(m.frameState(at: 8).camera.scale - 2) < 0.01)

        // A plain click (down + quick up) still eases out after the normal hold.
        let quick = model(clicks: [click(2, 0.3, 0.3), ClickEvent(t: 2.1, x: 0.3, y: 0.3, kind: .up)])
        #expect(abs(quick.zoomSegments[0].end - (2.1 + t.autoZoomHold)) < 1e-9)

        // Mismatched buttons do not pair.
        let other = model(clicks: [click(2, 0.3, 0.3), ClickEvent(t: 9, x: 0.3, y: 0.3, button: .right, kind: .up)])
        #expect(abs(other.zoomSegments[0].end - (2 + t.autoZoomHold)) < 1e-9)
    }

    @Test func spatialSplitIsEffectiveWithStandardTuning() {
        let t = MotionTuning.standard
        // Far apart, 2.5 s apart: zoom out between them and back in on the new spot.
        let far = model(clicks: [click(2, 0.1, 0.1), click(4.5, 0.9, 0.9)])
        #expect(far.zoomSegments.count == 2)
        #expect(far.zoomSegments[1].start - far.zoomSegments[0].end >= t.minZoomOutGap - 1e-9)
        #expect(far.zoomSegments[0].end >= 2 + t.autoZoomMinHold - 1e-9)
        // Same timing, nearby: one held segment.
        #expect(model(clicks: [click(2, 0.3, 0.3), click(4.5, 0.35, 0.3)]).zoomSegments.count == 1)
        // Far apart but only 1.5 s apart: not enough room to zoom out, so it pans in one segment.
        #expect(model(clicks: [click(2, 0.1, 0.1), click(3.5, 0.9, 0.9)]).zoomSegments.count == 1)
        // Far apart and within clusterPanGap: one segment.
        #expect(model(clicks: [click(2, 0.1, 0.1), click(2.8, 0.9, 0.9)]).zoomSegments.count == 1)
    }

    @Test func degenerateInputsDoNotCrash() {
        let empty = SpringMotionModel(track: .empty, timeline: Timeline(), cursorStyle: CursorStyle(), duration: 0)
        #expect(empty.frameState(at: 0).camera == .identity)
        #expect(empty.frameState(at: .nan).camera == .identity)
        let weird = model(
            samples: [CursorSample(t: 1, x: 0.5, y: 0.5), CursorSample(t: 0.5, x: .nan, y: 0.2)],
            clicks: [click(.infinity, 0.5, 0.5)],
            timeline: Timeline(trim: TrimRange(start: 30, end: 10), zoomBlocks: [ZoomBlock(start: 5, end: 2)]),
            duration: 20
        )
        _ = weird.frameState(at: 3)
    }

    @Test func factoryBuildsSpringModel() {
        let m = SpringMotionModel.factory(CursorTrack(samples: wander, clicks: [click(2, 0.3, 0.3)]), Timeline(), CursorStyle(), 20)
        #expect(m is SpringMotionModel)
        #expect(m.frameState(at: 3).camera.scale > 1.5)
    }

    @Test func cursorStateDecodesWithoutOpacity() throws {
        let json = #"{"position":{"x":0.1,"y":0.2},"velocity":{"x":0,"y":0},"visible":true,"ripples":[]}"#
        let s = try JSONDecoder().decode(CursorState.self, from: Data(json.utf8))
        #expect(s.opacity == 1 && s.position.x == 0.1)
    }
}

// MARK: - Performance

@Suite struct MotionPerformanceTests {
    #if DEBUG
    // Unoptimized test builds are several times slower; release builds are far under 100 ms.
    static let buildBudget = 1.0
    #else
    static let buildBudget = 0.1
    #endif

    /// Wall-clock budgets are asserted only with `NSS_BENCH=1` (loaded machines and CI VMs are unreliable).
    static let bench = ProcessInfo.processInfo.environment["NSS_BENCH"] == "1"

    @Test func tenMinuteBuildAndLookupAreFast() {
        let duration = 600.0
        let samples = makeSamples(duration: duration) { t in
            NormalizedPoint(x: 0.5 + 0.45 * sin(t * 0.37), y: 0.5 + 0.45 * sin(t * 0.53 + 1))
        }
        let clicks = stride(from: 1.0, to: duration, by: 1.7).map { t in click(t, 0.5 + 0.4 * sin(t), 0.5 + 0.4 * cos(t)) }
        let track = CursorTrack(samples: samples, clicks: clicks)

        let clock = ContinuousClock()
        var m: SpringMotionModel?
        let build = clock.measure {
            m = SpringMotionModel(track: track, timeline: Timeline(), cursorStyle: CursorStyle(hideWhenIdle: true), duration: duration)
        }
        let model = m!
        let buildSeconds = Double(build.components.attoseconds) / 1e18 + Double(build.components.seconds)
        print("NSSMotion: 10 min build took \(buildSeconds * 1000) ms")
        if Self.bench { #expect(buildSeconds < Self.buildBudget) }

        let lookups = 36_000 // 10 minutes of 60 fps frames
        var sink = 0.0
        let lookup = clock.measure {
            for i in 0..<lookups { sink += model.frameState(at: Double(i) / 60).camera.scale }
        }
        let perLookup = (Double(lookup.components.attoseconds) / 1e18 + Double(lookup.components.seconds)) / Double(lookups)
        print("NSSMotion: lookup \(perLookup * 1e6) µs")
        #expect(sink > 0)
        if Self.bench { #expect(perLookup < 50e-6) }
    }
}
