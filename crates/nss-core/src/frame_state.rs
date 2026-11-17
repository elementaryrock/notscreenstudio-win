//! Per-frame state: CameraState, CursorState, RippleState, FrameState.
//!
//! Port of upstream `NSSCore/FrameState.swift`. This is the contract between the motion
//! model and the renderer: `FrameState` is everything the renderer needs, besides pixels
//! and `Style`, to draw one frame. Preview and export both feed the renderer from here,
//! which is why they match.

use serde::{Deserialize, Serialize};

use crate::basics::{MediaTime, NormalizedPoint};
use crate::cursor::CursorShape;

/// Virtual camera over the SOURCE screen. `scale` 1 = whole source visible; `center` is
/// the normalized source point shown at the center of the content area.
///
/// The motion model is responsible for clamping so the viewport never leaves `0...1`
/// (see `nss-motion` `Viewport`, and the edge-gap guarantee in PLAN.md Appendix B).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CameraState {
    pub scale: f64,
    pub center: NormalizedPoint,
}

impl Default for CameraState {
    fn default() -> Self {
        Self {
            scale: 1.0,
            center: NormalizedPoint::CENTER,
        }
    }
}

impl CameraState {
    pub const IDENTITY: CameraState = CameraState {
        scale: 1.0,
        center: NormalizedPoint::CENTER,
    };

    pub const fn new(scale: f64, center: NormalizedPoint) -> Self {
        Self { scale, center }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RippleState {
    /// Normalized source position of the click.
    pub position: NormalizedPoint,
    /// 0 = just clicked, 1 = fully faded.
    pub progress: f64,
}

impl RippleState {
    pub const fn new(position: NormalizedPoint, progress: f64) -> Self {
        Self { position, progress }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CursorState {
    /// Smoothed position, normalized source space.
    pub position: NormalizedPoint,
    /// Normalized source units per second (used for motion blur direction/length).
    pub velocity: NormalizedPoint,
    pub visible: bool,
    pub ripples: Vec<RippleState>,
    /// `0...1` cursor opacity (idle fade). Multiply into the cursor's alpha; 1 = fully opaque.
    /// `visible == false` implies nothing is drawn regardless of opacity.
    pub opacity: f64,
    /// Cursor image to draw.
    pub shape: CursorShape,
    /// Scale of the cursor while the button is down (1 = normal, below 1 = squished).
    pub press_scale: f64,
}

impl Default for CursorState {
    fn default() -> Self {
        Self {
            position: NormalizedPoint::CENTER,
            velocity: NormalizedPoint::ZERO,
            visible: true,
            ripples: Vec::new(),
            opacity: 1.0,
            shape: CursorShape::Arrow,
            press_scale: 1.0,
        }
    }
}

impl CursorState {
    /// True when the renderer should draw a cursor at all.
    #[must_use]
    pub fn draws(&self) -> bool {
        self.visible && self.opacity > 0.0
    }
}

/// Everything the renderer needs, besides pixels and `Style`, to draw one frame at `time`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrameState {
    pub time: MediaTime,
    pub camera: CameraState,
    pub cursor: CursorState,
}

impl FrameState {
    pub fn new(time: MediaTime) -> Self {
        Self {
            time,
            camera: CameraState::IDENTITY,
            cursor: CursorState::default(),
        }
    }

    pub fn with(time: MediaTime, camera: CameraState, cursor: CursorState) -> Self {
        Self {
            time,
            camera,
            cursor,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_state_decodes_without_opacity_as_fully_opaque() {
        // Mirrors upstream CursorStateCodingTests.decodesWithoutOpacityAsFullyOpaque.
        let json = r#"{"position":{"x":0.25,"y":0.75},"velocity":{"x":1,"y":-2},"visible":false,"ripples":[]}"#;
        let s: CursorState = serde_json::from_str(json).unwrap();
        assert_eq!(s.opacity, 1.0);
        assert_eq!(s.position, NormalizedPoint::new(0.25, 0.75));
        assert_eq!(s.velocity, NormalizedPoint::new(1.0, -2.0));
        assert!(!s.visible);
        // Round 2 fields default when absent.
        assert_eq!(s.shape, CursorShape::Arrow);
        assert_eq!(s.press_scale, 1.0);
    }

    #[test]
    fn cursor_state_opacity_round_trips() {
        let s = CursorState {
            opacity: 0.3,
            ..CursorState::default()
        };
        let text = serde_json::to_string(&s).unwrap();
        let back: CursorState = serde_json::from_str(&text).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn round2_cursor_state_round_trips() {
        // Mirrors upstream Round2ContractTests.oldCursorJSONDecodes' second half.
        let rt = CursorState {
            shape: CursorShape::Crosshair,
            press_scale: 0.8,
            ..CursorState::default()
        };
        let text = serde_json::to_string(&rt).unwrap();
        let back: CursorState = serde_json::from_str(&text).unwrap();
        assert_eq!(back, rt);
    }

    #[test]
    fn partial_cursor_state_json_defaults() {
        let s: CursorState = serde_json::from_str(r#"{"position":{"x":0.5,"y":0.5}}"#).unwrap();
        assert_eq!(s.shape, CursorShape::Arrow);
        assert_eq!(s.press_scale, 1.0);
        assert_eq!(s.opacity, 1.0);
        assert!(s.visible);
    }

    #[test]
    fn camera_state_identity_is_scale_one_and_centered() {
        assert_eq!(CameraState::IDENTITY, CameraState::default());
        assert_eq!(CameraState::default().center, NormalizedPoint::CENTER);
    }

    #[test]
    fn draws_requires_visibility_and_opacity() {
        let mut c = CursorState::default();
        assert!(c.draws());
        c.opacity = 0.0;
        assert!(!c.draws());
        c.opacity = 1.0;
        c.visible = false;
        assert!(!c.draws());
    }
}
