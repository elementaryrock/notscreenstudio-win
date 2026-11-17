import CoreGraphics
import CoreVideo
import Testing
import NSSCore
@testable import NSSRender

@Suite struct NSSRenderTests {
    @Test func passthroughClearsToBackgroundColor() throws {
        let screen = try PixelBuffers.make(width: 16, height: 16)
        let out = try PixelBuffers.make(width: 32, height: 18)
        var style = Style()
        style.background = .solid(RGBAColor(r: 1, g: 0, b: 0))
        try PassthroughRenderer().render(screen: screen, camera: nil, state: FrameState(time: 0), style: style, into: out)
        CVPixelBufferLockBaseAddress(out, .readOnly)
        defer { CVPixelBufferUnlockBaseAddress(out, .readOnly) }
        let p = CVPixelBufferGetBaseAddress(out)!.assumingMemoryBound(to: UInt8.self)
        #expect(p[2] == 255 && p[1] == 0 && p[0] == 0)  // BGRA: red
    }

    @Test func solidColorFrameSourceReturnsFrames() async throws {
        let src = try SolidColorFrameSource(width: 4, height: 4, color: RGBAColor(r: 0, g: 1, b: 0))
        let f = try #require(await src.screenFrame(at: 3.2))
        #expect(CVPixelBufferGetWidth(f) == 4)
        CVPixelBufferLockBaseAddress(f, .readOnly)
        let p = CVPixelBufferGetBaseAddress(f)!.assumingMemoryBound(to: UInt8.self)
        #expect(p[1] == 255 && p[0] == 0 && p[2] == 0)
        CVPixelBufferUnlockBaseAddress(f, .readOnly)
        #expect(await src.cameraFrame(at: 0) == nil)
        let withCam = try SolidColorFrameSource(width: 2, height: 2, color: .white, hasCamera: true)
        #expect(await withCam.cameraFrame(at: 0) != nil)
    }
}

// MARK: - Pure layout (no GPU)

@Suite struct RenderLayoutTests {
    @Test(arguments: AspectRatio.allCases)
    func outputSizes(aspect: AspectRatio) {
        let s1080 = RenderLayout.outputSize(for: aspect, resolution: .p1080)
        let s2160 = RenderLayout.outputSize(for: aspect, resolution: .p2160)
        switch aspect {
        case .r16x9: #expect(s1080 == PixelSize(width: 1920, height: 1080) && s2160 == PixelSize(width: 3840, height: 2160))
        case .r1x1: #expect(s1080 == PixelSize(width: 1080, height: 1080) && s2160 == PixelSize(width: 2160, height: 2160))
        case .r9x16: #expect(s1080 == PixelSize(width: 1080, height: 1920) && s2160 == PixelSize(width: 2160, height: 3840))
        }
    }

    @Test(arguments: AspectRatio.allCases)
    func contentFitsInsidePaddingKeepingSourceAspect(aspect: AspectRatio) {
        let out = aspect.outputSize(for: .p1080)
        var style = Style()
        style.padding = 0.08
        let l = RenderLayout(outputSize: out, sourceSize: PixelSize(width: 2880, height: 1800), style: style, camera: .identity)
        let pad = 0.08 * Double(out.height)
        let r = l.contentRect
        #expect(r.minX >= pad - 0.001 && r.minY >= pad - 0.001)
        #expect(r.maxX <= Double(out.width) - pad + 0.001 && r.maxY <= Double(out.height) - pad + 0.001)
        #expect(abs(r.width / r.height - 1.6) < 1e-9)
        #expect(abs(r.midX - Double(out.width) / 2) < 1e-9 && abs(r.midY - Double(out.height) / 2) < 1e-9)
        // The content touches the padding on at least one axis.
        #expect(abs(r.minX - pad) < 0.001 || abs(r.minY - pad) < 0.001)
    }

    @Test func cameraTransformRoundTrips() {
        let cam = CameraState(scale: 2.5, center: NormalizedPoint(x: 0.3, y: 0.7))
        let l = RenderLayout(outputSize: PixelSize(width: 1920, height: 1080), sourceSize: PixelSize(width: 1920, height: 1080), style: Style(), camera: cam)
        let centre = l.outputPoint(forSource: cam.center)
        #expect(abs(centre.x - l.contentRect.midX) < 1e-9 && abs(centre.y - l.contentRect.midY) < 1e-9)
        let p = NormalizedPoint(x: 0.31, y: 0.66)
        let back = l.sourcePoint(forOutput: l.outputPoint(forSource: p))
        #expect(abs(back.x - p.x) < 1e-9 && abs(back.y - p.y) < 1e-9)
    }

    @Test func nonFiniteCameraFallsBackToIdentity() {
        let bad = CameraState(scale: .nan, center: NormalizedPoint(x: .infinity, y: -.infinity))
        let l = RenderLayout(outputSize: PixelSize(width: 1920, height: 1080), sourceSize: PixelSize(width: 1920, height: 1080), style: Style(), camera: bad)
        #expect(l.camera == .identity)
        let p = l.outputPoint(forSource: .center)
        #expect(p.x.isFinite && p.y.isFinite)
    }

    @Test func rippleGrowsAndFades() {
        let l = RenderLayout(outputSize: PixelSize(width: 1920, height: 1080), sourceSize: PixelSize(width: 1920, height: 1080), style: Style(), camera: .identity)
        #expect(l.rippleRadius(progress: 0) < l.rippleRadius(progress: 0.5))
        #expect(l.rippleRadius(progress: 0.5) < l.rippleRadius(progress: 1))
        #expect(l.rippleOpacity(progress: 0) == 1 && l.rippleOpacity(progress: 1) == 0)
    }

    @Test func bubbleRectFollowsShape() {
        let l = RenderLayout(outputSize: PixelSize(width: 1920, height: 1080), sourceSize: PixelSize(width: 1920, height: 1080), style: Style(), camera: .identity)
        var b = CameraBubble(shape: .circle, position: NormalizedPoint(x: 0.5, y: 0.5), size: 0.2)
        let circle = l.cameraBubbleRect(b, cameraSize: PixelSize(width: 1280, height: 720))
        #expect(circle.width == 384 && circle.height == 384 && circle.midX == 960 && circle.midY == 540)
        b.shape = .wide
        let rect = l.cameraBubbleRect(b, cameraSize: PixelSize(width: 1280, height: 720))
        #expect(rect.width == 384 && abs(rect.height - 216) < 1e-9)
    }

    @Test func footprintTaps() {
        #expect(MetalRenderer.taps(texelsPerPixel: 0.5) == 1)
        #expect(MetalRenderer.taps(texelsPerPixel: 1) == 1)
        #expect(MetalRenderer.taps(texelsPerPixel: 2) == 2)
        #expect(MetalRenderer.taps(texelsPerPixel: 2.6) == 3)
        #expect(MetalRenderer.taps(texelsPerPixel: 40) == 6)
    }

    @Test func shaderDefinesCoverEverySlot() {
        let src = ShaderSource.source
        for slot in Slot.allCases {
            #expect(src.contains("#define U_\(String(describing: slot).uppercased()) \(slot.rawValue)"))
        }
    }
}
