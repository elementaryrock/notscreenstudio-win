import Foundation
import NSSCore

/// One resolved zoom interval: an auto-zoom cluster or a manual `ZoomBlock`, after overlap resolution.
/// Segments returned by `ZoomPlanner.segments` are sorted and never overlap.
public struct ZoomSegment: Sendable, Equatable {
    public enum Origin: Sendable, Equatable {
        case auto
        case manual(UUID)
    }

    public var start: MediaTime
    public var end: MediaTime
    /// Target magnification (already clamped to 1...maxZoomScale).
    public var scale: Double
    /// Focus at the start of the segment (the first click for auto segments).
    public var initialFocus: NormalizedPoint
    /// When non-nil, the focus follows the cursor (with a dead zone) from this time on.
    /// nil = fixed focus (`ZoomFocus.manual`).
    public var followFrom: MediaTime?
    public var origin: Origin
    /// Zoom spring response (seconds) while easing in, from `start`.
    public var easeIn: Double
    /// Zoom spring response (seconds) while easing back out to 1x, from `end`.
    public var easeOut: Double

    public init(
        start: MediaTime, end: MediaTime, scale: Double, initialFocus: NormalizedPoint, followFrom: MediaTime?, origin: Origin,
        easeIn: Double = ZoomBlock.defaultEase, easeOut: Double = ZoomBlock.defaultEase
    ) {
        self.start = start
        self.end = end
        self.scale = scale
        self.initialFocus = initialFocus
        self.followFrom = followFrom
        self.origin = origin
        self.easeIn = easeIn
        self.easeOut = easeOut
    }

    public func contains(_ t: MediaTime) -> Bool { t >= start && t < end }
}

/// Turns clicks and manual `ZoomBlock`s into non-overlapping `ZoomSegment`s.
public enum ZoomPlanner {
    /// - Parameters:
    ///   - sourceAspect: width / height of the recorded source, so click distances are isotropic.
    public static func segments(
        track: CursorTrack,
        timeline: Timeline,
        duration: MediaTime,
        sourceAspect: Double,
        tuning: MotionTuning = .standard
    ) -> [ZoomSegment] {
        let manual = manualSegments(track: track, blocks: timeline.zoomBlocks, duration: duration, tuning: tuning)
        guard timeline.autoZoomEnabled else { return manual }
        let auto = autoSegments(track: track, duration: duration, sourceAspect: sourceAspect, tuning: tuning)
        var result = manual
        for seg in auto {
            result.append(contentsOf: subtract(manual, from: seg, track: track, tuning: tuning))
        }
        result.sort { $0.start < $1.start }
        return result
    }

    // MARK: Auto

    /// A mouse-down and the time its button was released (== `t` when no matching mouse-up).
    struct Press {
        var down: ClickEvent
        var release: MediaTime
    }

    /// Pairs each mouse-down with the next mouse-up of the same button, so drags count as one press.
    static func presses(_ clicks: [ClickEvent], duration: MediaTime) -> [Press] {
        let events = clicks
            .filter { $0.t.isFinite && $0.t >= 0 && $0.t <= duration }
            .enumerated()
            .sorted { $0.element.t != $1.element.t ? $0.element.t < $1.element.t : $0.offset < $1.offset }
            .map(\.element)
        var out: [Press] = []
        var open: [MouseButton: Int] = [:]
        for e in events {
            switch e.kind {
            case .down:
                open[e.button] = out.count
                out.append(Press(down: e, release: e.t))
            case .up:
                if let i = open.removeValue(forKey: e.button) { out[i].release = e.t }
            }
        }
        return out
    }

    static func autoSegments(track: CursorTrack, duration: MediaTime, sourceAspect: Double, tuning: MotionTuning) -> [ZoomSegment] {
        let presses = presses(track.clicks, duration: duration)
        guard !presses.isEmpty else { return [] }
        let aspect = sourceAspect.isFinite && sourceAspect > 0 ? sourceAspect : 16.0 / 9.0

        // 1. Cluster presses in time (gap measured from the previous release) and space.
        struct Cluster {
            var first: ClickEvent
            var last: ClickEvent
            var release: MediaTime
            /// The next cluster starts because of distance, not time.
            var spatialBreakAfter = false
        }
        var clusters: [Cluster] = []
        for p in presses {
            let c = p.down
            if var cur = clusters.last {
                let dt = c.t - cur.release
                let dx = (c.x - cur.last.x) * aspect, dy = c.y - cur.last.y
                let near = (dx * dx + dy * dy).squareRoot() <= tuning.clusterMaxDistance
                if dt <= tuning.clusterMaxGap && (near || dt <= tuning.clusterPanGap) {
                    cur.last = c
                    cur.release = max(cur.release, p.release)
                    clusters[clusters.count - 1] = cur
                    continue
                }
                if dt <= tuning.clusterMaxGap {
                    clusters[clusters.count - 1].spatialBreakAfter = true
                }
            }
            clusters.append(Cluster(first: c, last: c, release: p.release))
        }

        // 2. Each cluster -> interval: lead-in before the first click, hold after the last release
        //    while the cursor keeps moving.
        var segs: [(seg: ZoomSegment, release: MediaTime, spatialBreakAfter: Bool)] = clusters.map { cl in
            (ZoomSegment(
                start: cl.first.t - tuning.autoZoomLeadIn,
                end: holdEnd(after: cl.release, track: track, tuning: tuning),
                scale: clampScale(tuning.autoZoomScale, tuning),
                initialFocus: cl.first.point.clamped,
                followFrom: cl.first.t,
                origin: .auto,
                easeIn: tuning.zoomSpring.response,
                easeOut: tuning.zoomSpring.response
            ), cl.release, cl.spatialBreakAfter)
        }

        // 3. Spatial splits: shorten the earlier hold to leave room to zoom out and back in. If there
        //    is no room, the segments merge below and the camera pans instead.
        for i in segs.indices.dropLast() where segs[i].spatialBreakAfter {
            let latest = segs[i + 1].seg.start - tuning.minZoomOutGap
            if latest >= segs[i].release + tuning.autoZoomMinHold {
                segs[i].seg.end = min(segs[i].seg.end, latest)
            }
        }

        // 4. Merge segments whose gap is too short to zoom out and back in.
        var merged: [ZoomSegment] = []
        for (s, _, _) in segs {
            if var prev = merged.last, s.start - prev.end < tuning.minZoomOutGap {
                prev.end = max(prev.end, s.end)
                merged[merged.count - 1] = prev
            } else {
                merged.append(s)
            }
        }

        // 5. Clamp to the recording.
        return merged.compactMap { s in
            var s = s
            s.start = max(0, s.start)
            s.end = min(duration, s.end)
            return s.end - s.start >= tuning.minSegmentDuration ? s : nil
        }
    }

    /// End of the hold after the last release at `t0`: `autoZoomHold` after the cursor last moved,
    /// capped at `t0 + autoZoomMaxHold`.
    static func holdEnd(after t0: MediaTime, track: CursorTrack, tuning: MotionTuning) -> MediaTime {
        let cap = t0 + max(tuning.autoZoomMaxHold, tuning.autoZoomHold)
        var lastMove = t0
        let samples = track.samples
        var i = firstIndex(in: samples, after: t0)
        if i > 0 { i -= 1 }
        var prev: CursorSample? = nil
        while i < samples.count {
            let s = samples[i]
            if s.t > cap || s.t - lastMove > tuning.autoZoomHold { break }
            if let p = prev, s.t > t0, abs(s.x - p.x) + abs(s.y - p.y) > tuning.activityEpsilon {
                lastMove = s.t
            }
            prev = s
            i += 1
        }
        return min(lastMove + tuning.autoZoomHold, cap)
    }

    // MARK: Manual

    static func manualSegments(track: CursorTrack, blocks: [ZoomBlock], duration: MediaTime, tuning: MotionTuning) -> [ZoomSegment] {
        let sorted = blocks
            .filter { $0.start.isFinite && $0.end.isFinite && $0.end > $0.start }
            .sorted { $0.start < $1.start }
        var segs: [ZoomSegment] = []
        for b in sorted {
            let start = max(0, b.start)
            let end = duration > 0 ? min(b.end, duration) : b.end
            guard end > start else { continue }
            var seg: ZoomSegment
            switch b.focus {
            case .manual(let p):
                seg = ZoomSegment(start: start, end: end, scale: clampScale(b.scale, tuning), initialFocus: p.clamped, followFrom: nil, origin: .manual(b.id))
            case .auto:
                // Follow the cursor. A click in the block's lead-in is where the action is: aim there
                // first and start following from the click (like Round 2 auto-zooms did).
                if let c = leadInClick(track: track, start: start, end: end, tuning: tuning) {
                    seg = ZoomSegment(start: start, end: end, scale: clampScale(b.scale, tuning), initialFocus: c.point.clamped, followFrom: c.t, origin: .manual(b.id))
                } else {
                    let p = track.rawPosition(at: start)?.clamped ?? .center
                    seg = ZoomSegment(start: start, end: end, scale: clampScale(b.scale, tuning), initialFocus: p, followFrom: start, origin: .manual(b.id))
                }
            }
            seg.easeIn = clampEase(b.easeIn)
            seg.easeOut = clampEase(b.easeOut)
            // A later-starting block takes over only where it overlaps: earlier blocks keep the
            // parts before and after it (e.g. a short block nested inside a long one).
            var next: [ZoomSegment] = []
            for prev in segs {
                next.append(contentsOf: cut(prev, from: seg.start, to: seg.end, track: track).filter { $0.end > $0.start })
            }
            next.append(seg)
            next.sort { $0.start < $1.start }
            segs = next
        }
        return segs
    }

    /// Cut every manual interval out of an auto segment; manual blocks win where they overlap.
    static func subtract(_ manual: [ZoomSegment], from seg: ZoomSegment, track: CursorTrack, tuning: MotionTuning) -> [ZoomSegment] {
        var pieces = [seg]
        for m in manual where m.end > seg.start && m.start < seg.end {
            pieces = pieces.flatMap { cut($0, from: m.start, to: m.end, track: track) }
        }
        return pieces.filter { $0.end - $0.start >= tuning.minSegmentDuration }
    }

    /// The parts of `seg` outside `[start, end)`: up to one piece before and one after. A
    /// cursor-following piece that resumes after the cut is re-anchored on the cursor there.
    static func cut(_ seg: ZoomSegment, from start: MediaTime, to end: MediaTime, track: CursorTrack) -> [ZoomSegment] {
        if end <= seg.start || start >= seg.end { return [seg] }
        var out: [ZoomSegment] = []
        if start > seg.start {
            var a = seg
            a.end = start
            out.append(a)
        }
        if end < seg.end {
            var b = seg
            b.start = end
            if let f = b.followFrom, b.start > f {
                b.initialFocus = track.rawPosition(at: b.start)?.clamped ?? b.initialFocus
            }
            out.append(b)
        }
        return out
    }

    /// First mouse-down in `[start, start + autoZoomLeadIn]` (and before `end`), if any.
    static func leadInClick(track: CursorTrack, start: MediaTime, end: MediaTime, tuning: MotionTuning) -> ClickEvent? {
        let limit = min(end, start + max(tuning.autoZoomLeadIn, 0) + 1e-6)
        var best: ClickEvent?
        for c in track.clicks where c.kind == .down && c.t.isFinite && c.t >= start && c.t <= limit {
            if best == nil || c.t < best!.t { best = c }
        }
        return best
    }

    // MARK: Stored auto-zoom blocks

    /// Auto-zooms from clicks as stored `ZoomBlock`s (`isAuto`, follow-cursor focus, default ease).
    /// Parts that overlap `existing` blocks are cut away, and pieces shorter than
    /// `MotionTuning.minSegmentDuration` are dropped, so the result never overlaps `existing`.
    public static func autoZoomBlocks(
        track: CursorTrack,
        duration: MediaTime,
        sourceAspect: Double,
        avoiding existing: [ZoomBlock] = [],
        tuning: MotionTuning = .standard
    ) -> [ZoomBlock] {
        guard duration.isFinite, duration > 0 else { return [] }
        let segs = autoSegments(track: track, duration: duration, sourceAspect: sourceAspect, tuning: tuning)
        let taken = existing.filter { $0.start.isFinite && $0.end.isFinite && $0.end > $0.start }
        var out: [ZoomBlock] = []
        for seg in segs {
            var pieces: [(MediaTime, MediaTime)] = [(seg.start, seg.end)]
            for b in taken {
                pieces = pieces.flatMap { p -> [(MediaTime, MediaTime)] in
                    if b.end <= p.0 || b.start >= p.1 { return [p] }
                    var r: [(MediaTime, MediaTime)] = []
                    if b.start > p.0 { r.append((p.0, b.start)) }
                    if b.end < p.1 { r.append((b.end, p.1)) }
                    return r
                }
            }
            for (a, b) in pieces where b - a >= tuning.minSegmentDuration - 1e-9 {
                out.append(ZoomBlock(start: a, end: b, scale: seg.scale, focus: .auto, isAuto: true,
                                     easeIn: tuning.zoomSpring.response, easeOut: tuning.zoomSpring.response))
            }
        }
        return out.sorted { $0.start < $1.start }
    }

    // MARK: Helpers

    /// Safety clamp for stored ease durations (spring responses). The inspector offers the narrower
    /// `ZoomBlockEditing.easeRange`; this only guards hand-edited or corrupt project files.
    public static let easeClampRange: ClosedRange<Double> = 0.05...5

    static func clampEase(_ e: Double) -> Double {
        guard e.isFinite else { return ZoomBlock.defaultEase }
        return min(max(e, easeClampRange.lowerBound), easeClampRange.upperBound)
    }

    static func clampScale(_ s: Double, _ tuning: MotionTuning) -> Double {
        guard s.isFinite else { return 1 }
        return min(max(s, 1), max(1, tuning.maxZoomScale))
    }

    /// Index of the first sample with `t > time` (samples.count if none).
    static func firstIndex(in samples: [CursorSample], after time: MediaTime) -> Int {
        var lo = 0, hi = samples.count
        while lo < hi {
            let mid = (lo + hi) / 2
            if samples[mid].t <= time { lo = mid + 1 } else { hi = mid }
        }
        return lo
    }
}
