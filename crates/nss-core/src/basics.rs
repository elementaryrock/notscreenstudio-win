//! Conventions: time, normalised coordinates, colours, pixel sizes.
//!
//! Port of upstream `NSSCore/Basics.swift`. The numeric contract here is load-bearing:
//! every other crate in the workspace speaks this vocabulary.

use serde::{Deserialize, Serialize};

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
        // 3-digit shorthand expands each nibble.
        assert_eq!(
            RGBAColor::from_hex("f00"),
            Some(RGBAColor::new(1.0, 0.0, 0.0, 1.0))
        );
        assert_eq!(RGBAColor::from_hex("nope"), None);
    }

    #[test]
    fn colors_serialise_as_flat_objects() {
        // The on-disk key names must match upstream so fixtures stay loadable.
        let json = serde_json::to_value(RGBAColor::WHITE).expect("serialise");
        assert_eq!(json["r"], 1.0);
        assert_eq!(json["g"], 1.0);
        assert_eq!(json["b"], 1.0);
        assert_eq!(json["a"], 1.0);
    }
}
