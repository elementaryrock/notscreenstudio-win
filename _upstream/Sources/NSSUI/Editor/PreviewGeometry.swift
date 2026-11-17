import Foundation
import NSSCore

/// Pure layout maths for mapping preview clicks to source/output space. Mirrors the renderer's layout
/// contract: the screen is fit (aspect preserved) inside the output inset by `padding` (fraction of height).
public enum PreviewGeometry {
    /// Content rect in output-normalized units (0...1, origin top-left).
    public static func contentRect(outputAspect: Double, sourceAspect: Double, padding: Double) -> CGRect {
        let W = outputAspect
        let w = max(W - 2 * padding, 0.01)
        let h = max(1 - 2 * padding, 0.01)
        let cw: Double, ch: Double
        if w / h > sourceAspect { ch = h; cw = h * sourceAspect } else { cw = w; ch = w / sourceAspect }
        return CGRect(x: ((W - cw) / 2) / W, y: (1 - ch) / 2, width: cw / W, height: ch)
    }

    /// Source-normalized point under an output-normalized point, accounting for the zoom `camera`.
    /// nil when the point is outside the screen content.
    public static func sourcePoint(
        forOutput p: NormalizedPoint, outputAspect: Double, sourceAspect: Double, padding: Double, camera: CameraState
    ) -> NormalizedPoint? {
        let r = contentRect(outputAspect: outputAspect, sourceAspect: sourceAspect, padding: padding)
        let rx = (p.x - r.midX) / r.width
        let ry = (p.y - r.midY) / r.height
        guard abs(rx) <= 0.5, abs(ry) <= 0.5 else { return nil }
        let s = max(camera.scale, 1)
        return NormalizedPoint(x: camera.center.x + rx / s, y: camera.center.y + ry / s).clamped
    }

    /// True when an output-normalized point lies on the camera bubble.
    public static func hitsCameraBubble(_ p: NormalizedPoint, bubble: CameraBubble, outputAspect: Double) -> Bool {
        guard bubble.visible else { return false }
        let dx = (p.x - bubble.position.x) * outputAspect
        let dy = p.y - bubble.position.y
        return (dx * dx + dy * dy).squareRoot() <= bubble.size * outputAspect / 2
    }

    /// Bubble center that keeps the whole bubble inside the output.
    public static func clampedBubbleCenter(_ p: NormalizedPoint, size: Double, outputAspect: Double) -> NormalizedPoint {
        let hx = min(size / 2, 0.5)
        let hy = min(size * outputAspect / 2, 0.5)
        return NormalizedPoint(x: min(max(p.x, hx), 1 - hx), y: min(max(p.y, hy), 1 - hy))
    }
}
