//! Minimal reference model: identity camera, raw interpolated cursor, no ripples.
//!
//! Port of upstream `NSSMotion/LinearMotionModel.swift`. Kept so a caller that has no cursor
//! track (or wants to isolate the renderer from motion) still gets frames; the real model is
//! [`crate::spring_motion_model::SpringMotionModel`].

use nss_core::{
    CameraState, CursorState, CursorStyle, CursorTrack, FrameState, MediaTime, MotionModel,
    NormalizedPoint, Timeline,
};

pub struct LinearMotionModel {
    pub track: CursorTrack,
    pub timeline: Timeline,
    pub cursor_style: CursorStyle,
}

impl LinearMotionModel {
    #[must_use]
    pub fn new(track: CursorTrack, timeline: Timeline, cursor_style: CursorStyle) -> Self {
        Self {
            track,
            timeline,
            cursor_style,
        }
    }
}

impl MotionModel for LinearMotionModel {
    fn frame_state(&self, t: MediaTime) -> FrameState {
        let p = self.track.raw_position(t);
        let cursor = CursorState {
            position: p.unwrap_or(NormalizedPoint::CENTER),
            velocity: NormalizedPoint::ZERO,
            visible: p.is_some(),
            ripples: Vec::new(),
            opacity: if p.is_some() { 1.0 } else { 0.0 },
            shape: self.track.shape(t),
            press_scale: 1.0,
        };
        FrameState::with(t, CameraState::IDENTITY, cursor)
    }

    fn duration(&self) -> MediaTime {
        self.track.samples.last().map_or(0.0, |s| s.t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nss_core::CursorSample;

    #[test]
    fn identity_camera_and_raw_cursor() {
        let track = CursorTrack::new(vec![
            CursorSample::new(0.0, 0.0, 0.0),
            CursorSample::new(2.0, 1.0, 1.0),
        ]);
        let m = LinearMotionModel::new(track, Timeline::default(), CursorStyle::default());
        let s = m.frame_state(1.0);
        assert_eq!(s.camera, CameraState::IDENTITY);
        assert_eq!(s.cursor.position, NormalizedPoint::new(0.5, 0.5));
        assert!(s.cursor.ripples.is_empty());
        assert!(s.cursor.visible);
        assert_eq!(m.duration(), 2.0);
    }

    #[test]
    fn an_empty_track_hides_the_cursor() {
        let m = LinearMotionModel::new(
            CursorTrack::default(),
            Timeline::default(),
            CursorStyle::default(),
        );
        let s = m.frame_state(0.0);
        assert!(!s.cursor.visible);
        assert_eq!(s.cursor.position, NormalizedPoint::CENTER);
        assert_eq!(m.duration(), 0.0);
    }
}
