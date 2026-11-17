import AVFoundation
import CoreMedia
import Foundation
import NSSCore

/// Mixes the mic and system audio of a project, already trimmed, and feeds it to an `AVAssetWriterInput`
/// as AAC 48 kHz stereo. Output time 0 is the trim start, so it lines up with the video.
final class AudioPump {
    static let sampleRate: Double = 48_000
    static let channels = 2

    let input: AVAssetWriterInput
    private let reader: AVAssetReader
    private let output: AVAssetReaderAudioMixOutput
    private var pending: CMSampleBuffer?
    private(set) var isFinished = false

    private init(reader: AVAssetReader, output: AVAssetReaderAudioMixOutput, input: AVAssetWriterInput) {
        self.reader = reader
        self.output = output
        self.input = input
    }

    static var writerSettings: [String: Any] {
        [
            AVFormatIDKey: kAudioFormatMPEG4AAC,
            AVSampleRateKey: sampleRate,
            AVNumberOfChannelsKey: channels,
            AVEncoderBitRateKey: 192_000,
        ]
    }

    /// Returns nil when none of `urls` holds an audio track in the trimmed range.
    /// `urls` are the mic file and the screen file (system audio is recorded into `screen.mov`).
    static func make(urls: [URL], start: Double, length: Double) async throws -> AudioPump? {
        let composition = AVMutableComposition()
        let range = CMTimeRange(
            start: CMTime(seconds: start, preferredTimescale: 48_000),
            duration: CMTime(seconds: length, preferredTimescale: 48_000)
        )
        for url in urls where FileManager.default.fileExists(atPath: url.path) {
            let asset = AVURLAsset(url: url)
            guard let tracks = try? await asset.loadTracks(withMediaType: .audio) else { continue }
            for track in tracks {
                guard let timeRange = try? await track.load(.timeRange),
                      let slice = Optional(range.intersection(timeRange)), slice.duration.seconds > 0,
                      let target = composition.addMutableTrack(withMediaType: .audio, preferredTrackID: kCMPersistentTrackID_Invalid)
                else { continue }
                // Insert at the offset of the slice inside the trim range so a late-starting track stays in sync.
                let at = CMTimeSubtract(slice.start, range.start)
                do { try target.insertTimeRange(slice, of: track, at: at) } catch { composition.removeTrack(target) }
            }
        }
        let audioTracks = try await composition.loadTracks(withMediaType: .audio)
        guard !audioTracks.isEmpty else { return nil }

        var layout = AudioChannelLayout()
        layout.mChannelLayoutTag = kAudioChannelLayoutTag_Stereo
        let layoutData = Data(bytes: &layout, count: MemoryLayout<AudioChannelLayout>.size)
        let pcm: [String: Any] = [
            AVFormatIDKey: kAudioFormatLinearPCM,
            AVSampleRateKey: sampleRate,
            AVNumberOfChannelsKey: channels,
            AVChannelLayoutKey: layoutData,
            AVLinearPCMBitDepthKey: 32,
            AVLinearPCMIsFloatKey: true,
            AVLinearPCMIsBigEndianKey: false,
            AVLinearPCMIsNonInterleaved: false,
        ]
        let output = AVAssetReaderAudioMixOutput(audioTracks: audioTracks, audioSettings: pcm)
        output.alwaysCopiesSampleData = false
        let reader = try AVAssetReader(asset: composition)
        guard reader.canAdd(output) else { throw NSSError.unsupported("Cannot mix the audio tracks") }
        reader.add(output)
        reader.timeRange = CMTimeRange(start: .zero, duration: CMTime(seconds: length, preferredTimescale: 48_000))

        let input = AVAssetWriterInput(mediaType: .audio, outputSettings: writerSettings)
        input.expectsMediaDataInRealTime = false
        return AudioPump(reader: reader, output: output, input: input)
    }

    func startReading() throws {
        guard reader.startReading() else {
            throw NSSError.io("Cannot read audio: \(reader.error?.localizedDescription ?? "unknown error")")
        }
    }

    /// Appends buffered audio while the writer accepts it. With `upTo`, stops at the first buffer that starts
    /// after that time (seconds), which keeps audio interleaved with video. Marks the input finished at the end.
    func drain(upTo limit: Double?, writer: AVAssetWriter) throws {
        while !isFinished, input.isReadyForMoreMediaData {
            if pending == nil {
                pending = output.copyNextSampleBuffer()
                if pending == nil {
                    if reader.status == .failed {
                        throw NSSError.io("Audio decode failed: \(reader.error?.localizedDescription ?? "unknown error")")
                    }
                    input.markAsFinished()
                    isFinished = true
                    return
                }
            }
            guard let sample = pending else { return }
            if let limit, CMSampleBufferGetPresentationTimeStamp(sample).seconds > limit { return }
            guard input.append(sample) else {
                throw NSSError.io("Audio write failed: \(writer.error?.localizedDescription ?? "unknown error")")
            }
            pending = nil
        }
    }

    func cancel() { reader.cancelReading() }
}
