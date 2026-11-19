//! CursorTrack, samples, click events and cursor shapes.
//!
//! Port of upstream `NSSCore/Cursor.swift`. `CursorTrack` is persisted as `cursor.json`
//! inside a project bundle, and it is the input to the whole motion model, so its
//! decode defaults and its binary searches must match upstream exactly.

use serde::de::{self, Deserializer, Visitor};
use serde::{Deserialize, Serialize};
use std::fmt;

use crate::basics::{MediaTime, NormalizedPoint};

/// One cursor position. `x`,`y` normalized `0...1`, origin top-left (may fall slightly
/// outside for area/window capture).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CursorSample {
    pub t: MediaTime,
    pub x: f64,
    pub y: f64,
}

impl CursorSample {
    pub const fn new(t: MediaTime, x: f64, y: f64) -> Self {
        Self { t, x, y }
    }

    #[must_use]
    pub fn point(self) -> NormalizedPoint {
        NormalizedPoint::new(self.x, self.y)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MouseButton {
    Left,
    Right,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClickKind {
    Down,
    Up,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ClickEvent {
    pub t: MediaTime,
    pub x: f64,
    pub y: f64,
    pub button: MouseButton,
    pub kind: ClickKind,
}

impl ClickEvent {
    pub const fn new(t: MediaTime, x: f64, y: f64, button: MouseButton, kind: ClickKind) -> Self {
        Self {
            t,
            x,
            y,
            button,
            kind,
        }
    }

    #[must_use]
    pub fn point(self) -> NormalizedPoint {
        NormalizedPoint::new(self.x, self.y)
    }
}

/// System cursor shape. Unrecognized raw values decode to [`CursorShape::Unknown`].
///
/// The raw strings are Swift's case names, so `CursorShape::IBEAM` writes as `"iBeam"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum CursorShape {
    #[default]
    Arrow,
    IBeam,
    IBeamVertical,
    PointingHand,
    OpenHand,
    ClosedHand,
    Crosshair,
    ResizeLeftRight,
    ResizeUpDown,
    ResizeLeft,
    ResizeRight,
    ResizeUp,
    ResizeDown,
    NotAllowed,
    DragCopy,
    DragLink,
    ContextualMenu,
    Unknown,
}

impl CursorShape {
    /// Every shape, in upstream declaration order (the atlas keys off this).
    pub const ALL: [CursorShape; 18] = [
        Self::Arrow,
        Self::IBeam,
        Self::IBeamVertical,
        Self::PointingHand,
        Self::OpenHand,
        Self::ClosedHand,
        Self::Crosshair,
        Self::ResizeLeftRight,
        Self::ResizeUpDown,
        Self::ResizeLeft,
        Self::ResizeRight,
        Self::ResizeUp,
        Self::ResizeDown,
        Self::NotAllowed,
        Self::DragCopy,
        Self::DragLink,
        Self::ContextualMenu,
        Self::Unknown,
    ];

    /// Raw string exactly as Swift writes it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Arrow => "arrow",
            Self::IBeam => "iBeam",
            Self::IBeamVertical => "iBeamVertical",
            Self::PointingHand => "pointingHand",
            Self::OpenHand => "openHand",
            Self::ClosedHand => "closedHand",
            Self::Crosshair => "crosshair",
            Self::ResizeLeftRight => "resizeLeftRight",
            Self::ResizeUpDown => "resizeUpDown",
            Self::ResizeLeft => "resizeLeft",
            Self::ResizeRight => "resizeRight",
            Self::ResizeUp => "resizeUp",
            Self::ResizeDown => "resizeDown",
            Self::NotAllowed => "notAllowed",
            Self::DragCopy => "dragCopy",
            Self::DragLink => "dragLink",
            Self::ContextualMenu => "contextualMenu",
            Self::Unknown => "unknown",
        }
    }

    /// Parses a raw value; unknown strings become [`CursorShape::Unknown`] (upstream policy:
    /// a newer recording must not fail to load on an older build).
    #[must_use]
    pub fn from_raw(s: &str) -> Self {
        Self::ALL
            .iter()
            .copied()
            .find(|c| c.as_str() == s)
            .unwrap_or(Self::Unknown)
    }
}

impl fmt::Display for CursorShape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for CursorShape {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl Visitor<'_> for V {
            type Value = CursorShape;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a cursor shape name")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<CursorShape, E> {
                Ok(CursorShape::from_raw(v))
            }
        }
        d.deserialize_str(V)
    }
}

/// The cursor changed to `shape` at time `t`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CursorShapeChange {
    pub t: MediaTime,
    pub shape: CursorShape,
}

impl CursorShapeChange {
    pub const fn new(t: MediaTime, shape: CursorShape) -> Self {
        Self { t, shape }
    }
}

/// Logged input for a recording. Persisted as `cursor.json`.
/// Samples are sorted by `t`, nominally `sample_rate` Hz.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CursorTrack {
    pub samples: Vec<CursorSample>,
    pub clicks: Vec<ClickEvent>,
    pub sample_rate: f64,
    /// Shape changes sorted by `t`. Empty for tracks recorded before Round 2.
    pub shapes: Vec<CursorShapeChange>,
}

impl Default for CursorTrack {
    fn default() -> Self {
        Self {
            samples: Vec::new(),
            clicks: Vec::new(),
            sample_rate: 120.0,
            shapes: Vec::new(),
        }
    }
}

impl CursorTrack {
    pub fn new(samples: Vec<CursorSample>) -> Self {
        Self {
            samples,
            ..Self::default()
        }
    }

    /// Shape in effect at `t` (last change with time <= t); [`CursorShape::Arrow`] before the
    /// first change.
    #[must_use]
    pub fn shape(&self, t: MediaTime) -> CursorShape {
        let Some(first) = self.shapes.first() else {
            return CursorShape::Arrow;
        };
        if t < first.t {
            return CursorShape::Arrow;
        }
        // Binary search for the last change with time <= t (same scan as upstream).
        let (mut lo, mut hi) = (0usize, self.shapes.len() - 1);
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            if self.shapes[mid].t <= t {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        self.shapes[lo].shape
    }

    /// Linear interpolation of the raw cursor position at `t` (clamped to the first/last
    /// sample). `None` if the track has no samples.
    #[must_use]
    pub fn raw_position(&self, t: MediaTime) -> Option<NormalizedPoint> {
        let first = self.samples.first()?;
        if !t.is_finite() {
            return Some(first.point());
        }
        let last = self.samples.last()?;
        if t <= first.t {
            return Some(first.point());
        }
        if t >= last.t {
            return Some(last.point());
        }
        let (mut lo, mut hi) = (0usize, self.samples.len() - 1);
        while hi - lo > 1 {
            let mid = (lo + hi) / 2;
            if self.samples[mid].t <= t {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let (a, b) = (self.samples[lo], self.samples[hi]);
        let span = b.t - a.t;
        let f = if span > 0.0 { (t - a.t) / span } else { 0.0 };
        Some(NormalizedPoint::new(
            a.x + (b.x - a.x) * f,
            a.y + (b.y - a.y) * f,
        ))
    }

    /// Presses (`Down`) without a matching `Up`, in time order — what the click ripple
    /// generator and the zoom planner consume.
    pub fn click_downs(&self) -> impl Iterator<Item = &ClickEvent> {
        self.clicks.iter().filter(|c| c.kind == ClickKind::Down)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_shape_unknown_decodes() {
        // Mirrors upstream Round2ContractTests.cursorShapeUnknownDecodes.
        let shapes: Vec<CursorShape> = serde_json::from_str(r#"["iBeam","somethingNew"]"#).unwrap();
        assert_eq!(shapes, vec![CursorShape::IBeam, CursorShape::Unknown]);
        let text = serde_json::to_string(&vec![CursorShape::PointingHand]).unwrap();
        assert_eq!(text, r#"["pointingHand"]"#);
    }

    #[test]
    fn every_shape_name_round_trips() {
        for s in CursorShape::ALL {
            let text = serde_json::to_string(&s).unwrap();
            assert_eq!(text, format!("\"{}\"", s.as_str()));
            let back: CursorShape = serde_json::from_str(&text).unwrap();
            assert_eq!(back, s);
        }
    }

    #[test]
    fn shape_lookup() {
        // Mirrors upstream Round2ContractTests.shapeLookup.
        let t = CursorTrack {
            shapes: vec![
                CursorShapeChange::new(1.0, CursorShape::IBeam),
                CursorShapeChange::new(2.0, CursorShape::PointingHand),
                CursorShapeChange::new(3.0, CursorShape::Arrow),
            ],
            ..CursorTrack::default()
        };
        assert_eq!(t.shape(0.0), CursorShape::Arrow);
        assert_eq!(t.shape(1.0), CursorShape::IBeam);
        assert_eq!(t.shape(1.99), CursorShape::IBeam);
        assert_eq!(t.shape(2.0), CursorShape::PointingHand);
        assert_eq!(t.shape(100.0), CursorShape::Arrow);
        assert_eq!(CursorTrack::default().shape(5.0), CursorShape::Arrow);
    }

    #[test]
    fn old_cursor_json_decodes() {
        // Mirrors upstream Round2ContractTests.oldCursorJSONDecodes.
        let t: CursorTrack = serde_json::from_str(
            r#"{"samples":[{"t":0,"x":0.1,"y":0.2}],"clicks":[],"sampleRate":120}"#,
        )
        .unwrap();
        assert!(t.shapes.is_empty());
        assert_eq!(t.samples.len(), 1);
    }

    #[test]
    fn cursor_track_interpolates() {
        // Mirrors upstream NSSCoreTests.cursorTrackInterpolates.
        let track = CursorTrack::new(vec![
            CursorSample::new(0.0, 0.0, 0.0),
            CursorSample::new(1.0, 1.0, 0.5),
        ]);
        assert_eq!(track.sample_rate, 120.0);
        assert_eq!(
            track.raw_position(0.5),
            Some(NormalizedPoint::new(0.5, 0.25))
        );
        assert_eq!(
            track.raw_position(5.0),
            Some(NormalizedPoint::new(1.0, 0.5))
        );
        assert_eq!(CursorTrack::default().raw_position(0.0), None);
    }

    #[test]
    fn interpolation_survives_duplicate_timestamps() {
        // A zero-length span must not divide by zero.
        let track = CursorTrack::new(vec![
            CursorSample::new(0.0, 0.0, 0.0),
            CursorSample::new(1.0, 1.0, 1.0),
            CursorSample::new(1.0, 0.0, 0.0),
        ]);
        let p = track.raw_position(1.0);
        assert!(p.is_some());
    }

    #[test]
    fn click_downs_filters_presses() {
        let track = CursorTrack {
            clicks: vec![
                ClickEvent::new(0.5, 0.1, 0.1, MouseButton::Left, ClickKind::Down),
                ClickEvent::new(0.6, 0.1, 0.1, MouseButton::Left, ClickKind::Up),
                ClickEvent::new(1.5, 0.2, 0.2, MouseButton::Right, ClickKind::Down),
            ],
            ..CursorTrack::default()
        };
        assert_eq!(track.click_downs().count(), 2);
    }
}
