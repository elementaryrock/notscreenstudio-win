//! Built-in gradients, mesh backgrounds and legacy wallpapers.
//!
//! Port of upstream `NSSCore/BackgroundPresets.swift`. Pure data: the renderer draws a
//! background from the stored [`Background`] value, so a project keeps its look even if
//! these presets change later.

use crate::basics::RGBAColor;
use crate::style::{Background, GradientStop, MeshPoint};

/// A named, ready-made background shown in the picker.
#[derive(Debug, Clone, PartialEq)]
pub struct BackgroundPreset {
    pub id: String,
    pub name: String,
    pub background: Background,
}

impl BackgroundPreset {
    pub fn new(id: impl Into<String>, name: impl Into<String>, background: Background) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            background,
        }
    }
}

const fn p(x: f64, y: f64, r: f64, g: f64, b: f64) -> MeshPoint {
    MeshPoint {
        x,
        y,
        color: RGBAColor { r, g, b, a: 1.0 },
    }
}

fn linear(id: &str, name: &str, angle: f64, colors: &[u32]) -> BackgroundPreset {
    let n = (colors.len().saturating_sub(1)).max(1) as f64;
    let stops = colors
        .iter()
        .enumerate()
        .map(|(i, v)| GradientStop::new(i as f64 / n, RGBAColor::from_rgb24(*v)))
        .collect();
    BackgroundPreset::new(
        format!("linear.{id}"),
        name,
        Background::Gradient {
            stops,
            angle_degrees: angle,
        },
    )
}

fn mesh_preset(id: &str, name: &str, points: Vec<MeshPoint>, warp: f64) -> BackgroundPreset {
    BackgroundPreset::new(
        format!("mesh.{id}"),
        name,
        Background::Mesh { points, warp },
    )
}

/// Smooth linear gradients first, then soft multi-colour mesh gradients.
#[must_use]
pub fn gradients() -> Vec<BackgroundPreset> {
    let mut v = vec![
        linear("dusk", "Dusk", 45.0, &[0x5C4DD9, 0xED6B8C]),
        linear("sky", "Sky", 90.0, &[0x4FACFE, 0x00C6FB]),
        linear("apricot", "Apricot", 135.0, &[0xF6D365, 0xFDA085]),
        linear("mint", "Mint", 45.0, &[0x43E97B, 0x38F9D7]),
        linear("lavender", "Lavender", 90.0, &[0xA18CD1, 0xFBC2EB]),
        linear("ember", "Ember", 45.0, &[0xF83600, 0xF9D423]),
        linear("deepsea", "Deep Sea", 90.0, &[0x0F2027, 0x203A43, 0x2C5364]),
        linear("graphite", "Graphite", 90.0, &[0x3A3D40, 0x181A1B]),
    ];
    for (id, name) in [
        ("aurora", "Aurora"),
        ("sunset", "Sunset"),
        ("ocean", "Ocean"),
        ("forest", "Forest"),
        ("midnight", "Midnight"),
        ("peach", "Peach"),
    ] {
        let w = legacy_wallpaper(id);
        v.push(mesh_preset(id, name, w.points, w.warp));
    }
    let candy = vec![
        p(0.0, 0.0, 0.99, 0.60, 0.80),
        p(1.0, 0.1, 0.60, 0.70, 1.00),
        p(0.2, 1.0, 1.00, 0.85, 0.60),
        p(0.9, 0.9, 0.75, 0.55, 0.95),
        p(0.5, 0.45, 1.00, 0.75, 0.85),
    ];
    v.push(mesh_preset("candy", "Candy", candy, 0.05));
    let lagoon = vec![
        p(0.0, 0.2, 0.02, 0.18, 0.35),
        p(0.7, 0.0, 0.00, 0.55, 0.65),
        p(1.0, 0.8, 0.10, 0.80, 0.70),
        p(0.25, 0.95, 0.05, 0.35, 0.55),
    ];
    v.push(mesh_preset("lagoon", "Lagoon", lagoon, 0.06));
    v
}

/// Solid colour swatches (the picker adds a free colour picker on top).
#[must_use]
pub fn colors() -> Vec<BackgroundPreset> {
    [
        ("white", "White", 0xFFFFFF),
        ("silver", "Silver", 0xD9DCE1),
        ("slate", "Slate", 0x5B6270),
        ("charcoal", "Charcoal", 0x26282C),
        ("black", "Black", 0x000000),
        ("red", "Red", 0xE5484D),
        ("orange", "Orange", 0xF76B15),
        ("amber", "Amber", 0xFFC53D),
        ("green", "Green", 0x30A46C),
        ("teal", "Teal", 0x12A594),
        ("sky", "Sky", 0x0090FF),
        ("indigo", "Indigo", 0x3E63DD),
        ("violet", "Violet", 0x6E56CF),
        ("pink", "Pink", 0xD6409F),
        ("sand", "Sand", 0xE9DFCF),
        ("navy", "Navy", 0x14213D),
    ]
    .into_iter()
    .map(|(id, name, hex)| {
        BackgroundPreset::new(
            format!("solid.{id}"),
            name,
            Background::Solid(RGBAColor::from_rgb24(hex)),
        )
    })
    .collect()
}

/// Names the v1 `Wallpaper{name}` case understood (matched case-insensitively).
pub const LEGACY_WALLPAPER_NAMES: [&str; 6] =
    ["aurora", "sunset", "ocean", "forest", "midnight", "peach"];

/// The colour points of a v1 procedural wallpaper. Unknown names fall back to `aurora`,
/// as v1 did.
#[must_use]
pub fn legacy_wallpaper(name: &str) -> Wallpaper {
    let lower = name.to_lowercase();
    let (points, warp) = match lower.as_str() {
        "sunset" => (
            vec![
                p(0.1, 0.1, 0.98, 0.45, 0.35),
                p(0.9, 0.2, 0.99, 0.75, 0.40),
                p(0.2, 0.9, 0.60, 0.20, 0.55),
                p(0.85, 0.85, 0.95, 0.35, 0.50),
            ],
            0.05,
        ),
        "ocean" => (
            vec![
                p(0.0, 0.0, 0.05, 0.25, 0.55),
                p(1.0, 0.1, 0.10, 0.60, 0.80),
                p(0.3, 1.0, 0.15, 0.75, 0.70),
                p(0.9, 0.9, 0.05, 0.35, 0.65),
            ],
            0.04,
        ),
        "forest" => (
            vec![
                p(0.1, 0.2, 0.10, 0.35, 0.25),
                p(0.8, 0.1, 0.45, 0.70, 0.40),
                p(0.5, 0.9, 0.15, 0.50, 0.35),
                p(0.95, 0.75, 0.75, 0.85, 0.50),
            ],
            0.05,
        ),
        "midnight" => (
            vec![
                p(0.0, 0.0, 0.05, 0.05, 0.15),
                p(0.7, 0.3, 0.20, 0.15, 0.45),
                p(0.2, 0.8, 0.08, 0.10, 0.30),
                p(1.0, 1.0, 0.35, 0.20, 0.55),
            ],
            0.03,
        ),
        "peach" => (
            vec![
                p(0.1, 0.1, 1.00, 0.85, 0.75),
                p(0.9, 0.3, 0.98, 0.70, 0.65),
                p(0.3, 0.9, 0.95, 0.80, 0.90),
                p(0.95, 0.95, 1.00, 0.90, 0.70),
            ],
            0.05,
        ),
        // "aurora" and every unknown name.
        _ => (
            vec![
                p(0.0, 0.1, 0.20, 0.15, 0.55),
                p(0.6, 0.0, 0.25, 0.70, 0.85),
                p(1.0, 0.5, 0.55, 0.30, 0.85),
                p(0.3, 1.0, 0.20, 0.80, 0.60),
                p(0.85, 0.95, 0.90, 0.45, 0.70),
            ],
            0.06,
        ),
    };
    Wallpaper { points, warp }
}

/// The colour points and warp of a procedural wallpaper.
#[derive(Debug, Clone, PartialEq)]
pub struct Wallpaper {
    pub points: Vec<MeshPoint>,
    pub warp: f64,
}

/// The v2 equivalent of a v1 `Wallpaper{name}`: the same colour points as a `Mesh`.
#[must_use]
pub fn migrated_wallpaper(name: &str) -> Background {
    let w = legacy_wallpaper(name);
    Background::Mesh {
        points: w.points,
        warp: w.warp,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_cover_linear_and_mesh_gradients_and_colours() {
        // Mirrors upstream BackgroundModelTests.presetsCoverLinearAndMeshGradientsAndColours.
        let g = gradients();
        assert!(g.len() >= 12, "expected >= 12 gradients, got {}", g.len());
        let linear_count = g
            .iter()
            .filter(|p| matches!(p.background, Background::Gradient { .. }))
            .count();
        let mesh_count = g
            .iter()
            .filter(|p| matches!(p.background, Background::Mesh { .. }))
            .count();
        assert!(linear_count >= 6, "linear: {linear_count}");
        assert!(mesh_count >= 6, "mesh: {mesh_count}");
        assert!(colors().len() >= 12);

        let mut ids: Vec<String> = g
            .iter()
            .chain(colors().iter())
            .map(|p| p.id.clone())
            .collect();
        let total = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), total, "preset ids must be unique");

        for p in g {
            assert!(!p.background.is_image_based(), "{} is image based", p.id);
        }
    }

    #[test]
    fn legacy_wallpapers_map_to_mesh_and_unknown_names_fall_back_to_aurora() {
        // Mirrors upstream BackgroundModelTests.legacyWallpapersMapToMeshAndUnknownNamesFallBackToAurora.
        for name in LEGACY_WALLPAPER_NAMES {
            // Mixed case on purpose: v1 matched names case-insensitively.
            let mixed = format!("{}{}", name[..1].to_uppercase(), &name[1..]);
            match migrated_wallpaper(&mixed) {
                Background::Mesh { points, .. } => assert!(
                    points.len() >= 4,
                    "{name} should have at least 4 mesh points"
                ),
                other => panic!("{name} did not migrate to a mesh: {other:?}"),
            }
        }
        assert_eq!(
            migrated_wallpaper("Graphite"),
            migrated_wallpaper("aurora"),
            "an unknown name must fall back to aurora"
        );
    }

    #[test]
    fn linear_stops_span_the_full_range() {
        // A gradient must start at 0 and end at 1, or the shader clips it wrong.
        for p in gradients() {
            if let Background::Gradient { stops, .. } = p.background {
                assert_eq!(stops.first().unwrap().location, 0.0, "{}", p.id);
                assert_eq!(stops.last().unwrap().location, 1.0, "{}", p.id);
                assert!(stops.len() >= 2);
            }
        }
    }

    #[test]
    fn mesh_points_stay_inside_the_unit_square() {
        for p in gradients() {
            if let Background::Mesh { points, warp } = p.background {
                assert!((0.0..=0.1).contains(&warp), "{} warp {warp}", p.id);
                for pt in points {
                    assert!((0.0..=1.0).contains(&pt.x), "{} x {}", p.id, pt.x);
                    assert!((0.0..=1.0).contains(&pt.y), "{} y {}", p.id, pt.y);
                }
            }
        }
    }
}
