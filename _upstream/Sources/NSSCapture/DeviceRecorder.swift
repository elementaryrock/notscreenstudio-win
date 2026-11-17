@preconcurrency import AVFoundation
import CoreMedia
import Foundation
import NSSCore

/// Records one AVCaptureDevice (microphone -> .m4a AAC, camera -> .mov) with its writer session
/// aligned to the shared origin.
final class DeviceRecorder: NSObject, AVCaptureAudioDataOutputSampleBufferDelegate, AVCaptureVideoDataOutputSampleBufferDelegate, @unchecked Sendable {
    enum Kind: Sendable { case microphone, camera }

    private let kind: Kind
    private let session = AVCaptureSession()
    private let device: AVCaptureDevice
    private let url: URL
    private let origin: SharedOrigin
    private let pause: PauseClock
    private let onFailure: @Sendable (NSSError) -> Void
    private let queue: DispatchQueue
    private let lock = NSLock()
    private var writer: SampleFileWriter?
    private var stopped = false
    private var writerFailed = false
    private var endedHost: Double?
    private var observers: [NSObjectProtocol] = []

    init(kind: Kind, deviceID: String, url: URL, origin: SharedOrigin, pause: PauseClock = PauseClock(), onFailure: @escaping @Sendable (NSSError) -> Void) throws {
        guard let device = CaptureDevices.device(id: deviceID) else {
            throw NSSError.io("\(kind == .microphone ? "The microphone" : "The camera") is not connected.")
        }
        self.kind = kind
        self.device = device
        self.url = url
        self.origin = origin
        self.pause = pause
        self.onFailure = onFailure
        self.queue = DispatchQueue(label: "nss.capture.\(kind)", qos: .userInteractive)
        super.init()
        try configureSession()
    }

    private var context: CaptureErrorContext { kind == .microphone ? .microphone : .camera }

    private func configureSession() throws {
        session.beginConfiguration()
        defer { session.commitConfiguration() }
        do {
            let input = try AVCaptureDeviceInput(device: device)
            guard session.canAddInput(input) else { throw NSSError.unsupported("Cannot use this device.") }
            session.addInput(input)
        } catch {
            throw CaptureErrorMapper.map(error, context: context)
        }
        switch kind {
        case .microphone:
            let out = AVCaptureAudioDataOutput()
            out.setSampleBufferDelegate(self, queue: queue)
            guard session.canAddOutput(out) else { throw NSSError.unsupported("Cannot capture audio.") }
            session.addOutput(out)
        case .camera:
            if session.canSetSessionPreset(.hd1920x1080) { session.sessionPreset = .hd1920x1080 }
            let out = AVCaptureVideoDataOutput()
            out.alwaysDiscardsLateVideoFrames = true
            out.setSampleBufferDelegate(self, queue: queue)
            guard session.canAddOutput(out) else { throw NSSError.unsupported("Cannot capture video.") }
            session.addOutput(out)
        }
    }

    func start() async throws {
        let nc = NotificationCenter.default
        let fail: @Sendable (NSSError) -> Void = { [weak self] e in self?.deviceFailed(e) }
        let ctx = context
        observers.append(nc.addObserver(forName: AVCaptureDevice.wasDisconnectedNotification, object: device, queue: nil) { _ in
            fail(CaptureErrorMapper.map(AVError(.deviceWasDisconnected), context: ctx))
        })
        observers.append(nc.addObserver(forName: AVCaptureSession.runtimeErrorNotification, object: session, queue: nil) { note in
            let err = note.userInfo?[AVCaptureSessionErrorKey] as? Error ?? NSSError.io("Capture session error")
            fail(CaptureErrorMapper.map(err, context: ctx))
        })
        let session = self.session
        await withCheckedContinuation { (c: CheckedContinuation<Void, Never>) in
            DispatchQueue.global(qos: .userInitiated).async {
                session.startRunning()
                c.resume()
            }
        }
        if !session.isRunning { throw NSSError.io("\(CaptureErrorMapper.deviceName(context)) could not start.") }
    }

    private func deviceFailed(_ error: NSSError) {
        let first: Bool = lock.withLock {
            if stopped || endedHost != nil { return false }
            endedHost = CaptureMath.hostNow()
            return true
        }
        guard first else { return }
        let session = self.session
        DispatchQueue.global().async { session.stopRunning() }
        onFailure(error)
    }

    /// Returns true if the file contains media.
    func stop(endHost: Double) async throws -> Bool {
        let ended: Double? = lock.withLock {
            stopped = true
            return endedHost
        }
        for o in observers { NotificationCenter.default.removeObserver(o) }
        observers.removeAll()
        let session = self.session
        await withCheckedContinuation { (c: CheckedContinuation<Void, Never>) in
            DispatchQueue.global(qos: .userInitiated).async {
                session.stopRunning()
                c.resume()
            }
        }
        // Wait for in-flight delegate callbacks to drain before finalizing.
        await withCheckedContinuation { (c: CheckedContinuation<Void, Never>) in queue.async { c.resume() } }
        let w = lock.withLock { writer }
        guard let w else { return false }
        return try await w.finish(endHost: pause.shift(ended ?? endHost))
    }

    // MARK: Sample handling

    private func handle(_ sample: CMSampleBuffer) {
        guard sample.isValid, let o = origin.current else { return } // before the first screen frame: drop
        guard let hostSample = toHostClock(sample) else { return }
        let start = hostSample.presentationTimeStamp.seconds
        if o.isEntirelyBefore(start: start, duration: hostSample.duration.validOrZero.seconds) { return }
        // Samples inside a pause are dropped; later ones move earlier by the paused time.
        guard let sample = pause.apply(to: hostSample) else { return }

        let w: SampleFileWriter? = lock.withLock {
            if stopped || writerFailed || (endedHost != nil && writer == nil) { return nil }
            if writer == nil {
                writer = makeWriter(for: sample, origin: o)
                if writer == nil { writerFailed = true } // set up once, report once
            }
            return writer
        }
        w?.append(sample, to: "main")
    }

    private func makeWriter(for sample: CMSampleBuffer, origin o: TimeOrigin) -> SampleFileWriter? {
        guard let format = sample.formatDescription else { return nil }
        do {
            let w = try SampleFileWriter(url: url, fileType: kind == .microphone ? .m4a : .mov, context: context, onFailure: onFailure)
            switch kind {
            case .microphone:
                do {
                    try w.addInput("main", mediaType: .audio, settings: Self.audioSettings(format), hint: format)
                } catch {
                    try w.addInput("main", mediaType: .audio, settings: Self.audioSettings(format, withBitrate: false), hint: format)
                }
            case .camera:
                let dims = CMVideoFormatDescriptionGetDimensions(format)
                let size = PixelSize(width: Int(dims.width), height: Int(dims.height))
                do {
                    try w.addInput("main", mediaType: .video, settings: Self.videoSettings(size, .hevc))
                } catch {
                    try w.addInput("main", mediaType: .video, settings: Self.videoSettings(size, .h264))
                }
            }
            try w.start(atOrigin: o.hostTime)
            return w
        } catch {
            onFailure(CaptureErrorMapper.map(error, context: context))
            return nil
        }
    }

    /// Highest AAC bitrate the sample rate comfortably allows, per channel.
    static func aacBitrate(channels: Int, sampleRate: Double) -> Int {
        let base = channels == 1 ? 96_000 : 160_000
        let perChannelCap: Int
        switch sampleRate {
        case ..<12_000: perChannelCap = 32_000
        case ..<24_000: perChannelCap = 64_000
        default: perChannelCap = 128_000
        }
        return min(base, perChannelCap * channels)
    }

    static func audioSettings(_ format: CMFormatDescription, withBitrate: Bool = true) -> [String: Any] {
        let asbd = CMAudioFormatDescriptionGetStreamBasicDescription(format)?.pointee
        let channels = min(max(Int(asbd?.mChannelsPerFrame ?? 1), 1), 2)
        let rate = (asbd?.mSampleRate ?? 48_000) > 0 ? (asbd?.mSampleRate ?? 48_000) : 48_000
        var s: [String: Any] = [
            AVFormatIDKey: kAudioFormatMPEG4AAC,
            AVSampleRateKey: rate,
            AVNumberOfChannelsKey: channels,
        ]
        if withBitrate { s[AVEncoderBitRateKey] = aacBitrate(channels: channels, sampleRate: rate) }
        return s
    }

    static func videoSettings(_ size: PixelSize, _ codec: AVVideoCodecType) -> [String: Any] {
        [
            AVVideoCodecKey: codec,
            AVVideoWidthKey: size.width,
            AVVideoHeightKey: size.height,
            AVVideoCompressionPropertiesKey: [
                AVVideoAverageBitRateKey: CaptureMath.videoBitrate(width: size.width, height: size.height, frameRate: 30, bitsPerPixel: 0.1),
                AVVideoExpectedSourceFrameRateKey: 30,
                AVVideoAllowFrameReorderingKey: false,
            ] as [String: Any],
        ]
    }

    /// Device buffers are stamped on the session's synchronization clock. Retime to the host clock if it differs.
    private func toHostClock(_ sample: CMSampleBuffer) -> CMSampleBuffer? {
        let host = CMClockGetHostTimeClock()
        guard let clock = session.synchronizationClock, clock != host else { return sample }
        let pts = sample.presentationTimeStamp
        let converted = CMSyncConvertTime(pts, from: clock, to: host)
        guard converted.isValid else { return sample }
        let delta = CMTimeSubtract(converted, pts)
        var count = 0
        CMSampleBufferGetSampleTimingInfoArray(sample, entryCount: 0, arrayToFill: nil, entriesNeededOut: &count)
        guard count > 0 else { return sample }
        var infos = [CMSampleTimingInfo](repeating: CMSampleTimingInfo(), count: count)
        CMSampleBufferGetSampleTimingInfoArray(sample, entryCount: count, arrayToFill: &infos, entriesNeededOut: &count)
        for i in infos.indices {
            infos[i].presentationTimeStamp = CMTimeAdd(infos[i].presentationTimeStamp, delta)
            if infos[i].decodeTimeStamp.isValid { infos[i].decodeTimeStamp = CMTimeAdd(infos[i].decodeTimeStamp, delta) }
        }
        var out: CMSampleBuffer?
        CMSampleBufferCreateCopyWithNewTiming(allocator: kCFAllocatorDefault, sampleBuffer: sample, sampleTimingEntryCount: count, sampleTimingArray: &infos, sampleBufferOut: &out)
        return out
    }

    func captureOutput(_ output: AVCaptureOutput, didOutput sampleBuffer: CMSampleBuffer, from connection: AVCaptureConnection) {
        handle(sampleBuffer)
    }
}
