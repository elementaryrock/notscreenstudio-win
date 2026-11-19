//! Source-to-global coordinate geometry for normalizing cursor positions.
//!
//! Port of upstream `NSSInput/SourceGeometry.swift`. On Windows, virtual-desktop coordinates are
//! already top-left origin with Y pointing downward (matching `nss-core`'s convention), with
//! negative coordinates possible on monitors placed above or left of the primary display.

use nss_core::{CaptureSource, NormalizedPoint, Point2, Rect};

/// Resolves a [`CaptureSource`] to a rectangle in global virtual-desktop coordinates (origin
/// top-left of the primary monitor, Y pointing downward). Injectable for hardware-free tests.
pub trait ScreenGeometryProvider: Send + Sync {
    /// Current global top-left rect of `source`, or `None` when it cannot be resolved (e.g. a
    /// closed window or disconnected display).
    fn global_rect(&self, source: CaptureSource) -> Option<Rect>;

    /// True when the rect of `source` can move during a recording and should be refreshed often.
    fn is_dynamic(&self, source: CaptureSource) -> bool {
        matches!(source, CaptureSource::Window { .. })
    }
}

/// Pure, deterministic coordinate math shared by the input logger and tests.
pub struct CoordinateMath;

impl CoordinateMath {
    /// Normalizes a global top-left point against `rect`. The result is **not** clamped: it can
    /// fall outside `0..=1` for window and area captures, which the contract allows. Returns
    /// `None` for an empty or non-finite rect.
    #[must_use]
    pub fn normalize(point: Point2, rect: Rect) -> Option<NormalizedPoint> {
        let w = rect.width();
        let h = rect.height();
        if !w.is_finite() || !h.is_finite() || w <= 0.0 || h <= 0.0 {
            return None;
        }
        Some(NormalizedPoint::new(
            (point.x - rect.min_x()) / w,
            (point.y - rect.min_y()) / h,
        ))
    }

    /// Resolves a source to a global top-left rect from raw display/window lookups.
    ///
    /// An `Area` rect is in that display's points (origin top-left of the display), so it is
    /// scaled by the display's DPI scale factor and offset by the display's virtual-desktop origin.
    #[must_use]
    pub fn resolve<FD, FW>(
        source: CaptureSource,
        display_bounds: FD,
        window_bounds: FW,
    ) -> Option<Rect>
    where
        FD: FnOnce(u32) -> Option<(Rect, f64)>,
        FW: FnOnce(u32) -> Option<Rect>,
    {
        match source {
            CaptureSource::Display { id } => display_bounds(id).map(|(r, _)| r),
            CaptureSource::Window { id } => window_bounds(id),
            CaptureSource::Area { display_id, rect } => {
                let (display, scale) = display_bounds(display_id)?;
                let s = if scale > 0.0 && scale.is_finite() {
                    scale
                } else {
                    1.0
                };
                Some(Rect::new(
                    display.min_x() + rect.min_x() * s,
                    display.min_y() + rect.min_y() * s,
                    rect.width() * s,
                    rect.height() * s,
                ))
            }
        }
    }
}

/// Live Windows geometry backed by `EnumDisplayMonitors`, `GetDpiForMonitor` and
/// `DwmGetWindowAttribute(DWMWA_EXTENDED_FRAME_BOUNDS)`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemScreenGeometry;

impl SystemScreenGeometry {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl ScreenGeometryProvider for SystemScreenGeometry {
    fn global_rect(&self, source: CaptureSource) -> Option<Rect> {
        #[cfg(windows)]
        {
            let rect = CoordinateMath::resolve(
                source,
                crate::win::display_rect_and_scale,
                crate::win::window_rect,
            )?;
            (rect.width() > 0.0 && rect.height() > 0.0).then_some(rect)
        }
        #[cfg(not(windows))]
        {
            let _ = source;
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_primary_and_secondary_displays() {
        let primary = Rect::new(0.0, 0.0, 1440.0, 900.0);
        assert_eq!(
            CoordinateMath::normalize(Point2::new(720.0, 450.0), primary),
            Some(NormalizedPoint::new(0.5, 0.5))
        );
        assert_eq!(
            CoordinateMath::normalize(Point2::ZERO, primary),
            Some(NormalizedPoint::ZERO)
        );

        // Secondary display to the right, shifted up (negative Y).
        let secondary = Rect::new(1440.0, -200.0, 1920.0, 1080.0);
        let n = CoordinateMath::normalize(Point2::new(1440.0 + 960.0, -200.0 + 540.0), secondary);
        assert_eq!(n, Some(NormalizedPoint::new(0.5, 0.5)));

        // Display to the left of primary (negative X).
        let left = Rect::new(-1920.0, 0.0, 1920.0, 1080.0);
        assert_eq!(
            CoordinateMath::normalize(Point2::new(-960.0, 540.0), left),
            Some(NormalizedPoint::new(0.5, 0.5))
        );
    }

    #[test]
    fn normalize_is_not_clamped_outside_source() {
        let window = Rect::new(100.0, 100.0, 200.0, 100.0);
        assert_eq!(
            CoordinateMath::normalize(Point2::new(50.0, 250.0), window),
            Some(NormalizedPoint::new(-0.25, 1.5))
        );
        assert_eq!(CoordinateMath::normalize(Point2::ZERO, Rect::ZERO), None);
    }

    #[test]
    fn resolve_area_offsets_by_display_origin_and_dpi_scale() {
        let secondary = (Rect::new(1440.0, -200.0, 1920.0, 1080.0), 1.0);
        let r = CoordinateMath::resolve(
            CaptureSource::Area {
                display_id: 2,
                rect: Rect::new(100.0, 50.0, 400.0, 300.0),
            },
            |id| (id == 2).then_some(secondary),
            |_| None,
        );
        assert_eq!(r, Some(Rect::new(1540.0, -150.0, 400.0, 300.0)));

        // Mixed-DPI display at 1.5x scale.
        let hidpi = (Rect::new(1920.0, 0.0, 3840.0, 2160.0), 1.5);
        let scaled = CoordinateMath::resolve(
            CaptureSource::Area {
                display_id: 3,
                rect: Rect::new(100.0, 200.0, 400.0, 200.0),
            },
            |id| (id == 3).then_some(hidpi),
            |_| None,
        );
        assert_eq!(scaled, Some(Rect::new(2070.0, 300.0, 600.0, 300.0)));

        assert_eq!(
            CoordinateMath::resolve(CaptureSource::Display { id: 9 }, |_| None, |_| None),
            None
        );
        assert_eq!(
            CoordinateMath::resolve(
                CaptureSource::Window { id: 5 },
                |_| None,
                |id| (id == 5).then_some(Rect::new(1.0, 2.0, 3.0, 4.0))
            ),
            Some(Rect::new(1.0, 2.0, 3.0, 4.0))
        );
    }
}
