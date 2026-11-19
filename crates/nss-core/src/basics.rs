//! Conventions: time, normalised coordinates, colours, pixel sizes, rectangles, timestamps.
//!
//! Port of upstream `NSSCore/Basics.swift`. The numeric contract here is load-bearing:
//! every other crate in the workspace speaks this vocabulary.
//!
//! The `serde` representations deliberately match Swift's `Codable` output byte for byte
//! (PLAN.md §5 Stage 1: "keep the on-disk JSON key names identical to upstream"), because
//! the Mac `project.json` fixtures are reused as golden data.

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Seconds from recording start (t = 0 is the first screen frame).
pub type MediaTime = f64;

/// A point in normalized source space: `0...1`, origin top-left.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct NormalizedPoint {
    pub x: f64,
    pub y: f64,
}

impl NormalizedPoint {
    pub const CENTER: NormalizedPoint = NormalizedPoint { x: 0.5, y: 0.5 };
    pub const ZERO: NormalizedPoint = NormalizedPoint { x: 0.0, y: 0.0 };

    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    /// Clamped to `0...1` on both axes.
    #[must_use]
    pub fn clamped(self) -> Self {
        Self {
            x: self.x.clamp(0.0, 1.0),
            y: self.y.clamp(0.0, 1.0),
        }
    }

    /// True when both components are finite. Motion code rejects non-finite input.
    #[must_use]
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

/// A size in normalized units (fraction of the container).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct NormalizedSize {
    pub width: f64,
    pub height: f64,
}

impl NormalizedSize {
    pub const fn new(width: f64, height: f64) -> Self {
        Self { width, height }
    }
}

/// RGBA colour, components `0...1`, sRGB.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RGBAColor {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}

impl RGBAColor {
    pub const BLACK: RGBAColor = RGBAColor {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    };
    pub const WHITE: RGBAColor = RGBAColor {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a: 1.0,
    };
    pub const CLEAR: RGBAColor = RGBAColor {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    };

    pub const fn new(r: f64, g: f64, b: f64, a: f64) -> Self {
        Self { r, g, b, a }
    }

    /// From `0xRRGGBB` (the form the built-in presets are written in).
    #[must_use]
    pub fn from_rgb24(v: u32) -> Self {
        Self {
            r: f64::from((v >> 16) & 0xFF) / 255.0,
            g: f64::from((v >> 8) & 0xFF) / 255.0,
            b: f64::from(v & 0xFF) / 255.0,
            a: 1.0,
        }
    }

    /// `#RRGGBB` / `#RRGGBBAA` (the form the UI sends).
    #[must_use]
    pub fn to_hex(self) -> String {
        let c = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        if (self.a - 1.0).abs() < f64::EPSILON {
            format!("#{:02x}{:02x}{:02x}", c(self.r), c(self.g), c(self.b))
        } else {
            format!(
                "#{:02x}{:02x}{:02x}{:02x}",
                c(self.r),
                c(self.g),
                c(self.b),
                c(self.a)
            )
        }
    }

    /// Parses `#RGB`, `#RRGGBB` or `#RRGGBBAA` (leading `#` optional). `None` on bad input.
    #[must_use]
    pub fn from_hex(s: &str) -> Option<Self> {
        let s = s.strip_prefix('#').unwrap_or(s);
        if !s.is_ascii() {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).ok();
        let f = |v: u8| f64::from(v) / 255.0;
        match s.len() {
            3 => {
                let mut out = [0u8; 3];
                for (i, slot) in out.iter_mut().enumerate() {
                    let d = s[i..i + 1].chars().next()?;
                    *slot = u8::from_str_radix(&format!("{d}{d}"), 16).ok()?;
                }
                Some(Self::new(f(out[0]), f(out[1]), f(out[2]), 1.0))
            }
            6 => Some(Self::new(f(byte(0)?), f(byte(2)?), f(byte(4)?), 1.0)),
            8 => Some(Self::new(
                f(byte(0)?),
                f(byte(2)?),
                f(byte(4)?),
                f(byte(6)?),
            )),
            _ => None,
        }
    }
}

/// Integer pixel size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PixelSize {
    pub width: u32,
    pub height: u32,
}

impl PixelSize {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    /// width / height. Never divides by zero (returns 1.0 for a zero height).
    #[must_use]
    pub fn aspect(self) -> f64 {
        if self.height == 0 {
            1.0
        } else {
            f64::from(self.width) / f64::from(self.height)
        }
    }
}

/// A rectangle in display points, origin top-left (upstream `CGRect`).
///
/// Swift encodes `CGRect` as `{"origin":{"x":..,"y":..},"size":{"width":..,"height":..}}`;
/// we reproduce that so a `CaptureSource.area` written by either side reads back identically.
/// (This type only appears in runtime recording options, never in `project.json`.)
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Rect {
    pub origin: Point2,
    pub size: Size2,
}

impl Rect {
    pub const ZERO: Rect = Rect::new(0.0, 0.0, 0.0, 0.0);

    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            origin: Point2 { x, y },
            size: Size2 { width, height },
        }
    }

    #[must_use]
    pub fn width(self) -> f64 {
        self.size.width
    }

    #[must_use]
    pub fn height(self) -> f64 {
        self.size.height
    }

    #[must_use]
    pub fn min_x(self) -> f64 {
        self.origin.x
    }

    #[must_use]
    pub fn min_y(self) -> f64 {
        self.origin.y
    }

    #[must_use]
    pub fn max_x(self) -> f64 {
        self.origin.x + self.size.width
    }

    #[must_use]
    pub fn max_y(self) -> f64 {
        self.origin.y + self.size.height
    }

    #[must_use]
    pub fn mid_x(self) -> f64 {
        self.origin.x + self.size.width * 0.5
    }

    #[must_use]
    pub fn mid_y(self) -> f64 {
        self.origin.y + self.size.height * 0.5
    }

    #[must_use]
    pub fn mid(self) -> Point2 {
        Point2::new(self.mid_x(), self.mid_y())
    }

    #[must_use]
    pub fn half_size(self) -> Size2 {
        Size2::new(self.size.width * 0.5, self.size.height * 0.5)
    }

    /// Half-open containment: the min edges are inside, the max edges are not.
    #[must_use]
    pub fn contains(self, x: f64, y: f64) -> bool {
        x >= self.min_x() && x < self.max_x() && y >= self.min_y() && y < self.max_y()
    }
}

/// A point in display points (`CGPoint`).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Point2 {
    pub x: f64,
    pub y: f64,
}

impl Point2 {
    pub const ZERO: Point2 = Point2 { x: 0.0, y: 0.0 };

    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    #[must_use]
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

/// A size in display points (`CGSize`).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Size2 {
    pub width: f64,
    pub height: f64,
}

impl Size2 {
    pub const ZERO: Size2 = Size2 {
        width: 0.0,
        height: 0.0,
    };

    pub const fn new(width: f64, height: f64) -> Self {
        Self { width, height }
    }
}

/// An instant, stored as seconds since the Swift/CoreFoundation reference date
/// Monotonic process-wide host clock in seconds, shared by capture and input loggers so
/// `RecordingAssets::start_host_time` and `InputLogger::stop` always speak the same clock.
#[must_use]
pub fn host_now() -> f64 {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    let epoch = *EPOCH.get_or_init(Instant::now);
    epoch.elapsed().as_secs_f64()
}

/// (2001-01-01T00:00:00Z), which is exactly how Swift's `JSONEncoder` writes a `Date`
/// by default (`deferredToDate`).
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default)]
pub struct Timestamp(pub f64);

/// Seconds between the Unix epoch and the CoreFoundation reference date.
const REFERENCE_DATE_UNIX_OFFSET: f64 = 978_307_200.0;

impl Timestamp {
    pub const ZERO: Timestamp = Timestamp(0.0);

    /// The current instant.
    #[must_use]
    pub fn now() -> Self {
        Self::from_system_time(SystemTime::now())
    }

    #[must_use]
    pub fn from_system_time(t: SystemTime) -> Self {
        match t.duration_since(UNIX_EPOCH) {
            Ok(d) => Self(d.as_secs_f64() - REFERENCE_DATE_UNIX_OFFSET),
            Err(e) => Self(-(e.duration().as_secs_f64() + REFERENCE_DATE_UNIX_OFFSET)),
        }
    }

    #[must_use]
    pub fn to_system_time(self) -> SystemTime {
        let unix = self.0 + REFERENCE_DATE_UNIX_OFFSET;
        if !unix.is_finite() {
            return UNIX_EPOCH;
        }
        if unix >= 0.0 {
            Duration::try_from_secs_f64(unix)
                .ok()
                .and_then(|d| UNIX_EPOCH.checked_add(d))
                .unwrap_or(UNIX_EPOCH)
        } else {
            Duration::try_from_secs_f64(-unix)
                .ok()
                .and_then(|d| UNIX_EPOCH.checked_sub(d))
                .unwrap_or(UNIX_EPOCH)
        }
    }

    /// Seconds since the Unix epoch (for display and for the UI).
    #[must_use]
    pub fn unix_secs(self) -> f64 {
        self.0 + REFERENCE_DATE_UNIX_OFFSET
    }

    #[must_use]
    pub fn from_unix_secs(s: f64) -> Self {
        Self(s - REFERENCE_DATE_UNIX_OFFSET)
    }
}

impl Serialize for Timestamp {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_f64(self.0)
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl Visitor<'_> for V {
            type Value = Timestamp;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a number of seconds since 2001-01-01")
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Timestamp, E> {
                Ok(Timestamp(v))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Timestamp, E> {
                Ok(Timestamp(v as f64))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Timestamp, E> {
                Ok(Timestamp(v as f64))
            }
        }
        d.deserialize_f64(V)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamped_keeps_points_inside_the_unit_square() {
        assert_eq!(
            NormalizedPoint::new(-0.5, 1.5).clamped(),
            NormalizedPoint::new(0.0, 1.0)
        );
        assert_eq!(NormalizedPoint::CENTER.clamped(), NormalizedPoint::CENTER);
    }

    #[test]
    fn aspect_survives_a_zero_height() {
        assert_eq!(PixelSize::new(1920, 0).aspect(), 1.0);
        assert!((PixelSize::new(1920, 1080).aspect() - 16.0 / 9.0).abs() < 1e-12);
    }

    #[test]
    fn hex_round_trips_opaque_and_alpha() {
        assert_eq!(RGBAColor::new(1.0, 0.0, 0.0, 1.0).to_hex(), "#ff0000");
        assert_eq!(
            RGBAColor::from_hex("#ff0000"),
            Some(RGBAColor::new(1.0, 0.0, 0.0, 1.0))
        );
        let a = RGBAColor::from_hex("#00ff0080").expect("8-digit hex");
        assert_eq!(a.a, 128.0 / 255.0);
        assert_eq!(
            RGBAColor::from_hex("f00"),
            Some(RGBAColor::new(1.0, 0.0, 0.0, 1.0))
        );
        assert_eq!(RGBAColor::from_hex("nope"), None);
    }

    #[test]
    fn colors_serialise_as_flat_objects() {
        let json = serde_json::to_value(RGBAColor::WHITE).expect("serialise");
        assert_eq!(json["r"], 1.0);
        assert_eq!(json["g"], 1.0);
        assert_eq!(json["b"], 1.0);
        assert_eq!(json["a"], 1.0);
    }

    #[test]
    fn timestamp_matches_swift_reference_date_encoding() {
        // Swift writes Date as Double seconds since 2001-01-01T00:00:00Z.
        // 2020-01-01T00:00:00Z == unix 1577836800 == reference 599529600.
        let t = Timestamp::from_unix_secs(1_577_836_800.0);
        assert!((t.0 - 599_529_600.0).abs() < 1e-9);
        assert_eq!(serde_json::to_string(&t).unwrap(), "599529600.0");

        let back: Timestamp = serde_json::from_str("599529600.0").unwrap();
        assert!((back.unix_secs() - 1_577_836_800.0).abs() < 1e-6);
    }

    #[test]
    fn timestamp_round_trips_through_system_time() {
        let t = Timestamp::now();
        let back = Timestamp::from_system_time(t.to_system_time());
        assert!((t.0 - back.0).abs() < 1e-3, "{t:?} vs {back:?}");
    }

    #[test]
    fn rect_uses_swifts_origin_size_shape() {
        let json = serde_json::to_value(Rect::new(10.0, 20.0, 100.0, 50.0)).unwrap();
        assert_eq!(json["origin"]["x"], 10.0);
        assert_eq!(json["origin"]["y"], 20.0);
        assert_eq!(json["size"]["width"], 100.0);
        assert_eq!(json["size"]["height"], 50.0);
    }
}
