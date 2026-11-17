import Foundation
import Testing
import NSSCore
@testable import NSSMotion

private func change(_ t: Double, _ shape: CursorShape) -> CursorShapeChange { CursorShapeChange(t: t, shape: shape) }

private func clickEvent(_ t: Double, _ kind: ClickKind, _ button: MouseButton = .left) -> ClickEvent {
    ClickEvent(t: t, x: 0.5, y: 0.5, button: button, kind: kind)
}

private func pressModel(_ clicks: [ClickEvent], duration: Double = 10, trimStart: Double = 0) -> SpringMotionModel {
    let still = makeSamples(duration: duration) { _ in NormalizedPoint(x: 0.5, y: 0.5) }
    return SpringMotionModel(
        track: CursorTrack(samples: still, clicks: clicks),
        timeline: Timeline(trim: TrimRange(start: trimStart), autoZoomEnabled: false),
        cursorStyle: CursorStyle(),
        duration: duration
    )
}

private func pressScales(_ m: SpringMotionModel, _ from: Double, _ to: Double) -> [Double] {
    stride(from: from, through: to, by: 1.0 / 240).map { m.frameState(at: $0).cursor.pressScale }
}

@Suite struct CursorShapeMotionTests {
    @Test func flickersShorterThanMinimumAreDropped() {
        let raw = [
            change(0, .arrow), change(1, .iBeam), change(1.03, .pointingHand), change(2, .arrow),
            change(2.01, .iBeam), change(2.02, .arrow), change(4, .crosshair),
        ]
        let stable = SpringMotionModel.stableShapes(raw, minDuration: 0.05)
        // 1.0 iBeam lasted 30 ms (dropped), 2.0 arrow repeated the arrow-to-be, 2.01 iBeam lasted 10 ms.
        #expect(stable == [change(1.03, .pointingHand), change(2.02, .arrow), change(4, .crosshair)])
    }

    @Test func unsortedAndNonFiniteChangesAreHandled() {
        let raw = [change(3, .iBeam), change(.nan, .crosshair), change(1, .pointingHand), change(.infinity, .arrow)]
        #expect(SpringMotionModel.stableShapes(raw, minDuration: 0.05) == [change(1, .pointingHand), change(3, .iBeam)])
        #expect(SpringMotionModel.stableShapes([], minDuration: 0.05).isEmpty)
    }

    @Test func frameStateReportsTheStableShape() {
        let track = CursorTrack(
            samples: makeSamples(duration: 6) { _ in NormalizedPoint(x: 0.4, y: 0.4) },
            shapes: [change(1, .iBeam), change(1.02, .pointingHand), change(3, .unknown), change(5, .arrow)]
        )
        let m = SpringMotionModel(track: track, timeline: Timeline(), cursorStyle: CursorStyle(), duration: 6)
        #expect(m.frameState(at: 0.5).cursor.shape == .arrow)
        #expect(m.frameState(at: 1.01).cursor.shape == .arrow) // the 20 ms I-beam never shows
        #expect(m.frameState(at: 1.02).cursor.shape == .pointingHand)
        #expect(m.frameState(at: 2.99).cursor.shape == .pointingHand)
        #expect(m.frameState(at: 3.5).cursor.shape == .unknown)
        #expect(m.frameState(at: 5.5).cursor.shape == .arrow)
        // Any lookup order gives the same answer.
        #expect(m.frameState(at: 3.5) == m.frameState(at: 3.5))
    }

    @Test func roundOneTracksDrawTheArrow() throws {
        let json = #"{"samples":[{"t":0,"x":0.5,"y":0.5},{"t":1,"x":0.6,"y":0.5}],"clicks":[],"sampleRate":120}"#
        let track = try JSONDecoder().decode(CursorTrack.self, from: Data(json.utf8))
        let m = SpringMotionModel(track: track, timeline: Timeline(), cursorStyle: CursorStyle(), duration: 1)
        for t in stride(from: 0.0, through: 1, by: 0.1) {
            #expect(m.frameState(at: t).cursor.shape == .arrow)
            #expect(m.frameState(at: t).cursor.pressScale == 1)
        }
    }
}

@Suite struct PressSquishTests {
    let tuning = MotionTuning.standard

    @Test func holdSquishesQuicklyAndReleaseSpringsBackWithSmallOvershoot() {
        let m = pressModel([clickEvent(2, .down), clickEvent(2.6, .up)])
        // Untouched before the press.
        #expect(pressScales(m, 0, 1.999).allSatisfy { $0 == 1 })
        // Quick ease down: most of the way within 100 ms, monotone (critically damped), never below target.
        #expect(m.frameState(at: 2.1).cursor.pressScale < tuning.pressScale + 0.02)
        let down = pressScales(m, 2, 2.59)
        #expect(zip(down, down.dropFirst()).allSatisfy { $1 <= $0 + 1e-6 })
        #expect(down.allSatisfy { $0 >= tuning.pressScale - 1e-6 })
        #expect(abs(m.frameState(at: 2.55).cursor.pressScale - tuning.pressScale) < 1e-3)
        // Spring back: small overshoot above 1, then settles.
        let up = pressScales(m, 2.6, 4)
        let peak = up.max()!
        #expect(peak > 1.005 && peak < 1.05)
        #expect(abs(m.frameState(at: 4.5).cursor.pressScale - 1) < 1e-3)
    }

    @Test func clickWithoutVisibleHoldStillSquishes() {
        for up in [5.0, 5.005, 5.03] { // up at the same instant, 5 ms and 30 ms later
            let m = pressModel([clickEvent(5, .down), clickEvent(up, .up)])
            let dip = pressScales(m, 5, 5.4).min()!
            #expect(dip < 0.88, "click released after \(up - 5) s only dipped to \(dip)")
            #expect(abs(m.frameState(at: 7).cursor.pressScale - 1) < 1e-3)
        }
    }

    @Test func missingMouseUpAndOverlappingButtonsAreHandled() {
        let lost = pressModel([clickEvent(1, .down)])
        #expect(pressScales(lost, 1, 1.4).min()! < 0.88)
        #expect(abs(lost.frameState(at: 3).cursor.pressScale - 1) < 1e-3)

        // Left held 1...3, right pressed and released inside it: stays squished until the left is up.
        let both = pressModel([
            clickEvent(1, .down), clickEvent(1.5, .down, .right), clickEvent(1.7, .up, .right), clickEvent(3, .up),
        ])
        #expect(pressScales(both, 1.3, 3).allSatisfy { abs($0 - tuning.pressScale) < 0.01 })
    }

    @Test func trimStartingInsideAPressIsSettledSquished() {
        let m = pressModel([clickEvent(1, .down), clickEvent(4, .up)], trimStart: 2)
        #expect(abs(m.frameState(at: 2).cursor.pressScale - tuning.pressScale) < 1e-6)
    }

    @Test func pressTrackerMergesAndExtendsShortClicks() {
        var p = PressTracker(clicks: [clickEvent(1, .down), clickEvent(1.01, .up), clickEvent(1.05, .down), clickEvent(1.5, .up)], minHold: 0.08)
        #expect(p.intervals.count == 1)
        #expect(p.intervals[0].start == 1 && p.intervals[0].end == 1.5)
        let states = [0.9, 1, 1.03, 1.5].map { p.isPressed(at: $0) }
        #expect(states == [false, true, true, false])
    }
}
