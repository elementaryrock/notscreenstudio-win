import Foundation

/// A damped harmonic oscillator pulling a value toward a target, with a designer-friendly
/// parameterization (same meaning as SwiftUI's `.spring(response:dampingFraction:)`):
///
/// * `response`: the period, in seconds, of the undamped oscillation. Roughly "how long the move
///   takes". A critically damped spring is within ~1% of its target after about one `response`.
/// * `dampingRatio`: 1 = critically damped (fastest approach with no overshoot), < 1 = bouncy,
///   > 1 = sluggish.
///
/// The solution is analytic (exact for a target that is constant over a step), so it is
/// unconditionally stable for any step size and has no integration drift.
public struct Spring: Sendable, Equatable, Codable {
    public var response: Double
    public var dampingRatio: Double

    public init(response: Double, dampingRatio: Double = 1) {
        self.response = response
        self.dampingRatio = dampingRatio
    }

    /// Undamped angular frequency ω0 in rad/s.
    public var angularFrequency: Double { 2 * Double.pi / max(response, 1e-6) }

    /// State after `dt` seconds, starting `displacement` away from the target with `velocity`.
    /// Displacement is (value - target); the target is held constant over `dt`.
    public func evolve(displacement x0: Double, velocity v0: Double, dt: Double) -> (displacement: Double, velocity: Double) {
        let w = angularFrequency
        let z = max(0, dampingRatio)
        let t = max(0, dt)
        if abs(z - 1) < 1e-6 {
            // Critically damped: x = (c1 + c2 t) e^{-wt}
            let e = exp(-w * t)
            let c2 = v0 + w * x0
            let x = (x0 + c2 * t) * e
            let v = (c2 - w * (x0 + c2 * t)) * e
            return (x, v)
        } else if z < 1 {
            let wd = w * (1 - z * z).squareRoot()
            let e = exp(-z * w * t)
            let c = cos(wd * t), s = sin(wd * t)
            let x = e * (x0 * c + (v0 + z * w * x0) / wd * s)
            let v = e * (v0 * c - (z * w * v0 + w * w * x0) / wd * s)
            return (x, v)
        } else {
            let r = (z * z - 1).squareRoot()
            let r1 = -w * (z - r)
            let r2 = -w * (z + r)
            let a = (v0 - r2 * x0) / (r1 - r2)
            let b = x0 - a
            let e1 = exp(r1 * t), e2 = exp(r2 * t)
            return (a * e1 + b * e2, a * r1 * e1 + b * r2 * e2)
        }
    }

    /// A precomputed fixed-step integrator: the exact solution over `dt` is linear in (x, v),
    /// so one step is a 2x2 matrix multiply.
    public func stepper(dt: Double) -> SpringStepper {
        let (xx, vx) = evolve(displacement: 1, velocity: 0, dt: dt)
        let (xv, vv) = evolve(displacement: 0, velocity: 1, dt: dt)
        return SpringStepper(xx: xx, xv: xv, vx: vx, vv: vv)
    }
}

/// Exact fixed-step update for one `Spring` (see `Spring.stepper(dt:)`).
public struct SpringStepper: Sendable, Equatable {
    let xx: Double, xv: Double, vx: Double, vv: Double

    /// Advance `value`/`velocity` one step toward `target`.
    @inline(__always)
    public func step(_ value: inout Double, _ velocity: inout Double, target: Double) {
        let d = value - target
        let v = velocity
        value = target + xx * d + xv * v
        velocity = vx * d + vv * v
    }
}
