//! Analytic damped harmonic oscillator.
//!
//! Port of upstream `NSSMotion/Spring.swift`. The closed-form solution is exact for a target
//! held constant over a step, so any step size is stable and there is no integration drift —
//! which is what makes preview and export agree at different frame rates.

use serde::{Deserialize, Serialize};

/// A damped harmonic oscillator pulling a value toward a target, parameterised like SwiftUI's
/// `.spring(response:dampingFraction:)`:
///
/// * `response`: period of the undamped oscillation, in seconds. Roughly "how long the move
///   takes"; a critically damped spring is within ~1% of its target after about one `response`.
/// * `damping_ratio`: 1 = critically damped (fastest approach, no overshoot), < 1 = bouncy,
///   > 1 = sluggish.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Spring {
    pub response: f64,
    pub damping_ratio: f64,
}

impl Spring {
    pub const fn new(response: f64, damping_ratio: f64) -> Self {
        Self {
            response,
            damping_ratio,
        }
    }

    /// Undamped angular frequency ω₀, rad/s.
    #[must_use]
    pub fn angular_frequency(self) -> f64 {
        std::f64::consts::TAU / self.response.max(1e-6)
    }

    /// State after `dt` seconds, starting `displacement` away from the target with `velocity`.
    ///
    /// Displacement is `value - target`; the target is held constant over `dt`.
    #[must_use]
    pub fn evolve(self, x0: f64, v0: f64, dt: f64) -> (f64, f64) {
        if !x0.is_finite()
            || !v0.is_finite()
            || !self.response.is_finite()
            || !self.damping_ratio.is_finite()
        {
            return (0.0, 0.0);
        }
        if !dt.is_finite() {
            return if dt.is_sign_positive() {
                (0.0, 0.0)
            } else {
                (x0, v0)
            };
        }
        let w = self.angular_frequency();
        if !w.is_finite() || w <= 0.0 {
            return (x0, v0);
        }
        let z = self.damping_ratio.clamp(0.0, 1e6);
        let t = dt.max(0.0);
        if (z - 1.0).abs() < 1e-6 {
            // Critically damped: x = (c1 + c2·t)·e^{-w t}
            let e = (-w * t).exp();
            let c2 = v0 + w * x0;
            let x = (x0 + c2 * t) * e;
            let v = (c2 - w * (x0 + c2 * t)) * e;
            (x, v)
        } else if z < 1.0 {
            let wd = w * (1.0 - z * z).sqrt();
            let e = (-z * w * t).exp();
            let (c, s) = ((wd * t).cos(), (wd * t).sin());
            let x = e * (x0 * c + (v0 + z * w * x0) / wd * s);
            let v = e * (v0 * c - (z * w * v0 + w * w * x0) / wd * s);
            (x, v)
        } else {
            let r = (z * z - 1.0).sqrt();
            let r1 = -w * (z - r);
            let r2 = -w * (z + r);
            let denom = r1 - r2;
            if denom.abs() < 1e-12 || !denom.is_finite() {
                return (x0, v0);
            }
            let a = (v0 - r2 * x0) / denom;
            let b = x0 - a;
            let (e1, e2) = ((r1 * t).exp(), (r2 * t).exp());
            (a * e1 + b * e2, a * r1 * e1 + b * r2 * e2)
        }
    }

    /// A precomputed fixed-step integrator: one step is a 2x2 matrix multiply.
    #[must_use]
    pub fn stepper(self, dt: f64) -> SpringStepper {
        let (xx, vx) = self.evolve(1.0, 0.0, dt);
        let (xv, vv) = self.evolve(0.0, 1.0, dt);
        SpringStepper { xx, xv, vx, vv }
    }
}

/// Exact fixed-step update for one [`Spring`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpringStepper {
    xx: f64,
    xv: f64,
    vx: f64,
    vv: f64,
}

impl SpringStepper {
    /// Advance `value` and `velocity` one step toward `target`.
    #[inline]
    pub fn step(&self, value: &mut f64, velocity: &mut f64, target: f64) {
        let d = *value - target;
        let v = *velocity;
        *value = target + self.xx * d + self.xv * v;
        *velocity = self.vx * d + self.vv * v;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn critically_damped_converges_without_overshoot() {
        // Mirrors upstream SpringTests.criticallyDampedConvergesWithoutOvershoot.
        let stepper = Spring::new(0.5, 1.0).stepper(1.0 / 240.0);
        let (mut x, mut v) = (0.0, 0.0);
        let mut prev = x;
        for _ in 0..(240 * 3) {
            stepper.step(&mut x, &mut v, 1.0);
            assert!(x <= 1.0 + 1e-12);
            assert!(x >= prev - 1e-12, "must be monotone");
            prev = x;
        }
        assert!((x - 1.0).abs() < 1e-6);
        assert!(v.abs() < 1e-4);
    }

    #[test]
    fn critically_damped_reaches_target_within_about_one_response() {
        let s = Spring::new(0.6, 1.0);
        let (x, _) = s.evolve(1.0, 0.0, 0.6 * 1.2);
        assert!(x.abs() < 0.01);
    }

    #[test]
    fn stepper_matches_the_analytic_solution() {
        for zeta in [0.4, 1.0, 1.8] {
            let spring = Spring::new(0.3, zeta);
            let stepper = spring.stepper(1.0 / 240.0);
            let (mut x, mut v) = (0.7, -2.0);
            for _ in 0..240 {
                stepper.step(&mut x, &mut v, 0.0);
            }
            let (ex, ev) = spring.evolve(0.7, -2.0, 1.0);
            assert!((x - ex).abs() < 1e-9, "x {x} vs {ex}");
            assert!((v - ev).abs() < 1e-8, "v {v} vs {ev}");
        }
    }

    #[test]
    fn underdamped_overshoots_and_overdamped_converges() {
        let under = Spring::new(0.5, 0.5).stepper(1.0 / 240.0);
        let (mut x, mut v, mut peak) = (0.0f64, 0.0f64, 0.0f64);
        for _ in 0..480 {
            under.step(&mut x, &mut v, 1.0);
            peak = peak.max(x);
        }
        assert!(peak > 1.05, "peak {peak}");

        let over = Spring::new(0.5, 2.0).stepper(1.0 / 240.0);
        x = 0.0;
        v = 0.0;
        for _ in 0..(240 * 5) {
            over.step(&mut x, &mut v, 1.0);
            assert!(x <= 1.0 + 1e-12);
        }
        assert!((x - 1.0).abs() < 1e-3);
    }

    #[test]
    fn a_large_step_is_still_stable() {
        // 20x the response: an integrator would explode, the analytic solution does not.
        let s = Spring::new(0.05, 1.0).stepper(1.0);
        let (mut x, mut v) = (1.0, 50.0);
        for _ in 0..10 {
            s.step(&mut x, &mut v, 0.0);
        }
        assert!(x.is_finite() && x.abs() < 1e-6, "x {x}");
    }
}
