import Foundation

/// Every tuning constant of the motion system, in one place. Times are seconds, distances are
/// normalized source units unless noted. `MotionTuning()` is the shipped default.
public struct MotionTuning: Sendable, Equatable {
    // MARK: Simulation

    /// Fixed simulation rate (Hz) the springs are integrated at during the build.
    public var simulationRate: Double = 240
    /// Hard cap on the simulated table (36 bytes per frame, so 1M frames is 36 MB, about 69 min at
    /// 240 Hz). Longer ranges lower the effective rate instead (the springs are analytic, so any step
    /// is stable). Guards against bogus durations or absolute timestamps in the track.
    public var maxSimulationFrames: Int = 1_000_000
    /// Durations (and derived track spans) beyond this are clamped.
    public var maxDuration: Double = 24 * 60 * 60

    // MARK: Camera springs

    /// Spring on log(scale). Critically damped: zooms never overshoot.
    public var zoomSpring = Spring(response: 0.75, dampingRatio: 1)
    /// Spring on the camera focus point (panning).
    public var panSpring = Spring(response: 0.55, dampingRatio: 1)

    // MARK: Auto-zoom

    /// Magnification of auto-zoom segments.
    public var autoZoomScale: Double = 2
    /// Upper bound applied to every zoom (auto and manual).
    public var maxZoomScale: Double = 6
    /// The zoom starts this long before the first click of a segment.
    public var autoZoomLeadIn: Double = 0.5
    /// After the last click (and after the cursor last moved), stay zoomed this long before easing out.
    public var autoZoomHold: Double = 1.5
    /// Cursor movement after the last click keeps a segment zoomed, but at most this long after it.
    public var autoZoomMaxHold: Double = 5
    /// Clicks further apart in time than this always start a new segment. Measured from the previous
    /// click's release (mouse-up), so a drag keeps the zoom held until the button is released.
    public var clusterMaxGap: Double = 3
    /// Clicks further apart than this (in source-height units, aspect-corrected) start a new
    /// segment, unless they are closer in time than `clusterPanGap` (then the camera pans instead).
    /// For such a spatial split the earlier segment's hold is shortened (down to `autoZoomMinHold`)
    /// to leave `minZoomOutGap` for a zoom out and back in; if there is no room, the camera pans.
    /// With the defaults: far clicks 1...2 s apart pan, 2...3 s apart zoom out and back in.
    public var clusterMaxDistance: Double = 0.35
    /// Far-apart clicks closer in time than this stay in one segment (pan instead of zoom out/in).
    public var clusterPanGap: Double = 1
    /// Segments separated by less than this are merged, so the camera does not briefly zoom out.
    public var minZoomOutGap: Double = 1
    /// Shortest hold after the last release when a segment is shortened for a spatial split.
    public var autoZoomMinHold: Double = 0.5
    /// Between segments, the focus is only re-aimed at the next segment once the zoom has eased out
    /// below this scale, so the camera does not swing sideways while zooming out.
    public var preAimMaxScale: Double = 1.02
    /// Segments (or remnants after manual blocks cut them) shorter than this are dropped.
    public var minSegmentDuration: Double = 0.3
    /// Dead zone for cursor following, as a fraction of the viewport half-extent per axis. The
    /// camera only pans once the cursor leaves the central `deadZone` part of the viewport.
    public var deadZone: Double = 0.5

    // MARK: Cursor

    /// Cursor spring response when `CursorStyle.smoothing` is just above 0 ...
    public var cursorMinResponse: Double = 0.04
    /// ... and when it is 1. `smoothing == 0` means the raw cursor (no spring).
    public var cursorMaxResponse: Double = 0.3
    public var cursorDampingRatio: Double = 1
    /// Movement between consecutive samples above this counts as activity (for idle hide and holds).
    public var activityEpsilon: Double = 1e-4
    /// With `hideWhenIdle`, the cursor starts fading after this long without movement or clicks.
    public var idleHideDelay: Double = 2
    public var idleFadeOutDuration: Double = 0.35
    public var idleFadeInDuration: Double = 0.12

    // MARK: Ripples

    /// A click ripple runs from progress 0 to 1 over this duration after each mouse-down.
    public var rippleDuration: Double = 0.45

    // MARK: Cursor shape

    /// Shape changes that last less than this are ignored, so a cursor that flickers between shapes
    /// (for example while crossing a one-pixel border) keeps drawing the surrounding shape.
    public var minShapeDuration: Double = 0.05

    // MARK: Press squish

    /// `CursorState.pressScale` while a mouse button is held down.
    public var pressScale: Double = 0.82
    /// Spring toward `pressScale` on mouse-down: quick, critically damped (no bounce).
    public var pressDownSpring = Spring(response: 0.1, dampingRatio: 1)
    /// Spring back to 1 on mouse-up: underdamped for a small overshoot (about 2% above 1 with the defaults).
    public var pressReleaseSpring = Spring(response: 0.35, dampingRatio: 0.55)
    /// A press counts as held for at least this long, so a click with no visible hold still squishes.
    public var pressMinHold: Double = 0.08

    public init() {}

    public static let standard = MotionTuning()

    /// Cursor spring for a `CursorStyle.smoothing` value, or nil for the raw cursor.
    public func cursorSpring(smoothing: Double) -> Spring? {
        let s = min(max(smoothing, 0), 1)
        guard s > 0 else { return nil }
        return Spring(response: cursorMinResponse + s * (cursorMaxResponse - cursorMinResponse), dampingRatio: cursorDampingRatio)
    }
}
