//! The source region a `CameraState` shows, and the clamp that keeps it inside the source.
//!
//! Port of upstream `NSSMotion/Viewport.swift`. This is the "no gap at the edges" guarantee
//! (PLAN.md Appendix B) and it is enforced on **every** lookup, after the springs, so no
//! overshoot can escape it.
//!
//! ## What the renderer can rely on
//!
//! For a camera with `scale` s and `center` (cx, cy) the visible source region is
//!
//! ```text
//! x in [cx - 0.5 / s, cx + 0.5 / s],   y in [cy - 0.5 / s, cy + 0.5 / s]
//! ```
//!
//! the same fraction `1 / s` of both axes, so the viewport always has the SOURCE aspect ratio.
//! The renderer maps it onto the content rect, which also keeps the source aspect; the OUTPUT
//! aspect only changes the padding around that rect, never which part of the source is shown.
//!
//! Every camera returned by `SpringMotionModel::frame_state` satisfies, at every time, for
//! every track, timeline and tuning (including underdamped springs that overshoot, panning
//! between clicks, clicks at or beyond the corners, and manual blocks focused outside the
//! source):
//!
//! * `scale` is finite and `>= 1`,
//! * `center` is finite,
//! * `0 <= min_x`, `max_x <= 1`, `0 <= min_y`, `max_y <= 1` — exactly, with no tolerance.

use nss_core::{CameraState, NormalizedPoint};

/// The part of the source frame a camera shows, in normalized source coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceViewport {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

impl SourceViewport {
    #[must_use]
    pub fn width(self) -> f64 {
        self.max_x - self.min_x
    }

    #[must_use]
    pub fn height(self) -> f64 {
        self.max_y - self.min_y
    }

    /// True when the viewport lies inside the source, `[0, 1]` on both axes.
    #[must_use]
    pub fn is_inside_source(self) -> bool {
        self.min_x >= 0.0 && self.min_y >= 0.0 && self.max_x <= 1.0 && self.max_y <= 1.0
    }
}

/// The visible source region of a camera, and the clamp that keeps it inside the source.
///
/// A trait rather than inherent methods because [`CameraState`] lives in `nss-core`, which must
/// not depend on motion concerns (ADR 0009).
pub trait CameraViewport {
    /// The visible source region.
    fn source_viewport(self) -> SourceViewport;
}

impl CameraViewport for CameraState {
    fn source_viewport(self) -> SourceViewport {
        let half = 0.5 / self.scale;
        SourceViewport {
            min_x: self.center.x - half,
            min_y: self.center.y - half,
            max_x: self.center.x + half,
            max_y: self.center.y + half,
        }
    }
}

/// A camera whose viewport is inside the source.
///
/// Scale is at least 1 (non-finite becomes 1); the center is clamped to
/// `[0.5 / scale, 1 - 0.5 / scale]` (non-finite becomes 0.5). The final nudges by one ULP make
/// `center -/+ 0.5 / scale` land inside `[0, 1]` **exactly** despite rounding.
#[must_use]
pub fn clamped_to_source(scale: f64, center: NormalizedPoint) -> CameraState {
    let s = if scale.is_finite() {
        scale.max(1.0)
    } else {
        1.0
    };
    let half = 0.5 / s;
    let clamp = |v: f64| -> f64 {
        let mut c = if v.is_finite() {
            v.clamp(half, 1.0 - half)
        } else {
            0.5
        };
        while c + half > 1.0 {
            c = c.next_down();
        }
        while c - half < 0.0 {
            c = c.next_up();
        }
        c
    };
    CameraState::new(s, NormalizedPoint::new(clamp(center.x), clamp(center.y)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic generator, so a failing case can be replayed from its seed.
    struct SplitMix64(u64);

    impl SplitMix64 {
        fn new(seed: u64) -> Self {
            Self(seed)
        }
        fn next_u64(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }
        /// Uniform in `[lo, hi)`.
        fn range(&mut self, lo: f64, hi: f64) -> f64 {
            let u = (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64;
            lo + u * (hi - lo)
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next_u64() % n as u64) as usize
        }
    }

    #[test]
    fn viewport_is_the_documented_region() {
        let c = CameraState::new(2.0, NormalizedPoint::new(0.5, 0.5));
        let v = c.source_viewport();
        assert_eq!((v.min_x, v.max_x), (0.25, 0.75));
        assert_eq!((v.min_y, v.max_y), (0.25, 0.75));
        assert!(v.is_inside_source());
        assert!((v.width() - 1.0 / 2.0).abs() < 1e-12);
    }

    #[test]
    fn clamp_is_exact_for_any_input() {
        // Mirrors upstream ViewportPropertyTests.clampIsExactForAnyInput.
        let mut r = SplitMix64::new(7);
        let specials = [
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            0.0,
            1.0,
            -1.0,
            2.0,
            0.5,
            1e-300,
            1e300,
            f64::EPSILON,
        ];
        for _ in 0..20_000 {
            let scale = if r.next_u64() % 2 == 0 {
                specials[r.below(specials.len())]
            } else {
                r.range(0.0, 50.0)
            };
            let cx = if r.next_u64() % 2 == 0 {
                specials[r.below(specials.len())]
            } else {
                r.range(-2.0, 3.0)
            };
            let cy = r.range(-2.0, 3.0);
            let c = clamped_to_source(scale, NormalizedPoint::new(cx, cy));
            assert!(c.scale.is_finite() && c.scale >= 1.0);
            assert!(c.center.x.is_finite() && c.center.y.is_finite());
            assert!(
                c.source_viewport().is_inside_source(),
                "scale {scale} center {cx},{cy} -> {:?}",
                c.source_viewport()
            );
        }

        // Values already inside are untouched.
        let inside = clamped_to_source(2.0, NormalizedPoint::new(0.3, 0.7));
        assert_eq!(
            inside,
            CameraState::new(2.0, NormalizedPoint::new(0.3, 0.7))
        );
    }

    #[test]
    fn a_settled_corner_zoom_is_pinned_to_the_corner() {
        let c = clamped_to_source(2.0, NormalizedPoint::new(1.0, 1.0));
        let v = c.source_viewport();
        assert_eq!((v.max_x, v.max_y), (1.0, 1.0));
        assert!((v.min_x - 0.5).abs() < 1e-12);
        let d = clamped_to_source(2.0, NormalizedPoint::new(0.0, 0.0));
        assert_eq!(
            (d.source_viewport().min_x, d.source_viewport().min_y),
            (0.0, 0.0)
        );
    }
}
