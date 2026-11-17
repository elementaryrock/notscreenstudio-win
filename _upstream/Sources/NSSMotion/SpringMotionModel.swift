import Foundation
import NSSCore

/// The real `MotionModel`: spring camera with auto-zoom, spring-smoothed cursor, idle fade, click
/// ripples, cursor shapes (with short flickers removed) and the press squish.
///
/// The camera's viewport never leaves the source frame; see `SourceViewport` for the exact contract.
///
/// Determinism: everything stateful (springs, dead zone, idle fade) is simulated ONCE at init with
/// a fixed step (`MotionTuning.simulationRate`, 240 Hz) from the trim start, where all springs
/// start settled, to the trim end. `frameState(at:)` then reads that table: a direct index on the
/// uniform grid plus linear interpolation, O(1), no allocation (except the ripple array when a
/// ripple is visible). Same `t` gives the same `FrameState`, in any order, from any thread, so the
/// preview, scrubbing and export match exactly. Times outside the trim range are clamped to it.
public struct SpringMotionModel: MotionModel {
    public let tuning: MotionTuning
    public let cursorStyle: CursorStyle
    public let sourceAspect: Double
    /// Resolved zoom segments (auto + manual), sorted and non-overlapping. Useful for timeline UI.
    public let zoomSegments: [ZoomSegment]
    /// First and last simulated source time (the trim range).
    public let startTime: MediaTime
    public let endTime: MediaTime

    private let track: CursorTrack
    private let rate: Double
    private let frames: [SimFrame]
    private let rawCursor: Bool
    private let rippleTimes: [MediaTime]
    private let ripplePoints: [NormalizedPoint]
    /// Shape changes with flickers shorter than `MotionTuning.minShapeDuration` removed.
    private let shapeTrack: CursorTrack

    /// Compact per-step state: 36 bytes. 10 minutes at 240 Hz is ~5.2 MB.
    struct SimFrame {
        var logScale: Float
        var focusX: Float
        var focusY: Float
        var cursorX: Float
        var cursorY: Float
        var cursorVX: Float
        var cursorVY: Float
        var opacity: Float
        var pressScale: Float
    }

    /// - Parameters:
    ///   - duration: recording duration; if not positive, derived from the last sample/click.
    ///   - sourceAspect: width / height of the recorded source (e.g. `sourcePixelSize.aspect`).
    public init(
        track: CursorTrack,
        timeline: Timeline,
        cursorStyle: CursorStyle,
        duration: MediaTime,
        sourceAspect: Double = 16.0 / 9.0,
        tuning: MotionTuning = .standard
    ) {
        let samples = track.samples.filter { $0.t.isFinite && $0.x.isFinite && $0.y.isFinite }
        let track = CursorTrack(
            samples: samples.isSorted ? samples : samples.sorted { $0.t < $1.t },
            clicks: track.clicks.filter { $0.t.isFinite && $0.x.isFinite && $0.y.isFinite },
            sampleRate: track.sampleRate,
            shapes: track.shapes
        )
        var duration = duration.isFinite ? duration : 0
        if duration <= 0 {
            duration = max(track.samples.last?.t ?? 0, track.clicks.map(\.t).filter(\.isFinite).max() ?? 0)
        }
        // Bound the simulated range: a bogus duration or absolute timestamps must not trap or
        // allocate without limit.
        let maxDuration = tuning.maxDuration.isFinite && tuning.maxDuration > 0 ? tuning.maxDuration : 24 * 60 * 60
        duration = min(max(duration, 0), maxDuration)
        let trimEnd = timeline.trim.resolvedEnd(duration: duration)
        let start = min(max(0, timeline.trim.start.isFinite ? timeline.trim.start : 0), duration)
        let end = max(start, trimEnd.isFinite ? trimEnd : duration)
        var rate = tuning.simulationRate.isFinite && tuning.simulationRate > 0 ? tuning.simulationRate : 240
        let maxSteps = Double(max(tuning.maxSimulationFrames, 2) - 1)
        if (end - start) * rate > maxSteps { rate = maxSteps / (end - start) }

        self.tuning = tuning
        self.cursorStyle = cursorStyle
        self.sourceAspect = sourceAspect
        self.track = track
        self.rate = rate
        self.startTime = start
        self.endTime = end
        self.rawCursor = tuning.cursorSpring(smoothing: cursorStyle.smoothing) == nil
        self.shapeTrack = CursorTrack(shapes: Self.stableShapes(track.shapes, minDuration: tuning.minShapeDuration))

        let segments = ZoomPlanner.segments(track: track, timeline: timeline, duration: duration, sourceAspect: sourceAspect, tuning: tuning)
        self.zoomSegments = segments

        let downs = track.clicks.filter { $0.kind == .down && $0.t.isFinite }.sorted { $0.t < $1.t }
        self.rippleTimes = downs.map(\.t)
        self.ripplePoints = downs.map(\.point)

        self.frames = Self.simulate(track: track, segments: segments, cursorStyle: cursorStyle, start: start, end: end, rate: rate, tuning: tuning)
    }

    /// Factory with a known source aspect (width / height).
    public static func factory(sourceAspect: Double, tuning: MotionTuning = .standard) -> MotionModelFactory {
        { track, timeline, style, duration in
            SpringMotionModel(track: track, timeline: timeline, cursorStyle: style, duration: duration, sourceAspect: sourceAspect, tuning: tuning)
        }
    }

    /// Factory assuming a 16:9 source. Prefer `factory(sourceAspect:)` when the source size is known.
    public static let factory: MotionModelFactory = SpringMotionModel.factory(sourceAspect: 16.0 / 9.0, tuning: .standard)

    // MARK: Lookup

    public func frameState(at t: MediaTime) -> FrameState {
        let tc = clampedTime(t)
        let u = (tc - startTime) * rate
        var i = Int(u)
        if i >= frames.count - 1 { i = frames.count - 2 }
        if i < 0 { i = 0 }
        let f = Float(min(max(u - Double(i), 0), 1))
        let a = frames[i], b = frames[i + 1]
        @inline(__always) func mix(_ x: Float, _ y: Float) -> Double { Double(x + (y - x) * f) }

        // Camera: interpolated spring state, then clamp the viewport inside the source. This clamp runs
        // after the springs, so overshoot can never move the viewport outside (see `SourceViewport`).
        let camera = CameraState.clampedToSource(
            scale: exp(mix(a.logScale, b.logScale)),
            center: NormalizedPoint(x: mix(a.focusX, b.focusX), y: mix(a.focusY, b.focusY))
        )

        // Cursor.
        let hasCursor = !track.samples.isEmpty
        var position = NormalizedPoint(x: mix(a.cursorX, b.cursorX), y: mix(a.cursorY, b.cursorY))
        if rawCursor, let raw = track.rawPosition(at: tc) { position = raw }
        let velocity = NormalizedPoint(x: mix(a.cursorVX, b.cursorVX), y: mix(a.cursorVY, b.cursorVY))
        let opacity = hasCursor ? min(max(mix(a.opacity, b.opacity), 0), 1) : 0
        let cursor = CursorState(
            position: hasCursor ? position : .center,
            velocity: hasCursor ? velocity : .zero,
            visible: hasCursor && opacity > 0,
            ripples: ripples(at: tc),
            opacity: opacity,
            shape: shapeTrack.shape(at: tc),
            pressScale: hasCursor ? mix(a.pressScale, b.pressScale) : 1
        )
        return FrameState(time: t, camera: camera, cursor: cursor)
    }

    /// `t` clamped to the simulated (trim) range, like every part of the frame state.
    func clampedTime(_ t: MediaTime) -> MediaTime {
        t.isFinite ? min(max(t, startTime), endTime) : startTime
    }

    /// Number of simulated frames (bounded by `MotionTuning.maxSimulationFrames`).
    var simulatedFrameCount: Int { frames.count }

    /// Ripples for every mouse-down in (t - rippleDuration, t]. `t` is already clamped.
    func ripples(at t: MediaTime) -> [RippleState] {
        guard cursorStyle.clickRipples, !rippleTimes.isEmpty, t.isFinite else { return [] }
        let d = max(tuning.rippleDuration, 1e-6)
        // first click with time > t - d
        var lo = 0, hi = rippleTimes.count
        while lo < hi {
            let mid = (lo + hi) / 2
            if rippleTimes[mid] <= t - d { lo = mid + 1 } else { hi = mid }
        }
        guard lo < rippleTimes.count, rippleTimes[lo] <= t else { return [] }
        var out: [RippleState] = []
        var k = lo
        while k < rippleTimes.count, rippleTimes[k] <= t {
            out.append(RippleState(position: ripplePoints[k], progress: min(max((t - rippleTimes[k]) / d, 0), 1)))
            k += 1
        }
        return out
    }

    /// Removes shape changes that are replaced within `minDuration` (flicker): the shape before a
    /// flicker simply continues. Then merges repeats. Changes that are kept apply at their own time.
    static func stableShapes(_ changes: [CursorShapeChange], minDuration: Double) -> [CursorShapeChange] {
        let sorted = changes.enumerated()
            .filter { $0.element.t.isFinite }
            .sorted { $0.element.t != $1.element.t ? $0.element.t < $1.element.t : $0.offset < $1.offset }
            .map(\.element)
        let minDuration = minDuration.isFinite ? max(minDuration, 0) : 0
        var out: [CursorShapeChange] = []
        out.reserveCapacity(sorted.count)
        for (i, c) in sorted.enumerated() {
            let next = i + 1 < sorted.count ? sorted[i + 1].t : .infinity
            if next - c.t < minDuration { continue }
            // Before the first change the arrow is implied, so a leading arrow is redundant too.
            if (out.last?.shape ?? .arrow) == c.shape { continue }
            out.append(c)
        }
        return out
    }

    // MARK: Simulation

    private static func simulate(
        track: CursorTrack,
        segments: [ZoomSegment],
        cursorStyle: CursorStyle,
        start: MediaTime,
        end: MediaTime,
        rate: Double,
        tuning: MotionTuning
    ) -> [SimFrame] {
        let dt = 1 / rate
        // `rate` was already lowered so the span fits; min() again before Int() so it cannot trap.
        let maxSteps = max(tuning.maxSimulationFrames, 2) - 1
        let span = ((end - start) * rate).rounded(.up)
        let steps = max(1, Int(min(span, Double(maxSteps))))
        var frames: [SimFrame] = []
        frames.reserveCapacity(steps + 1)

        // Zoom springs: the tuning's spring between blocks with no ease set, otherwise one spring per
        // segment edge (ease-in from `start`, ease-out from `end`) with the segment's ease as response.
        let zoomStep = tuning.zoomSpring.stepper(dt: dt)
        var zoomSteps: [SpringStepper] = []
        zoomSteps.reserveCapacity(segments.count * 2)
        for seg in segments {
            var spring = tuning.zoomSpring
            spring.response = seg.easeIn
            zoomSteps.append(spring.stepper(dt: dt))
            spring.response = seg.easeOut
            zoomSteps.append(spring.stepper(dt: dt))
        }
        let panStep = tuning.panSpring.stepper(dt: dt)
        let cursorSpring = tuning.cursorSpring(smoothing: cursorStyle.smoothing)
        let cursorStep = cursorSpring?.stepper(dt: dt)

        var cursor = RawCursor(samples: track.samples)
        var planner = CameraPlanner(segments: segments, tuning: tuning)
        var activity = ActivityTracker(track: track, tuning: tuning, hideWhenIdle: cursorStyle.hideWhenIdle)
        var presses = PressTracker(clicks: track.clicks, minHold: tuning.pressMinHold)
        let pressDownStep = tuning.pressDownSpring.stepper(dt: dt)
        let pressReleaseStep = tuning.pressReleaseSpring.stepper(dt: dt)
        let pressedScale = tuning.pressScale.isFinite ? min(max(tuning.pressScale, 0.1), 1) : 1

        // Settled initial state at the trim start.
        let raw0 = cursor.position(at: start)
        let target0 = planner.target(at: start, cursor: raw0, currentScale: 1)
        var logScale = log(target0.scale), logScaleV = 0.0
        var fx = target0.focus.x, fxV = 0.0
        var fy = target0.focus.y, fyV = 0.0
        var cx = raw0?.x ?? 0.5, cxV = 0.0
        var cy = raw0?.y ?? 0.5, cyV = 0.0
        var opacity = activity.settledOpacity(at: start)
        var press = presses.isPressed(at: start) ? pressedScale : 1, pressV = 0.0

        func record() {
            frames.append(SimFrame(
                logScale: Float(logScale), focusX: Float(fx), focusY: Float(fy),
                cursorX: Float(cx), cursorY: Float(cy), cursorVX: Float(cxV), cursorVY: Float(cyV),
                opacity: Float(opacity), pressScale: Float(press)
            ))
        }
        // Raw-cursor velocity: slope of the raw track (finite difference over one step).
        func rawVelocity(_ p: NormalizedPoint?, _ prev: NormalizedPoint?) -> (Double, Double) {
            guard let p, let prev else { return (0, 0) }
            return ((p.x - prev.x) * rate, (p.y - prev.y) * rate)
        }

        record()
        var prevRaw = raw0
        for k in 1...steps {
            let t = start + Double(k) * dt
            let raw = cursor.position(at: t)
            let target = planner.target(at: t, cursor: raw, currentScale: exp(logScale))
            let zi = planner.zoomSpringIndex
            (zi >= 0 && zi < zoomSteps.count ? zoomSteps[zi] : zoomStep).step(&logScale, &logScaleV, target: log(target.scale))
            panStep.step(&fx, &fxV, target: target.focus.x)
            panStep.step(&fy, &fyV, target: target.focus.y)
            if let raw {
                if let cursorStep {
                    cursorStep.step(&cx, &cxV, target: raw.x)
                    cursorStep.step(&cy, &cyV, target: raw.y)
                } else {
                    (cxV, cyV) = rawVelocity(raw, prevRaw)
                    cx = raw.x
                    cy = raw.y
                }
            }
            opacity = activity.opacity(after: opacity, at: t, dt: dt)
            if presses.isPressed(at: t) {
                pressDownStep.step(&press, &pressV, target: pressedScale)
            } else {
                pressReleaseStep.step(&press, &pressV, target: 1)
            }
            prevRaw = raw
            record()
        }
        return frames
    }
}

// MARK: - Simulation helpers (sequential cursors over sorted input; used only during the build)

/// Linear interpolation of the raw cursor with a forward-only index (O(1) amortized per step).
struct RawCursor {
    let samples: [CursorSample]
    var i = 0
    init(samples: [CursorSample]) { self.samples = samples }

    mutating func position(at t: MediaTime) -> NormalizedPoint? {
        guard let first = samples.first, let last = samples.last else { return nil }
        if t <= first.t { return first.point }
        if t >= last.t { return last.point }
        while i + 1 < samples.count, samples[i + 1].t <= t { i += 1 }
        let a = samples[i], b = samples[i + 1]
        let span = b.t - a.t
        let f = span > 0 ? (t - a.t) / span : 0
        return NormalizedPoint(x: a.x + (b.x - a.x) * f, y: a.y + (b.y - a.y) * f)
    }
}

/// Camera target (scale + focus) over time, including the dead-zone follow state.
struct CameraPlanner {
    let segments: [ZoomSegment]
    let tuning: MotionTuning
    var index = 0
    var active: Int? = nil
    var focus = NormalizedPoint.center
    /// Which zoom spring the last `target` call asks for: `2 * i` = ease-in of segment i, `2 * i + 1` =
    /// ease-out of segment i, -1 = the default spring (before the first segment).
    private(set) var zoomSpringIndex = -1

    init(segments: [ZoomSegment], tuning: MotionTuning) {
        self.segments = segments
        self.tuning = tuning
    }

    static func clampFocus(_ p: NormalizedPoint, scale: Double) -> NormalizedPoint {
        CameraState.clampedToSource(scale: scale, center: p).center
    }

    /// - Parameter currentScale: the camera's current (spring) scale, used to delay re-aiming
    ///   between segments until the zoom-out has finished.
    mutating func target(at t: MediaTime, cursor: NormalizedPoint?, currentScale: Double) -> (scale: Double, focus: NormalizedPoint) {
        while index < segments.count, segments[index].end <= t { index += 1 }
        zoomSpringIndex = index > 0 ? 2 * (index - 1) + 1 : -1
        guard index < segments.count else {
            active = nil
            return (1, focus)
        }
        let seg = segments[index]
        guard seg.start <= t else {
            // Between segments: zoom out around the current focus. Once (nearly) fully zoomed out,
            // quietly pre-aim at the next segment so its zoom-in heads straight for it; this is
            // invisible at scale ~1, where the center is clamped to the middle.
            active = nil
            if currentScale <= tuning.preAimMaxScale {
                focus = Self.clampFocus(seg.initialFocus, scale: seg.scale)
            }
            return (1, focus)
        }
        zoomSpringIndex = 2 * index
        if active != index {
            active = index
            focus = Self.clampFocus(seg.initialFocus, scale: seg.scale)
        }
        if let from = seg.followFrom, t >= from, let p = cursor {
            // Dead zone: pan only when the cursor leaves the central part of the viewport.
            let dz = max(0, tuning.deadZone) * 0.5 / seg.scale
            var f = focus
            if p.x > f.x + dz { f.x = p.x - dz } else if p.x < f.x - dz { f.x = p.x + dz }
            if p.y > f.y + dz { f.y = p.y - dz } else if p.y < f.y - dz { f.y = p.y + dz }
            focus = Self.clampFocus(f, scale: seg.scale)
        }
        return (seg.scale, focus)
    }
}

/// When a mouse button is held. Each mouse-down is paired with the next mouse-up of the same button and
/// held for at least `minHold` (a down without an up is held for `minHold`). Overlapping holds (several
/// buttons) are merged. Queried with non-decreasing times.
struct PressTracker {
    private(set) var intervals: [(start: MediaTime, end: MediaTime)] = []
    private var i = 0

    init(clicks: [ClickEvent], minHold: Double) {
        let hold = minHold.isFinite ? max(minHold, 0) : 0
        let events = clicks.enumerated()
            .filter { $0.element.t.isFinite }
            .sorted { $0.element.t != $1.element.t ? $0.element.t < $1.element.t : $0.offset < $1.offset }
            .map(\.element)
        var raw: [(start: MediaTime, end: MediaTime)] = []
        var open: [MouseButton: Int] = [:]
        for e in events {
            switch e.kind {
            case .down:
                open[e.button] = raw.count
                raw.append((e.t, e.t + hold))
            case .up:
                if let k = open.removeValue(forKey: e.button) { raw[k].end = max(raw[k].end, e.t) }
            }
        }
        raw.sort { $0.start < $1.start }
        for r in raw {
            if let last = intervals.last, r.start <= last.end {
                intervals[intervals.count - 1].end = max(last.end, r.end)
            } else {
                intervals.append(r)
            }
        }
    }

    mutating func isPressed(at t: MediaTime) -> Bool {
        while i < intervals.count, intervals[i].end <= t { i += 1 }
        return i < intervals.count && intervals[i].start <= t
    }
}

/// Idle detection and the cursor fade (only when `hideWhenIdle`).
struct ActivityTracker {
    let samples: [CursorSample]
    let clicks: [MediaTime]
    let tuning: MotionTuning
    let enabled: Bool
    var si = 0
    var ci = 0
    var lastActivity: MediaTime = 0

    init(track: CursorTrack, tuning: MotionTuning, hideWhenIdle: Bool) {
        samples = track.samples
        clicks = track.clicks.filter { $0.t.isFinite }.map(\.t).sorted()
        self.tuning = tuning
        enabled = hideWhenIdle
    }

    /// Advance to `t` and return the time of the last movement or click at or before `t`.
    mutating func advance(to t: MediaTime) -> MediaTime {
        while si < samples.count, samples[si].t <= t {
            if si > 0 {
                let a = samples[si - 1], b = samples[si]
                if abs(b.x - a.x) + abs(b.y - a.y) > tuning.activityEpsilon { lastActivity = max(lastActivity, b.t) }
            }
            si += 1
        }
        while ci < clicks.count, clicks[ci] <= t {
            lastActivity = max(lastActivity, clicks[ci])
            ci += 1
        }
        return lastActivity
    }

    private func targetOpacity(at t: MediaTime) -> Double {
        t - lastActivity >= tuning.idleHideDelay ? 0 : 1
    }

    mutating func settledOpacity(at t: MediaTime) -> Double {
        guard enabled else { return 1 }
        _ = advance(to: t)
        return targetOpacity(at: t)
    }

    mutating func opacity(after current: Double, at t: MediaTime, dt: Double) -> Double {
        guard enabled else { return 1 }
        _ = advance(to: t)
        let target = targetOpacity(at: t)
        if target > current {
            return min(target, current + dt / max(tuning.idleFadeInDuration, 1e-6))
        } else {
            return max(target, current - dt / max(tuning.idleFadeOutDuration, 1e-6))
        }
    }
}

extension Array where Element == CursorSample {
    var isSorted: Bool {
        var prev = -Double.infinity
        for s in self {
            if s.t < prev { return false }
            prev = s.t
        }
        return true
    }
}
