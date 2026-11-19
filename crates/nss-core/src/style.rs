//! Style, Background, Shadow, AspectRatio, CursorStyle, CameraBubble.
//!
//! Port of upstream `NSSCore/Style.swift`. Defaults are significant: they are what an
//! older `project.json` falls back to when a key is absent, so changing one changes how
//! every existing project looks.

use serde::{Deserialize, Deserializer, Serialize};
use std::path::Path;

use crate::basics::{NormalizedPoint, PixelSize, RGBAColor};

// MARK: - Background

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GradientStop {
    pub location: f64,
    pub color: RGBAColor,
}

impl GradientStop {
    pub const fn new(location: f64, color: RGBAColor) -> Self {
        Self { location, color }
    }
}

/// One colour point of a `.mesh` background. Position is normalized output space
/// (`0...1`, origin top-left).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeshPoint {
    pub x: f64,
    pub y: f64,
    pub color: RGBAColor,
}

impl MeshPoint {
    pub const fn new(x: f64, y: f64, color: RGBAColor) -> Self {
        Self { x, y, color }
    }
}

/// What sits behind the screen content.
///
/// Serialised exactly as Swift's synthesized `Codable` does — externally tagged, with
/// camelCase variant names and struct payloads:
/// `{"solid":{…}}`, `{"gradient":{"stops":[…],"angleDegrees":45}}`,
/// `{"image":{"file":"x"}}`, `{"wallpaper":{"name":"ocean"}}`,
/// `{"mesh":{"points":[…],"warp":0.06}}`, `{"systemWallpaper":{"path":"/w.heic"}}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Background {
    /// A flat colour.
    Solid(RGBAColor),
    /// Linear gradient; `angle_degrees` 0 = left->right, 90 = top->bottom.
    Gradient {
        stops: Vec<GradientStop>,
        angle_degrees: f64,
    },
    /// Image file inside the project bundle (relative name) or an absolute path.
    Image { file: String },
    /// Legacy (schema v1) built-in procedural wallpaper identified by name. Schema v2 projects
    /// store [`Background::Mesh`] instead; the migrator converts these, and the renderer still
    /// draws them via [`crate::background_presets::legacy_wallpaper`].
    Wallpaper { name: String },
    /// Soft multi-colour "mesh" gradient: up to 8 colour points blended smoothly in OKLab,
    /// with a gentle sine warp (`warp` 0 = none, ~0.06 = soft).
    Mesh { points: Vec<MeshPoint>, warp: f64 },
    /// A desktop picture, by absolute path. Drawn exactly like [`Background::Image`]; kept
    /// separate so the picker knows which tab it came from.
    SystemWallpaper { path: String },
}

impl Background {
    /// True for backgrounds drawn from an image file (the ones `Style::background_blur` applies to).
    #[must_use]
    pub fn is_image_based(&self) -> bool {
        matches!(self, Self::Image { .. } | Self::SystemWallpaper { .. })
    }

    /// The colour a caller should paint before the real background is ready (missing image).
    #[must_use]
    pub fn fallback_color(&self) -> RGBAColor {
        match self {
            Self::Solid(c) => *c,
            Self::Gradient { stops, .. } => stops.first().map_or(RGBAColor::BLACK, |s| s.color),
            Self::Mesh { points, .. } => points.first().map_or(RGBAColor::BLACK, |p| p.color),
            Self::Wallpaper { name } => crate::background_presets::legacy_wallpaper(name)
                .points
                .first()
                .map_or(RGBAColor::BLACK, |p| p.color),
            Self::Image { .. } | Self::SystemWallpaper { .. } => RGBAColor::new(0.5, 0.5, 0.5, 1.0),
        }
    }
}

// MARK: - Shadow

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Shadow {
    /// Blur radius as a fraction of output height (`0...~0.1`).
    pub radius: f64,
    pub opacity: f64,
    /// Offset as a fraction of output height.
    pub offset_x: f64,
    pub offset_y: f64,
}

impl Default for Shadow {
    fn default() -> Self {
        Self {
            radius: 0.03,
            opacity: 0.4,
            offset_x: 0.0,
            offset_y: 0.012,
        }
    }
}

// MARK: - Aspect ratio and export resolution

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AspectRatio {
    #[default]
    #[serde(rename = "r16x9")]
    R16x9,
    #[serde(rename = "r1x1")]
    R1x1,
    #[serde(rename = "r9x16")]
    R9x16,
}

impl AspectRatio {
    /// width / height
    #[must_use]
    pub fn ratio(self) -> f64 {
        match self {
            Self::R16x9 => 16.0 / 9.0,
            Self::R1x1 => 1.0,
            Self::R9x16 => 9.0 / 16.0,
        }
    }

    /// Output pixel size; the shorter side equals `resolution.short_side()`.
    /// 16:9 -> 1920x1080 / 3840x2160; 1:1 -> 1080x1080 / 2160x2160; 9:16 -> 1080x1920 / 2160x3840.
    #[must_use]
    pub fn output_size(self, resolution: ExportResolution) -> PixelSize {
        let short = resolution.short_side();
        match self {
            Self::R16x9 => PixelSize::new(short * 16 / 9, short),
            Self::R1x1 => PixelSize::new(short, short),
            Self::R9x16 => PixelSize::new(short, short * 16 / 9),
        }
    }

    /// Every aspect ratio, in the order the picker shows them.
    pub const ALL: [AspectRatio; 3] = [Self::R16x9, Self::R1x1, Self::R9x16];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ExportResolution {
    #[default]
    #[serde(rename = "p1080")]
    P1080,
    #[serde(rename = "p2160")]
    P2160,
}

impl ExportResolution {
    /// Shorter side in pixels (1080 or 2160).
    #[must_use]
    pub fn short_side(self) -> u32 {
        match self {
            Self::P1080 => 1080,
            Self::P2160 => 2160,
        }
    }

    pub const ALL: [ExportResolution; 2] = [Self::P1080, Self::P2160];
}

// MARK: - Cursor style

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CursorStyle {
    /// Multiplier on the default cursor size (1 = native size).
    pub size: f64,
    /// 0 = raw cursor, 1 = heavy smoothing.
    pub smoothing: f64,
    /// 0 = none, 1 = strong.
    pub motion_blur: f64,
    pub click_ripples: bool,
    pub hide_when_idle: bool,
}

impl Default for CursorStyle {
    fn default() -> Self {
        Self {
            size: 1.5,
            smoothing: 0.5,
            motion_blur: 0.4,
            click_ripples: true,
            hide_when_idle: false,
        }
    }
}

// MARK: - Camera bubble

/// Bubble silhouette. Aspect ratios live in [`CameraBubbleGeometry`] (one place).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum CameraShape {
    #[default]
    Circle,
    Square,
    Tall,
    Wide,
}

impl CameraShape {
    /// Every shape, in the order the picker shows them.
    pub const ALL: [Self; 4] = [Self::Circle, Self::Square, Self::Tall, Self::Wide];
}

/// A shape plus the roundness a legacy value implies.
///
/// Round 1 wrote `shape` as `{"circle":{}}`, `{"roundedRect":{"radius":r}}`,
/// `{"squircle":{}}` or `{"rectangle":{}}`. Round 2 writes a plain string. This decodes
/// both, and carries the implied roundness so `CameraBubble` can honour it when the
/// project has no explicit `roundness` key.
#[derive(Debug, Clone, Copy, PartialEq)]
struct LegacyCameraShape {
    shape: CameraShape,
    roundness: Option<f64>,
}

impl<'de> Deserialize<'de> for LegacyCameraShape {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::{self, MapAccess, Visitor};
        use std::fmt;

        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = LegacyCameraShape;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a camera shape name or a legacy shape object")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(match v {
                    "circle" => LegacyCameraShape {
                        shape: CameraShape::Circle,
                        roundness: None,
                    },
                    "square" => LegacyCameraShape {
                        shape: CameraShape::Square,
                        roundness: None,
                    },
                    "tall" => LegacyCameraShape {
                        shape: CameraShape::Tall,
                        roundness: None,
                    },
                    "wide" => LegacyCameraShape {
                        shape: CameraShape::Wide,
                        roundness: None,
                    },
                    "squircle" => LegacyCameraShape {
                        shape: CameraShape::Square,
                        roundness: Some(0.6),
                    },
                    "rectangle" => LegacyCameraShape {
                        shape: CameraShape::Wide,
                        roundness: Some(0.0),
                    },
                    // `roundedRect` only appears with a radius payload; a bare string means the
                    // Round 1 default radius of 0.2, i.e. roundness 0.4.
                    "roundedRect" => LegacyCameraShape {
                        shape: CameraShape::Square,
                        roundness: Some(0.4),
                    },
                    // Unknown names fall back to circle, as upstream does.
                    _ => LegacyCameraShape {
                        shape: CameraShape::Circle,
                        roundness: None,
                    },
                })
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut out = LegacyCameraShape {
                    shape: CameraShape::Circle,
                    roundness: None,
                };
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "roundedRect" => {
                            let payload: Option<RoundedRectPayload> = map.next_value()?;
                            let r = payload
                                .and_then(|p| p.radius)
                                .filter(|v| v.is_finite())
                                .unwrap_or(0.2);
                            out = LegacyCameraShape {
                                shape: CameraShape::Square,
                                // radius is a fraction of half the width in Round 1.
                                roundness: Some((r / 0.5).clamp(0.0, 1.0)),
                            };
                        }
                        "squircle" => {
                            let _: serde::de::IgnoredAny = map.next_value()?;
                            out = LegacyCameraShape {
                                shape: CameraShape::Square,
                                roundness: Some(0.6),
                            };
                        }
                        "rectangle" => {
                            let _: serde::de::IgnoredAny = map.next_value()?;
                            out = LegacyCameraShape {
                                shape: CameraShape::Wide,
                                roundness: Some(0.0),
                            };
                        }
                        "circle" => {
                            let _: serde::de::IgnoredAny = map.next_value()?;
                            out = LegacyCameraShape {
                                shape: CameraShape::Circle,
                                roundness: None,
                            };
                        }
                        _ => {
                            let _: serde::de::IgnoredAny = map.next_value()?;
                        }
                    }
                }
                Ok(out)
            }
        }

        d.deserialize_any(V)
    }
}

#[derive(Deserialize)]
struct RoundedRectPayload {
    #[serde(default)]
    radius: Option<f64>,
}

/// Single source of truth for bubble dimensions, shared by the recording overlay, the
/// renderer and the inspector.
pub struct CameraBubbleGeometry;

/// Dimensions and corner radius for a bubble of a given width.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BubbleLayout {
    pub width: f64,
    pub height: f64,
    pub corner_radius: f64,
}

impl CameraBubbleGeometry {
    /// Camera scale at (and above) which the bubble reaches `zoomed_scale`.
    pub const REFERENCE_ZOOM: f64 = 2.0;

    /// Width / height of each shape.
    #[must_use]
    pub fn aspect(shape: CameraShape) -> f64 {
        match shape {
            CameraShape::Circle | CameraShape::Square => 1.0,
            CameraShape::Tall => 3.0 / 4.0,
            CameraShape::Wide => 16.0 / 9.0,
        }
    }

    /// Dimensions and corner radius for a bubble of `width` (in whatever unit the caller uses).
    /// Radius = roundness * shorter side / 2; a circle is always fully round.
    #[must_use]
    pub fn layout(shape: CameraShape, roundness: f64, width: f64) -> BubbleLayout {
        let w = if width.is_finite() {
            width.max(0.0)
        } else {
            0.0
        };
        let h = w / Self::aspect(shape);
        let shorter = w.min(h);
        let r = if shape == CameraShape::Circle {
            1.0
        } else if roundness.is_finite() {
            roundness.clamp(0.0, 1.0)
        } else {
            0.0
        };
        BubbleLayout {
            width: w,
            height: h,
            corner_radius: r * shorter / 2.0,
        }
    }

    /// Size multiplier for the bubble: 1 at camera scale <= 1, `zoomed_scale` at
    /// `REFERENCE_ZOOM` or more (smoothstep).
    #[must_use]
    pub fn size_multiplier(camera_scale: f64, zoomed_scale: f64) -> f64 {
        if !camera_scale.is_finite() || !zoomed_scale.is_finite() {
            return 1.0;
        }
        let t = ((camera_scale - 1.0) / (Self::REFERENCE_ZOOM - 1.0)).clamp(0.0, 1.0);
        let s = t * t * (3.0 - 2.0 * t);
        1.0 + (zoomed_scale - 1.0) * s
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CameraBorder {
    /// Width as a fraction of output height.
    pub width: f64,
    pub color: RGBAColor,
}

impl Default for CameraBorder {
    fn default() -> Self {
        Self {
            width: 0.004,
            color: RGBAColor::WHITE,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraBubble {
    pub visible: bool,
    pub shape: CameraShape,
    /// Center of the bubble in normalized OUTPUT space (`0...1`, origin top-left).
    pub position: NormalizedPoint,
    /// Bubble width as a fraction of output width; height follows
    /// [`CameraBubbleGeometry::aspect`].
    pub size: f64,
    /// `None` means no border. A missing `border` key decodes to `None`, not the default.
    pub border: Option<CameraBorder>,
    pub mirror: bool,
    /// 0 = sharp corners, 1 = fully round (circle for square, pill for tall/wide).
    /// Ignored for [`CameraShape::Circle`].
    pub roundness: f64,
    /// Size multiplier when the view is fully zoomed in (1 = no shrink).
    pub zoomed_scale: f64,
}

impl Default for CameraBubble {
    fn default() -> Self {
        Self {
            visible: true,
            shape: CameraShape::Circle,
            position: NormalizedPoint::new(0.88, 0.82),
            size: 0.18,
            border: Some(CameraBorder::default()),
            mirror: true,
            roundness: 0.3,
            zoomed_scale: 0.6,
        }
    }
}

impl CameraBubble {
    /// Size multiplier for the current camera zoom; shared by preview and export.
    #[must_use]
    pub fn size_multiplier(&self, camera_scale: f64) -> f64 {
        CameraBubbleGeometry::size_multiplier(camera_scale, self.zoomed_scale)
    }

    /// Layout at a given output width.
    #[must_use]
    pub fn layout(&self, width: f64) -> BubbleLayout {
        CameraBubbleGeometry::layout(self.shape, self.roundness, width)
    }
}

/// Free-function form of [`CameraBubbleGeometry::size_multiplier`] using the given
/// `zoomed_scale` (upstream default 0.6).
#[must_use]
pub fn bubble_size_multiplier(camera_scale: f64, zoomed_scale: f64) -> f64 {
    CameraBubbleGeometry::size_multiplier(camera_scale, zoomed_scale)
}

impl<'de> Deserialize<'de> for CameraBubble {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Repr {
            visible: Option<bool>,
            shape: Option<LegacyCameraShape>,
            roundness: Option<f64>,
            zoomed_scale: Option<f64>,
            position: Option<NormalizedPoint>,
            size: Option<f64>,
            border: Option<CameraBorder>,
            mirror: Option<bool>,
        }

        let r = Repr::deserialize(d)?;
        let d = CameraBubble::default();
        let legacy = r.shape;
        Ok(CameraBubble {
            visible: r.visible.unwrap_or(d.visible),
            shape: legacy.map_or(d.shape, |l| l.shape),
            // An explicit roundness always wins over the one implied by a legacy shape.
            roundness: r
                .roundness
                .or(legacy.and_then(|l| l.roundness))
                .unwrap_or(d.roundness),
            zoomed_scale: r.zoomed_scale.unwrap_or(d.zoomed_scale),
            position: r.position.unwrap_or(d.position),
            size: r.size.unwrap_or(d.size),
            // v1 field: a missing key means nil (border off), never the default.
            border: r.border,
            mirror: r.mirror.unwrap_or(d.mirror),
        })
    }
}

impl Serialize for CameraBubble {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = s.serialize_struct("CameraBubble", 8)?;
        st.serialize_field("visible", &self.visible)?;
        st.serialize_field("shape", &self.shape)?;
        st.serialize_field("position", &self.position)?;
        st.serialize_field("size", &self.size)?;
        st.serialize_field("border", &self.border)?;
        st.serialize_field("mirror", &self.mirror)?;
        st.serialize_field("roundness", &self.roundness)?;
        st.serialize_field("zoomedScale", &self.zoomed_scale)?;
        st.end()
    }
}

// MARK: - Style

/// The look of the output frame: background, padding, corners, shadow, aspect, cursor, camera.
#[derive(Debug, Clone, PartialEq)]
pub struct Style {
    pub background: Background,
    /// Padding around the screen content as a fraction of output height (`0...~0.2`).
    pub padding: f64,
    /// Corner radius of the screen content as a fraction of output height.
    pub corner_radius: f64,
    pub shadow: Shadow,
    pub aspect_ratio: AspectRatio,
    pub cursor: CursorStyle,
    pub camera: CameraBubble,
    /// Blur of image backgrounds (`Image`, `SystemWallpaper`), 0 = sharp ... 1 = strongest.
    /// Ignored for colours and gradients. See [`BackgroundBlur`] for the radius rule.
    pub background_blur: f64,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            background: Background::Gradient {
                stops: vec![
                    GradientStop::new(0.0, RGBAColor::new(0.36, 0.30, 0.85, 1.0)),
                    GradientStop::new(1.0, RGBAColor::new(0.93, 0.42, 0.55, 1.0)),
                ],
                angle_degrees: 45.0,
            },
            padding: 0.06,
            corner_radius: 0.018,
            shadow: Shadow::default(),
            aspect_ratio: AspectRatio::R16x9,
            cursor: CursorStyle::default(),
            camera: CameraBubble::default(),
            background_blur: 0.0,
        }
    }
}

impl Style {
    /// A representative flat color for the background (first gradient stop / solid color /
    /// mid-gray for images).
    #[must_use]
    pub fn background_fallback_color(&self) -> RGBAColor {
        self.background.fallback_color()
    }

    /// A copy whose bundle-relative [`Background::Image`] is an absolute path inside `bundle`
    /// (the `.nssproj` folder).
    ///
    /// The preview and the exporter both render this, so a picked image (copied into the
    /// bundle) resolves the same way in both, without any per-renderer base URL.
    #[must_use]
    pub fn resolving_assets(&self, bundle: &Path) -> Style {
        let Background::Image { file } = &self.background else {
            return self.clone();
        };
        // Upstream: skip empty names and names that are already absolute
        // (`!file.hasPrefix("/")`; on Windows we also treat a drive or UNC path as absolute).
        let already_absolute = file.starts_with('/')
            || file.starts_with('\\')
            || Path::new(file).is_absolute()
            || file.get(1..2).is_some_and(|s| s == ":");
        if file.is_empty() || already_absolute {
            return self.clone();
        }
        let mut s = self.clone();
        s.background = Background::Image {
            file: bundle.join(file).to_string_lossy().into_owned(),
        };
        s
    }
}

impl Serialize for Style {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = s.serialize_struct("Style", 8)?;
        st.serialize_field("background", &self.background)?;
        st.serialize_field("padding", &self.padding)?;
        st.serialize_field("cornerRadius", &self.corner_radius)?;
        st.serialize_field("shadow", &self.shadow)?;
        st.serialize_field("aspectRatio", &self.aspect_ratio)?;
        st.serialize_field("cursor", &self.cursor)?;
        st.serialize_field("camera", &self.camera)?;
        st.serialize_field("backgroundBlur", &self.background_blur)?;
        st.end()
    }
}

impl<'de> Deserialize<'de> for Style {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Repr {
            background: Option<Background>,
            padding: Option<f64>,
            corner_radius: Option<f64>,
            shadow: Option<Shadow>,
            aspect_ratio: Option<AspectRatio>,
            cursor: Option<CursorStyle>,
            camera: Option<CameraBubble>,
            background_blur: Option<f64>,
        }
        let r = Repr::deserialize(d)?;
        let d = Style::default();
        Ok(Style {
            background: r.background.unwrap_or(d.background),
            padding: r.padding.unwrap_or(d.padding),
            corner_radius: r.corner_radius.unwrap_or(d.corner_radius),
            shadow: r.shadow.unwrap_or(d.shadow),
            aspect_ratio: r.aspect_ratio.unwrap_or(d.aspect_ratio),
            cursor: r.cursor.unwrap_or(d.cursor),
            camera: r.camera.unwrap_or(d.camera),
            background_blur: r.background_blur.unwrap_or(d.background_blur),
        })
    }
}

// MARK: - Background blur

/// The blur rule shared by the renderer and the UI: one place, so preview and export agree.
pub struct BackgroundBlur;

impl BackgroundBlur {
    /// Gaussian sigma at `background_blur == 1`, as a fraction of the image's shorter side.
    /// It is relative to the image (not the output), so the same blur looks the same at every
    /// output size.
    pub const MAX_SIGMA_FRACTION: f64 = 0.035;

    /// Sigma in image pixels for an image whose shorter side is `short_side` pixels.
    #[must_use]
    pub fn sigma(amount: f64, short_side: i32) -> f64 {
        if !amount.is_finite() || amount <= 0.0 {
            return 0.0;
        }
        amount.min(1.0) * Self::MAX_SIGMA_FRACTION * f64::from(short_side.max(0))
    }

    /// The amount rounded to 1/100, so a slider drag reuses cached blurs and the cache key
    /// is stable.
    #[must_use]
    pub fn quantized(amount: f64) -> f64 {
        if !amount.is_finite() {
            return 0.0;
        }
        (amount.clamp(0.0, 1.0) * 100.0).round() / 100.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dec<'a, T: Deserialize<'a>>(json: &'a str) -> T {
        serde_json::from_str(json).expect("decode")
    }

    #[test]
    fn style_defaults_round_trip() {
        let s = Style::default();
        let text = serde_json::to_string(&s).unwrap();
        let back: Style = serde_json::from_str(&text).unwrap();
        assert_eq!(back, s);
        // Key names must match upstream exactly.
        assert!(text.contains("\"cornerRadius\":"));
        assert!(text.contains("\"aspectRatio\":\"r16x9\""));
        assert!(text.contains("\"angleDegrees\":45.0"));
    }

    #[test]
    fn new_backgrounds_round_trip() {
        // Mirrors upstream BackgroundModelTests.newBackgroundsRoundTrip.
        let cases = vec![
            Background::Mesh {
                points: vec![
                    MeshPoint::new(0.0, 0.0, RGBAColor::WHITE),
                    MeshPoint::new(1.0, 1.0, RGBAColor::BLACK),
                ],
                warp: 0.05,
            },
            Background::SystemWallpaper {
                path: "/System/Library/Desktop Pictures/Sonoma.heic".into(),
            },
            Background::Image {
                file: "background-abc-photo.png".into(),
            },
            Background::Wallpaper {
                name: "ocean".into(),
            },
        ];
        for bg in cases {
            let style = Style {
                background: bg.clone(),
                background_blur: 0.42,
                ..Style::default()
            };
            let text = serde_json::to_string(&style).unwrap();
            let back: Style = serde_json::from_str(&text).unwrap();
            assert_eq!(back, style, "round trip failed for {text}");
        }
    }

    #[test]
    fn blur_defaults_to_zero_when_missing() {
        let mut json = serde_json::to_value(Style::default()).unwrap();
        json.as_object_mut().unwrap().remove("backgroundBlur");
        let style: Style = serde_json::from_value(json).unwrap();
        assert_eq!(style.background_blur, 0.0);
    }

    #[test]
    fn blur_sigma_is_relative_to_the_image_and_quantized() {
        assert_eq!(BackgroundBlur::sigma(0.0, 2000), 0.0);
        assert_eq!(BackgroundBlur::sigma(-1.0, 2000), 0.0);
        assert_eq!(BackgroundBlur::sigma(f64::NAN, 2000), 0.0);
        let full = BackgroundBlur::sigma(1.0, 2000);
        assert!((full - BackgroundBlur::MAX_SIGMA_FRACTION * 2000.0).abs() < 1e-9);
        assert_eq!(BackgroundBlur::sigma(2.0, 2000), full);
        // Scale invariance: half the pixels, half the sigma.
        let half = BackgroundBlur::sigma(0.5, 1000);
        assert!((half * 2.0 - BackgroundBlur::sigma(0.5, 2000)).abs() < 1e-9);
        assert_eq!(BackgroundBlur::quantized(0.123), 0.12);
        assert_eq!(BackgroundBlur::quantized(1.7), 1.0);
        assert_eq!(BackgroundBlur::quantized(-0.2), 0.0);
    }

    #[test]
    fn resolving_assets_makes_only_relative_images_absolute() {
        let bundle = Path::new("/tmp/p.nssproj");
        let style = Style {
            background: Background::Image {
                file: "bg.png".into(),
            },
            ..Style::default()
        };
        let resolved = style.resolving_assets(bundle);
        assert_eq!(
            resolved.background,
            Background::Image {
                file: bundle.join("bg.png").to_string_lossy().into_owned()
            }
        );

        for bg in [
            Background::Image {
                file: "/abs/x.png".into(),
            },
            Background::Image {
                file: String::new(),
            },
            Background::SystemWallpaper {
                path: "/w.heic".into(),
            },
            Background::Solid(RGBAColor::WHITE),
        ] {
            let style = Style {
                background: bg,
                ..Style::default()
            };
            assert_eq!(style.resolving_assets(bundle), style);
        }
    }

    #[test]
    fn image_based_flag() {
        assert!(Background::Image { file: "a".into() }.is_image_based());
        assert!(Background::SystemWallpaper { path: "/a".into() }.is_image_based());
        assert!(!Background::Solid(RGBAColor::WHITE).is_image_based());
        assert!(!Background::Wallpaper {
            name: "aurora".into()
        }
        .is_image_based());
    }

    #[test]
    fn bubble_migrations() {
        // Mirrors upstream Round2ContractTests.bubbleMigrations.
        let bubble = |shape: &str| -> CameraBubble {
            dec(&format!(r#"{{"visible":true,"shape":{shape},"size":0.2}}"#))
        };
        assert_eq!(bubble(r#"{"circle":{}}"#).shape, CameraShape::Circle);
        let rr = bubble(r#"{"roundedRect":{"radius":0.2}}"#);
        assert_eq!(rr.shape, CameraShape::Square);
        assert!((rr.roundness - 0.4).abs() < 1e-9);
        let sq = bubble(r#"{"squircle":{}}"#);
        assert_eq!((sq.shape, sq.roundness), (CameraShape::Square, 0.6));
        let rect = bubble(r#"{"rectangle":{}}"#);
        assert_eq!((rect.shape, rect.roundness), (CameraShape::Wide, 0.0));

        // A missing shape and border key fall back to the documented defaults.
        let none: CameraBubble = dec(r#"{"size":0.2}"#);
        assert_eq!(none.zoomed_scale, 0.6);
        assert!(none.border.is_none());

        let new = CameraBubble {
            shape: CameraShape::Tall,
            roundness: 0.7,
            zoomed_scale: 0.5,
            ..CameraBubble::default()
        };
        let text = serde_json::to_string(&new).unwrap();
        let back: CameraBubble = serde_json::from_str(&text).unwrap();
        assert_eq!(back, new);
    }

    #[test]
    fn bubble_geometry() {
        let sq = CameraBubbleGeometry::layout(CameraShape::Square, 1.0, 100.0);
        assert_eq!(sq.height, 100.0);
        assert_eq!(sq.corner_radius, 50.0);

        let tall = CameraBubbleGeometry::layout(CameraShape::Tall, 1.0, 300.0);
        assert_eq!(tall.height, 400.0);
        assert_eq!(tall.corner_radius, 150.0);

        let wide = CameraBubbleGeometry::layout(CameraShape::Wide, 0.0, 160.0);
        assert!((wide.height - 90.0).abs() < 1e-9);
        assert_eq!(wide.corner_radius, 0.0);

        assert_eq!(
            CameraBubbleGeometry::layout(CameraShape::Circle, 0.0, 80.0).corner_radius,
            40.0
        );
    }

    #[test]
    fn size_multiplier() {
        assert_eq!(bubble_size_multiplier(1.0, 0.6), 1.0);
        assert_eq!(bubble_size_multiplier(0.5, 0.6), 1.0);
        assert!((bubble_size_multiplier(2.0, 0.6) - 0.6).abs() < 1e-12);
        assert!((bubble_size_multiplier(4.0, 0.6) - 0.6).abs() < 1e-12);
        assert!((bubble_size_multiplier(1.5, 0.6) - 0.8).abs() < 1e-12);
        let no_shrink = CameraBubble {
            zoomed_scale: 1.0,
            ..CameraBubble::default()
        };
        assert_eq!(no_shrink.size_multiplier(2.0), 1.0);
    }

    #[test]
    fn aspect_ratio_output_sizes() {
        assert_eq!(
            AspectRatio::R16x9.output_size(ExportResolution::P1080),
            PixelSize::new(1920, 1080)
        );
        assert_eq!(
            AspectRatio::R16x9.output_size(ExportResolution::P2160),
            PixelSize::new(3840, 2160)
        );
        assert_eq!(
            AspectRatio::R1x1.output_size(ExportResolution::P1080),
            PixelSize::new(1080, 1080)
        );
        assert_eq!(
            AspectRatio::R9x16.output_size(ExportResolution::P2160),
            PixelSize::new(2160, 3840)
        );
    }
}
