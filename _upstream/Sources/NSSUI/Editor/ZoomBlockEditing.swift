import Foundation
import NSSCore
import NSSMotion

/// Pure editing rules for the zoom track: blocks never overlap, never shorter than `minLength`,
/// and always stay inside `0...duration`. All functions return a new array (or nil when the edit is impossible).
public enum ZoomBlockEditing {
    public static let minLength: MediaTime = 0.3
    public static let defaultLength: MediaTime = 2
    public static let scaleRange: ClosedRange<Double> = 1.1...4

    public enum Edge: Sendable { case start, end }

    private static func sorted(_ blocks: [ZoomBlock]) -> [ZoomBlock] { blocks.sorted { $0.start < $1.start } }

    /// Free gap around `t`, excluding block `ignoring`: (lower bound, upper bound). nil if `t` lies inside another block.
    static func gap(around t: MediaTime, in blocks: [ZoomBlock], duration: MediaTime, ignoring: UUID? = nil) -> (lo: MediaTime, hi: MediaTime)? {
        var lo: MediaTime = 0
        var hi: MediaTime = duration
        for b in blocks where b.id != ignoring {
            if t >= b.start && t < b.end { return nil }
            if b.end <= t { lo = max(lo, b.end) }
            if b.start >= t { hi = min(hi, b.start) }
        }
        return (lo, hi)
    }

    /// Adds a block of up to `length` seconds starting at `time`, shrinking it to fit the free gap.
    /// Returns nil when there is no room for `minLength`.
    public static func adding(
        at time: MediaTime, length: MediaTime = defaultLength, scale: Double = 2, focus: ZoomFocus = .auto,
        to blocks: [ZoomBlock], duration: MediaTime, id: UUID = UUID()
    ) -> (blocks: [ZoomBlock], id: UUID)? {
        let t = min(max(time, 0), max(duration - minLength, 0))
        guard let g = gap(around: t, in: blocks, duration: duration) else { return nil }
        let end = min(t + length, g.hi)
        guard end - t >= minLength - 1e-9 else {
            // Not enough room after t: try sliding start back within the gap.
            let s = max(g.lo, end - length)
            guard end - s >= minLength - 1e-9 else { return nil }
            return (sorted(blocks + [ZoomBlock(id: id, start: s, end: end, scale: scale, focus: focus)]), id)
        }
        return (sorted(blocks + [ZoomBlock(id: id, start: t, end: end, scale: scale, focus: focus)]), id)
    }

    /// Moves a block by `delta` seconds, preserving its length and stopping at neighbours / the ends.
    public static func moving(_ id: UUID, by delta: MediaTime, in blocks: [ZoomBlock], duration: MediaTime) -> [ZoomBlock] {
        guard let b = blocks.first(where: { $0.id == id }) else { return blocks }
        let len = b.end - b.start
        var lo: MediaTime = 0
        var hi: MediaTime = duration
        for o in blocks where o.id != id {
            if o.end <= b.start + 1e-9 { lo = max(lo, o.end) }
            if o.start >= b.end - 1e-9 { hi = min(hi, o.start) }
        }
        let newStart = min(max(b.start + delta, lo), max(lo, hi - len))
        return replacing(id, in: blocks) { $0.start = newStart; $0.end = newStart + len }
    }

    /// Moves one edge to `time`, clamped by `minLength`, neighbours and the timeline ends.
    public static func resizing(_ id: UUID, edge: Edge, to time: MediaTime, in blocks: [ZoomBlock], duration: MediaTime) -> [ZoomBlock] {
        guard let b = blocks.first(where: { $0.id == id }) else { return blocks }
        var lo: MediaTime = 0
        var hi: MediaTime = duration
        for o in blocks where o.id != id {
            if o.end <= b.start + 1e-9 { lo = max(lo, o.end) }
            if o.start >= b.end - 1e-9 { hi = min(hi, o.start) }
        }
        switch edge {
        case .start:
            let v = min(max(time, lo), b.end - minLength)
            return replacing(id, in: blocks) { $0.start = v }
        case .end:
            let v = max(min(time, hi), b.start + minLength)
            return replacing(id, in: blocks) { $0.end = v }
        }
    }

    public static func deleting(_ id: UUID, from blocks: [ZoomBlock]) -> [ZoomBlock] { blocks.filter { $0.id != id } }

    public static func settingScale(_ id: UUID, to scale: Double, in blocks: [ZoomBlock]) -> [ZoomBlock] {
        replacing(id, in: blocks) { $0.scale = min(max(scale, scaleRange.lowerBound), scaleRange.upperBound) }
    }

    public static func settingFocus(_ id: UUID, to focus: ZoomFocus, in blocks: [ZoomBlock]) -> [ZoomBlock] {
        let f: ZoomFocus
        if case .manual(let p) = focus { f = .manual(p.clamped) } else { f = focus }
        return replacing(id, in: blocks) { $0.focus = f }
    }

    /// Drops/clips blocks to a new duration or trim so none extends past `duration` or falls under `minLength`.
    public static func normalized(_ blocks: [ZoomBlock], duration: MediaTime) -> [ZoomBlock] {
        var result: [ZoomBlock] = []
        var cursor: MediaTime = 0
        for var b in sorted(blocks) {
            b.start = max(b.start, cursor)
            b.end = min(b.end, duration)
            if b.end - b.start >= minLength - 1e-9 {
                result.append(b)
                cursor = b.end
            }
        }
        return result
    }

    /// Applies `f` to block `id`. A block that actually changes becomes manual (`isAuto = false`), so
    /// "Regenerate auto zooms" never throws away a zoom the user adjusted.
    private static func replacing(_ id: UUID, in blocks: [ZoomBlock], _ f: (inout ZoomBlock) -> Void) -> [ZoomBlock] {
        blocks.map { b in
            guard b.id == id else { return b }
            var c = b
            f(&c)
            if c != b { c.isAuto = false }
            return c
        }
    }

    // MARK: Ease

    public static let easeRange: ClosedRange<Double> = 0.1...2

    public static func settingEase(_ id: UUID, easeIn: MediaTime? = nil, easeOut: MediaTime? = nil, in blocks: [ZoomBlock]) -> [ZoomBlock] {
        func clamp(_ v: MediaTime) -> MediaTime { v.isFinite ? min(max(v, easeRange.lowerBound), easeRange.upperBound) : ZoomBlock.defaultEase }
        return replacing(id, in: blocks) { b in
            if let easeIn { b.easeIn = clamp(easeIn) }
            if let easeOut { b.easeOut = clamp(easeOut) }
        }
    }

    // MARK: Auto zooms

    public static func removingAuto(from blocks: [ZoomBlock]) -> [ZoomBlock] { blocks.filter { !$0.isAuto } }

    /// Replaces every auto block with fresh ones from the clicks; manual blocks are kept and win where
    /// they overlap.
    public static func regeneratingAuto(
        in blocks: [ZoomBlock], track: CursorTrack, duration: MediaTime, sourceAspect: Double, tuning: MotionTuning = .standard
    ) -> [ZoomBlock] {
        let manual = removingAuto(from: blocks)
        let auto = ZoomPlanner.autoZoomBlocks(track: track, duration: duration, sourceAspect: sourceAspect, avoiding: manual, tuning: tuning)
        return sorted(manual + auto)
    }

    /// One-time migration on open: a timeline that still derives auto-zooms on the fly
    /// (`autoZoomEnabled`) gets them stored as `isAuto` blocks and the flag cleared. From then on the
    /// stored blocks are the only zooms the preview and export use. Returns nil when nothing changes.
    public static func materializingAutoZooms(
        _ timeline: Timeline, track: CursorTrack, duration: MediaTime, sourceAspect: Double, tuning: MotionTuning = .standard
    ) -> Timeline? {
        guard timeline.autoZoomEnabled else { return nil }
        var t = timeline
        t.autoZoomEnabled = false
        t.zoomBlocks = regeneratingAuto(in: timeline.zoomBlocks, track: track, duration: duration, sourceAspect: sourceAspect, tuning: tuning)
        return t
    }
}

/// Trim handle rules.
public enum TrimEditing {
    public static let minLength: MediaTime = 0.5

    /// New start, clamped to `0 ... end - minLength`.
    public static func settingStart(_ t: MediaTime, in trim: TrimRange, duration: MediaTime) -> TrimRange {
        let end = trim.resolvedEnd(duration: duration)
        let minLen = min(minLength, duration)
        var r = trim
        r.start = min(max(t, 0), max(end - minLen, 0))
        return r
    }

    /// New end, clamped to `start + minLength ... duration`. Returns `end == nil` when it reaches the full duration.
    public static func settingEnd(_ t: MediaTime, in trim: TrimRange, duration: MediaTime) -> TrimRange {
        let minLen = min(minLength, duration)
        let v = min(max(t, trim.start + minLen), duration)
        var r = trim
        r.end = v >= duration - 1e-9 ? nil : v
        return r
    }

    public static func clamp(_ t: MediaTime, to trim: TrimRange, duration: MediaTime) -> MediaTime {
        min(max(t, trim.start), trim.resolvedEnd(duration: duration))
    }
}

/// Maps between time and x on the timeline (pixel x relative to the track's left edge).
public struct TimelineGeometry: Sendable, Equatable {
    public var duration: MediaTime
    public var width: Double

    public init(duration: MediaTime, width: Double) {
        self.duration = max(duration, 0.001)
        self.width = max(width, 1)
    }

    public var pixelsPerSecond: Double { width / duration }

    public func x(for t: MediaTime) -> Double { t * pixelsPerSecond }

    public func time(forX x: Double) -> MediaTime { min(max(x / pixelsPerSecond, 0), duration) }

    /// Ruler tick spacing (seconds) giving roughly `targetPixels` between labelled ticks.
    public func tickInterval(targetPixels: Double = 80) -> MediaTime {
        let raw = targetPixels / pixelsPerSecond
        let steps: [MediaTime] = [0.1, 0.25, 0.5, 1, 2, 5, 10, 15, 30, 60, 120, 300, 600]
        return steps.first(where: { $0 >= raw }) ?? 600
    }

    public static func label(for t: MediaTime) -> String {
        let total = Int(t.rounded(.down))
        let frac = Int(((t - Double(total)) * 10 + 1e-6).rounded(.down))
        return String(format: "%d:%02d.%d", total / 60, total % 60, min(frac, 9))
    }
}
