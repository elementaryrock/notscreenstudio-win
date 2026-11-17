import AppKit
import SwiftUI
import Testing
@testable import NSSUI

@MainActor
@Suite struct BarStylePreviewTests {
    private static let outDir: String? = ProcessInfo.processInfo.environment["NSS_BAR_PREVIEW_DIR"]

    private func backdrop(_ scheme: ColorScheme) -> LinearGradient {
        scheme == .dark
            ? LinearGradient(colors: [Color(white: 0.12), Color(red: 0.2, green: 0.25, blue: 0.4)], startPoint: .topLeading, endPoint: .bottomTrailing)
            : LinearGradient(colors: [Color(white: 0.96), Color(red: 0.75, green: 0.85, blue: 1)], startPoint: .topLeading, endPoint: .bottomTrailing)
    }

    private func render<V: View>(_ name: String, _ scheme: ColorScheme, size: CGSize, @ViewBuilder _ view: () -> V) throws {
        let framed = view()
            .environment(\.colorScheme, scheme)
            .environment(\.barUsesSystemBlur, false)
            .frame(width: size.width, height: size.height)
            .background(backdrop(scheme))
        let r = ImageRenderer(content: framed)
        r.scale = 2
        let image = r.nsImage
        #expect(image != nil)
        if let image { try Self.check(image, name: name, size: size) }
        guard let dir = Self.outDir, let image, let tiff = image.tiffRepresentation,
              let png = NSBitmapImageRep(data: tiff)?.representation(using: .png, properties: [:]) else { return }
        try FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
        try png.write(to: URL(fileURLWithPath: dir).appendingPathComponent("\(name)-\(scheme == .dark ? "dark" : "light").png"))
    }

    /// The image has the expected pixel size, is not a flat colour, and holds no yellow placeholder pixels.
    private static func check(_ image: NSImage, name: String, size: CGSize) throws {
        let cg = try #require(image.cgImage(forProposedRect: nil, context: nil, hints: nil))
        #expect(cg.width == Int(size.width * 2) && cg.height == Int(size.height * 2), "\(name) size")
        let rep = NSBitmapImageRep(cgImage: cg)
        var colours = Set<UInt32>()
        var placeholder = 0
        for y in stride(from: 0, to: rep.pixelsHigh, by: 2) {
            for x in stride(from: 0, to: rep.pixelsWide, by: 2) {
                guard let c = rep.colorAt(x: x, y: y)?.usingColorSpace(.sRGB) else { continue }
                let r = UInt32(c.redComponent * 255), g = UInt32(c.greenComponent * 255), b = UInt32(c.blueComponent * 255)
                colours.insert(r << 16 | g << 8 | b)
                if r > 220 && g > 170 && b < 60 { placeholder += 1 } // the yellow "unsupported control" box
            }
        }
        #expect(colours.count > 50, "\(name) looks empty")
        #expect(placeholder == 0, "\(name) has placeholder pixels")
    }

    @Test func rendersEveryBarState() throws {
        let cams = [DeviceItem(id: "c", name: "FaceTime HD Camera")]
        for scheme in [ColorScheme.light, .dark] {
            try render("setup-bar", scheme, size: CGSize(width: 640, height: 150)) {
                SetupBarContent(
                    sourceSymbol: "display", sourceTitle: "Built-in Retina Display",
                    cameraEnabled: true, cameras: cams, cameraID: "c", micEnabled: false, microphones: [], micID: nil)
            }
            try render("recording", scheme, size: CGSize(width: 320, height: 110)) {
                RecordingBarContent(elapsed: 83, isPaused: false, canPause: true)
            }
            try render("paused", scheme, size: CGSize(width: 320, height: 110)) {
                RecordingBarContent(elapsed: 83, isPaused: true, canPause: true)
            }
            try render("countdown", scheme, size: CGSize(width: 240, height: 240)) { CountdownFace(number: 3) }
        }
    }

    @Test func countdownRendersAtSmallPanelSizes() throws {
        for side in [40.0, 100.0, 216.0] {
            let r = ImageRenderer(content: CountdownFace(number: 2).frame(width: side, height: side))
            #expect(r.nsImage != nil)
        }
    }

    @Test func tokensAreOnTheEightPointGrid() {
        #expect(BarStyle.unit == 8)
        #expect(BarStyle.control.truncatingRemainder(dividingBy: 4) == 0)
        #expect(BarStyle.hairline == 0.5)
    }
}
