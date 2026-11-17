import CoreGraphics
import Foundation
import NSSCore

/// Pure-Swift geometry of one rendered frame, in OUTPUT pixels (origin top-left, y down).
///
/// `MetalRenderer` derives every shader uniform from this type, so the Editor can use it to hit-test
/// the screen content, the cursor and the camera bubble exactly where the GPU draws them.
public struct RenderLayout: Sendable, Equatable {
    public let outputSize: PixelSize
    public let sourceSize: PixelSize
    public let camera: CameraState
    /// Where the (rounded, shadowed) screen content sits in the output.
    public let contentRect: CGRect
    /// Corner radius of the content in output pixels (already clamped to half the shorter side).
    public let cornerRadius: Double

    public init(outputSize: PixelSize, sourceSize: PixelSize, style: Style, camera: CameraState) {
        self.outputSize = outputSize
        self.sourceSize = sourceSize
        self.camera = Self.clampedCamera(camera)

        let w = Double(max(outputSize.width, 1))
        let h = Double(max(outputSize.height, 1))
        let pad = max(0, style.padding) * h
        let availW = max(1, w - 2 * pad)
        let availH = max(1, h - 2 * pad)
        let srcAspect = sourceSize.width > 0 && sourceSize.height > 0 ? sourceSize.aspect : 16.0 / 9.0
        let cw: Double, ch: Double
        if srcAspect > availW / availH {
            cw = availW
            ch = availW / srcAspect
        } else {
            ch = availH
            cw = availH * srcAspect
        }
        contentRect = CGRect(x: (w - cw) / 2, y: (h - ch) / 2, width: cw, height: ch)
        cornerRadius = min(max(0, style.cornerRadius) * h, min(cw, ch) / 2)
    }

    /// The camera the renderer actually uses: the viewport always lies inside the source, so the screen
    /// content never shows a gap or a stretched edge. Scale is at least 1 and the centre is kept
    /// `0.5 / scale` away from every source edge (the same rule as the motion model). Non-finite values
    /// (e.g. a spring that blew up) fall back to identity per component instead of poisoning every
    /// uniform with NaN.
    public static func clampedCamera(_ camera: CameraState) -> CameraState {
        let scale = camera.scale.isFinite ? max(camera.scale, 1) : 1
        let half = 0.5 / scale
        func axis(_ v: Double) -> Double { v.isFinite ? min(max(v, half), 1 - half) : 0.5 }
        return CameraState(scale: scale, center: NormalizedPoint(x: axis(camera.center.x), y: axis(camera.center.y)))
    }

    /// Output pixel size for an aspect ratio and export resolution (same as `AspectRatio.outputSize(for:)`).
    public static func outputSize(for aspect: AspectRatio, resolution: ExportResolution) -> PixelSize {
        aspect.outputSize(for: resolution)
    }

    // MARK: Camera transform

    /// Maps a normalized SOURCE point through the camera (zoom/pan) to output pixels.
    public func outputPoint(forSource p: NormalizedPoint) -> CGPoint {
        let ux = (p.x - camera.center.x) * camera.scale + 0.5
        let uy = (p.y - camera.center.y) * camera.scale + 0.5
        return CGPoint(x: contentRect.minX + ux * contentRect.width, y: contentRect.minY + uy * contentRect.height)
    }

    /// Inverse of `outputPoint(forSource:)`.
    public func sourcePoint(forOutput p: CGPoint) -> NormalizedPoint {
        let ux = (p.x - contentRect.minX) / contentRect.width
        let uy = (p.y - contentRect.minY) / contentRect.height
        return NormalizedPoint(x: camera.center.x + (ux - 0.5) / camera.scale, y: camera.center.y + (uy - 0.5) / camera.scale)
    }

    /// Source texels covered by one output pixel along x (> 1 means downscaling).
    public var sourceTexelsPerOutputPixel: Double {
        Double(sourceSize.width) / (contentRect.width * camera.scale)
    }

    /// A size that tracks the screen content independent of its aspect (geometric mean of the content sides).
    var contentScale: Double { (contentRect.width * contentRect.height).squareRoot() }

    // MARK: Cursor and ripples

    /// Height of the drawn cursor arrow in output pixels. Scales with the content, the zoom and `CursorStyle.size`.
    public func cursorHeight(style: CursorStyle) -> Double {
        0.0165 * contentScale * camera.scale * max(0, style.size)
    }

    /// Output pixels per cursor point (the unit of `NSCursor` images and hotspots) at normal size,
    /// before `CursorState.pressScale`. The 19 pt reference is the height of the macOS arrow with its outline.
    public func cursorPixelsPerPoint(style: CursorStyle) -> Double {
        cursorHeight(style: style) / 19
    }

    /// Output-pixel blur vector for the cursor (full shutter length, centred on the cursor position).
    public func cursorBlurVector(velocity: NormalizedPoint, style: CursorStyle) -> CGVector {
        let shutter = max(0, min(1, style.motionBlur)) / 30.0
        var dx = velocity.x * contentRect.width * camera.scale * shutter
        var dy = velocity.y * contentRect.height * camera.scale * shutter
        let len = (dx * dx + dy * dy).squareRoot()
        let cap = 3 * cursorHeight(style: style)
        if len > cap, len > 0 {
            dx *= cap / len
            dy *= cap / len
        }
        return CGVector(dx: dx, dy: dy)
    }

    /// Largest ripple radius in output pixels.
    public var rippleMaxRadius: Double { 0.03 * contentScale * camera.scale }

    /// Ring radius in output pixels for `progress` 0...1 (ease-out expansion).
    public func rippleRadius(progress: Double) -> Double {
        let p = max(0, min(1, progress))
        let e = 1 - pow(1 - p, 3)
        return rippleMaxRadius * (0.15 + 0.85 * e)
    }

    /// Ring opacity for `progress` 0...1.
    public func rippleOpacity(progress: Double) -> Double {
        let p = max(0, min(1, progress))
        return pow(1 - p, 1.5)
    }

    /// Ring stroke width in output pixels.
    public var rippleThickness: Double { max(1.5, rippleMaxRadius * 0.11) }

    // MARK: Camera bubble

    /// Bubble size multiplier for this layout's camera zoom (`CameraBubble.sizeMultiplier(cameraScale:)`).
    public func cameraBubbleSizeMultiplier(_ bubble: CameraBubble) -> Double {
        let m = bubble.sizeMultiplier(cameraScale: camera.scale)
        return m.isFinite ? max(0, m) : 1
    }

    /// Bubble frame in output pixels as drawn at this layout's camera zoom, from the shared
    /// `CameraBubbleGeometry`. At scale 1 it is centred on `bubble.position`. When zoomed in it shrinks
    /// by `cameraBubbleSizeMultiplier` toward its anchor (see `bubbleAnchor`), so a bubble in a corner
    /// stays tucked into that corner instead of drifting toward the middle.
    public func cameraBubbleRect(_ bubble: CameraBubble, cameraSize: PixelSize?) -> CGRect {
        let full = cameraBubbleFullRect(bubble)
        let m = cameraBubbleSizeMultiplier(bubble)
        let w = full.width * m, h = full.height * m
        let ax = Self.bubbleAnchor(bubble.position.x), ay = Self.bubbleAnchor(bubble.position.y)
        return CGRect(x: full.minX + ax * (full.width - w), y: full.minY + ay * (full.height - h), width: w, height: h)
    }

    /// Bubble frame at full size (camera scale 1), centred on `bubble.position`.
    public func cameraBubbleFullRect(_ bubble: CameraBubble) -> CGRect {
        let w = Double(outputSize.width), h = Double(outputSize.height)
        let l = CameraBubbleGeometry.layout(bubble, width: max(0, bubble.size) * w)
        return CGRect(x: bubble.position.x * w - l.width / 2, y: bubble.position.y * h - l.height / 2, width: l.width, height: l.height)
    }

    /// Which point of the full-size bubble stays fixed while it shrinks, per axis, from the bubble's
    /// normalized output position: 0 (left/top edge) in the first third, 1 (right/bottom edge) in the
    /// last third, and a linear blend through the middle third (0.5 = centre at 0.5). Continuous, so
    /// dragging the bubble while zoomed never makes it jump.
    public static func bubbleAnchor(_ position: Double) -> Double {
        guard position.isFinite else { return 0.5 }
        return min(max((position - 1.0 / 3.0) * 3, 0), 1)
    }

    /// Corner radius of the bubble in output pixels.
    public func cameraBubbleCornerRadius(_ bubble: CameraBubble, rect: CGRect) -> Double {
        CameraBubbleGeometry.layout(bubble, width: rect.width).cornerRadius
    }
}
