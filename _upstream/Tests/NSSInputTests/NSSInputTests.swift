import CoreGraphics
import Foundation
import Testing
import NSSCore
import os
@testable import NSSInput

/// Fixed-rect geometry with a mutable window frame.
final class FakeGeometry: ScreenGeometryProvider, @unchecked Sendable {
    let rect = OSAllocatedUnfairLock<CGRect?>(initialState: nil)
    let primaryDisplayHeight: Double
    let dynamic: Bool
    init(rect: CGRect?, primaryHeight: Double = 1000, dynamic: Bool = false) {
        self.rect.withLock { $0 = rect }
        primaryDisplayHeight = primaryHeight
        self.dynamic = dynamic
    }
    func globalRect(for source: CaptureSource) -> CGRect? { rect.withLock { $0 } }
    func isDynamic(_ source: CaptureSource) -> Bool { dynamic }
}

@Suite struct NSSInputTests {
    @Test func noopLoggerReturnsEmptyTrack() async throws {
        let l = NoopInputLogger()
        try await l.start(source: .display(id: 1))
        #expect(await l.stop(recordingStartHostTime: 0) == .empty)
    }

    // MARK: Coordinate math

    @Test func appKitFlipUsesPrimaryHeight() {
        let p = CoordinateMath.appKitToTopLeft(CGPoint(x: 10, y: 100), primaryDisplayHeight: 900)
        #expect(p == CGPoint(x: 10, y: 800))
        // A display above the primary has y > primaryHeight in AppKit and negative y in top-left space.
        let above = CoordinateMath.appKitToTopLeft(CGPoint(x: 5, y: 1000), primaryDisplayHeight: 900)
        #expect(above.y == -100)
    }

    @Test func normalizeDisplayAndSecondaryDisplayToTheRight() {
        let primary = CGRect(x: 0, y: 0, width: 1440, height: 900)
        #expect(CoordinateMath.normalize(CGPoint(x: 720, y: 450), in: primary) == NormalizedPoint(x: 0.5, y: 0.5))
        #expect(CoordinateMath.normalize(.zero, in: primary) == .zero)
        // Secondary display to the right, shifted up: origin (1440, -200).
        let secondary = CGRect(x: 1440, y: -200, width: 1920, height: 1080)
        let n = CoordinateMath.normalize(CGPoint(x: 1440 + 960, y: -200 + 540), in: secondary)
        #expect(n == NormalizedPoint(x: 0.5, y: 0.5))
    }

    @Test func normalizeIsNotClampedOutsideTheSource() {
        let window = CGRect(x: 100, y: 100, width: 200, height: 100)
        let n = CoordinateMath.normalize(CGPoint(x: 50, y: 250), in: window)
        #expect(n == NormalizedPoint(x: -0.25, y: 1.5))
        #expect(CoordinateMath.normalize(.zero, in: .zero) == nil)
    }

    @Test func resolveAreaOffsetsByDisplayOrigin() {
        let displays: [UInt32: CGRect] = [2: CGRect(x: 1440, y: -200, width: 1920, height: 1080)]
        let r = CoordinateMath.resolve(
            .area(displayID: 2, rect: CGRect(x: 100, y: 50, width: 400, height: 300)),
            displayBounds: { displays[$0] }, windowBounds: { _ in nil })
        #expect(r == CGRect(x: 1540, y: -150, width: 400, height: 300))
        #expect(CoordinateMath.resolve(.display(id: 9), displayBounds: { displays[$0] }, windowBounds: { _ in nil }) == nil)
        #expect(CoordinateMath.resolve(.window(id: 5), displayBounds: { _ in nil }, windowBounds: { _ in CGRect(x: 1, y: 2, width: 3, height: 4) })
            == CGRect(x: 1, y: 2, width: 3, height: 4))
    }

    @Test func appKitClickLandsOnTheRightNormalizedSpot() {
        // Primary 1440x900. Click at AppKit (720, 675) is 225 pt below the top: y = 0.25.
        let top = CoordinateMath.appKitToTopLeft(CGPoint(x: 720, y: 675), primaryDisplayHeight: 900)
        let n = CoordinateMath.normalize(top, in: CGRect(x: 0, y: 0, width: 1440, height: 900))
        #expect(n == NormalizedPoint(x: 0.5, y: 0.25))
    }

    // MARK: Sampling-rate logic

    @Test func scheduleIntervalsAndClamping() {
        #expect(SamplingSchedule().intervalNanoseconds == 8_333_333)
        #expect(abs(SamplingSchedule(rate: 120).interval - 1.0 / 120) < 1e-12)
        #expect(SamplingSchedule(rate: 0).rate == 1)
        #expect(SamplingSchedule(rate: 1e6).rate == 1000)
    }

    @Test func statsDetectRateJitterAndGaps() {
        let steady = (0..<121).map { Double($0) / 120 }
        let s = SamplingStats(times: steady)
        #expect(abs(s.meanRate - 120) < 1e-6)
        #expect(s.jitter < 1e-9)
        var gappy = steady
        gappy[60] += 0.003
        let g = SamplingStats(times: gappy)
        #expect(g.maxGap > 1.0 / 120)
        #expect(g.jitter > 0)
        #expect(SamplingStats(times: [1]).meanRate == 0)
    }

    // MARK: Recorder

    private func makeRecorder(
        rect: CGRect = CGRect(x: 0, y: 0, width: 1000, height: 500),
        geometry: FakeGeometry? = nil,
        position: @escaping @Sendable () -> CGPoint?,
        clock: @escaping @Sendable () -> Double
    ) -> CursorRecorder {
        CursorRecorder(
            source: .display(id: 1),
            geometry: geometry ?? FakeGeometry(rect: rect),
            schedule: SamplingSchedule(rate: 120),
            position: position, clock: clock)
    }

    @Test func recorderRebasesRoundsAndKeepsPositionAtOrigin() {
        let time = OSAllocatedUnfairLock(initialState: 100.0)
        let pos = OSAllocatedUnfairLock(initialState: CGPoint(x: 250, y: 125))
        let rec = makeRecorder(position: { pos.withLock { $0 } }, clock: { time.withLock { $0 } })
        // Two ticks before the origin, then three after.
        for (t, x) in [(100.0, 250.0), (100.01, 300.0), (100.02, 500.0), (100.03, 600.0), (100.04, 1000.0)] {
            time.withLock { $0 = t }
            pos.withLock { $0 = CGPoint(x: x, y: 125) }
            rec.tick()
        }
        let track = rec.finish(recordingStartHostTime: 100.015, sampleRate: 120)
        #expect(track.sampleRate == 120)
        #expect(track.samples.map(\.t) == [0, 0.005, 0.015, 0.025])
        // The sample at the origin is the last one at or before it (x = 300 -> 0.3), clamped to t = 0.
        #expect(track.samples.first == CursorSample(t: 0, x: 0.3, y: 0.25))
        #expect(track.samples.last?.x == 1)
    }

    @Test func recorderDropsClicksBeforeOriginAndSortsRest() {
        let rec = makeRecorder(position: { .zero }, clock: { 0 })
        rec.click(at: CGPoint(x: 500, y: 250), hostTime: 9.9, button: .left, kind: .down)
        rec.click(at: CGPoint(x: 500, y: 250), hostTime: 11, button: .right, kind: .up)
        rec.click(at: CGPoint(x: 100, y: 50), hostTime: 10.5, button: .left, kind: .down)
        let t = rec.finish(recordingStartHostTime: 10, sampleRate: 120)
        #expect(t.clicks.count == 2)
        #expect(t.clicks.map(\.t) == [0.5, 1])
        #expect(t.clicks[0] == ClickEvent(t: 0.5, x: 0.1, y: 0.1, button: .left, kind: .down))
        #expect(t.clicks[1].button == .right && t.clicks[1].kind == .up)
    }

    @Test func recorderTracksAMovingWindow() {
        let geo = FakeGeometry(rect: CGRect(x: 0, y: 0, width: 200, height: 100), dynamic: true)
        let time = OSAllocatedUnfairLock(initialState: 1.0)
        let rec = CursorRecorder(
            source: .window(id: 7), geometry: geo, schedule: SamplingSchedule(rate: 120),
            position: { CGPoint(x: 150, y: 50) }, clock: { time.withLock { $0 } })
        rec.tick()
        // Window moves 100 pt right. Ticks use the cached frame until the separate refresh runs.
        geo.rect.withLock { $0 = CGRect(x: 100, y: 0, width: 200, height: 100) }
        time.withLock { $0 = 1 + 1.0 / 120 }
        rec.tick()
        rec.refreshRect()
        time.withLock { $0 = 1 + 2.0 / 120 }
        rec.tick()
        let track = rec.finish(recordingStartHostTime: 0, sampleRate: 120)
        #expect(track.samples.map(\.x) == [0.75, 0.75, 0.25])
        #expect(rec.refreshInterval == 1.0 / 30)
    }

    @Test func recorderKeepsLastRectWhenWindowDisappears() {
        let geo = FakeGeometry(rect: CGRect(x: 0, y: 0, width: 100, height: 100), dynamic: true)
        let rec = CursorRecorder(
            source: .window(id: 7), geometry: geo, schedule: SamplingSchedule(rate: 120),
            position: { CGPoint(x: 50, y: 50) }, clock: { 1 })
        geo.rect.withLock { $0 = nil }
        rec.refreshRect()
        for _ in 0..<10 { rec.tick() }
        #expect(rec.sampleCount() == 10)
    }

    @Test func stopRightAfterStartLeavesNothingRunning() async throws {
        let logger = SystemInputLogger(geometry: FakeGeometry(rect: CGRect(x: 0, y: 0, width: 100, height: 100)), monitorClicks: false)
        try await logger.start(source: .display(id: 1))
        let t = await logger.stop(recordingStartHostTime: 0)
        #expect(!t.samples.isEmpty)
        #expect(await logger.stop(recordingStartHostTime: 0) == .empty)
        // Can start again after a stop.
        try await logger.start(source: .display(id: 1))
        _ = await logger.stop(recordingStartHostTime: 0)
    }

    @Test func recorderSkipsTicksWithoutAPosition() {
        let rec = makeRecorder(position: { nil }, clock: { 1 })
        rec.tick()
        #expect(rec.sampleCount() == 0)
    }

    @Test func tenMinuteTrackStaysCompactAndFast() throws {
        var samples: [CursorSample] = []
        samples.reserveCapacity(72_000)
        for i in 0..<72_000 {
            samples.append(CursorSample(t: Double(i) / 120, x: Double(i % 1000) / 1000, y: 0.5))
        }
        let track = CursorTrack(samples: samples)
        let data = try JSONEncoder().encode(track)
        #expect(data.count < 4_000_000)
        let start = ContinuousClock.now
        let back = try JSONDecoder().decode(CursorTrack.self, from: data)
        #expect(back.samples.count == 72_000)
        // Wall-clock budget: asserted only with NSS_BENCH=1 (loaded machines and CI VMs are unreliable).
        if ProcessInfo.processInfo.environment["NSS_BENCH"] == "1" { #expect(ContinuousClock.now - start < .seconds(5)) }
    }

    // MARK: Live logger (skips without a window server)

    @Test(.enabled(if: CGMainDisplayID() != 0 && CGDisplayBounds(CGMainDisplayID()).width > 0))
    func liveLoggerSamplesNearRate() async throws {
        let logger = SystemInputLogger(monitorClicks: false)
        try await logger.start(source: .display(id: CGMainDisplayID()))
        try await Task.sleep(for: .milliseconds(500))
        let origin = CACurrentMediaTimeShim.now - 0.5
        let track = await logger.stop(recordingStartHostTime: origin)
        let stats = SamplingStats(times: track.samples.map(\.t))
        #expect(track.samples.count > 40)
        #expect(stats.meanRate > 80 && stats.meanRate < 130)
    }
}

import QuartzCore
enum CACurrentMediaTimeShim { static var now: Double { CACurrentMediaTime() } }
