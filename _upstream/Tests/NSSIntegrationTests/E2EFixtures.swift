import AVFoundation
import CoreVideo
import Foundation
import NSSCore
import NSSProject
import NSSRender
import VideoToolbox

/// Whether this machine has an H.264 encoder (CI VMs may not).
let h264Encodable: Bool = {
    var list: CFArray?
    guard VTCopyVideoEncoderList(nil, &list) == noErr, let infos = list as? [[String: Any]] else { return false }
    return infos.contains { ($0[kVTVideoEncoderList_CodecType as String] as? CMVideoCodecType) == kCMVideoCodecType_H264 }
}()

let gpuAvailable = MetalRenderer.isSupported
let e2eAvailable = h264Encodable && gpuAvailable
let env = ProcessInfo.processInfo.environment
let run4K = env["NSS_E2E_4K"] == "1"
let runBench = env["NSS_BENCH"] == "1"

/// Geometry of the synthetic recording.
enum Synthetic {
    static let size = PixelSize(width: 960, height: 540)
    static let cameraSize = PixelSize(width: 320, height: 240)
    static let duration = 2.5
    /// Clicks (down, then up 80 ms later).
    static let clicks: [(t: Double, x: Double, y: Double)] = [(0.8, 0.25, 0.30), (1.6, 0.70, 0.60)]
    static let trim = TrimRange(start: 0.4, end: 2.0)
    static let cameraColor = RGBAColor(r: 0.9, g: 0.45, b: 0.08)
    static let background = RGBAColor(r: 0.10, g: 0.50, b: 0.90)

    /// Cursor path at 120 Hz: a smooth sweep that passes through every click position at its time.
    static func cursorTrack() -> CursorTrack {
        var samples: [CursorSample] = []
        let n = Int(duration * 120)
        for i in 0...n {
            let t = Double(i) / 120
            // Piecewise-linear through the click points, so the cursor is where the click happens.
            let keys: [(Double, Double, Double)] = [(0, 0.1, 0.1)] + clicks.map { ($0.t, $0.x, $0.y) } + [(duration, 0.9, 0.8)]
            var k = 0
            while k < keys.count - 2 && t > keys[k + 1].0 { k += 1 }
            let a = keys[k], b = keys[k + 1]
            let f = min(max((t - a.0) / (b.0 - a.0), 0), 1)
            samples.append(CursorSample(t: t, x: a.1 + (b.1 - a.1) * f, y: a.2 + (b.2 - a.2) * f))
        }
        var events: [ClickEvent] = []
        for c in clicks {
            events.append(ClickEvent(t: c.t, x: c.x, y: c.y, kind: .down))
            events.append(ClickEvent(t: c.t + 0.08, x: c.x, y: c.y, kind: .up))
        }
        return CursorTrack(samples: samples, clicks: events, sampleRate: 120)
    }
}

/// Writes an H.264 movie with ScreenCaptureKit-like timing: nominally 60 fps, but with jittered presentation
/// times and dropped frames, ending with a long gap. `draw(frameIndex, time)` fills one frame.
func writeMovie(
    to url: URL, size: PixelSize, times: [Double], endTime: Double,
    draw: (_ base: UnsafeMutablePointer<UInt8>, _ stride: Int, _ index: Int, _ t: Double) -> Void
) async throws {
    let writer = try AVAssetWriter(outputURL: url, fileType: .mov)
    let input = AVAssetWriterInput(mediaType: .video, outputSettings: [
        AVVideoCodecKey: AVVideoCodecType.h264, AVVideoWidthKey: size.width, AVVideoHeightKey: size.height,
        AVVideoCompressionPropertiesKey: [AVVideoAllowFrameReorderingKey: false, AVVideoAverageBitRateKey: 6_000_000],
    ])
    let adaptor = AVAssetWriterInputPixelBufferAdaptor(assetWriterInput: input, sourcePixelBufferAttributes: [
        kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
        kCVPixelBufferWidthKey as String: size.width, kCVPixelBufferHeightKey as String: size.height,
    ])
    writer.add(input)
    guard writer.startWriting() else { throw writer.error ?? NSSError.io("cannot start writer") }
    writer.startSession(atSourceTime: .zero)
    for (i, t) in times.enumerated() {
        while !input.isReadyForMoreMediaData { try await Task.sleep(for: .milliseconds(1)) }
        let pb = try PixelBuffers.make(width: size.width, height: size.height)
        CVPixelBufferLockBaseAddress(pb, [])
        draw(CVPixelBufferGetBaseAddress(pb)!.assumingMemoryBound(to: UInt8.self), CVPixelBufferGetBytesPerRow(pb), i, t)
        CVPixelBufferUnlockBaseAddress(pb, [])
        if !adaptor.append(pb, withPresentationTime: CMTime(seconds: t, preferredTimescale: 6000)) {
            throw writer.error ?? NSSError.io("append failed")
        }
    }
    input.markAsFinished()
    writer.endSession(atSourceTime: CMTime(seconds: endTime, preferredTimescale: 6000))
    await writer.finishWriting()
    if writer.status != .completed { throw writer.error ?? NSSError.io("movie failed") }
}

/// 60 fps ticks with deterministic jitter and ~8% dropped frames.
func variableTimes(duration: Double) -> [Double] {
    var out: [Double] = []
    var seed: UInt64 = 12345
    func rnd() -> Double { seed = seed &* 6364136223846793005 &+ 1442695040888963407; return Double(seed >> 33) / Double(1 << 31) }
    var k = 0
    while Double(k) / 60 < duration - 0.05 {
        let drop = k > 0 && rnd() < 0.08
        if !drop { out.append(max(0, Double(k) / 60 + (rnd() - 0.5) * 0.004)) }
        k += 1
    }
    out[0] = 0
    return out
}

/// Left-to-right red ramp (red = x / width), a moving green band, constant blue. The ramp lets tests read
/// back which source column an output pixel shows.
func drawScreen(_ base: UnsafeMutablePointer<UInt8>, _ stride: Int, _ i: Int, _ t: Double, _ w: Int, _ h: Int) {
    let band = Int((t * 200).truncatingRemainder(dividingBy: Double(w)))
    for x in 0..<w {
        let p = base + x * 4
        p[0] = 90
        p[1] = abs(x - band) < 12 ? 255 : 40
        p[2] = UInt8(min(255, x * 255 / (w - 1)))
        p[3] = 255
    }
    // Every row is the same: copy the first (keeps debug builds fast).
    for y in 1..<h { memcpy(base + y * stride, base, w * 4) }
}

struct E2EBundle {
    let dir: URL
    let ref: ProjectReference
    let store: FileProjectStore
    var project: Project
    var track: CursorTrack
}

/// Creates the whole bundle through `FileProjectStore`, then loads it back the way the editor does.
func makeE2EBundle(mic: Bool = true, camera: Bool = true, duration: Double = Synthetic.duration, trim: TrimRange = Synthetic.trim) async throws -> E2EBundle {
    let library = FileManager.default.temporaryDirectory.appendingPathComponent("nss-e2e-\(UUID().uuidString)")
    let store = FileProjectStore(libraryURL: library)
    let ref = try await store.createProject(named: "E2E")
    let s = Synthetic.size
    try await writeMovie(
        to: ref.url.appendingPathComponent("screen.mov"), size: s, times: variableTimes(duration: duration),
        endTime: duration
    ) { b, st, i, t in drawScreen(b, st, i, t, s.width, s.height) }
    if camera {
        let c = Synthetic.cameraColor
        let cs = Synthetic.cameraSize
        try await writeMovie(
            to: ref.url.appendingPathComponent("camera.mov"), size: cs,
            times: (0..<Int(duration * 30)).map { Double($0) / 30 }, endTime: duration
        ) { b, st, _, _ in
            for x in 0..<cs.width {
                let p = b + x * 4
                p[0] = UInt8(c.b * 255); p[1] = UInt8(c.g * 255); p[2] = UInt8(c.r * 255); p[3] = 255
            }
            for y in 1..<cs.height { memcpy(b + y * st, b, cs.width * 4) }
        }
    }
    if mic { try writeTone(to: ref.url.appendingPathComponent("mic.m4a"), seconds: duration) }

    let assets = RecordingAssets(
        cameraFile: camera ? "camera.mov" : nil, micFile: mic ? "mic.m4a" : nil,
        sourcePixelSize: s, frameRate: 60, duration: duration, startHostTime: 1234.5)
    var style = Style()
    style.background = .solid(Synthetic.background)
    style.camera = CameraBubble(visible: camera, shape: .circle, position: NormalizedPoint(x: 0.88, y: 0.82), size: 0.18, border: nil, mirror: false)
    var project = Project(name: "E2E", assets: assets, timeline: Timeline(trim: trim), style: style)
    project.modifiedAt = Date()
    try await store.saveCursorTrack(Synthetic.cursorTrack(), for: ref, assets: assets)
    try await store.save(project, to: ref)

    // Reload exactly as the editor does.
    let loaded = try await store.load(ref)
    let track = try await store.loadCursorTrack(for: ref, assets: loaded.assets)
    return E2EBundle(dir: ref.url, ref: ref, store: store, project: loaded, track: track)
}

func writeTone(to url: URL, seconds: Double, sampleRate: Double = 44_100) throws {
    let format = AVAudioFormat(standardFormatWithSampleRate: sampleRate, channels: 1)!
    let file = try AVAudioFile(forWriting: url, settings: [
        AVFormatIDKey: kAudioFormatMPEG4AAC, AVSampleRateKey: sampleRate, AVNumberOfChannelsKey: 1,
    ])
    let count = AVAudioFrameCount(seconds * sampleRate)
    let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: count)!
    buffer.frameLength = count
    let p = buffer.floatChannelData![0]
    for n in 0..<Int(count) { p[n] = 0.3 * sinf(Float(n) * 2 * .pi * 440 / Float(sampleRate)) }
    try file.write(from: buffer)
}

struct VideoProbe {
    var width = 0, height = 0
    var duration = 0.0
    var nominalFPS: Float = 0
    var frameTimes: [Double] = []
    var audioTracks = 0
    var audioDuration = 0.0
}

func probe(_ url: URL) async throws -> VideoProbe {
    let asset = AVURLAsset(url: url)
    var p = VideoProbe()
    let video = try await asset.loadTracks(withMediaType: .video)[0]
    let size = try await video.load(.naturalSize)
    p.width = Int(size.width); p.height = Int(size.height)
    p.nominalFPS = try await video.load(.nominalFrameRate)
    p.duration = try await asset.load(.duration).seconds
    let reader = try AVAssetReader(asset: asset)
    let out = AVAssetReaderTrackOutput(track: video, outputSettings: nil)
    reader.add(out)
    reader.startReading()
    while let s = out.copyNextSampleBuffer() {
        if CMSampleBufferGetNumSamples(s) > 0 { p.frameTimes.append(CMSampleBufferGetPresentationTimeStamp(s).seconds) }
    }
    let audio = try await asset.loadTracks(withMediaType: .audio)
    p.audioTracks = audio.count
    if let a = audio.first { p.audioDuration = try await a.load(.timeRange).duration.seconds }
    return p
}

/// Decodes the frame at-or-before `t` of an MP4 into BGRA.
func decodeFrame(_ url: URL, at t: Double) async throws -> CVPixelBuffer {
    let asset = AVURLAsset(url: url)
    let video = try await asset.loadTracks(withMediaType: .video)[0]
    let reader = try AVAssetReader(asset: asset)
    let out = AVAssetReaderTrackOutput(track: video, outputSettings: [
        kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
    ])
    reader.add(out)
    reader.startReading()
    var best: CVPixelBuffer?
    while let s = out.copyNextSampleBuffer() {
        guard CMSampleBufferGetPresentationTimeStamp(s).seconds <= t + 1e-6 else { break }
        if let pb = CMSampleBufferGetImageBuffer(s) { best = pb }
    }
    reader.cancelReading()
    guard let best else { throw NSSError.io("no frame at \(t)") }
    return best
}

struct Px { var r: Int, g: Int, b: Int }

func pixel(_ pb: CVPixelBuffer, _ x: Int, _ y: Int) -> Px {
    CVPixelBufferLockBaseAddress(pb, .readOnly)
    defer { CVPixelBufferUnlockBaseAddress(pb, .readOnly) }
    let base = CVPixelBufferGetBaseAddress(pb)!.assumingMemoryBound(to: UInt8.self)
    let p = base + y * CVPixelBufferGetBytesPerRow(pb) + x * 4
    return Px(r: Int(p[2]), g: Int(p[1]), b: Int(p[0]))
}

/// Mean over a small window, to ride over H.264 noise.
func meanPixel(_ pb: CVPixelBuffer, _ cx: Int, _ cy: Int, radius: Int = 3) -> Px {
    var r = 0, g = 0, b = 0, n = 0
    let w = CVPixelBufferGetWidth(pb), h = CVPixelBufferGetHeight(pb)
    for y in max(0, cy - radius)...min(h - 1, cy + radius) {
        for x in max(0, cx - radius)...min(w - 1, cx + radius) {
            let p = pixel(pb, x, y)
            r += p.r; g += p.g; b += p.b; n += 1
        }
    }
    return Px(r: r / n, g: g / n, b: b / n)
}

func near(_ a: Px, _ r: Double, _ g: Double, _ b: Double, tol: Int) -> Bool {
    abs(a.r - Int(r * 255)) <= tol && abs(a.g - Int(g * 255)) <= tol && abs(a.b - Int(b * 255)) <= tol
}
