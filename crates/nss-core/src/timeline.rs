//! Timeline, Trim, ZoomBlock and ZoomFocus.
//!
//! Port of upstream `NSSCore/Timeline.swift`. Every persisted struct decodes with a
//! default for each non-essential field, so a `project.json` written by an older build
//! still loads (PLAN.md contract, mirroring upstream's `decodeIfPresent` policy).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::basics::{MediaTime, NormalizedPoint};

/// Visible part of the recording. `end == None` means "until the end of the recording".
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TrimRange {
    pub start: MediaTime,
    pub end: Option<MediaTime>,
}

impl TrimRange {
    pub const fn new(start: MediaTime, end: Option<MediaTime>) -> Self {
        Self { start, end }
    }

    #[must_use]
    pub fn resolved_end(&self, duration: MediaTime) -> MediaTime {
        self.end.unwrap_or(duration).min(duration)
    }

    #[must_use]
    pub fn length(&self, duration: MediaTime) -> MediaTime {
        (self.resolved_end(duration) - self.start).max(0.0)
    }
}

/// Where a zoom block looks.
///
/// Swift encodes the unit case as `{"auto":{}}` and the newtype case as
/// `{"manual":{"x":..,"y":..}}`. The Rust variants reproduce that exactly.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ZoomFocus {
    /// Follow the cursor / click targets automatically.
    Auto {},
    /// Fixed focus point (normalized source space).
    Manual(NormalizedPoint),
}

impl ZoomFocus {
    /// The fixed point, if this focus is manual.
    #[must_use]
    pub fn manual_point(self) -> Option<NormalizedPoint> {
        match self {
            Self::Manual(p) => Some(p),
            Self::Auto {} => None,
        }
    }
}

/// One zoom interval on the Zoom track.
#[derive(Debug, Clone, PartialEq)]
pub struct ZoomBlock {
    pub id: Uuid,
    pub start: MediaTime,
    pub end: MediaTime,
    /// 1 = no zoom, 2 = 2x magnification.
    pub scale: f64,
    /// [`ZoomFocus::Auto`] follows the cursor; [`ZoomFocus::Manual`] holds a fixed point.
    pub focus: ZoomFocus,
    /// True for blocks generated from clicks ("Regenerate auto zooms" replaces only these).
    /// Editing a block by hand makes it manual.
    pub is_auto: bool,
    /// How long the zoom takes to arrive (spring response, seconds), starting at `start`.
    pub ease_in: MediaTime,
    /// How long the zoom takes to return to 1x (spring response, seconds), starting at `end`.
    pub ease_out: MediaTime,
}

impl ZoomBlock {
    /// Ease duration (seconds) used when a block does not say otherwise. Matches the
    /// Round 2 zoom spring.
    pub const DEFAULT_EASE: MediaTime = 0.75;

    /// The shortest block the editor allows (PLAN.md Appendix B: the 0.3 s floor).
    pub const MIN_DURATION: MediaTime = 0.3;

    pub fn new(start: MediaTime, end: MediaTime) -> Self {
        Self {
            id: Uuid::new_v4(),
            start,
            end,
            scale: 2.0,
            focus: ZoomFocus::Auto {},
            is_auto: false,
            ease_in: Self::DEFAULT_EASE,
            ease_out: Self::DEFAULT_EASE,
        }
    }

    #[must_use]
    pub fn contains(&self, t: MediaTime) -> bool {
        t >= self.start && t < self.end
    }

    /// Duration of the block.
    #[must_use]
    pub fn duration(&self) -> MediaTime {
        self.end - self.start
    }
}

impl Serialize for ZoomBlock {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = s.serialize_struct("ZoomBlock", 8)?;
        st.serialize_field("id", &UuidString(self.id))?;
        st.serialize_field("start", &self.start)?;
        st.serialize_field("end", &self.end)?;
        st.serialize_field("scale", &self.scale)?;
        st.serialize_field("focus", &self.focus)?;
        st.serialize_field("isAuto", &self.is_auto)?;
        st.serialize_field("easeIn", &self.ease_in)?;
        st.serialize_field("easeOut", &self.ease_out)?;
        st.end()
    }
}

/// Adapter so a UUID writes as Swift's uppercase string.
struct UuidString(Uuid);

impl Serialize for UuidString {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0.to_string().to_uppercase())
    }
}

impl<'de> Deserialize<'de> for ZoomBlock {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Repr {
            #[serde(default, with = "crate::serde_util::uppercase_uuid::option")]
            id: Option<Uuid>,
            start: MediaTime,
            end: MediaTime,
            scale: Option<f64>,
            focus: Option<ZoomFocus>,
            is_auto: Option<bool>,
            ease_in: Option<MediaTime>,
            ease_out: Option<MediaTime>,
        }
        let r = Repr::deserialize(d)?;
        Ok(ZoomBlock {
            id: r.id.unwrap_or_else(Uuid::new_v4),
            start: r.start,
            end: r.end,
            scale: r.scale.unwrap_or(2.0),
            focus: r.focus.unwrap_or(ZoomFocus::Auto {}),
            is_auto: r.is_auto.unwrap_or(false),
            ease_in: r.ease_in.unwrap_or(Self::DEFAULT_EASE),
            ease_out: r.ease_out.unwrap_or(Self::DEFAULT_EASE),
        })
    }
}

/// Editing state of the recording: what to show, and where to zoom.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Timeline {
    pub trim: TrimRange,
    pub zoom_blocks: Vec<ZoomBlock>,
    /// Legacy flag. When true, the motion model derives auto-zooms from clicks on the fly, in
    /// addition to `zoom_blocks`. The editor materializes those auto-zooms into `zoom_blocks`
    /// (with `is_auto`) when a project is first opened and then clears this flag, so the stored
    /// blocks are the single source of truth.
    #[serde(rename = "autoZoomEnabled")]
    pub auto_zoom_enabled: bool,
}

impl Default for Timeline {
    fn default() -> Self {
        Self {
            trim: TrimRange::default(),
            zoom_blocks: Vec::new(),
            auto_zoom_enabled: true,
        }
    }
}

impl Timeline {
    /// Blocks sorted by start time (the invariant the motion model relies on).
    #[must_use]
    pub fn sorted_blocks(&self) -> Vec<ZoomBlock> {
        let mut v = self.zoom_blocks.clone();
        v.sort_by(|a, b| {
            a.start
                .partial_cmp(&b.start)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round2_block_json_decodes_with_defaults() {
        // Verbatim from upstream ZoomBlockModelTests.round2BlockJSONDecodesWithDefaults.
        let json = r#"{"id":"7F0C3A9E-2B7E-4C55-9D1D-0D4B7B8E2F11","start":1,"end":3,"scale":2.5,"focus":{"auto":{}}}"#;
        let b: ZoomBlock = serde_json::from_str(json).unwrap();
        assert_eq!((b.start, b.end, b.scale), (1.0, 3.0, 2.5));
        assert_eq!(b.focus, ZoomFocus::Auto {});
        assert!(!b.is_auto);
        assert_eq!(b.ease_in, ZoomBlock::DEFAULT_EASE);
        assert_eq!(b.ease_out, ZoomBlock::DEFAULT_EASE);
    }

    #[test]
    fn new_fields_round_trip() {
        let b = ZoomBlock {
            start: 2.0,
            end: 4.0,
            scale: 3.0,
            focus: ZoomFocus::Manual(NormalizedPoint::new(0.2, 0.9)),
            is_auto: true,
            ease_in: 0.4,
            ease_out: 1.2,
            ..ZoomBlock::new(2.0, 4.0)
        };
        let text = serde_json::to_string(&b).unwrap();
        let back: ZoomBlock = serde_json::from_str(&text).unwrap();
        assert_eq!(back, b);
    }

    #[test]
    fn minimal_block_json_decodes() {
        let b: ZoomBlock = serde_json::from_str(r#"{"start":0.5,"end":1}"#).unwrap();
        assert_eq!(b.scale, 2.0);
        assert_eq!(b.focus, ZoomFocus::Auto {});
        assert!(!b.is_auto);
    }

    #[test]
    fn focus_encodings_match_swift() {
        assert_eq!(
            serde_json::to_string(&ZoomFocus::Auto {}).unwrap(),
            r#"{"auto":{}}"#
        );
        assert_eq!(
            serde_json::to_string(&ZoomFocus::Manual(NormalizedPoint::new(0.25, 0.5))).unwrap(),
            r#"{"manual":{"x":0.25,"y":0.5}}"#
        );
    }

    #[test]
    fn empty_timeline_json_decodes_with_defaults() {
        let t: Timeline = serde_json::from_str("{}").unwrap();
        assert!(t.auto_zoom_enabled);
        assert!(t.zoom_blocks.is_empty());
        assert_eq!(t.trim, TrimRange::default());
    }

    #[test]
    fn trim_range_clamps_to_the_recording() {
        let t = TrimRange::new(1.0, Some(99.0));
        assert_eq!(t.resolved_end(10.0), 10.0);
        assert_eq!(t.length(10.0), 9.0);
        assert_eq!(TrimRange::new(5.0, Some(2.0)).length(10.0), 0.0);
        assert_eq!(TrimRange::default().resolved_end(7.5), 7.5);
    }
}
