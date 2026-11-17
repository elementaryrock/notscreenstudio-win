import Foundation
import Testing
import NSSCore
@testable import NSSMotion

/// Round 3: auto-zooms stored as `ZoomBlock`s, follow vs fixed focus and per-zoom ease durations.
@Suite struct StoredZoomTests {
    let wander = makeSamples(duration: 20) { t in NormalizedPoint(x: 0.3 + 0.2 * sin(t), y: 0.4 + 0.1 * cos(t * 0.7)) }
    let clicks = [click(2, 0.3, 0.3), click(2.6, 0.32, 0.31), click(8, 0.8, 0.7), click(14, 0.2, 0.8)]

    @Test func autoBlocksMatchTheOnTheFlySegments() {
        let track = CursorTrack(samples: wander, clicks: clicks)
        let segs = ZoomPlanner.autoSegments(track: track, duration: 20, sourceAspect: 16.0 / 9.0, tuning: .standard)
        let blocks = ZoomPlanner.autoZoomBlocks(track: track, duration: 20, sourceAspect: 16.0 / 9.0)
        #expect(blocks.count == segs.count && !blocks.isEmpty)
        for (b, s) in zip(blocks, segs) {
            #expect(b.start == s.start && b.end == s.end && b.scale == s.scale)
            #expect(b.isAuto && b.focus == .auto)
            #expect(b.easeIn == MotionTuning.standard.zoomSpring.response && b.easeOut == b.easeIn)
        }
        for (a, b) in zip(blocks, blocks.dropFirst()) { #expect(a.end <= b.start) }
    }

    @Test func autoBlocksAvoidExistingBlocks() {
        let track = CursorTrack(samples: wander, clicks: clicks)
        let manual = ZoomBlock(start: 1, end: 2.5, scale: 3)
        let blocks = ZoomPlanner.autoZoomBlocks(track: track, duration: 20, sourceAspect: 16.0 / 9.0, avoiding: [manual])
        for b in blocks {
            #expect(b.end <= manual.start || b.start >= manual.end)
            #expect(b.end - b.start >= MotionTuning.standard.minSegmentDuration - 1e-9)
        }
        #expect(blocks.contains { abs($0.start - 2.5) < 1e-9 })
        #expect(ZoomPlanner.autoZoomBlocks(track: track, duration: 0, sourceAspect: 1.7).isEmpty)
    }

    /// The stored blocks reproduce the Round 2 auto-zoom motion: same spring, same focus.
    @Test func storedBlocksMoveLikeRound2AutoZoom() {
        let track = CursorTrack(samples: wander, clicks: clicks)
        let legacy = model(samples: wander, clicks: clicks, timeline: Timeline(autoZoomEnabled: true))
        let blocks = ZoomPlanner.autoZoomBlocks(track: track, duration: 20, sourceAspect: 16.0 / 9.0)
        let stored = model(samples: wander, clicks: clicks, timeline: Timeline(zoomBlocks: blocks, autoZoomEnabled: false))
        var maxScale = 0.0, maxCenter = 0.0
        for t in stride(from: 0.0, through: 20, by: 1.0 / 60) {
            let a = legacy.frameState(at: t).camera, b = stored.frameState(at: t).camera
            maxScale = max(maxScale, abs(a.scale - b.scale))
            maxCenter = max(maxCenter, abs(a.center.x - b.center.x), abs(a.center.y - b.center.y))
        }
        #expect(maxScale < 1e-4)
        #expect(maxCenter < 1e-4)
    }

    @Test func followBlockAimsAtLeadInClickThenFollows() {
        let track = CursorTrack(samples: wander, clicks: [click(5.3, 0.7, 0.6)])
        let segs = ZoomPlanner.manualSegments(track: track, blocks: [ZoomBlock(start: 5, end: 9, focus: .auto)], duration: 20, tuning: .standard)
        #expect(segs.count == 1)
        #expect(segs[0].followFrom == 5.3)
        #expect(segs[0].initialFocus == NormalizedPoint(x: 0.7, y: 0.6))
        // A click later in the block does not move the start of following.
        let late = CursorTrack(samples: wander, clicks: [click(7, 0.7, 0.6)])
        let s2 = ZoomPlanner.manualSegments(track: late, blocks: [ZoomBlock(start: 5, end: 9, focus: .auto)], duration: 20, tuning: .standard)
        #expect(s2[0].followFrom == 5)
    }

    @Test func followCursorPansButFixedPointHolds() {
        // Cursor sweeps left to right while zoomed in.
        let sweep = makeSamples(duration: 10) { t in NormalizedPoint(x: min(0.95, 0.1 + 0.1 * t), y: 0.5) }
        let p = NormalizedPoint(x: 0.3, y: 0.4)
        let follow = model(samples: sweep, timeline: Timeline(zoomBlocks: [ZoomBlock(start: 1, end: 9, scale: 2, focus: .auto)], autoZoomEnabled: false), duration: 10)
        let fixed = model(samples: sweep, timeline: Timeline(zoomBlocks: [ZoomBlock(start: 1, end: 9, scale: 2, focus: .manual(p))], autoZoomEnabled: false), duration: 10)
        let f3 = follow.frameState(at: 3).camera.center.x, f8 = follow.frameState(at: 8).camera.center.x
        #expect(f8 - f3 > 0.2)
        let a = fixed.frameState(at: 4).camera, b = fixed.frameState(at: 8).camera
        #expect(abs(a.center.x - p.x) < 1e-3 && abs(a.center.y - p.y) < 1e-3)
        #expect(abs(b.center.x - p.x) < 1e-3)
        #expect(abs(a.scale - 2) < 1e-2)
    }

    private func scale(_ block: ZoomBlock, at t: Double) -> Double {
        model(timeline: Timeline(zoomBlocks: [block], autoZoomEnabled: false), duration: 12).frameState(at: t).camera.scale
    }

    @Test func easeInControlsHowFastTheZoomArrives() {
        let fast = ZoomBlock(start: 2, end: 8, scale: 2, focus: .manual(.center), easeIn: 0.3)
        let slow = ZoomBlock(start: 2, end: 8, scale: 2, focus: .manual(.center), easeIn: 1.5)
        #expect(scale(fast, at: 2.4) > scale(slow, at: 2.4) + 0.3)
        #expect(abs(scale(fast, at: 2.4) - 2) < 0.1)
        #expect(abs(scale(slow, at: 2 + 1.5 * 1.3) - 2) < 0.05)   // settles in about one response
        #expect(abs(scale(slow, at: 1.99) - 1) < 1e-6)             // nothing before the block starts
    }

    @Test func easeOutControlsHowFastItReturns() {
        let fast = ZoomBlock(start: 1, end: 5, scale: 2, focus: .manual(.center), easeOut: 0.3)
        let slow = ZoomBlock(start: 1, end: 5, scale: 2, focus: .manual(.center), easeOut: 1.5)
        #expect(abs(scale(fast, at: 4.99) - scale(slow, at: 4.99)) < 1e-6)  // same until the block ends
        #expect(scale(slow, at: 5.4) > scale(fast, at: 5.4) + 0.3)
        #expect(abs(scale(fast, at: 5.4) - 1) < 0.1)
    }

    /// The ease keeps the spring's feel: smooth, monotone and no overshoot (not a linear tween).
    @Test func easeIsASpringNotALinearTween() {
        let block = ZoomBlock(start: 1, end: 6, scale: 2, focus: .manual(.center), easeIn: 1, easeOut: 1)
        let m = model(timeline: Timeline(zoomBlocks: [block], autoZoomEnabled: false), duration: 10)
        var prev = 1.0
        var steps: [Double] = []
        for t in stride(from: 1.0, through: 3, by: 1.0 / 240) {
            let s = m.frameState(at: t).camera.scale
            #expect(s >= prev - 1e-9 && s <= 2 + 1e-9)
            steps.append(s - prev)
            prev = s
        }
        // Spring: starts slowly (zero initial velocity), speeds up, then slows; a linear tween is constant.
        let early = steps[1...10].reduce(0, +), mid = steps[60...69].reduce(0, +), late = steps[400...409].reduce(0, +)
        #expect(mid > early * 2)
        #expect(mid > late * 2)
        for t in stride(from: 6.0, through: 9, by: 1.0 / 60) {
            #expect(m.frameState(at: t).camera.scale >= 1 - 1e-9)
        }
    }

    @Test func easeValuesAreClampedAndNonFiniteFallsBack() {
        #expect(ZoomPlanner.clampEase(.nan) == ZoomBlock.defaultEase)
        #expect(ZoomPlanner.clampEase(0) == ZoomPlanner.easeClampRange.lowerBound)
        #expect(ZoomPlanner.clampEase(100) == ZoomPlanner.easeClampRange.upperBound)
        let b = ZoomBlock(start: 1, end: 3, easeIn: .infinity, easeOut: -1)
        let segs = ZoomPlanner.manualSegments(track: .empty, blocks: [b], duration: 5, tuning: .standard)
        #expect(segs[0].easeIn == ZoomBlock.defaultEase && segs[0].easeOut == ZoomPlanner.easeClampRange.lowerBound)
    }
}
