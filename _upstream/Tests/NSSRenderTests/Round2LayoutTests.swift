import CoreGraphics
import Foundation
import Testing
import NSSCore
@testable import NSSRender

/// CPU-only checks for Round 2: the cursor atlas, camera clamping and bubble shrink geometry.
struct Round2LayoutTests {
    let out = PixelSize(width: 1920, height: 1080)

    func layout(_ cam: CameraState = .identity, style: Style = Style()) -> RenderLayout {
        RenderLayout(outputSize: out, sourceSize: PixelSize(width: 1920, height: 1080), style: style, camera: cam)
    }

    // MARK: Cursor atlas

    @Test func atlasHasAnArrowAndEveryCellIsSane() {
        let atlas = CursorAtlas.build()
        #expect(atlas.width > 0 && atlas.width <= CursorAtlas.maxWidth && atlas.height > 0)
        #expect(atlas.pixels.count == atlas.width * atlas.height * 4)
        let arrow = try! #require(atlas.cells[.arrow])
        #expect(atlas.cell(for: .unknown) == arrow)
        let rects = atlas.cells.values.map {
            CGRect(x: $0.x - CursorAtlas.padding, y: $0.y - CursorAtlas.padding,
                   width: $0.width + 2 * CursorAtlas.padding, height: $0.height + 2 * CursorAtlas.padding)
        }
        for (shape, c) in atlas.cells {
            #expect(c.hotspot.x >= 0 && c.hotspot.x <= c.size.width && c.hotspot.y >= 0 && c.hotspot.y <= c.size.height, "\(shape) hotspot")
            #expect(c.x >= CursorAtlas.padding && c.y >= CursorAtlas.padding)
            #expect(c.x + c.width + CursorAtlas.padding <= atlas.width && c.y + c.height + CursorAtlas.padding <= atlas.height)
            #expect(abs(Double(c.width) - c.size.width * CursorAtlas.scale) < 1)
            // The image has opaque pixels.
            var maxA: Float = 0
            for y in c.y..<(c.y + c.height) { for x in c.x..<(c.x + c.width) { maxA = max(maxA, atlas.pixels[(y * atlas.width + x) * 4 + 3]) } }
            #expect(maxA > 0.9, "\(shape) is empty")
        }
        // Cells (with padding) never overlap.
        for i in rects.indices { for j in rects.indices where i < j { #expect(!rects[i].intersects(rects[j])) } }
        // The padding is clear.
        let top = arrow.y - CursorAtlas.padding
        for x in arrow.x..<(arrow.x + arrow.width) { #expect(atlas.pixels[(top * atlas.width + x) * 4 + 3] == 0) }
    }

    @Test func atlasUsesTheRealSystemCursorsWhenAppKitHasThem() {
        let atlas = CursorAtlas.build()
        let available = CursorShape.allCases.filter { CursorAtlas.systemImage(for: $0) != nil }
        for shape in available {
            let c = atlas.cell(for: shape)
            #expect(c.isSystemImage)
            #expect(c.hotspot == CursorAtlas.systemCursor(for: shape)!.hotSpot)
        }
        // Distinct shapes get distinct cells.
        if available.contains(.iBeam) { #expect(atlas.cell(for: .iBeam) != atlas.cell(for: .arrow)) }
    }

    @Test func fallbackArrowHasTheSystemArrowFrame() {
        let (img, size, hot) = CursorAtlas.fallbackArrow()
        #expect(size == CGSize(width: 17, height: 23) && hot == CGPoint(x: 4, y: 4))
        #expect(img.width == Int(17 * CursorAtlas.scale))
    }

    @Test func cursorLODFollowsMinification() {
        #expect(MetalRenderer.cursorLOD(pixelsPerPoint: CursorAtlas.scale) == 0)
        #expect(MetalRenderer.cursorLOD(pixelsPerPoint: 40) == 0)
        #expect(abs(MetalRenderer.cursorLOD(pixelsPerPoint: CursorAtlas.scale / 4) - 2) < 1e-9)
        #expect(MetalRenderer.cursorLOD(pixelsPerPoint: 0.01) == CursorAtlas.maxLOD)
        #expect(MetalRenderer.cursorLOD(pixelsPerPoint: .nan) == 0)
    }

    // MARK: Camera clamp (edge guarantee)

    @Test(arguments: [
        CameraState(scale: 0.5, center: .center),
        CameraState(scale: 2, center: NormalizedPoint(x: 0, y: 1)),
        CameraState(scale: 3, center: NormalizedPoint(x: -0.4, y: 1.7)),
        CameraState(scale: 1.0001, center: NormalizedPoint(x: 0.9, y: 0.1)),
    ])
    func clampedCameraKeepsTheViewportInsideTheSource(_ cam: CameraState) {
        let c = RenderLayout.clampedCamera(cam)
        #expect(c.scale >= 1)
        let half = 0.5 / c.scale
        #expect(c.center.x - half >= -1e-12 && c.center.x + half <= 1 + 1e-12)
        #expect(c.center.y - half >= -1e-12 && c.center.y + half <= 1 + 1e-12)
        // The content's corners map inside the source.
        let l = layout(cam)
        for p in [CGPoint(x: l.contentRect.minX, y: l.contentRect.minY), CGPoint(x: l.contentRect.maxX, y: l.contentRect.maxY)] {
            let s = l.sourcePoint(forOutput: p)
            #expect(s.x >= -1e-9 && s.x <= 1 + 1e-9 && s.y >= -1e-9 && s.y <= 1 + 1e-9)
        }
    }

    @Test func validCamerasAreUntouched() {
        let cam = CameraState(scale: 2.5, center: NormalizedPoint(x: 0.3, y: 0.7))
        #expect(RenderLayout.clampedCamera(cam) == cam)
        #expect(RenderLayout.clampedCamera(.identity) == .identity)
    }

    // MARK: Cursor size

    @Test func cursorPixelsPerPointScalesWithZoomAndSize() {
        var s = CursorStyle()
        let a = layout().cursorPixelsPerPoint(style: s)
        #expect(abs(a * 19 - layout().cursorHeight(style: s)) < 1e-9)
        #expect(abs(layout(CameraState(scale: 2, center: .center)).cursorPixelsPerPoint(style: s) - 2 * a) < 1e-9)
        s.size *= 2
        #expect(abs(layout().cursorPixelsPerPoint(style: s) - 2 * a) < 1e-9)
    }

    // MARK: Bubble shrink

    @Test func bubbleAnchorIsContinuousAndPicksTheNearSide() {
        #expect(RenderLayout.bubbleAnchor(0.05) == 0)
        #expect(RenderLayout.bubbleAnchor(0.3) == 0)
        #expect(abs(RenderLayout.bubbleAnchor(0.5) - 0.5) < 1e-12)
        #expect(RenderLayout.bubbleAnchor(0.7) == 1)
        #expect(RenderLayout.bubbleAnchor(0.95) == 1)
        #expect(RenderLayout.bubbleAnchor(.nan) == 0.5)
        var prev = RenderLayout.bubbleAnchor(0)
        for i in 1...1000 {
            let a = RenderLayout.bubbleAnchor(Double(i) / 1000)
            #expect(a >= prev && a - prev < 0.004)
            prev = a
        }
    }

    @Test func bubbleIsFullSizeWhenNotZoomed() {
        let b = CameraBubble()
        let l = layout()
        #expect(l.cameraBubbleSizeMultiplier(b) == 1)
        #expect(l.cameraBubbleRect(b, cameraSize: nil) == l.cameraBubbleFullRect(b))
    }

    @Test(arguments: [1.0, 1.3, 1.7, 2.0, 3.5])
    func bubbleShrinksIntoItsCorner(scale: Double) {
        let b = CameraBubble(shape: .wide, position: NormalizedPoint(x: 0.88, y: 0.82), size: 0.2, zoomedScale: 0.5)
        let l = layout(CameraState(scale: scale, center: .center))
        let full = l.cameraBubbleFullRect(b), r = l.cameraBubbleRect(b, cameraSize: nil)
        let m = bubbleSizeMultiplier(cameraScale: scale, zoomedScale: 0.5)
        #expect(abs(l.cameraBubbleSizeMultiplier(b) - m) < 1e-12)
        #expect(abs(r.width - full.width * m) < 1e-9 && abs(r.height - full.height * m) < 1e-9)
        // Bottom-right corner stays put.
        #expect(abs(r.maxX - full.maxX) < 1e-9 && abs(r.maxY - full.maxY) < 1e-9)
        #expect(full.contains(r))
    }

    @Test func bubbleShrinkAnchorsPerAxis() {
        let l = layout(CameraState(scale: 2, center: .center))
        // Top-left corner.
        var b = CameraBubble(position: NormalizedPoint(x: 0.1, y: 0.15), size: 0.2)
        var full = l.cameraBubbleFullRect(b), r = l.cameraBubbleRect(b, cameraSize: nil)
        #expect(abs(r.minX - full.minX) < 1e-9 && abs(r.minY - full.minY) < 1e-9)
        // Bottom edge, centred horizontally.
        b.position = NormalizedPoint(x: 0.5, y: 0.9)
        full = l.cameraBubbleFullRect(b); r = l.cameraBubbleRect(b, cameraSize: nil)
        #expect(abs(r.midX - full.midX) < 1e-9 && abs(r.maxY - full.maxY) < 1e-9)
        // zoomedScale 1 means no shrink.
        b.zoomedScale = 1
        #expect(l.cameraBubbleRect(b, cameraSize: nil) == l.cameraBubbleFullRect(b))
    }

    @Test(arguments: CameraShape.allCases)
    func bubbleCornerRadiusComesFromTheSharedGeometry(_ shape: CameraShape) {
        let b = CameraBubble(shape: shape, size: 0.2, roundness: 0.4)
        let l = layout(CameraState(scale: 2, center: .center))
        let r = l.cameraBubbleRect(b, cameraSize: nil)
        let g = CameraBubbleGeometry.layout(b, width: r.width)
        #expect(abs(r.height - g.height) < 1e-9)
        #expect(abs(r.width / r.height - CameraBubbleGeometry.aspect(for: shape)) < 1e-9)
        #expect(l.cameraBubbleCornerRadius(b, rect: r) == g.cornerRadius)
        if shape == .circle { #expect(abs(g.cornerRadius - r.width / 2) < 1e-9) }
    }
}
