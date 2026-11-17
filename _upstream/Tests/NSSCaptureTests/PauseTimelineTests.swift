import CoreMedia
import NSSCore
import Testing
@testable import NSSCapture

@Suite("Pause timeline")
struct PauseTimelineTests {
    @Test func noPausesChangesNothing() {
        let t = PauseTimeline()
        #expect(t.shift(5) == 5)
        #expect(t.map(5) == 5)
        #expect(!t.isPaused)
    }

    @Test func samplesInsideAPauseAreDropped() {
        var t = PauseTimeline()
        t.pause(at: 10)
        #expect(t.isPaused)
        #expect(t.map(10) == nil)
        #expect(t.map(50) == nil) // an open pause drops everything after it
        t.resume(at: 14)
        #expect(!t.isPaused)
        #expect(t.map(12) == nil)
        #expect(t.map(14) == 10) // the first sample after resume follows the last one before the pause
        #expect(t.map(9.99) == 9.99)
    }

    @Test func laterSamplesMoveEarlierByTheTotalPausedTime() {
        var t = PauseTimeline()
        t.pause(at: 10); t.resume(at: 14) // 4 s
        t.pause(at: 20); t.resume(at: 23) // 3 s
        #expect(t.shift(15) == 11)
        #expect(t.shift(30) == 23)
        #expect(t.map(21) == nil)
        #expect(t.map(23) == 16)
    }

    @Test func shiftedTimesNeverGoBackwards() {
        var t = PauseTimeline()
        t.pause(at: 1); t.resume(at: 2); t.pause(at: 3); t.resume(at: 3.5)
        var last = -Double.infinity
        for i in 0..<100 {
            let x = Double(i) * 0.06
            if let m = t.map(x) { #expect(m >= last); last = m }
        }
    }

    @Test func endTimeWhilePausedIsTheStartOfThePause() {
        var t = PauseTimeline()
        t.pause(at: 10); t.resume(at: 12)
        t.pause(at: 20)
        #expect(t.shift(25) == 18) // 20 - 2 s earlier pause
        #expect(t.pausedDuration(before: 25) == 7)
    }

    @Test func doublePauseAndResumeAreIgnored() {
        var t = PauseTimeline()
        t.resume(at: 1)
        t.pause(at: 2); t.pause(at: 3)
        t.resume(at: 4); t.resume(at: 5)
        #expect(t.intervals == [.init(start: 2, end: 4)])
    }

    @Test func cursorTrackLosesPausedStretchesAndStaysInStepWithTheVideo() {
        var t = PauseTimeline()
        t.pause(at: 102); t.resume(at: 105) // origin 100: paused from t = 2 to t = 5
        let track = CursorTrack(
            samples: [0, 1, 2.5, 4, 5, 6].map { CursorSample(t: $0, x: $0 / 10, y: 0) },
            clicks: [
                ClickEvent(t: 1, x: 0, y: 0, kind: .down), ClickEvent(t: 3, x: 0, y: 0, kind: .up), // release inside: kept, collapsed
                ClickEvent(t: 3.2, x: 0, y: 0, kind: .down), ClickEvent(t: 3.4, x: 0, y: 0, kind: .up), // fully inside: dropped
                ClickEvent(t: 4, x: 0, y: 0, kind: .down), ClickEvent(t: 5.5, x: 0, y: 0, kind: .up), // down inside, up after: both dropped
                ClickEvent(t: 6, x: 0, y: 0, kind: .down), ClickEvent(t: 6.5, x: 0, y: 0, kind: .up),
            ],
            shapes: [CursorShapeChange(t: 3, shape: .iBeam), CursorShapeChange(t: 6, shape: .arrow)])
        let out = t.removingPauses(from: track, origin: 100)
        #expect(out.samples.map(\.t) == [0, 1, 2, 3]) // 5 -> 2, 6 -> 3; 2.5 and 4 dropped
        #expect(out.clicks.map(\.t) == [1, 2, 3, 3.5]) // only the click that started outside a pause survives, plus the later one
        #expect(out.clicks.map(\.kind) == [.down, .up, .down, .up])
        #expect(out.shapes.map(\.t) == [2, 3])
    }

    @Test func pauseClockRetimesSampleBuffers() throws {
        let clock = PauseClock()
        clock.pause(at: 10)
        clock.resume(at: 14)
        func sample(at seconds: Double) throws -> CMSampleBuffer {
            var format: CMFormatDescription?
            CMAudioFormatDescriptionCreate(
                allocator: nil, asbd: [AudioStreamBasicDescription(
                    mSampleRate: 48_000, mFormatID: kAudioFormatLinearPCM, mFormatFlags: kAudioFormatFlagIsSignedInteger | kAudioFormatFlagIsPacked,
                    mBytesPerPacket: 2, mFramesPerPacket: 1, mBytesPerFrame: 2, mChannelsPerFrame: 1, mBitsPerChannel: 16, mReserved: 0)],
                layoutSize: 0, layout: nil, magicCookieSize: 0, magicCookie: nil, extensions: nil, formatDescriptionOut: &format)
            var timing = CMSampleTimingInfo(
                duration: CMTime(value: 1, timescale: 48_000), presentationTimeStamp: CaptureMath.hostTime(seconds), decodeTimeStamp: .invalid)
            var buffer: CMSampleBuffer?
            var data = [Int16](repeating: 0, count: 1)
            var block: CMBlockBuffer?
            data.withUnsafeMutableBytes { raw in
                CMBlockBufferCreateWithMemoryBlock(
                    allocator: nil, memoryBlock: nil, blockLength: 2, blockAllocator: nil, customBlockSource: nil,
                    offsetToData: 0, dataLength: 2, flags: 0, blockBufferOut: &block)
                if let block { CMBlockBufferReplaceDataBytes(with: raw.baseAddress!, blockBuffer: block, offsetIntoDestination: 0, dataLength: 2) }
            }
            CMSampleBufferCreate(
                allocator: nil, dataBuffer: block, dataReady: true, makeDataReadyCallback: nil, refcon: nil,
                formatDescription: format, sampleCount: 1, sampleTimingEntryCount: 1, sampleTimingArray: &timing,
                sampleSizeEntryCount: 0, sampleSizeArray: nil, sampleBufferOut: &buffer)
            return try #require(buffer)
        }
        #expect(clock.apply(to: try sample(at: 12)) == nil)
        let before = try #require(clock.apply(to: try sample(at: 9)))
        #expect(abs(before.presentationTimeStamp.seconds - 9) < 1e-6)
        let after = try #require(clock.apply(to: try sample(at: 20)))
        #expect(abs(after.presentationTimeStamp.seconds - 16) < 1e-6)
    }
}
