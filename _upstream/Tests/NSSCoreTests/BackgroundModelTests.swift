import Foundation
import Testing
@testable import NSSCore

struct BackgroundModelTests {
    @Test(arguments: [
        Background.mesh(points: [MeshPoint(x: 0, y: 0, color: .white), MeshPoint(x: 1, y: 1, color: .black)], warp: 0.05),
        Background.systemWallpaper(path: "/System/Library/Desktop Pictures/Sonoma.heic"),
        Background.image(file: "background-abc-photo.png"),
        Background.wallpaper(name: "ocean"),
    ])
    func newBackgroundsRoundTrip(_ bg: Background) throws {
        var style = Style()
        style.background = bg
        style.backgroundBlur = 0.42
        let back = try JSONDecoder().decode(Style.self, from: JSONEncoder().encode(style))
        #expect(back == style)
    }

    @Test func blurDefaultsToZeroWhenMissing() throws {
        var json = try JSONSerialization.jsonObject(with: JSONEncoder().encode(Style())) as! [String: Any]
        json.removeValue(forKey: "backgroundBlur")
        let style = try JSONDecoder().decode(Style.self, from: JSONSerialization.data(withJSONObject: json))
        #expect(style.backgroundBlur == 0)
    }

    @Test func presetsCoverLinearAndMeshGradientsAndColours() {
        let g = BackgroundPresets.gradients
        #expect(g.count >= 12)
        #expect(g.filter { if case .gradient = $0.background { true } else { false } }.count >= 6)
        #expect(g.filter { if case .mesh = $0.background { true } else { false } }.count >= 6)
        #expect(BackgroundPresets.colors.count >= 12)
        let ids = (g + BackgroundPresets.colors).map(\.id)
        #expect(Set(ids).count == ids.count, "preset ids must be unique")
        for p in g { #expect(!p.background.isImageBased) }
    }

    @Test func legacyWallpapersMapToMeshAndUnknownNamesFallBackToAurora() {
        for name in BackgroundPresets.legacyWallpaperNames {
            guard case .mesh(let pts, _) = BackgroundPresets.migratedWallpaper(named: name.capitalized) else {
                Issue.record("\(name) did not migrate to a mesh"); continue
            }
            #expect(pts.count >= 4)
        }
        #expect(BackgroundPresets.migratedWallpaper(named: "Graphite") == BackgroundPresets.migratedWallpaper(named: "aurora"))
    }

    @Test func blurSigmaIsRelativeToTheImageAndQuantized() {
        #expect(BackgroundBlur.sigma(amount: 0, shortSide: 2000) == 0)
        #expect(BackgroundBlur.sigma(amount: -1, shortSide: 2000) == 0)
        #expect(BackgroundBlur.sigma(amount: .nan, shortSide: 2000) == 0)
        let full = BackgroundBlur.sigma(amount: 1, shortSide: 2000)
        #expect(abs(full - BackgroundBlur.maxSigmaFraction * 2000) < 1e-9)
        #expect(BackgroundBlur.sigma(amount: 2, shortSide: 2000) == full)
        // Scale invariance: half the pixels, half the sigma.
        #expect(abs(BackgroundBlur.sigma(amount: 0.5, shortSide: 1000) * 2 - BackgroundBlur.sigma(amount: 0.5, shortSide: 2000)) < 1e-9)
        #expect(BackgroundBlur.quantized(0.123) == 0.12)
        #expect(BackgroundBlur.quantized(1.7) == 1)
        #expect(BackgroundBlur.quantized(-0.2) == 0)
    }

    @Test func resolvingAssetsMakesOnlyRelativeImagesAbsolute() {
        let bundle = URL(fileURLWithPath: "/tmp/p.nssproj")
        var style = Style()
        style.background = .image(file: "bg.png")
        #expect(style.resolvingAssets(relativeTo: bundle).background == .image(file: "/tmp/p.nssproj/bg.png"))
        for bg in [Background.image(file: "/abs/x.png"), .image(file: ""), .systemWallpaper(path: "/w.heic"), .solid(.white)] {
            style.background = bg
            #expect(style.resolvingAssets(relativeTo: bundle) == style)
        }
    }

    @Test func imageBasedFlag() {
        #expect(Background.image(file: "a").isImageBased)
        #expect(Background.systemWallpaper(path: "/a").isImageBased)
        #expect(!Background.solid(.white).isImageBased)
        #expect(!Background.wallpaper(name: "aurora").isImageBased)
    }
}
