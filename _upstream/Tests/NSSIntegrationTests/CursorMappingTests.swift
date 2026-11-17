import CoreGraphics
import Foundation
import Testing
import NSSCore
import os
@testable import NSSCapture
@testable import NSSInput

/// Mutable geometry standing in for CoreGraphics, so a window can "move".
final class MovingGeometry: ScreenGeometryProvider, @unchecked Sendable {
    private let rect: OSAllocatedUnfairLock<CGRect>
    let primaryDisplayHeight: Double = 900
    init(_ r: CGRect) { rect = OSAllocatedUnfairLock(initialState: r) }
    func move(to r: CGRect) { rect.withLock { $0 = r } }
    func globalRect(for source: CaptureSource) -> CGRect? { rect.withLock { $0 } }
    func isDynamic(_ source: CaptureSource) -> Bool { if case .window = source { return true }; return false }
}

/// Where a cursor at a global point lands in the recorded video: NSSInput's normalized, top-left coordinates
/// times the pixel size NSSCapture will record. Compared with an independent computation in points.
@Suite struct CursorMappingTests {
    /// Logs one sample and one click at `global` (points, global top-left) and returns the video pixel.
    private func logged(
        at global: CGPoint, source: CaptureSource, geometry: MovingGeometry, videoPixels: PixelSize
    ) throws -> (sample: CGPoint, click: CGPoint) {
        let recorder = CursorRecorder(
            source: source, geometry: geometry, schedule: SamplingSchedule(rate: 120),
            position: { global }, clock: { 10 })
        recorder.tick()
        recorder.click(at: global, hostTime: 10.001, button: .left, kind: .down)
        let track = recorder.finish(recordingStartHostTime: 9, sampleRate: 120)
        let s = try #require(track.samples.last), c = try #require(track.clicks.first)
        func px(_ x: Double, _ y: Double) -> CGPoint { CGPoint(x: x * Double(videoPixels.width), y: y * Double(videoPixels.height)) }
        #expect(abs(s.t - 1) < 1e-3 && abs(c.t - 1.001) < 1e-3)   // rebased to the first frame's host time
        return (px(s.x, s.y), px(c.x, c.y))
    }

    private func expectNear(_ a: CGPoint, _ b: CGPoint, _ tol: CGFloat = 0.01, _ note: String = "") {
        #expect(abs(a.x - b.x) <= tol && abs(a.y - b.y) <= tol, "\(a) vs \(b) \(note)")
    }

    @Test func retinaDisplay() throws {
        let display = CGRect(x: 0, y: 0, width: 1440, height: 900)
        let plan = try ScreenStreamPlan.make(contentPoints: display.size, scale: 2, area: nil, isWindow: false, systemAudio: false)
        #expect(plan.pixelSize == PixelSize(width: 2880, height: 1800))
        let p = CGPoint(x: 360, y: 675)   // a quarter across, three quarters down (y grows downward)
        let r = try logged(at: p, source: .display(id: 1), geometry: MovingGeometry(display), videoPixels: plan.pixelSize)
        expectNear(r.sample, CGPoint(x: 720, y: 1350))
        expectNear(r.click, r.sample)
    }

    @Test func secondaryDisplayAboveAndRightOfPrimary() throws {
        // Global top-left space: a 1920x1080 display to the right whose top edge is 200 pt above the primary's.
        let display = CGRect(x: 1440, y: -200, width: 1920, height: 1080)
        let plan = try ScreenStreamPlan.make(contentPoints: display.size, scale: 1, area: nil, isWindow: false, systemAudio: false)
        let p = CGPoint(x: 1440 + 100, y: -200 + 540)
        let r = try logged(at: p, source: .display(id: 2), geometry: MovingGeometry(display), videoPixels: plan.pixelSize)
        expectNear(r.sample, CGPoint(x: 100, y: 540))
    }

    @Test func areaCrop() throws {
        let display = CGRect(x: 1440, y: 0, width: 1920, height: 1080)
        let area = CGRect(x: 200, y: 100, width: 400, height: 300)   // display points, origin top-left
        let plan = try ScreenStreamPlan.make(contentPoints: display.size, scale: 2, area: area, isWindow: false, systemAudio: false)
        #expect(plan.pixelSize == PixelSize(width: 800, height: 600))
        #expect(plan.sourceRect == area)
        // Resolved the same way the logger does: the area offset by its display's origin.
        let global = CoordinateMath.resolve(.area(displayID: 2, rect: area), displayBounds: { _ in display }, windowBounds: { _ in nil })!
        let p = CGPoint(x: global.minX + 50, y: global.minY + 60)
        let r = try logged(at: p, source: .area(displayID: 2, rect: area), geometry: MovingGeometry(global), videoPixels: plan.pixelSize)
        expectNear(r.sample, CGPoint(x: 100, y: 120))
        // Outside the area the contract allows values beyond 0...1 (the cursor leaves the video).
        let out = try logged(at: CGPoint(x: global.minX - 40, y: global.minY), source: .area(displayID: 2, rect: area), geometry: MovingGeometry(global), videoPixels: plan.pixelSize)
        #expect(out.sample.x < 0)
    }

    @Test func windowThatMoves() throws {
        let start = CGRect(x: 100, y: 80, width: 800, height: 600)
        let geometry = MovingGeometry(start)
        let plan = try ScreenStreamPlan.make(contentPoints: start.size, scale: 2, area: nil, isWindow: true, systemAudio: false)
        #expect(plan.pixelSize == PixelSize(width: 1600, height: 1200))
        let source = CaptureSource.window(id: 9)

        // The cursor stays still on screen while the window is dragged 150 pt right and 40 pt down underneath it.
        let now = OSAllocatedUnfairLock(initialState: 10.0)
        let still = CGPoint(x: 500, y: 380)
        let recorder = CursorRecorder(
            source: source, geometry: geometry, schedule: SamplingSchedule(rate: 120), position: { still }, clock: { now.withLock { $0 } })
        recorder.tick()                                            // window at its start position
        geometry.move(to: start.offsetBy(dx: 150, dy: 40))
        recorder.refreshRect()                                     // the logger's 30 Hz geometry refresh
        now.withLock { $0 = 11 }
        recorder.tick()
        let track = recorder.finish(recordingStartHostTime: 9.5, sampleRate: 120)
        func px(_ s: CursorSample) -> CGPoint { CGPoint(x: s.x * 1600, y: s.y * 1200) }
        expectNear(px(track.samples[0]), CGPoint(x: (500 - 100) * 2, y: (380 - 80) * 2), 0.01, "before the move")
        expectNear(px(track.samples[1]), CGPoint(x: (500 - 250) * 2, y: (380 - 120) * 2), 0.01, "after the move")
    }

    @Test func appKitPointsFlipToTopLeft() {
        // AppKit (bottom-left origin, primary 900 pt tall): a point 100 pt above the bottom is 800 pt from the top.
        let p = CoordinateMath.appKitToTopLeft(CGPoint(x: 10, y: 100), primaryDisplayHeight: 900)
        #expect(p == CGPoint(x: 10, y: 800))
        #expect(CoordinateMath.normalize(p, in: CGRect(x: 0, y: 0, width: 1440, height: 900))!.y == 800.0 / 900)
    }
}
