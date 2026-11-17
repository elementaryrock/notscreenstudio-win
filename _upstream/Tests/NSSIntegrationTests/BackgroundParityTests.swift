import AVFoundation
import CoreGraphics
import CoreVideo
import Foundation
import ImageIO
import Testing
import UniformTypeIdentifiers
import NSSCore
import NSSExport
import NSSMotion
import NSSRender
@testable import NSSUI

/// Round 3 backgrounds: the export equals the preview for every new background kind, including a
/// bundle-relative picked image and a system wallpaper, both blurred.
///
/// Same method as `ParityTests`: (1) every exported frame against a direct render at export size (H.264 loss
/// only), and (2) the real `PreviewEngine` byte for byte against what the exporter feeds the renderer.
@Suite(.serialized) struct BackgroundParityTests {
    enum Kind: String, CaseIterable, Sendable {
        case linearPreset, meshPreset, bundleImageBlurred, systemWallpaperBlurred
    }

    static let pictureName = "background-test-picture.png"

    /// Builds the bundle and returns it with the background in place (image files written into or next to it).
    private func bundle(for kind: Kind) async throws -> (E2EBundle, URL?) {
        var spec = VerifySpec()
        spec.trim = TrimRange(start: 0.4, end: 0.9)
        spec.mic = false
        spec.style.padding = 0.12
        var external: URL?
        switch kind {
        case .linearPreset:
            spec.style.background = try #require(BackgroundPresets.gradients.first { if case .gradient = $0.background { true } else { false } }).background
        case .meshPreset:
            spec.style.background = try #require(BackgroundPresets.gradients.first { if case .mesh = $0.background { true } else { false } }).background
        case .bundleImageBlurred:
            spec.style.background = .image(file: Self.pictureName)
            spec.style.backgroundBlur = 0.5
        case .systemWallpaperBlurred:
            // A stand-in for a file in /System/Library/Desktop Pictures (absolute path, outside the bundle).
            let dir = FileManager.default.temporaryDirectory.appendingPathComponent("nss-wall-\(UUID().uuidString)")
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            external = dir.appendingPathComponent("Wallpaper.png")
            try Self.writePicture(external!)
            spec.style.background = .systemWallpaper(path: external!.path)
            spec.style.backgroundBlur = 0.3
        }
        let b = try await makeVerifyBundle(spec)
        if kind == .bundleImageBlurred { try Self.writePicture(b.dir.appendingPathComponent(Self.pictureName)) }
        return (b, external)
    }

    @Test(.enabled(if: e2eAvailable), arguments: Kind.allCases)
    func exportMatchesDirectRender(kind: Kind) async throws {
        let (bundle, external) = try await bundle(for: kind)
        defer {
            try? FileManager.default.removeItem(at: bundle.store.libraryURL)
            if let external { try? FileManager.default.removeItem(at: external.deletingLastPathComponent()) }
        }
        let project = bundle.project
        let out = bundle.dir.appendingPathComponent("bg-parity.mp4")
        let preset = ExportPreset(resolution: .p1080, fps: 60, codec: .h264)
        let renderer = try MetalRenderer.makeDefault()
        try await EndToEndExportTests.exporter(renderer: renderer).export(
            project: project, projectURL: bundle.dir, track: bundle.track, preset: preset, to: out, progress: { _ in })
        let size = preset.outputSize(for: project.style.aspectRatio)

        let source = try await AssetFrameSource.make(
            screenURL: bundle.dir.appendingPathComponent("screen.mov"), cameraURL: bundle.dir.appendingPathComponent("camera.mov"))
        let fileDuration = try await AVURLAsset(url: bundle.dir.appendingPathComponent("screen.mov")).load(.duration).seconds
        let motion = SpringMotionModel.factory(sourceAspect: Synthetic.size.aspect)(bundle.track, project.timeline, project.style.cursor, fileDuration)
        // The exporter resolves bundle-relative images exactly like this.
        let style = project.style.resolvingAssets(relativeTo: bundle.dir)
        let reference = try MetalRenderer.makeDefault()
        var psnrs: [Double] = []
        var corner: (r: Int, g: Int, b: Int)?
        try await forEachFrame(out) { i, _, pb in
            let t = project.timeline.trim.start + Double(i) / 60
            let a = Frame(pb)
            let b = try await directRender(source: source, renderer: reference, motion: motion, style: style, t: t, size: size)
            psnrs.append(psnr(a, b, step: 4).psnr)
            if i == 0 { corner = b.px(8, 8) }
        }
        let expected = Int(((project.timeline.trim.end ?? 0) - project.timeline.trim.start) * 60 + 0.5)
        #expect(psnrs.count == expected)
        #expect((psnrs.min() ?? 0) > ParityTests.minPSNR, "\(kind): worst frame PSNR \(psnrs.min() ?? 0) dB")
        // The background really drew (an unresolved image would be the flat grey fallback).
        if kind == .bundleImageBlurred || kind == .systemWallpaperBlurred, let c = corner {
            #expect(!(abs(c.r - 128) < 4 && abs(c.g - 128) < 4 && abs(c.b - 128) < 4), "\(kind): background is the missing-image grey")
            #expect(c.r > c.b + 40, "\(kind): expected the picture's warm corner, got \(c)")
        }
        print("background parity \(kind.rawValue): \(psnrs.count) frames, PSNR min \(String(format: "%.1f", psnrs.min() ?? 0)) dB")
    }

    @MainActor
    @Test(.enabled(if: e2eAvailable), arguments: Kind.allCases)
    func previewIsByteIdenticalToExporterInputs(kind: Kind) async throws {
        let (bundle, external) = try await bundle(for: kind)
        defer {
            try? FileManager.default.removeItem(at: bundle.store.libraryURL)
            if let external { try? FileManager.default.removeItem(at: external.deletingLastPathComponent()) }
        }
        let renderer = try MetalRenderer.makeDefault()
        let deps = EditorDependencies(
            store: bundle.store, motionFactory: SpringMotionModel.factory, renderer: renderer,
            exporter: MP4Exporter(motionFactory: SpringMotionModel.factory, renderer: renderer),
            motionFactoryForAspect: { SpringMotionModel.factory(sourceAspect: $0) })
        let model = EditorModel(project: bundle.project, reference: bundle.ref, track: bundle.track, dependencies: deps)
        let engine = PreviewEngine(model: model)
        model.changeHandler = { [engine] in engine.modelChanged($0) }
        defer { engine.teardown() }
        var presented: Frame?
        var presentCount = 0
        engine.present = { pb in presented = Frame(pb); presentCount += 1 }

        let size = PreviewEngine.previewSize(for: bundle.project.style.aspectRatio)
        let source = try await AssetFrameSource.make(
            screenURL: bundle.dir.appendingPathComponent("screen.mov"), cameraURL: bundle.dir.appendingPathComponent("camera.mov"))
        let motion = SpringMotionModel.factory(sourceAspect: Synthetic.size.aspect)(
            bundle.track, bundle.project.timeline, bundle.project.style.cursor, model.duration)
        let style = bundle.project.style.resolvingAssets(relativeTo: bundle.dir)
        let reference = try MetalRenderer.makeDefault()
        for k in [0, 12, 25] {
            let t = bundle.project.timeline.trim.start + Double(k) / 60
            let want = try await directRender(source: source, renderer: reference, motion: motion, style: style, t: t, size: size)
            engine.seek(to: t)
            var attempts = 0, matched = false, lastErr = Int.max
            while attempts < 3000 && !matched {
                let before = presentCount
                engine.tick()
                try await Task.sleep(for: .milliseconds(10))
                attempts += 1
                guard presentCount > before, engine.motionIsCurrent, let got = presented else { continue }
                lastErr = psnr(got, want, step: 1).maxError
                matched = lastErr == 0
            }
            #expect(matched, "\(kind): preview at t=\(t) never matched the exporter's inputs; last max error \(lastErr)")
        }
    }

    /// 480x270: warm top-left, cool elsewhere, with stripes so the blur has something to smooth.
    static func writePicture(_ url: URL) throws {
        let w = 480, h = 270
        var data = [UInt8](repeating: 255, count: w * h * 4)
        for y in 0..<h {
            for x in 0..<w {
                let warm = x < w / 2 && y < h / 2
                let stripe: UInt8 = (x / 6) % 2 == 0 ? 0 : 40
                let i = (y * w + x) * 4
                data[i] = warm ? 240 : 30
                data[i + 1] = 60 + stripe
                data[i + 2] = warm ? 40 : 220
            }
        }
        let image = CGImage(width: w, height: h, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: w * 4,
                            space: CGColorSpace(name: CGColorSpace.sRGB)!,
                            bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.noneSkipLast.rawValue),
                            provider: CGDataProvider(data: Data(data) as CFData)!, decode: nil, shouldInterpolate: false, intent: .defaultIntent)!
        let dest = CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil)!
        CGImageDestinationAddImage(dest, image, nil)
        #expect(CGImageDestinationFinalize(dest))
    }
}
