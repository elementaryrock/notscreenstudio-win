//! Pure shell state machine, pause-aware elapsed clock, bar placement, countdown and bubble math.
//!
//! Port of upstream `NSSUI/Shell/ShellPhase.swift`, `ElapsedClock.swift`, `BubbleGeometry.swift`,
//! `ShellCountdown.swift`, and `BarStyle.swift`. Everything here is platform-neutral and tested
//! without a GUI session.

#![allow(dead_code)]

use nss_core::{
    CameraBubbleGeometry, CameraShape, NormalizedPoint, Point2, ProjectReference, Rect, Size2,
};
use serde::{Deserialize, Serialize};

/// Where the app sits in the record -> save -> edit lifecycle.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ShellPhase {
    #[default]
    Idle,
    Preparing,
    Recording {
        #[serde(rename = "startedAt")]
        started_at: f64,
    },
    Saving,
    Editing {
        reference: ProjectReference,
    },
    Failed {
        message: String,
    },
}

impl ShellPhase {
    #[must_use]
    pub fn is_recording(&self) -> bool {
        matches!(self, Self::Recording { .. })
    }

    #[must_use]
    pub fn is_busy(&self) -> bool {
        matches!(
            self,
            Self::Preparing | Self::Recording { .. } | Self::Saving
        )
    }

    #[must_use]
    pub fn can_start(&self) -> bool {
        !self.is_busy()
    }
}

/// Inputs to [`ShellStateMachine::reduce`].
#[derive(Debug, Clone, PartialEq)]
pub enum ShellEvent {
    StartRequested,
    Started(f64),
    StartFailed(String),
    StartCancelled,
    StopRequested,
    DiscardRequested,
    Saved(ProjectReference),
    SaveFailed(String),
    CaptureFailed(String),
    DismissError,
}

/// Pure shell transition table (exact port of upstream `ShellStateMachine`).
pub struct ShellStateMachine;

impl ShellStateMachine {
    #[must_use]
    pub fn reduce(phase: &ShellPhase, event: ShellEvent) -> Option<ShellPhase> {
        match (phase, event) {
            (
                ShellPhase::Idle | ShellPhase::Editing { .. } | ShellPhase::Failed { .. },
                ShellEvent::StartRequested,
            ) => Some(ShellPhase::Preparing),
            (ShellPhase::Preparing, ShellEvent::Started(started_at)) => {
                Some(ShellPhase::Recording { started_at })
            }
            (ShellPhase::Preparing, ShellEvent::StartFailed(message)) => {
                Some(ShellPhase::Failed { message })
            }
            (ShellPhase::Preparing, ShellEvent::StartCancelled) => Some(ShellPhase::Idle),
            (ShellPhase::Recording { .. }, ShellEvent::StopRequested) => Some(ShellPhase::Saving),
            (ShellPhase::Recording { .. }, ShellEvent::DiscardRequested) => Some(ShellPhase::Idle),
            (ShellPhase::Recording { .. }, ShellEvent::CaptureFailed(message)) => {
                Some(ShellPhase::Failed { message })
            }
            (ShellPhase::Saving, ShellEvent::Saved(reference)) => {
                Some(ShellPhase::Editing { reference })
            }
            (ShellPhase::Saving, ShellEvent::SaveFailed(message)) => {
                Some(ShellPhase::Failed { message })
            }
            (ShellPhase::Failed { .. }, ShellEvent::DismissError) => Some(ShellPhase::Idle),
            _ => None,
        }
    }
}

/// Elapsed recording time that freezes while paused (exact port of upstream `ElapsedClock`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ElapsedClock {
    pub started_at: f64,
    paused_at: Option<f64>,
    paused_total: f64,
}

impl ElapsedClock {
    #[must_use]
    pub const fn new(started_at: f64) -> Self {
        Self {
            started_at,
            paused_at: None,
            paused_total: 0.0,
        }
    }

    #[must_use]
    pub const fn is_paused(&self) -> bool {
        self.paused_at.is_some()
    }

    #[must_use]
    pub fn elapsed(&self, now: f64) -> f64 {
        let end = self.paused_at.unwrap_or(now);
        (end - self.started_at - self.paused_total).max(0.0)
    }

    pub fn pause(&mut self, now: f64) {
        if self.paused_at.is_none() {
            self.paused_at = Some(now);
        }
    }

    pub fn resume(&mut self, now: f64) {
        if let Some(p) = self.paused_at.take() {
            self.paused_total += (now - p).max(0.0);
        }
    }
}

/// Formats elapsed seconds as `MM:SS` (or `H:MM:SS` once >= 1 hour) for tray tooltips and HUDs.
#[must_use]
pub fn format_elapsed(seconds: f64) -> String {
    let total = seconds.max(0.0).floor() as u64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

/// Floating bar placement on top-left Windows virtual-desktop work areas.
pub struct BarPlacement;

impl BarPlacement {
    pub const BOTTOM_MARGIN: f64 = 28.0;

    /// Centred horizontally above the bottom edge of `work_area` (origin top-left, Y down).
    #[must_use]
    pub fn frame(size: Size2, work_area: Rect) -> Rect {
        let width = size.width.min(work_area.width());
        let height = size.height.min(work_area.height());
        let x = work_area.min_x() + (work_area.width() - width) * 0.5;
        let y = (work_area.max_y() - Self::BOTTOM_MARGIN - height).max(work_area.min_y());
        Rect::new(x, y, width, height)
    }

    /// Index of the screen holding the largest overlap with `source` (0 if none).
    #[must_use]
    pub fn screen_index(source: Option<Rect>, screens: &[Rect]) -> usize {
        let Some(src) = source else {
            return 0;
        };
        if screens.is_empty() {
            return 0;
        }
        let mut best = 0usize;
        let mut best_area = -1.0f64;
        for (i, &s) in screens.iter().enumerate() {
            let x0 = s.min_x().max(src.min_x());
            let y0 = s.min_y().max(src.min_y());
            let x1 = s.max_x().min(src.max_x());
            let y1 = s.max_y().min(src.max_y());
            let area = if x1 > x0 && y1 > y0 {
                (x1 - x0) * (y1 - y0)
            } else {
                0.0
            };
            if area > best_area {
                best_area = area;
                best = i;
            }
        }
        best
    }
}

/// Geometry of the draggable camera bubble overlay in top-left Windows screen coordinates.
pub struct BubbleGeometry;

impl BubbleGeometry {
    #[must_use]
    pub fn aspect(shape: CameraShape) -> f64 {
        CameraBubbleGeometry::aspect(shape)
    }

    /// Normalizes a top-left bubble overlay rect against the capture source rect.
    #[must_use]
    pub fn normalize(panel_frame: Rect, source_rect: Rect) -> Option<(NormalizedPoint, f64)> {
        if source_rect.width() <= 0.0 || source_rect.height() <= 0.0 {
            return None;
        }
        let mid_x = panel_frame.min_x() + panel_frame.width() * 0.5;
        let mid_y = panel_frame.min_y() + panel_frame.height() * 0.5;
        let x = (mid_x - source_rect.min_x()) / source_rect.width();
        let y = (mid_y - source_rect.min_y()) / source_rect.height();
        Some((
            NormalizedPoint::new(x, y).clamped(),
            panel_frame.width() / source_rect.width(),
        ))
    }

    /// Inverse of [`Self::normalize`] in top-left screen coordinates.
    #[must_use]
    pub fn panel_frame(
        center: NormalizedPoint,
        size: f64,
        shape: CameraShape,
        source_rect: Rect,
    ) -> Rect {
        let width = size * source_rect.width();
        let height = width / Self::aspect(shape);
        let mid_x = source_rect.min_x() + center.x * source_rect.width();
        let mid_y = source_rect.min_y() + center.y * source_rect.height();
        Rect::new(mid_x - width * 0.5, mid_y - height * 0.5, width, height)
    }

    /// Re-expresses a bubble positioned relative to the capture source into output space
    /// (`Style.camera`), matching `RenderLayout`'s content rect.
    #[must_use]
    pub fn to_output_space(
        center: NormalizedPoint,
        size: f64,
        output_aspect: f64,
        source_aspect: f64,
        padding: f64,
    ) -> (NormalizedPoint, f64) {
        let p = padding.clamp(0.0, 0.45);
        let avail_w = (1.0 - 2.0 * p).max(0.01);
        let avail_h = (1.0 - 2.0 * p).max(0.01);
        let avail_aspect = (avail_w * output_aspect) / avail_h;
        let (w, h) = if source_aspect >= avail_aspect {
            (
                avail_w,
                (avail_w * output_aspect / source_aspect).min(avail_h),
            )
        } else {
            (
                (avail_h * source_aspect / output_aspect).min(avail_w),
                avail_h,
            )
        };
        let min_x = (1.0 - w) * 0.5;
        let min_y = (1.0 - h) * 0.5;
        (
            NormalizedPoint::new(min_x + center.x * w, min_y + center.y * h),
            size * w,
        )
    }

    /// Slides `frame` so at least `min_visible` px stay reachable inside `bounds`.
    #[must_use]
    pub fn keep_reachable(frame: Rect, bounds: Rect, min_visible: f64) -> Rect {
        let x = frame.min_x().clamp(
            bounds.min_x() - frame.width() + min_visible,
            bounds.max_x() - min_visible,
        );
        let y = frame.min_y().clamp(
            bounds.min_y() - frame.height() + min_visible,
            bounds.max_y() - min_visible,
        );
        Rect::new(x, y, frame.width(), frame.height())
    }

    #[must_use]
    pub fn clamped_width(width: f64, source_width: f64) -> f64 {
        let min_w = 80.0;
        let max_w = (source_width * 0.5).max(min_w);
        width.clamp(min_w, max_w)
    }
}

/// Drag-to-select area geometry (port of upstream `AreaSelectionMath`).
pub struct AreaSelectionMath;

impl AreaSelectionMath {
    pub const MINIMUM_SIDE: f64 = 32.0;

    #[must_use]
    pub fn rect(a: Point2, b: Point2, bounds: Rect) -> Option<Rect> {
        let x0 = a.x.min(b.x).max(bounds.min_x()).floor();
        let y0 = a.y.min(b.y).max(bounds.min_y()).floor();
        let x1 = a.x.max(b.x).min(bounds.max_x()).ceil();
        let y1 = a.y.max(b.y).min(bounds.max_y()).ceil();
        let w = x1 - x0;
        let h = y1 - y0;
        if w >= Self::MINIMUM_SIDE && h >= Self::MINIMUM_SIDE {
            Some(Rect::new(x0, y0, w, h))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_machine_covers_record_save_edit_and_cancel_flows() {
        let mut p = ShellPhase::Idle;
        assert!(p.can_start());
        p = ShellStateMachine::reduce(&p, ShellEvent::StartRequested).unwrap();
        assert_eq!(p, ShellPhase::Preparing);
        assert!(p.is_busy());

        // Cancel during countdown returns cleanly to Idle.
        let cancelled = ShellStateMachine::reduce(&p, ShellEvent::StartCancelled).unwrap();
        assert_eq!(cancelled, ShellPhase::Idle);

        // Full record -> stop -> saved -> editing -> restart directly from editing.
        p = ShellStateMachine::reduce(&p, ShellEvent::Started(10.0)).unwrap();
        assert!(p.is_recording());
        p = ShellStateMachine::reduce(&p, ShellEvent::StopRequested).unwrap();
        assert_eq!(p, ShellPhase::Saving);
        let r = ProjectReference::new("demo.nssproj");
        p = ShellStateMachine::reduce(&p, ShellEvent::Saved(r.clone())).unwrap();
        assert_eq!(p, ShellPhase::Editing { reference: r });
        assert!(p.can_start());
        assert_eq!(
            ShellStateMachine::reduce(&p, ShellEvent::StartRequested),
            Some(ShellPhase::Preparing)
        );
    }

    #[test]
    fn state_machine_rejects_invalid_transitions() {
        assert_eq!(
            ShellStateMachine::reduce(&ShellPhase::Idle, ShellEvent::StopRequested),
            None
        );
        assert_eq!(
            ShellStateMachine::reduce(&ShellPhase::Preparing, ShellEvent::StopRequested),
            None
        );
        assert_eq!(
            ShellStateMachine::reduce(&ShellPhase::Saving, ShellEvent::StartRequested),
            None
        );
    }

    #[test]
    fn elapsed_clock_excludes_paused_intervals() {
        let mut c = ElapsedClock::new(100.0);
        assert_eq!(c.elapsed(105.0), 5.0);
        c.pause(105.0);
        assert!(c.is_paused());
        assert_eq!(c.elapsed(120.0), 5.0);
        c.resume(115.0);
        assert!(!c.is_paused());
        assert_eq!(c.elapsed(120.0), 10.0);
        assert_eq!(format_elapsed(65.0), "01:05");
        assert_eq!(format_elapsed(3661.0), "1:01:01");
    }

    #[test]
    fn bar_placement_centers_above_bottom_margin_and_picks_dominant_screen() {
        let work = Rect::new(0.0, 0.0, 1920.0, 1040.0);
        let f = BarPlacement::frame(Size2::new(400.0, 56.0), work);
        assert_eq!(f.min_x(), 760.0);
        assert_eq!(f.max_y(), 1040.0 - BarPlacement::BOTTOM_MARGIN);

        let screens = [
            Rect::new(0.0, 0.0, 1920.0, 1080.0),
            Rect::new(1920.0, 0.0, 1280.0, 720.0),
        ];
        assert_eq!(
            BarPlacement::screen_index(Some(Rect::new(2000.0, 50.0, 400.0, 300.0)), &screens),
            1
        );
    }

    #[test]
    fn bubble_geometry_round_trips_and_area_math_enforces_minimum() {
        let src = Rect::new(100.0, 50.0, 1600.0, 900.0);
        let center = NormalizedPoint::new(0.8, 0.75);
        let frame = BubbleGeometry::panel_frame(center, 0.2, CameraShape::Circle, src);
        let (back_center, back_size) = BubbleGeometry::normalize(frame, src).unwrap();
        assert!((back_center.x - center.x).abs() < 1e-9);
        assert!((back_center.y - center.y).abs() < 1e-9);
        assert!((back_size - 0.2).abs() < 1e-9);

        assert!(
            AreaSelectionMath::rect(Point2::new(10.0, 10.0), Point2::new(20.0, 20.0), src)
                .is_none()
        );
        let drag =
            AreaSelectionMath::rect(Point2::new(500.0, 400.0), Point2::new(200.0, 150.0), src)
                .unwrap();
        assert_eq!(drag, Rect::new(200.0, 150.0, 300.0, 250.0));
    }
}
