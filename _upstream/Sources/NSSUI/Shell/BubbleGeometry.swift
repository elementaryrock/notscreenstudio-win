import CoreGraphics
import Foundation
import NSSCore

/// Pure geometry for the camera bubble overlay and the capture source.
///
/// Screen rects come in two coordinate systems:
/// * "global top-left" (what ScreenCaptureKit and `CGDisplay` report): origin at the top-left of the primary display, y down.
/// * "AppKit" (what `NSWindow.frame` and `NSScreen.frame` use): origin at the bottom-left of the primary display, y up.
public enum BubbleGeometry {
    /// Aspect (width / height) of a bubble whose shape is not square. Matches a typical webcam.
    public static let defaultCameraAspect = 16.0 / 9.0

    /// Width / height of the bubble for a shape.
    public static func aspect(for shape: CameraShape, cameraAspect: Double = defaultCameraAspect) -> Double {
        CameraBubbleGeometry.aspect(for: shape)
    }

    /// Converts a rect between the two global coordinate systems. The operation is its own inverse.
    public static func flip(_ rect: CGRect, primaryHeight: CGFloat) -> CGRect {
        CGRect(x: rect.minX, y: primaryHeight - rect.maxY, width: rect.width, height: rect.height)
    }

    /// Normalized center and width fraction of the bubble, relative to the capture source.
    /// Both rects are in the SAME coordinate system, and `sourceRect` is the AppKit rect of the capture source.
    /// The center is clamped to 0...1 so a bubble dragged off the source still lands on its edge.
    public static func normalize(panelFrame: CGRect, in sourceRect: CGRect) -> (center: NormalizedPoint, size: Double)? {
        guard sourceRect.width > 0, sourceRect.height > 0 else { return nil }
        let x = (panelFrame.midX - sourceRect.minX) / sourceRect.width
        let y = (sourceRect.maxY - panelFrame.midY) / sourceRect.height // flip to top-left origin
        let center = NormalizedPoint(x: Double(x), y: Double(y)).clamped
        return (center, Double(panelFrame.width / sourceRect.width))
    }

    /// Re-expresses a bubble given relative to the capture source (what the user dragged over) in the
    /// OUTPUT space that `Style.camera` uses: the source is drawn inside the output inset by `padding`, so the
    /// bubble keeps its place over the content. `size` is a fraction of the source width and becomes a fraction
    /// of the output width.
    public static func toOutputSpace(
        center: NormalizedPoint, size: Double, outputAspect: Double, sourceAspect: Double, padding: Double
    ) -> (center: NormalizedPoint, size: Double) {
        let r = PreviewGeometry.contentRect(outputAspect: outputAspect, sourceAspect: sourceAspect, padding: padding)
        return (
            NormalizedPoint(x: r.minX + center.x * r.width, y: r.minY + center.y * r.height),
            size * r.width)
    }

    /// Inverse of `normalize`: the panel frame (AppKit coordinates) for a normalized center and size.
    public static func panelFrame(
        center: NormalizedPoint, size: Double, shape: CameraShape, in sourceRect: CGRect,
        cameraAspect: Double = defaultCameraAspect
    ) -> CGRect {
        let width = CGFloat(size) * sourceRect.width
        let height = width / CGFloat(aspect(for: shape, cameraAspect: cameraAspect))
        let midX = sourceRect.minX + CGFloat(center.x) * sourceRect.width
        let midY = sourceRect.maxY - CGFloat(center.y) * sourceRect.height
        return CGRect(x: midX - width / 2, y: midY - height / 2, width: width, height: height)
    }

    /// Slides `frame` so it stays at least partly reachable inside `bounds` (never fully off-screen).
    public static func keepReachable(_ frame: CGRect, in bounds: CGRect, minVisible: CGFloat = 40) -> CGRect {
        var f = frame
        f.origin.x = min(max(f.origin.x, bounds.minX - f.width + minVisible), bounds.maxX - minVisible)
        f.origin.y = min(max(f.origin.y, bounds.minY - f.height + minVisible), bounds.maxY - minVisible)
        return f
    }

    /// Corner radius in points for a shape drawn in a bubble of `size`.
    public static func cornerRadius(for shape: CameraShape, roundness: Double = 0.3, size: CGSize) -> CGFloat {
        CGFloat(CameraBubbleGeometry.layout(shape: shape, roundness: roundness, width: Double(size.width)).cornerRadius)
    }

    /// Clamps a requested bubble width to a sensible range for the source width.
    public static func clampedWidth(_ width: CGFloat, sourceWidth: CGFloat) -> CGFloat {
        let minW: CGFloat = 80
        let maxW = max(minW, sourceWidth * 0.5)
        return min(max(width, minW), maxW)
    }
}

/// Geometry of the area-selection drag.
public enum AreaSelectionMath {
    public static let minimumSide: CGFloat = 32

    /// Rect spanned by two drag points, clamped to `bounds`. nil when smaller than `minimumSide` on either side.
    public static func rect(from a: CGPoint, to b: CGPoint, in bounds: CGRect) -> CGRect? {
        let raw = CGRect(x: min(a.x, b.x), y: min(a.y, b.y), width: abs(a.x - b.x), height: abs(a.y - b.y))
        let r = raw.intersection(bounds)
        guard !r.isNull, r.width >= minimumSide, r.height >= minimumSide else { return nil }
        return r.integral.intersection(bounds)
    }
}

/// Where the capture source is on screen, for placing the camera bubble.
public enum SourceLocator {
    /// AppKit-coordinate rect of the capture source. nil if the source cannot be located.
    public static func appKitRect(
        for source: CaptureSource, displays: [DisplayItem], windows: [WindowItem], primaryHeight: CGFloat
    ) -> CGRect? {
        let global: CGRect?
        switch source {
        case .display(let id):
            global = displays.first { $0.id == id }?.frame
        case .window(let id):
            global = windows.first { $0.id == id }?.frame
        case .area(let displayID, let rect):
            global = displays.first { $0.id == displayID }.map {
                CGRect(x: $0.frame.minX + rect.minX, y: $0.frame.minY + rect.minY, width: rect.width, height: rect.height)
            }
        }
        return global.map { BubbleGeometry.flip($0, primaryHeight: primaryHeight) }
    }
}
