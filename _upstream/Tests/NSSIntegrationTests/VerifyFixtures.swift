import AVFoundation
import CoreVideo
import Foundation
import NSSCore
import NSSExport
import NSSMotion
import NSSProject
import NSSRender

/// A synthetic recording whose pixels, sound and cursor make timing and edges easy to read back.
///
/// * Screen: red = x ramp, green = y ramp, blue 90, with a bright moving band (blue 230, 200 px/s). The ramps tell
///   which source pixel an output pixel shows. During a flash the whole frame is white for `flashLength` seconds.
/// * Mic: silence, with a 1 kHz tone burst starting exactly when each flash starts (a clap and its flash).
/// * Camera: orange whose green channel climbs with time, so the camera frame choice is visible too.
struct VerifySpec {
    var duration = 2.5
    var trim = TrimRange(start: 0.4, end: 2.0)
    var clicks: [(t: Double, x: Double, y: Double)] = Synthetic.clicks
    var flashes: [Double] = []
    var mic = true
    var camera = true
    var style: Style = {
        var s = Style()
        s.background = .solid(Synthetic.background)
        s.camera = CameraBubble(visible: true, shape: .circle, position: NormalizedPoint(x: 0.88, y: 0.82), size: 0.18, border: nil, mirror: false)
        return s
    }()
    static let flashLength = 0.1
    static let burstLength = 0.08
}

/// Waypoint path through the clicks at 120 Hz, with a down/up pair per click and optional shape changes.
func verifyTrack(_ spec: VerifySpec, shapes: [CursorShapeChange] = []) -> CursorTrack {
    var keys: [(Double, Double, Double)] = [(0, 0.1, 0.1)] + spec.clicks.map { ($0.t, $0.x, $0.y) }
    keys.append((spec.duration, 0.9, 0.8))
    var samples: [CursorSample] = []
    for i in 0...Int(spec.duration * 120) {
        let t = Double(i) / 120
        var k = 0
        while k < keys.count - 2 && t > keys[k + 1].0 { k += 1 }
        let a = keys[k], b = keys[k + 1]
        let f = min(max((t - a.0) / max(b.0 - a.0, 1e-9), 0), 1)
        samples.append(CursorSample(t: t, x: a.1 + (b.1 - a.1) * f, y: a.2 + (b.2 - a.2) * f))
    }
    var events: [ClickEvent] = []
    for c in spec.clicks {
        events.append(ClickEvent(t: c.t, x: c.x, y: c.y, kind: .down))
        events.append(ClickEvent(t: c.t + 0.08, x: c.x, y: c.y, kind: .up))
    }
    return CursorTrack(samples: samples, clicks: events, sampleRate: 120, shapes: shapes)
}

/// Pixels of the static part of a screen frame: red = x ramp, green = y ramp, blue 90 (BGRA).
private func rampFrame(_ w: Int, _ h: Int) -> [UInt8] {
    var out = [UInt8](repeating: 255, count: w * h * 4)
    for y in 0..<h {
        let g = UInt8(y * 255 / (h - 1))
        for x in 0..<w {
            let o = (y * w + x) * 4
            out[o] = 90; out[o + 1] = g; out[o + 2] = UInt8(x * 255 / (w - 1))
        }
    }
    return out
}

func isFlash(_ t: Double, flashes: [Double]) -> Bool {
    flashes.contains { t >= $0 - 1e-9 && t < $0 + VerifySpec.flashLength - 1e-9 }
}

func writeBurstAudio(to url: URL, seconds: Double, starts: [Double], sampleRate: Double = 44_100) throws {
    let format = AVAudioFormat(standardFormatWithSampleRate: sampleRate, channels: 1)!
    let file = try AVAudioFile(forWriting: url, settings: [
        AVFormatIDKey: kAudioFormatMPEG4AAC, AVSampleRateKey: sampleRate, AVNumberOfChannelsKey: 1,
    ])
    let count = AVAudioFrameCount(seconds * sampleRate)
    let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: count)!
    buffer.frameLength = count
    let p = buffer.floatChannelData![0]
    for n in 0..<Int(count) { p[n] = 0 }
    for s in starts {
        let from = Int((s * sampleRate).rounded())
        for k in 0..<Int(VerifySpec.burstLength * sampleRate) where from + k < Int(count) {
            p[from + k] = 0.5 * sinf(Float(k) * 2 * .pi * 1000 / Float(sampleRate))
        }
    }
    try file.write(from: buffer)
}

func makeVerifyBundle(_ spec: VerifySpec, shapes: [CursorShapeChange] = []) async throws -> E2EBundle {
    let library = FileManager.default.temporaryDirectory.appendingPathComponent("nss-verify-\(UUID().uuidString)")
    let store = FileProjectStore(libraryURL: library)
    let ref = try await store.createProject(named: "Verify")
    let s = Synthetic.size
    let base = rampFrame(s.width, s.height)
    let flashes = spec.flashes
    try await writeMovie(
        to: ref.url.appendingPathComponent("screen.mov"), size: s, times: variableTimes(duration: spec.duration), endTime: spec.duration
    ) { b, stride, _, t in
        base.withUnsafeBufferPointer { src in
            for y in 0..<s.height { memcpy(b + y * stride, src.baseAddress! + y * s.width * 4, s.width * 4) }
        }
        if isFlash(t, flashes: flashes) {
            for y in 0..<s.height { memset(b + y * stride, 255, s.width * 4) }
        } else {
            let band = Int((t * 200).truncatingRemainder(dividingBy: Double(s.width)))
            for y in 0..<s.height { for x in band..<min(band + 12, s.width) { b[y * stride + x * 4] = 230 } }
        }
    }
    if spec.camera {
        let cs = Synthetic.cameraSize, c = Synthetic.cameraColor
        try await writeMovie(
            to: ref.url.appendingPathComponent("camera.mov"), size: cs,
            times: (0..<Int(spec.duration * 30)).map { Double($0) / 30 }, endTime: spec.duration
        ) { b, st, _, t in
            let g = UInt8(min(255, t / spec.duration * 255))
            for x in 0..<cs.width { let p = b + x * 4; p[0] = UInt8(c.b * 255); p[1] = g; p[2] = UInt8(c.r * 255); p[3] = 255 }
            for y in 1..<cs.height { memcpy(b + y * st, b, cs.width * 4) }
        }
    }
    if spec.mic { try writeBurstAudio(to: ref.url.appendingPathComponent("mic.m4a"), seconds: spec.duration, starts: spec.flashes) }

    let assets = RecordingAssets(
        cameraFile: spec.camera ? "camera.mov" : nil, micFile: spec.mic ? "mic.m4a" : nil,
        sourcePixelSize: s, frameRate: 60, duration: spec.duration, startHostTime: 1234.5)
    var style = spec.style
    style.camera.visible = spec.camera
    var project = Project(name: "Verify", assets: assets, timeline: Timeline(trim: spec.trim), style: style)
    project.modifiedAt = Date()
    let track = verifyTrack(spec, shapes: shapes)
    try await store.saveCursorTrack(track, for: ref, assets: assets)
    try await store.save(project, to: ref)
    let loaded = try await store.load(ref)
    let loadedTrack = try await store.loadCursorTrack(for: ref, assets: loaded.assets)
    return E2EBundle(dir: ref.url, ref: ref, store: store, project: loaded, track: loadedTrack)
}

// MARK: - Reading pixels

/// A decoded frame copied out as tightly packed BGRA, so a test can index it without locking.
struct Frame: Sendable {
    let width: Int, height: Int
    var bytes: [UInt8]

    init(_ pb: CVPixelBuffer) {
        width = CVPixelBufferGetWidth(pb); height = CVPixelBufferGetHeight(pb)
        bytes = [UInt8](repeating: 0, count: width * height * 4)
        CVPixelBufferLockBaseAddress(pb, .readOnly)
        defer { CVPixelBufferUnlockBaseAddress(pb, .readOnly) }
        let base = CVPixelBufferGetBaseAddress(pb)!.assumingMemoryBound(to: UInt8.self)
        let stride = CVPixelBufferGetBytesPerRow(pb)
        bytes.withUnsafeMutableBufferPointer { dst in
            for y in 0..<height { memcpy(dst.baseAddress! + y * width * 4, base + y * stride, width * 4) }
        }
    }

    private init(width: Int, height: Int, bytes: [UInt8]) { self.width = width; self.height = height; self.bytes = bytes }

    /// Every `step`-th pixel in both axes.
    func subsampled(_ step: Int) -> Frame {
        let w = width / step, h = height / step
        var out = [UInt8](repeating: 0, count: w * h * 4)
        for y in 0..<h { for x in 0..<w {
            let s = ((y * step) * width + x * step) * 4, d = (y * w + x) * 4
            out[d] = bytes[s]; out[d + 1] = bytes[s + 1]; out[d + 2] = bytes[s + 2]; out[d + 3] = bytes[s + 3]
        } }
        return Frame(width: w, height: h, bytes: out)
    }

    func px(_ x: Int, _ y: Int) -> (r: Int, g: Int, b: Int) {
        let o = (min(max(y, 0), height - 1) * width + min(max(x, 0), width - 1)) * 4
        return (Int(bytes[o + 2]), Int(bytes[o + 1]), Int(bytes[o]))
    }

    /// Mean over a (2r+1)^2 window.
    func mean(_ cx: Int, _ cy: Int, r: Int = 1) -> (r: Double, g: Double, b: Double) {
        var sr = 0, sg = 0, sb = 0, n = 0
        for y in (cy - r)...(cy + r) { for x in (cx - r)...(cx + r) { let p = px(x, y); sr += p.r; sg += p.g; sb += p.b; n += 1 } }
        return (Double(sr) / Double(n), Double(sg) / Double(n), Double(sb) / Double(n))
    }
}

/// Calls `body(index, pts, pixelBuffer)` for every frame of a video file in presentation order (decoded as BGRA).
/// Streaming, so a long clip never has to fit in memory.
func forEachFrame(_ url: URL, _ body: (Int, Double, CVPixelBuffer) async throws -> Void) async throws {
    let asset = AVURLAsset(url: url)
    let video = try await asset.loadTracks(withMediaType: .video)[0]
    let reader = try AVAssetReader(asset: asset)
    let out = AVAssetReaderTrackOutput(track: video, outputSettings: [
        kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
    ])
    reader.add(out)
    reader.startReading()
    var i = 0
    while let s = out.copyNextSampleBuffer() {
        guard let pb = CMSampleBufferGetImageBuffer(s) else { continue }
        try await body(i, CMSampleBufferGetPresentationTimeStamp(s).seconds, pb)
        i += 1
    }
}

/// Mono float samples of the first audio track, and their sample rate.
func decodeAudio(_ url: URL) async throws -> (samples: [Float], rate: Double) {
    let asset = AVURLAsset(url: url)
    let track = try await asset.loadTracks(withMediaType: .audio)[0]
    let reader = try AVAssetReader(asset: asset)
    let rate = 48_000.0
    let out = AVAssetReaderTrackOutput(track: track, outputSettings: [
        AVFormatIDKey: kAudioFormatLinearPCM, AVSampleRateKey: rate, AVNumberOfChannelsKey: 1,
        AVLinearPCMBitDepthKey: 32, AVLinearPCMIsFloatKey: true, AVLinearPCMIsBigEndianKey: false, AVLinearPCMIsNonInterleaved: false,
    ])
    reader.add(out)
    reader.startReading()
    var samples: [Float] = []
    while let s = out.copyNextSampleBuffer() {
        guard let block = CMSampleBufferGetDataBuffer(s) else { continue }
        let n = CMBlockBufferGetDataLength(block) / 4
        var chunk = [Float](repeating: 0, count: n)
        chunk.withUnsafeMutableBytes { _ = CMBlockBufferCopyDataBytes(block, atOffset: 0, dataLength: n * 4, destination: $0.baseAddress!) }
        samples.append(contentsOf: chunk)
    }
    return (samples, rate)
}

/// Times (seconds) at which a tone burst starts: the first sample of each run whose 1 ms RMS reaches half the
/// burst amplitude after at least `quiet` seconds of silence.
func burstOnsets(_ audio: (samples: [Float], rate: Double)) -> [Double] {
    let win = Int(audio.rate / 1000)
    var onsets: [Double] = []
    var lastLoud = -Double.infinity
    var i = 0
    while i + win <= audio.samples.count {
        var e: Float = 0
        for k in 0..<win { e += audio.samples[i + k] * audio.samples[i + k] }
        let rms = (e / Float(win)).squareRoot()
        let t = Double(i) / audio.rate
        if rms > 0.17 {   // burst amplitude 0.5 -> RMS 0.35; half of that
            if t - lastLoud > 0.2 { onsets.append(t) }
            lastLoud = t
        }
        i += win / 4
    }
    return onsets
}

/// PSNR in dB between two frames over the (r, g, b) channels, sampling every `step`-th pixel in both axes.
func psnr(_ a: Frame, _ b: Frame, step: Int = 3) -> (psnr: Double, maxError: Int) {
    var sum = 0.0, n = 0.0, worst = 0
    var y = 0
    while y < a.height {
        var x = 0
        while x < a.width {
            let o = (y * a.width + x) * 4
            for c in 0..<3 {
                let d = Int(a.bytes[o + c]) - Int(b.bytes[o + c])
                sum += Double(d * d); n += 1; worst = max(worst, abs(d))
            }
            x += step
        }
        y += step
    }
    let mse = sum / n
    return (mse == 0 ? 99 : 10 * log10(255 * 255 / mse), worst)
}

/// Renders what the exporter would render at source time `t` (frames from `AssetFrameSource`, the given motion model)
/// into a fresh buffer of `size` and copies it out.
func directRender(
    source: AssetFrameSource, renderer: any FrameRenderer, motion: any MotionModel, style: Style, t: Double, size: PixelSize
) async throws -> Frame {
    guard let screen = await source.screenFrame(at: t) else { throw NSSError.io("no screen frame at \(t)") }
    let camera = await source.cameraFrame(at: t)
    let out = try PixelBuffers.make(width: size.width, height: size.height)
    try renderer.render(screen: screen, camera: camera, state: motion.frameState(at: t), style: style, into: out)
    return Frame(out)
}
