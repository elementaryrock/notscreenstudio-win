import AVFoundation
import CoreMedia
import Foundation
import NSSCore

/// Thin lock-protected wrapper over `AVAssetWriter`. All inputs are real-time. The writer session is
/// started at the shared origin so every file's t = 0 is the same host-clock instant.
final class SampleFileWriter: @unchecked Sendable {
    enum AppendResult { case appended, dropped, failed }

    private let lock = NSLock()
    private let writer: AVAssetWriter
    private var inputs: [String: AVAssetWriterInput] = [:]
    private var started = false
    private var finished = false
    private var reportedFailure = false
    private var lastEnd: Double = 0
    private var appended = 0
    private let onFailure: @Sendable (NSSError) -> Void
    private let context: CaptureErrorContext

    init(url: URL, fileType: AVFileType, context: CaptureErrorContext, onFailure: @escaping @Sendable (NSSError) -> Void) throws {
        try? FileManager.default.removeItem(at: url)
        do {
            writer = try AVAssetWriter(outputURL: url, fileType: fileType)
        } catch {
            throw CaptureErrorMapper.map(error, context: context)
        }
        writer.shouldOptimizeForNetworkUse = false
        self.onFailure = onFailure
        self.context = context
    }

    var isStarted: Bool { lock.withLock { started } }
    var sampleCount: Int { lock.withLock { appended } }

    func addInput(_ name: String, mediaType: AVMediaType, settings: [String: Any]?, hint: CMFormatDescription? = nil) throws {
        try lock.withLock {
            guard !started else { throw NSSError.unsupported("Cannot add inputs after the writer started.") }
            if let settings, !writer.canApply(outputSettings: settings, forMediaType: mediaType) {
                throw NSSError.unsupported("Unsupported \(mediaType.rawValue) output settings.")
            }
            let input = AVAssetWriterInput(mediaType: mediaType, outputSettings: settings, sourceFormatHint: hint)
            input.expectsMediaDataInRealTime = true
            guard writer.canAdd(input) else { throw NSSError.unsupported("Cannot add \(mediaType.rawValue) input.") }
            writer.add(input)
            inputs[name] = input
        }
    }

    /// Starts writing with the session beginning at the origin. Idempotent.
    func start(atOrigin origin: Double) throws {
        try lock.withLock {
            guard !started, !finished else { return }
            guard writer.startWriting() else {
                throw CaptureErrorMapper.map(writer.error ?? NSSError.io("Could not start writing."), context: context)
            }
            writer.startSession(atSourceTime: CaptureMath.hostTime(origin))
            started = true
        }
    }

    @discardableResult
    func append(_ sample: CMSampleBuffer, to name: String) -> AppendResult {
        var failure: NSSError?
        let result: AppendResult = lock.withLock {
            guard started, !finished, let input = inputs[name] else { return .dropped }
            if writer.status == .failed {
                failure = reportOnce(writer.error)
                return .failed
            }
            guard input.isReadyForMoreMediaData else { return .dropped }
            if input.append(sample) {
                appended += 1
                let end = (sample.presentationTimeStamp + sample.duration.validOrZero).seconds
                if end.isFinite { lastEnd = max(lastEnd, end) }
                return .appended
            }
            failure = reportOnce(writer.error)
            return .failed
        }
        if let failure { onFailure(failure) }
        return result
    }

    private func reportOnce(_ error: Error?) -> NSSError? {
        guard !reportedFailure else { return nil }
        reportedFailure = true
        return CaptureErrorMapper.map(error ?? NSSError.io("Writing failed."), context: .writer)
    }

    /// Finalizes the file. Returns true if the file contains media. Throws if the writer failed.
    func finish(endHost: Double) async throws -> Bool {
        let (wasStarted, count): (Bool, Int) = lock.withLock {
            guard !finished else { return (false, 0) }
            finished = true
            if started, writer.status == .writing {
                for input in inputs.values { input.markAsFinished() }
                writer.endSession(atSourceTime: CaptureMath.hostTime(max(endHost, lastEnd)))
            }
            return (started, appended)
        }
        guard wasStarted else {
            lock.withLock { if writer.status == .writing || writer.status == .unknown { writer.cancelWriting() } }
            return false
        }
        await writer.finishWriting()
        if writer.status == .failed {
            throw CaptureErrorMapper.map(writer.error ?? NSSError.io("Finalizing failed."), context: .writer)
        }
        return count > 0
    }
}

extension CMTime {
    var validOrZero: CMTime { isValid && !isIndefinite ? self : .zero }
}
