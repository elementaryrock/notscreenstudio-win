import AVFoundation
import AppKit
import Observation
import NSSCore

/// Generates a fixed row of screen thumbnails off the main thread.
@MainActor
@Observable
final class ThumbnailStore {
    struct Thumb: Identifiable { let id: Int; let time: MediaTime; let image: CGImage }
    private(set) var thumbs: [Thumb] = []
    @ObservationIgnored private var task: Task<Void, Never>?

    func load(url: URL, duration: MediaTime, count: Int = 24) {
        task?.cancel()
        guard duration > 0, count > 0 else { return }
        task = Task { [weak self] in
            let asset = AVURLAsset(url: url)
            let gen = AVAssetImageGenerator(asset: asset)
            gen.appliesPreferredTrackTransform = true
            gen.maximumSize = CGSize(width: 240, height: 135)
            gen.requestedTimeToleranceBefore = CMTime(seconds: 0.5, preferredTimescale: 600)
            gen.requestedTimeToleranceAfter = CMTime(seconds: 0.5, preferredTimescale: 600)
            for i in 0..<count {
                if Task.isCancelled { return }
                let t = duration * (Double(i) + 0.5) / Double(count)
                guard let result = try? await gen.image(at: CMTime(seconds: t, preferredTimescale: 600)) else { continue }
                let image = result.image
                self?.thumbs.append(Thumb(id: i, time: t, image: image))
            }
        }
    }

    func cancel() { task?.cancel() }
}
