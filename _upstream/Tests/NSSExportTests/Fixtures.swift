import AVFoundation
import CoreVideo
import Foundation
import NSSCore
import NSSRender
import VideoToolbox
@testable import NSSExport

/// Whether this machine has an encoder for `codec` (CI VMs may lack one).
func canEncode(_ codec: CMVideoCodecType) -> Bool {
    var list: CFArray?
    guard VTCopyVideoEncoderList(nil, &list) == noErr, let infos = list as? [[String: Any]] else { return false }
    return infos.contains { ($0[kVTVideoEncoderList_CodecType as String] as? CMVideoCodecType) == codec }
}

let h264Available = canEncode(kCMVideoCodecType_H264)
let hevcAvailable = canEncode(kCMVideoCodecType_HEVC)

/// A scratch directory, removed by the caller.
func makeTempDir() throws -> URL {
    let url = FileManager.default.temporaryDirectory.appendingPathComponent("nssexport-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
    return url
}

/// Writes a small H.264 movie. Frame `k` is a flat gray of value `16 * k + 8`, so tests can tell
/// which source frame the exporter picked.
func writeSyntheticVideo(to url: URL, frames: Int, fps: Int, width: Int = 320, height: Int = 180) async throws {
    let writer = try AVAssetWriter(outputURL: url, fileType: .mov)
    let input = AVAssetWriterInput(mediaType: .video, outputSettings: [
        AVVideoCodecKey: AVVideoCodecType.h264, AVVideoWidthKey: width, AVVideoHeightKey: height,
    ])
    let adaptor = AVAssetWriterInputPixelBufferAdaptor(assetWriterInput: input, sourcePixelBufferAttributes: [
        kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
        kCVPixelBufferWidthKey as String: width, kCVPixelBufferHeightKey as String: height,
    ])
    writer.add(input)
    writer.startWriting()
    writer.startSession(atSourceTime: .zero)
    for k in 0..<frames {
        while !input.isReadyForMoreMediaData { try await Task.sleep(for: .milliseconds(1)) }
        let pb = try PixelBuffers.make(width: width, height: height)
        CVPixelBufferLockBaseAddress(pb, [])
        let base = CVPixelBufferGetBaseAddress(pb)!
        let value = Int32(min(255, 16 * k + 8))
        memset(base, value, CVPixelBufferGetBytesPerRow(pb) * height)
        CVPixelBufferUnlockBaseAddress(pb, [])
        adaptor.append(pb, withPresentationTime: CMTime(value: CMTimeValue(k), timescale: CMTimeScale(fps)))
    }
    input.markAsFinished()
    writer.endSession(atSourceTime: CMTime(value: CMTimeValue(frames), timescale: CMTimeScale(fps)))
    await writer.finishWriting()
    if writer.status != .completed { throw writer.error ?? NSSError.io("synthetic video failed") }
}

/// Writes an AAC sine tone (`m4a`).
func writeSyntheticAudio(to url: URL, seconds: Double, sampleRate: Double = 44_100, channels: AVAudioChannelCount = 1) throws {
    let format = AVAudioFormat(standardFormatWithSampleRate: sampleRate, channels: channels)!
    let file = try AVAudioFile(forWriting: url, settings: [
        AVFormatIDKey: kAudioFormatMPEG4AAC, AVSampleRateKey: sampleRate, AVNumberOfChannelsKey: Int(channels),
    ])
    let count = AVAudioFrameCount(seconds * sampleRate)
    let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: count)!
    buffer.frameLength = count
    for c in 0..<Int(channels) {
        let p = buffer.floatChannelData![c]
        for n in 0..<Int(count) { p[n] = 0.3 * sinf(Float(n) * 2 * .pi * 440 / Float(sampleRate)) }
    }
    try file.write(from: buffer)
}

struct Bundle {
    let dir: URL
    let project: Project
}

/// A project bundle with a synthetic screen.mov (15 frames at 30 fps = 0.5 s) and optional mic.m4a.
func makeBundle(frames: Int = 15, fps: Int = 30, mic: Bool = false, trim: TrimRange = TrimRange(), aspect: AspectRatio = .r16x9) async throws -> Bundle {
    let dir = try makeTempDir()
    try await writeSyntheticVideo(to: dir.appendingPathComponent("screen.mov"), frames: frames, fps: fps)
    let duration = Double(frames) / Double(fps)
    if mic { try writeSyntheticAudio(to: dir.appendingPathComponent("mic.m4a"), seconds: duration) }
    let assets = RecordingAssets(
        micFile: mic ? "mic.m4a" : nil,
        sourcePixelSize: PixelSize(width: 320, height: 180), frameRate: Double(fps), duration: duration
    )
    var project = Project(name: "t", assets: assets, timeline: Timeline(trim: trim))
    project.style.aspectRatio = aspect
    return Bundle(dir: dir, project: project)
}

/// Records what the exporter feeds the renderer, then delegates to the stub renderer.
final class RecordingRenderer: FrameRenderer, @unchecked Sendable {
    struct Call { var time: Double; var gray: Int; var width: Int; var height: Int; var ioSurface: Bool }
    private let lock = NSLock()
    private var _calls: [Call] = []
    private let inner = PassthroughRenderer()
    var onRender: (@Sendable (Int) -> Void)?
    var delay: Duration?

    var calls: [Call] { lock.lock(); defer { lock.unlock() }; return _calls }

    func render(screen: CVPixelBuffer, camera: CVPixelBuffer?, state: FrameState, style: Style, into output: CVPixelBuffer) throws {
        CVPixelBufferLockBaseAddress(screen, .readOnly)
        let base = CVPixelBufferGetBaseAddress(screen)!.assumingMemoryBound(to: UInt8.self)
        // Sample the middle of the frame, channel 0.
        let mid = CVPixelBufferGetBytesPerRow(screen) * (CVPixelBufferGetHeight(screen) / 2) + (CVPixelBufferGetWidth(screen) / 2) * 4
        let gray = Int(base[mid])
        CVPixelBufferUnlockBaseAddress(screen, .readOnly)
        lock.lock()
        _calls.append(Call(time: state.time, gray: gray, width: CVPixelBufferGetWidth(output), height: CVPixelBufferGetHeight(output),
                           ioSurface: CVPixelBufferGetIOSurface(output) != nil))
        let n = _calls.count
        lock.unlock()
        onRender?(n)
        try inner.render(screen: screen, camera: camera, state: state, style: style, into: output)
    }
}

struct Probe {
    var duration: Double
    var width: Int
    var height: Int
    var nominalFPS: Float
    var videoFrames: Int
    var audioTracks: Int
    var audioSampleRate: Double
    var audioChannels: Int
    var audioDuration: Double
    var videoCodec: FourCharCode
    var firstPTS: Double
}

func probe(_ url: URL) async throws -> Probe {
    let asset = AVURLAsset(url: url)
    let video = try await asset.loadTracks(withMediaType: .video)[0]
    let size = try await video.load(.naturalSize)
    let fps = try await video.load(.nominalFrameRate)
    let codec = (try await video.load(.formatDescriptions).first).map { CMFormatDescriptionGetMediaSubType($0) } ?? 0
    let reader = try AVAssetReader(asset: asset)
    let out = AVAssetReaderTrackOutput(track: video, outputSettings: nil)
    reader.add(out)
    reader.startReading()
    var n = 0
    var firstPTS: Double?
    while let s = out.copyNextSampleBuffer() { if CMSampleBufferGetNumSamples(s) > 0 { n += 1; if firstPTS == nil { firstPTS = CMSampleBufferGetPresentationTimeStamp(s).seconds } } }
    let audio = try await asset.loadTracks(withMediaType: .audio)
    var rate = 0.0, ch = 0, adur = 0.0
    if let a = audio.first {
        adur = try await a.load(.timeRange).duration.seconds
        if let fd = try await a.load(.formatDescriptions).first, let asbd = CMAudioFormatDescriptionGetStreamBasicDescription(fd)?.pointee {
            rate = asbd.mSampleRate; ch = Int(asbd.mChannelsPerFrame)
        }
    }
    return Probe(duration: try await asset.load(.duration).seconds, width: Int(size.width), height: Int(size.height),
                 nominalFPS: fps, videoFrames: n, audioTracks: audio.count, audioSampleRate: rate, audioChannels: ch,
                 audioDuration: adur, videoCodec: codec, firstPTS: firstPTS ?? -1)
}

final class Collector: @unchecked Sendable {
    private let lock = NSLock()
    private var _events: [ExportProgress] = []
    var events: [ExportProgress] { lock.lock(); defer { lock.unlock() }; return _events }
    func add(_ e: ExportProgress) { lock.lock(); _events.append(e); lock.unlock() }
}

/// Writes a variable frame rate movie: frame `k` is shown at `pts[k]` seconds, gray `16 * k + 8`,
/// like a ScreenCaptureKit recording of a static screen (long gaps between frames).
func writeSparseVideo(to url: URL, pts: [Double], endTime: Double, width: Int = 320, height: Int = 180) async throws {
    let writer = try AVAssetWriter(outputURL: url, fileType: .mov)
    let input = AVAssetWriterInput(mediaType: .video, outputSettings: [
        AVVideoCodecKey: AVVideoCodecType.h264, AVVideoWidthKey: width, AVVideoHeightKey: height,
        AVVideoCompressionPropertiesKey: [AVVideoAllowFrameReorderingKey: false],
    ])
    let adaptor = AVAssetWriterInputPixelBufferAdaptor(assetWriterInput: input, sourcePixelBufferAttributes: [
        kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
        kCVPixelBufferWidthKey as String: width, kCVPixelBufferHeightKey as String: height,
    ])
    writer.add(input)
    writer.startWriting()
    writer.startSession(atSourceTime: .zero)
    for (k, t) in pts.enumerated() {
        while !input.isReadyForMoreMediaData { try await Task.sleep(for: .milliseconds(1)) }
        let pb = try PixelBuffers.make(width: width, height: height)
        CVPixelBufferLockBaseAddress(pb, [])
        memset(CVPixelBufferGetBaseAddress(pb)!, Int32(16 * k + 8), CVPixelBufferGetBytesPerRow(pb) * height)
        CVPixelBufferUnlockBaseAddress(pb, [])
        adaptor.append(pb, withPresentationTime: CMTime(seconds: t, preferredTimescale: 600))
    }
    input.markAsFinished()
    writer.endSession(atSourceTime: CMTime(seconds: endTime, preferredTimescale: 600))
    await writer.finishWriting()
    if writer.status != .completed { throw writer.error ?? NSSError.io("sparse video failed") }
}

/// Gray index (value / 16) of the middle pixel.
func grayIndex(_ pb: CVPixelBuffer) -> Int {
    CVPixelBufferLockBaseAddress(pb, .readOnly); defer { CVPixelBufferUnlockBaseAddress(pb, .readOnly) }
    let base = CVPixelBufferGetBaseAddress(pb)!.assumingMemoryBound(to: UInt8.self)
    let mid = CVPixelBufferGetBytesPerRow(pb) * (CVPixelBufferGetHeight(pb) / 2) + (CVPixelBufferGetWidth(pb) / 2) * 4
    return Int(base[mid]) / 16
}

/// A source that hands out frames but reports a decode error after `failAfter` requests,
/// like a reader that failed mid-file while still holding its last good frame.
final class FailingFrameSource: FrameSource, DecodeErrorReporting, @unchecked Sendable {
    private let lock = NSLock()
    private var count = 0
    private let failAfter: Int
    private let frame: CVPixelBuffer
    init(failAfter: Int) throws {
        self.failAfter = failAfter
        frame = try PixelBuffers.make(width: 64, height: 36)
    }
    private func bump() { lock.lock(); count += 1; lock.unlock() }
    func screenFrame(at t: MediaTime) async -> CVPixelBuffer? {
        bump()
        return frame
    }
    func cameraFrame(at t: MediaTime) async -> CVPixelBuffer? { nil }
    var screenError: NSSError? {
        lock.lock(); defer { lock.unlock() }
        return count > failAfter ? .io("simulated decode failure") : nil
    }
    var cameraError: NSSError? { nil }
}
