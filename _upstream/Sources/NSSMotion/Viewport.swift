import NSSCore

/// The part of the SOURCE frame a `CameraState` shows, in normalized source coordinates (0...1, origin
/// top-left, x right, y down).
///
/// ## What the renderer can rely on
///
/// For a camera with `scale` s and `center` (cx, cy), the visible source region is
///
///     x in [cx - 0.5 / s, cx + 0.5 / s],   y in [cy - 0.5 / s, cy + 0.5 / s]
///
/// that is, `1 / s` of the source width by `1 / s` of the source height, centred on `center`. It is the
/// same fraction of both axes, so the viewport always has the SOURCE aspect ratio. The renderer maps it
/// onto `RenderLayout.contentRect`, which also keeps the source aspect; the OUTPUT aspect (16:9, 1:1, 9:16)
/// only changes the padding around that rect, never which part of the source is shown. This is exactly
/// the inverse of `RenderLayout.sourcePoint(forOutput:)` evaluated at the corners of `contentRect`.
///
/// Every `CameraState` returned by `SpringMotionModel.frameState(at:)` satisfies, at every time, for every
/// track, timeline and `MotionTuning` (including underdamped springs that overshoot, panning between
/// clicks, clicks at or beyond the corners, and manual blocks focused outside the source):
///
/// * `scale` is finite and `>= 1`,
/// * `center` is finite,
/// * `0 <= minX`, `maxX <= 1`, `0 <= minY`, `maxY <= 1` EXACTLY (in `Double`, with no tolerance).
///
/// So the screen content never shows a gap or the background at its edges. The guarantee comes from
/// clamping the interpolated spring state in `CameraState.clampedToSource(scale:center:)` on every lookup,
/// after the springs, so no spring overshoot can escape it.
public struct SourceViewport: Sendable, Equatable {
    public var minX: Double
    public var minY: Double
    public var maxX: Double
    public var maxY: Double

    public var width: Double { maxX - minX }
    public var height: Double { maxY - minY }

    /// True when the viewport lies inside the source, [0, 1] on both axes.
    public var isInsideSource: Bool { minX >= 0 && minY >= 0 && maxX <= 1 && maxY <= 1 }
}

extension CameraState {
    /// The visible source region (see `SourceViewport`).
    public var sourceViewport: SourceViewport {
        let half = 0.5 / scale
        return SourceViewport(minX: center.x - half, minY: center.y - half, maxX: center.x + half, maxY: center.y + half)
    }

    /// A camera whose viewport is inside the source: scale at least 1 (non-finite becomes 1), and the
    /// center clamped to `[0.5 / scale, 1 - 0.5 / scale]` (non-finite becomes 0.5). The final nudges make
    /// `center -/+ 0.5 / scale` land inside [0, 1] exactly despite floating-point rounding.
    public static func clampedToSource(scale: Double, center: NormalizedPoint) -> CameraState {
        let s = scale.isFinite ? max(scale, 1) : 1
        let half = 0.5 / s
        func clamp(_ v: Double) -> Double {
            var c = v.isFinite ? min(max(v, half), 1 - half) : 0.5
            while c + half > 1 { c = c.nextDown }
            while c - half < 0 { c = c.nextUp }
            return c
        }
        return CameraState(scale: s, center: NormalizedPoint(x: clamp(center.x), y: clamp(center.y)))
    }
}
