import CoreGraphics
import Foundation
import NSSCore
import os

/// The logging core, free of timers and AppKit so tests can drive it with a fake clock.
/// `tick()` is called from one high-priority queue, `click(...)` from the main thread; state is lock-protected.
final class CursorRecorder: Sendable {
    typealias PositionProvider = @Sendable () -> CGPoint?
    typealias Clock = @Sendable () -> Double

    private struct State {
        var samples: [CursorSample] = []
        var clicks: [ClickEvent] = []
        var rect: CGRect?
    }

    private let source: CaptureSource
    private let geometry: any ScreenGeometryProvider
    private let position: PositionProvider
    private let clock: Clock
    private let state: OSAllocatedUnfairLock<State>

    /// - Parameter position: current cursor location in global top-left points.
    /// - Parameter clock: host time in seconds (`CACurrentMediaTime`).
    init(
        source: CaptureSource,
        geometry: any ScreenGeometryProvider,
        schedule: SamplingSchedule,
        position: @escaping PositionProvider,
        clock: @escaping Clock
    ) {
        self.source = source
        self.geometry = geometry
        self.position = position
        self.clock = clock
        var initial = State()
        initial.samples.reserveCapacity(Int(schedule.rate) * 60 * 10)
        initial.rect = geometry.globalRect(for: source)
        self.state = OSAllocatedUnfairLock(initialState: initial)
    }

    /// How often the source frame should be re-read, on its own queue (never the sampling queue):
    /// a window can move, so about 30 times a second; displays and areas only change on reconfiguration.
    var refreshInterval: Double { geometry.isDynamic(source) ? 1.0 / 30 : 1 }

    /// Re-reads the source frame (may call CoreGraphics window lookups). Keeps the last frame when the
    /// source cannot be resolved, for example a window that just closed.
    func refreshRect() {
        if let r = geometry.globalRect(for: source) { state.withLock { $0.rect = r } }
    }

    /// Takes one sample from the cached frame. Does no system geometry lookups and no allocation except
    /// the occasional amortized array growth.
    func tick() {
        let now = clock()
        guard let p = position() else { return }
        state.withLock { s in
            guard let rect = s.rect, let n = CoordinateMath.normalize(p, in: rect) else { return }
            s.samples.append(CursorSample(t: now, x: n.x, y: n.y))
        }
    }

    /// Records a click at a global top-left point and host time.
    func click(at p: CGPoint, hostTime: Double, button: MouseButton, kind: ClickKind) {
        state.withLock { s in
            guard let rect = s.rect, let n = CoordinateMath.normalize(p, in: rect) else { return }
            s.clicks.append(ClickEvent(t: hostTime, x: n.x, y: n.y, button: button, kind: kind))
        }
    }

    func sampleCount() -> Int { state.withLock { $0.samples.count } }

    /// Rebases to the recording origin and rounds for a compact `cursor.json`.
    /// The last sample at or before the origin is kept and clamped to t = 0, so the position at the
    /// first frame is known. Earlier samples and clicks before the origin are dropped.
    func finish(recordingStartHostTime origin: Double, sampleRate: Double) -> CursorTrack {
        let (samples, clicks) = state.withLock { ($0.samples, $0.clicks) }
        var out: [CursorSample] = []
        out.reserveCapacity(samples.count)
        var carry: CursorSample?
        for s in samples {
            if s.t - origin <= 0 {
                carry = s
            } else {
                if let c = carry {
                    out.append(Self.round(CursorSample(t: 0, x: c.x, y: c.y)))
                    carry = nil
                }
                out.append(Self.round(CursorSample(t: s.t - origin, x: s.x, y: s.y)))
            }
        }
        if out.isEmpty, let c = carry { out.append(Self.round(CursorSample(t: 0, x: c.x, y: c.y))) }

        let rebased = clicks
            .filter { $0.t >= origin }
            .map { c -> ClickEvent in
                var r = c
                r.t = Self.r(c.t - origin, 1e4)
                r.x = Self.r(c.x, 1e5)
                r.y = Self.r(c.y, 1e5)
                return r
            }
            .sorted { $0.t < $1.t }
        return CursorTrack(samples: out, clicks: rebased, sampleRate: sampleRate)
    }

    private static func r(_ v: Double, _ scale: Double) -> Double { (v * scale).rounded() / scale }
    private static func round(_ s: CursorSample) -> CursorSample {
        CursorSample(t: r(s.t, 1e4), x: r(s.x, 1e5), y: r(s.y, 1e5))
    }
}
