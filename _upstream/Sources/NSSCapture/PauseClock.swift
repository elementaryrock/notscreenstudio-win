import CoreMedia
import Foundation
import NSSCore

/// The stretches of host-clock time that were paused. Pure value type, so the timestamp maths is unit-tested.
///
/// Pausing leaves the paused time out of every file: samples that start inside a pause are dropped, and
/// every later sample is moved earlier by the total paused time before it. The recording therefore has no gap.
public struct PauseTimeline: Sendable, Equatable {
    public struct Interval: Sendable, Equatable {
        public var start: Double
        /// nil while the pause is still open.
        public var end: Double?
        public init(start: Double, end: Double? = nil) {
            self.start = start
            self.end = end
        }
    }

    public private(set) var intervals: [Interval] = []

    public init() {}

    /// True while a pause is open.
    public var isPaused: Bool { intervals.last.map { $0.end == nil } ?? false }

    /// Opens a pause at `t`. Ignored if already paused.
    public mutating func pause(at t: Double) {
        guard !isPaused else { return }
        intervals.append(Interval(start: t))
    }

    /// Closes the open pause at `t`. Ignored if not paused.
    public mutating func resume(at t: Double) {
        guard isPaused, let i = intervals.indices.last else { return }
        intervals[i].end = max(t, intervals[i].start)
    }

    /// True when `t` falls inside a pause (start inclusive, end exclusive).
    public func isPaused(at t: Double) -> Bool {
        intervals.contains { t >= $0.start && t < ($0.end ?? .infinity) }
    }

    /// Total paused time strictly before `t`. An open pause counts up to `t`.
    public func pausedDuration(before t: Double) -> Double {
        intervals.reduce(0) { total, i in
            total + max(0, min(t, i.end ?? .infinity) - i.start)
        }
    }

    /// `t` moved onto the pause-free timeline. A time inside a pause collapses to the start of that pause.
    public func shift(_ t: Double) -> Double { t - pausedDuration(before: t) }

    /// The pause-free time for a sample starting at `t`, or nil if it starts inside a pause (drop it).
    public func map(_ t: Double) -> Double? { isPaused(at: t) ? nil : shift(t) }

    /// Total paused time up to `t`.
    public func totalPaused(until t: Double) -> Double { pausedDuration(before: t) }

    /// A cursor track (times relative to `origin`, a host time) with the paused stretches removed, matching the video.
    /// Samples inside a pause are dropped. Clicks made during a pause are dropped (see below); shape changes keep their order and
    /// collapse to the pause start.
    public func removingPauses(from track: CursorTrack, origin: Double) -> CursorTrack {
        guard !intervals.isEmpty else { return track }
        func host(_ t: MediaTime) -> Double { origin + t }
        var out = track
        out.samples = track.samples.compactMap { s in
            guard let mapped = map(host(s.t)) else { return nil }
            var s = s
            s.t = max(0, mapped - origin)
            return s
        }
        // A click made during a pause never happened in the recording: drop a press that starts inside a pause,
        // and its release even if that comes after resume. A press from before the pause whose release falls
        // inside it keeps the release, collapsed to the pause start, so the button is not left held down.
        var droppingRelease: Set<MouseButton> = []
        out.clicks = track.clicks.compactMap { c in
            switch c.kind {
            case .down:
                if isPaused(at: host(c.t)) { droppingRelease.insert(c.button); return nil }
                droppingRelease.remove(c.button)
            case .up:
                if droppingRelease.remove(c.button) != nil { return nil }
            }
            var c = c
            c.t = max(0, shift(host(c.t)) - origin)
            return c
        }
        out.shapes = track.shapes.map { c in
            var c = c
            c.t = max(0, shift(host(c.t)) - origin)
            return c
        }
        return out
    }
}

/// Thread-safe `PauseTimeline` shared by the screen, microphone and camera recorders of one session.
final class PauseClock: @unchecked Sendable {
    private let lock = NSLock()
    private var timeline = PauseTimeline()

    var snapshot: PauseTimeline { lock.withLock { timeline } }
    var isPaused: Bool { lock.withLock { timeline.isPaused } }

    func pause(at t: Double) { lock.withLock { timeline.pause(at: t) } }
    func resume(at t: Double) { lock.withLock { timeline.resume(at: t) } }
    func shift(_ t: Double) -> Double { lock.withLock { timeline.shift(t) } }

    /// The sample moved onto the pause-free timeline, or nil when it starts inside a pause and must be dropped.
    func apply(to sample: CMSampleBuffer) -> CMSampleBuffer? {
        let start = sample.presentationTimeStamp.seconds
        let mapped: Double? = lock.withLock { timeline.intervals.isEmpty ? start : timeline.map(start) }
        guard let mapped else { return nil }
        let delta = mapped - start
        if delta == 0 { return sample }
        return sample.retimed(by: CaptureMath.hostTime(delta))
    }
}

extension CMSampleBuffer {
    /// A copy with every time stamp moved by `delta` (negative moves earlier). nil if the copy fails.
    func retimed(by delta: CMTime) -> CMSampleBuffer? {
        var count = 0
        CMSampleBufferGetSampleTimingInfoArray(self, entryCount: 0, arrayToFill: nil, entriesNeededOut: &count)
        guard count > 0 else { return self }
        var infos = [CMSampleTimingInfo](repeating: CMSampleTimingInfo(), count: count)
        CMSampleBufferGetSampleTimingInfoArray(self, entryCount: count, arrayToFill: &infos, entriesNeededOut: &count)
        for i in infos.indices {
            infos[i].presentationTimeStamp = CMTimeAdd(infos[i].presentationTimeStamp, delta)
            if infos[i].decodeTimeStamp.isValid { infos[i].decodeTimeStamp = CMTimeAdd(infos[i].decodeTimeStamp, delta) }
        }
        var out: CMSampleBuffer?
        CMSampleBufferCreateCopyWithNewTiming(
            allocator: kCFAllocatorDefault, sampleBuffer: self, sampleTimingEntryCount: count,
            sampleTimingArray: &infos, sampleBufferOut: &out)
        return out
    }
}

/// A recording session that can pause and resume. Paused time is left out of every file and out of `duration`.
public protocol PausableRecordingSession: RecordingSession {
    /// Starts leaving time out of the recording. Throws if nothing is recording.
    func pause() async throws
    func resume() async throws
    /// The pauses of the current (or just finished) recording, for rebasing the cursor track.
    func pauseTimeline() async -> PauseTimeline
}
