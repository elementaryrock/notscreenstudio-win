import AVFoundation
import CoreMedia
import CoreVideo
import Foundation
import NSSCore
import NSSMotion
import NSSRender

/// 60 fps (or `preset.fps`) MP4 exporter.
///
/// Pipeline per output frame `i`: source time `t = trim.start + i / fps`, then `motion.frameState(at: t)`,
/// decode the at-or-before screen/camera frames, `renderer.render` into a buffer taken from the writer's
/// pixel buffer pool, append. Frames are written strictly one at a time, gated on `isReadyForMoreMediaData`,
/// so memory stays bounded. The work runs on the caller's task (never the main actor unless the caller is).
///
/// Output goes to a temporary file next to the destination and is moved into place only after a clean
/// finish, so a crash, error or cancellation never leaves a partial file at `outputURL`.
public struct MP4Exporter: Exporter {
    public typealias FrameSourceFactory = @Sendable (_ screenURL: URL, _ cameraURL: URL?) async throws -> any FrameSource

    public let motionFactory: MotionModelFactory
    /// When set, the motion model is built for the recording's real source aspect (width / height of
    /// `assets.sourcePixelSize`) instead of `motionFactory`, so auto-zoom matches the preview for any source.
    public let motionFactoryForAspect: (@Sendable (_ sourceAspect: Double) -> MotionModelFactory)?
    public let renderer: any FrameRenderer
    public let frameSourceFactory: FrameSourceFactory

    public init(
        motionFactory: @escaping MotionModelFactory = LinearMotionModel.factory,
        renderer: any FrameRenderer = PassthroughRenderer(),
        motionFactoryForAspect: (@Sendable (_ sourceAspect: Double) -> MotionModelFactory)? = nil,
        frameSourceFactory: @escaping FrameSourceFactory = { screen, camera in
            try await AssetFrameSource.make(screenURL: screen, cameraURL: camera)
        }
    ) {
        self.motionFactory = motionFactory
        self.motionFactoryForAspect = motionFactoryForAspect
        self.renderer = renderer
        self.frameSourceFactory = frameSourceFactory
    }

    public func export(
        project: Project,
        projectURL: URL,
        track: CursorTrack,
        preset: ExportPreset,
        to outputURL: URL,
        progress: @escaping @Sendable (ExportProgress) -> Void
    ) async throws {
        do {
            try await run(project: project, projectURL: projectURL, track: track, preset: preset, to: outputURL, progress: progress)
            progress(.finished(outputURL))
        } catch is CancellationError {
            progress(.cancelled)
            throw CancellationError()
        } catch {
            let mapped = (error as? NSSError) ?? .io(error.localizedDescription)
            progress(.failed(message: Self.message(for: mapped)))
            throw mapped
        }
    }

    // MARK: - Pipeline

    private func run(
        project: Project,
        projectURL: URL,
        track: CursorTrack,
        preset: ExportPreset,
        to outputURL: URL,
        progress: @escaping @Sendable (ExportProgress) -> Void
    ) async throws {
        guard preset.fps > 0, preset.fps <= 240 else { throw NSSError.unsupported("Unsupported frame rate \(preset.fps)") }
        let fps = Int32(preset.fps)
        let assets = project.assets
        let screenURL = projectURL.appendingPathComponent(assets.screenFile)
        guard FileManager.default.fileExists(atPath: screenURL.path) else {
            throw NSSError.invalidProject("Missing screen recording \(assets.screenFile)")
        }
        let cameraURL = assets.cameraFile.map { projectURL.appendingPathComponent($0) }
            .flatMap { FileManager.default.fileExists(atPath: $0.path) ? $0 : nil }
        let micURL = assets.micFile.map { projectURL.appendingPathComponent($0) }

        // Time range: the file's real duration wins over metadata when it can be read.
        let actual = (try? await AVURLAsset(url: screenURL).load(.duration).seconds) ?? 0
        let duration = actual.isFinite && actual > 0 ? actual : assets.duration
        let trim = project.timeline.trim
        let start = min(max(0, trim.start), duration)
        let end = trim.resolvedEnd(duration: duration)
        let length = end - start
        guard length > 0 else { throw NSSError.invalidProject("The trim range is empty") }
        let totalFrames = max(1, Int((length * Double(fps)).rounded()))

        let size = preset.outputSize(for: project.style.aspectRatio)
        // Same asset resolution as the preview, so bundle-relative background images draw in both.
        let renderStyle = project.style.resolvingAssets(relativeTo: projectURL)
        let factory = motionFactoryForAspect?(assets.sourcePixelSize.aspect) ?? motionFactory
        let motion = factory(track, project.timeline, project.style.cursor, duration)
        let frames = try await frameSourceFactory(screenURL, cameraURL)
        let audio = try await AudioPump.make(urls: [micURL, screenURL].compactMap { $0 }, start: start, length: length)

        // Writer setup. All files go to a temp sibling, then move.
        let outputDir = outputURL.deletingLastPathComponent()
        try FileManager.default.createDirectory(at: outputDir, withIntermediateDirectories: true)
        let tempURL = outputDir.appendingPathComponent(".\(outputURL.lastPathComponent).\(UUID().uuidString).partial.mp4")
        let writer: AVAssetWriter
        do { writer = try AVAssetWriter(outputURL: tempURL, fileType: .mp4) } catch {
            throw NSSError.io("Cannot create the output file: \(error.localizedDescription)")
        }
        var finished = false
        defer {
            if !finished {
                if writer.status == .writing { writer.cancelWriting() }
                audio?.cancel()
                Self.removeTemp(tempURL)
            }
        }
        writer.shouldOptimizeForNetworkUse = true

        let videoSettings = Self.videoSettings(size: size, preset: preset)
        guard writer.canApply(outputSettings: videoSettings, forMediaType: .video) else {
            throw NSSError.unsupported("This Mac cannot encode \(preset.codec.rawValue) at \(size.width)x\(size.height)")
        }
        let videoInput = AVAssetWriterInput(mediaType: .video, outputSettings: videoSettings)
        videoInput.expectsMediaDataInRealTime = false
        let adaptor = AVAssetWriterInputPixelBufferAdaptor(assetWriterInput: videoInput, sourcePixelBufferAttributes: [
            kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
            kCVPixelBufferWidthKey as String: size.width,
            kCVPixelBufferHeightKey as String: size.height,
            kCVPixelBufferIOSurfacePropertiesKey as String: [:] as [String: Any],
            kCVPixelBufferMetalCompatibilityKey as String: true,
        ])
        guard writer.canAdd(videoInput) else { throw NSSError.unsupported("Cannot add a video track to the output") }
        writer.add(videoInput)
        if let audio {
            guard writer.canAdd(audio.input) else { throw NSSError.unsupported("Cannot add an audio track to the output") }
            writer.add(audio.input)
        }

        try audio?.startReading()
        guard writer.startWriting() else { throw Self.writerError(writer, "Cannot start writing") }
        writer.startSession(atSourceTime: .zero)
        guard let pool = adaptor.pixelBufferPool else { throw NSSError.renderFailed("The encoder gave no pixel buffer pool") }

        progress(.running(fraction: 0, framesWritten: 0, totalFrames: totalFrames))
        let reportEvery = max(1, totalFrames / 100)

        for i in 0..<totalFrames {
            try Task.checkCancellation()
            let t = start + Double(i) / Double(fps)
            let state = motion.frameState(at: t)
            let screenFrame = await frames.screenFrame(at: t)
            let camera = await frames.cameraFrame(at: t)
            // A decode failure must abort the export, even if the source still hands back a held frame.
            if let reporter = frames as? any DecodeErrorReporting, let e = reporter.screenError ?? reporter.cameraError { throw e }
            guard let screen = screenFrame else { throw NSSError.io("No screen frame at \(t) s") }

            try await waitUntilReady(videoInput, writer: writer)
            var out: CVPixelBuffer?
            let status = CVPixelBufferPoolCreatePixelBuffer(nil, pool, &out)
            guard status == kCVReturnSuccess, let out else { throw NSSError.renderFailed("Pixel buffer pool failed: \(status)") }
            try renderer.render(screen: screen, camera: camera, state: state, style: renderStyle, into: out)
            guard adaptor.append(out, withPresentationTime: CMTime(value: CMTimeValue(i), timescale: fps)) else {
                throw Self.writerError(writer, "Video write failed")
            }
            try audio?.drain(upTo: Double(i) / Double(fps) + 1, writer: writer)

            let written = i + 1
            if written % reportEvery == 0 || written == totalFrames {
                progress(.running(fraction: Double(written) / Double(totalFrames), framesWritten: written, totalFrames: totalFrames))
            }
        }

        videoInput.markAsFinished()
        if let audio {
            while !audio.isFinished {
                try Task.checkCancellation()
                if writer.status == .failed || writer.status == .cancelled { throw Self.writerError(writer, "Writer stopped") }
                try audio.drain(upTo: nil, writer: writer)
                if !audio.isFinished { try await Task.sleep(for: .milliseconds(1)) }
            }
        }
        writer.endSession(atSourceTime: CMTime(value: CMTimeValue(totalFrames), timescale: fps))
        await writer.finishWriting()
        guard writer.status == .completed else { throw Self.writerError(writer, "Finishing the file failed") }

        try Task.checkCancellation()
        if FileManager.default.fileExists(atPath: outputURL.path) {
            _ = try FileManager.default.replaceItemAt(outputURL, withItemAt: tempURL)
        } else {
            try FileManager.default.moveItem(at: tempURL, to: outputURL)
        }
        finished = true
    }

    private func waitUntilReady(_ input: AVAssetWriterInput, writer: AVAssetWriter) async throws {
        while !input.isReadyForMoreMediaData {
            if writer.status == .failed || writer.status == .cancelled { throw Self.writerError(writer, "Writer stopped") }
            try await Task.sleep(for: .milliseconds(1))
        }
    }

    // MARK: - Settings

    /// Bits per pixel per frame, tuned for screen content (flat areas, sharp text).
    static func bitrate(size: PixelSize, fps: Int, codec: VideoCodec) -> Int {
        let bitsPerPixel = codec == .hevc ? 0.07 : 0.12
        let raw = Double(size.width * size.height * fps) * bitsPerPixel
        return Int(min(max(raw, 4_000_000), 80_000_000))
    }

    static func videoSettings(size: PixelSize, preset: ExportPreset) -> [String: Any] {
        var compression: [String: Any] = [
            AVVideoAverageBitRateKey: bitrate(size: size, fps: preset.fps, codec: preset.codec),
            AVVideoExpectedSourceFrameRateKey: preset.fps,
            AVVideoMaxKeyFrameIntervalKey: preset.fps * 2,
            // No B-frames: presentation times start at 0 and match decode order, so no edit list is needed.
            AVVideoAllowFrameReorderingKey: false,
        ]
        if preset.codec == .h264 { compression[AVVideoProfileLevelKey] = AVVideoProfileLevelH264HighAutoLevel }
        return [
            AVVideoCodecKey: preset.codec == .hevc ? AVVideoCodecType.hevc : AVVideoCodecType.h264,
            AVVideoWidthKey: size.width,
            AVVideoHeightKey: size.height,
            AVVideoCompressionPropertiesKey: compression,
            AVVideoEncoderSpecificationKey: ["EnableHardwareAcceleratedVideoEncoder": true],
            AVVideoColorPropertiesKey: [
                AVVideoColorPrimariesKey: AVVideoColorPrimaries_ITU_R_709_2,
                AVVideoTransferFunctionKey: AVVideoTransferFunction_ITU_R_709_2,
                AVVideoYCbCrMatrixKey: AVVideoYCbCrMatrix_ITU_R_709_2,
            ],
        ]
    }

    /// Removes the temp file and any side file AVAssetWriter created next to it (it may leave `.sb-` files).
    private static func removeTemp(_ url: URL) {
        let fm = FileManager.default
        try? fm.removeItem(at: url)
        let dir = url.deletingLastPathComponent()
        for name in (try? fm.contentsOfDirectory(atPath: dir.path)) ?? [] where name.hasPrefix(url.lastPathComponent) {
            try? fm.removeItem(at: dir.appendingPathComponent(name))
        }
    }

    // MARK: - Errors

    private static func writerError(_ writer: AVAssetWriter, _ what: String) -> NSSError {
        .io("\(what): \(writer.error?.localizedDescription ?? "unknown error")")
    }

    private static func message(for error: NSSError) -> String { error.messageText }
}

extension NSSError {
    /// Human-readable text of the payload.
    var messageText: String {
        switch self {
        case .notImplemented(let m), .unsupported(let m), .permissionDenied(let m),
             .invalidProject(let m), .renderFailed(let m), .io(let m):
            return m
        }
    }
}
