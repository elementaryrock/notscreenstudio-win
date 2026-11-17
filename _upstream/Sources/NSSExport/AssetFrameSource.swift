import AVFoundation
import CoreMedia
import CoreVideo
import Foundation
import NSSCore

/// Sequential `AVAssetReader`-based `FrameSource`.
///
/// Frame selection rule (shared with every other `FrameSource`): the decoded frame nearest AT OR BEFORE `t`.
/// For `t` before the first frame, the first frame is returned. A source with a lower frame rate than the
/// export holds its last frame; one with a higher rate has frames skipped.
///
/// Access is expected to be (mostly) forward in time, which is what an export does. A backward jump, or a
/// jump far ahead, restarts the reader a little before the requested time. One instance owns its readers
/// and is safe to call from any thread: calls are serialized per track with a lock.
public final class AssetFrameSource: FrameSource, DecodeErrorReporting, @unchecked Sendable {
    private let screen: SequentialVideoReader
    private let camera: SequentialVideoReader?

    private init(screen: SequentialVideoReader, camera: SequentialVideoReader?) {
        self.screen = screen
        self.camera = camera
    }

    /// - Parameters:
    ///   - screenURL: the screen recording (`screen.mov`).
    ///   - cameraURL: the webcam recording (`camera.mov`), nil when there is none.
    /// - Throws: `NSSError.invalidProject` when a file has no video track.
    public static func make(screenURL: URL, cameraURL: URL? = nil) async throws -> AssetFrameSource {
        let screen = try await SequentialVideoReader.make(url: screenURL)
        let camera = try await cameraURL.asyncMap { try await SequentialVideoReader.make(url: $0) }
        return AssetFrameSource(screen: screen, camera: camera)
    }

    public func screenFrame(at t: MediaTime) async -> CVPixelBuffer? { screen.frame(at: t) }
    public func cameraFrame(at t: MediaTime) async -> CVPixelBuffer? { camera?.frame(at: t) }

    /// First decode error seen on the screen track, if any (the protocol can only return nil).
    public var screenError: NSSError? { screen.lastError }
    public var cameraError: NSSError? { camera?.lastError }
}

private extension Optional {
    func asyncMap<T>(_ f: (Wrapped) async throws -> T) async rethrows -> T? {
        switch self {
        case .some(let v): return try await f(v)
        case .none: return nil
        }
    }
}

/// Decodes one video track in order, holding at most two frames (current and one look-ahead).
final class SequentialVideoReader: @unchecked Sendable {
    private struct Frame {
        var pts: Double
        var buffer: CVPixelBuffer
    }

    private let url: URL
    private let asset: AVURLAsset
    private let track: AVAssetTrack
    private let trackEnd: Double
    private let lock = NSLock()
    private var reader: AVAssetReader?
    private var output: AVAssetReaderTrackOutput?
    private var current: Frame?
    private var next: Frame?
    /// Last frame before a restart: held when the restarted reader finds nothing (a request past the end).
    private var fallback: Frame?
    /// PTS of the first frame of the track, once a reader that started at 0 has produced it.
    private var firstPTS: Double?
    /// Start of the current reader's time range, seconds.
    private var readerStart: Double = 0
    private var exhausted = false
    private var failed = false
    private var _lastError: NSSError?

    /// Restart instead of decoding through when the request is further ahead than this (seconds).
    private let jumpThreshold: Double = 4
    /// How far before the requested time a restarted reader begins, so an at-or-before frame exists.
    private let restartLeadIn: Double = 1

    private init(url: URL, asset: AVURLAsset, track: AVAssetTrack, trackEnd: Double) {
        self.url = url
        self.trackEnd = trackEnd
        self.asset = asset
        self.track = track
    }

    static func make(url: URL) async throws -> SequentialVideoReader {
        guard FileManager.default.fileExists(atPath: url.path) else {
            throw NSSError.invalidProject("Missing media file \(url.lastPathComponent)")
        }
        let asset = AVURLAsset(url: url)
        let tracks: [AVAssetTrack]
        do { tracks = try await asset.loadTracks(withMediaType: .video) } catch {
            throw NSSError.io("Cannot open \(url.lastPathComponent): \(error.localizedDescription)")
        }
        guard let track = tracks.first else {
            throw NSSError.invalidProject("No video track in \(url.lastPathComponent)")
        }
        let range = (try? await track.load(.timeRange)) ?? .invalid
        let end = range.isValid ? range.end.seconds : .infinity
        return SequentialVideoReader(url: url, asset: asset, track: track, trackEnd: end.isFinite ? end : .infinity)
    }

    var lastError: NSSError? {
        lock.lock(); defer { lock.unlock() }
        return _lastError
    }

    func frame(at t: Double) -> CVPixelBuffer? {
        lock.lock(); defer { lock.unlock() }
        if failed { return nil }
        if needsRestart(for: t) { restart(at: t, leadIn: restartLeadIn) }
        advance(to: t)
        // Sparse (variable frame rate) sources: the sample at or before t may start before the reader's
        // range. If the reader only found later frames, widen the lead-in until one at or before t shows
        // up, down to 0 (start of file).
        var lead = restartLeadIn
        while !failed, current == nil, let n = next, n.pts > t, readerStart > 0 {
            lead *= 4
            restart(at: t, leadIn: lead)
            advance(to: t)
        }
        if failed { return nil }
        // t is before the very first frame of the track: hold it (nothing earlier exists).
        if current == nil, let n = next {
            current = n
            next = nil
        }
        if let c = current { fallback = nil; return c.buffer }
        if let f = fallback, f.pts <= t { return f.buffer }
        return nil
    }

    private func advance(to t: Double) {
        while !failed {
            if next == nil, !exhausted { next = readFrame() }
            guard let n = next, n.pts <= t else { break }
            current = n
            next = nil
        }
    }

    private func needsRestart(for t: Double) -> Bool {
        guard reader != nil else { return true }
        // A held first frame (t before the track's first frame) is not a reason to restart.
        if let c = current, t < c.pts, c.pts != firstPTS { return true }
        if let n = next, t - n.pts > jumpThreshold { return true }
        return false
    }

    private func restart(at t: Double, leadIn: Double) {
        reader?.cancelReading()
        if let c = current ?? next { fallback = c }
        reader = nil; output = nil; current = nil; next = nil; exhausted = false
        do {
            let reader = try AVAssetReader(asset: asset)
            let out = AVAssetReaderTrackOutput(track: track, outputSettings: [
                kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
                kCVPixelBufferIOSurfacePropertiesKey as String: [:] as [String: Any],
                kCVPixelBufferMetalCompatibilityKey as String: true,
            ])
            out.alwaysCopiesSampleData = false
            guard reader.canAdd(out) else { throw NSSError.unsupported("Cannot decode \(url.lastPathComponent)") }
            reader.add(out)
            let start = max(0, min(t, trackEnd) - leadIn)
            readerStart = start
            reader.timeRange = CMTimeRange(start: CMTime(seconds: start, preferredTimescale: 60_000), duration: .positiveInfinity)
            guard reader.startReading() else {
                throw NSSError.io("Cannot read \(url.lastPathComponent): \(reader.error?.localizedDescription ?? "unknown error")")
            }
            self.reader = reader
            self.output = out
        } catch {
            fail(error)
        }
    }

    private func readFrame() -> Frame? {
        guard let output, let reader else { exhausted = true; return nil }
        while let sample = output.copyNextSampleBuffer() {
            guard let image = CMSampleBufferGetImageBuffer(sample) else { continue }
            let pts = CMSampleBufferGetPresentationTimeStamp(sample).seconds
            if readerStart == 0, firstPTS == nil { firstPTS = pts }
            return Frame(pts: pts, buffer: image)
        }
        exhausted = true
        if reader.status == .failed {
            fail(reader.error ?? NSSError.io("Decoding \(url.lastPathComponent) failed"))
        }
        return nil
    }

    private func fail(_ error: Error) {
        failed = true
        _lastError = (error as? NSSError) ?? .io("\(url.lastPathComponent): \(error.localizedDescription)")
    }
}
