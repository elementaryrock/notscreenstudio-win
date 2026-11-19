//! The procedural part of a background reduced to shader inputs, plus its CPU twin.
//!
//! Port of upstream `NSSRender/BackgroundRasterizer.swift`. The renderer writes [`BackgroundShading`]
//! into uniforms and [`BackgroundRasterizer`] evaluates the same maths on the CPU, so the two cannot
//! drift: the background picker's thumbnails and the GPU tests both go through this type.

use nss_core::background_presets;
use nss_core::{Background, MeshPoint, RGBAColor};

use crate::color::{linear, linear_to_byte, oklab, oklab_to_linear};
use crate::shaders::MAX_STOPS;

/// A background reduced to what the shader needs. Colours are linear light.
#[derive(Debug, Clone, PartialEq)]
pub enum BackgroundShading {
    /// Linear-light premultiplied colour: rgb + alpha.
    Solid([f64; 4]),
    /// OKLab + alpha stops sorted by location; `dir` is the unit direction, `inv_length` =
    /// 1 / gradient length in pixels.
    Linear {
        stops: Vec<[f64; 4]>,
        locations: Vec<f64>,
        dir: [f64; 2],
        inv_length: f64,
    },
    /// OKLab colour points at normalized positions, with the sine warp amount.
    Mesh {
        colors: Vec<[f64; 3]>,
        positions: Vec<[f64; 2]>,
        warp: f64,
    },
    /// Drawn from an image texture (or mid-grey when the image is missing).
    Image,
}

/// CPU rendering of colour, gradient and mesh backgrounds with exactly the shader's maths.
pub struct BackgroundRasterizer;

impl BackgroundRasterizer {
    /// The grey a missing image background falls back to.
    pub const MISSING_IMAGE_GREY: RGBAColor = RGBAColor::new(0.5, 0.5, 0.5, 1.0);

    /// The shader inputs for `background` at a `w` x `h` output.
    #[must_use]
    pub fn shading(background: &Background, w: f64, h: f64) -> BackgroundShading {
        match background {
            Background::Solid(c) => Self::solid(*c),
            Background::Gradient {
                stops: raw,
                angle_degrees,
            } => {
                let mut sorted: Vec<_> = raw.iter().collect();
                sorted.sort_by(|a, b| {
                    a.location
                        .partial_cmp(&b.location)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                sorted.truncate(MAX_STOPS);
                let Some(first) = sorted.first() else {
                    return Self::solid(RGBAColor::BLACK);
                };
                if sorted.len() == 1 {
                    return Self::solid(first.color);
                }
                let angle = if angle_degrees.is_finite() {
                    *angle_degrees
                } else {
                    0.0
                };
                let a = angle * std::f64::consts::PI / 180.0;
                let (dx, dy) = (a.cos(), a.sin());
                let half_extent = 0.5 * (w * dx.abs() + h * dy.abs());
                BackgroundShading::Linear {
                    stops: sorted
                        .iter()
                        .map(|s| {
                            let l = oklab(s.color);
                            let alpha = if s.color.a.is_finite() {
                                s.color.a.clamp(0.0, 1.0)
                            } else {
                                1.0
                            };
                            [l[0], l[1], l[2], alpha]
                        })
                        .collect(),
                    locations: sorted
                        .iter()
                        .map(|s| {
                            if s.location.is_finite() {
                                s.location
                            } else {
                                0.0
                            }
                        })
                        .collect(),
                    dir: [dx, dy],
                    inv_length: 1.0 / (2.0 * half_extent).max(1.0),
                }
            }
            Background::Mesh { points, warp } => Self::mesh(points, *warp),
            Background::Wallpaper { name } => {
                let w = background_presets::legacy_wallpaper(name);
                Self::mesh(&w.points, w.warp)
            }
            Background::Image { .. } | Background::SystemWallpaper { .. } => {
                BackgroundShading::Image
            }
        }
    }

    /// A solid shading (linear-light premultiplied).
    #[must_use]
    pub fn solid(c: RGBAColor) -> BackgroundShading {
        let lin = linear(c);
        let alpha = if c.a.is_finite() {
            c.a.clamp(0.0, 1.0)
        } else {
            1.0
        };
        BackgroundShading::Solid([lin[0], lin[1], lin[2], alpha])
    }

    /// sRGB 8-bit RGBA pixels of `background` at `width` x `height` (see
    /// [`BackgroundShading::to_rgba`]).
    #[must_use]
    pub fn rgba(background: &Background, width: usize, height: usize) -> Vec<u8> {
        let w = width.max(1) as f64;
        let h = height.max(1) as f64;
        Self::shading(background, w, h).to_rgba(width.max(1), height.max(1))
    }

    fn mesh(raw: &[MeshPoint], warp: f64) -> BackgroundShading {
        let points: Vec<&MeshPoint> = raw.iter().take(MAX_STOPS).collect();
        let Some(first) = points.first() else {
            return Self::solid(RGBAColor::BLACK);
        };
        if points.len() == 1 {
            return Self::solid(first.color);
        }
        BackgroundShading::Mesh {
            colors: points.iter().map(|p| oklab(p.color)).collect(),
            positions: points
                .iter()
                .map(|p| {
                    [
                        if p.x.is_finite() { p.x } else { 0.5 },
                        if p.y.is_finite() { p.y } else { 0.5 },
                    ]
                })
                .collect(),
            warp: if warp.is_finite() { warp } else { 0.0 },
        }
    }
}

impl BackgroundShading {
    /// Linear-light colour at pixel centre `p` of a `size` output: the CPU twin of the shader's
    /// `background()`.
    #[must_use]
    pub fn linear_color(&self, p: [f64; 2], size: [f64; 2]) -> [f64; 3] {
        match self {
            Self::Solid(c) => [c[0] * c[3], c[1] * c[3], c[2] * c[3]],
            Self::Linear {
                stops,
                locations,
                dir,
                inv_length,
            } => {
                let d = [p[0] - 0.5 * size[0], p[1] - 0.5 * size[1]];
                let t = ((d[0] * dir[0] + d[1] * dir[1]) * inv_length + 0.5).clamp(0.0, 1.0);
                let mut c = stops[0];
                for i in 1..stops.len() {
                    let l0 = locations[i - 1];
                    let l1 = locations[i];
                    let f = if t >= l0 {
                        ((t - l0) / (l1 - l0).max(1e-5)).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    for k in 0..4 {
                        c[k] += (stops[i][k] - c[k]) * f;
                    }
                }
                let rgb = oklab_to_linear([c[0], c[1], c[2]]);
                [rgb[0] * c[3], rgb[1] * c[3], rgb[2] * c[3]]
            }
            Self::Mesh {
                colors,
                positions,
                warp,
            } => {
                let ux = p[0] / size[0];
                let uy = p[1] / size[1];
                let uv = [
                    ux + warp * (uy * 5.1 + 1.3).sin(),
                    uy + warp * (ux * 4.3 + 0.7).sin(),
                ];
                let m = size[0].max(size[1]);
                let aspect = [size[0] / m, size[1] / m];
                let mut acc = [0.0_f64; 3];
                let mut wsum = 0.0;
                for i in 0..colors.len() {
                    let d = [
                        (uv[0] - positions[i][0]) * aspect[0],
                        (uv[1] - positions[i][1]) * aspect[1],
                    ];
                    let w = 1.0 / (d[0] * d[0] + d[1] * d[1] + 0.015).powf(1.6);
                    for k in 0..3 {
                        acc[k] += colors[i][k] * w;
                    }
                    wsum += w;
                }
                oklab_to_linear([acc[0] / wsum, acc[1] / wsum, acc[2] / wsum])
            }
            Self::Image => linear(Self::grey()),
        }
    }

    fn grey() -> RGBAColor {
        BackgroundRasterizer::MISSING_IMAGE_GREY
    }

    /// sRGB 8-bit pixels (RGBA, row-major, no dither) of `background` at `width` x `height`.
    ///
    /// Image backgrounds come out mid-grey: use the image file for their thumbnails.
    #[must_use]
    pub fn to_rgba(&self, width: usize, height: usize) -> Vec<u8> {
        let w = width.max(1);
        let h = height.max(1);
        let shading = self;
        let size = [w as f64, h as f64];
        let mut out = vec![255_u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let c = shading.linear_color([x as f64 + 0.5, y as f64 + 0.5], size);
                let i = (y * w + x) * 4;
                out[i] = linear_to_byte(c[0]);
                out[i + 1] = linear_to_byte(c[1]);
                out[i + 2] = linear_to_byte(c[2]);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nss_core::GradientStop;

    fn red() -> RGBAColor {
        RGBAColor::new(1.0, 0.0, 0.0, 1.0)
    }
    fn green() -> RGBAColor {
        RGBAColor::new(0.0, 1.0, 0.0, 1.0)
    }
    fn blue() -> RGBAColor {
        RGBAColor::new(0.0, 0.0, 1.0, 1.0)
    }

    #[test]
    fn a_solid_background_is_its_colour() {
        let px = BackgroundRasterizer::rgba(
            &Background::Solid(RGBAColor::new(1.0, 0.0, 0.0, 1.0)),
            2,
            2,
        );
        assert_eq!(&px[0..4], &[255, 0, 0, 255]);
        assert_eq!(&px[12..16], &[255, 0, 0, 255]);
    }

    #[test]
    fn a_gradient_runs_left_to_right_at_zero_degrees() {
        let bg = Background::Gradient {
            stops: vec![
                GradientStop::new(0.0, red()),
                GradientStop::new(1.0, blue()),
            ],
            angle_degrees: 0.0,
        };
        let px = BackgroundRasterizer::rgba(&bg, 64, 4);
        let left = &px[0..3];
        let right = &px[(63 * 4)..(63 * 4) + 3];
        assert!(left[0] > 240 && left[2] < 20, "{left:?}");
        assert!(right[2] > 240 && right[0] < 20, "{right:?}");
        // The middle is a blend of the two, not a step.
        let mid = &px[(32 * 4)..(32 * 4) + 3];
        assert!(
            mid[0] > 20 && mid[0] < 240 && mid[2] > 20 && mid[2] < 240,
            "{mid:?}"
        );
    }

    #[test]
    fn a_gradient_sorts_its_stops_and_single_stop_is_a_solid() {
        let bg = Background::Gradient {
            stops: vec![
                GradientStop::new(1.0, blue()),
                GradientStop::new(0.0, red()),
            ],
            angle_degrees: 0.0,
        };
        match BackgroundRasterizer::shading(&bg, 100.0, 100.0) {
            BackgroundShading::Linear { locations, .. } => {
                assert!(locations[0] < locations[1], "{locations:?}");
            }
            other => panic!("expected a linear shading, got {other:?}"),
        }
        let one = Background::Gradient {
            stops: vec![GradientStop::new(0.5, green())],
            angle_degrees: 45.0,
        };
        match BackgroundRasterizer::shading(&one, 10.0, 10.0) {
            BackgroundShading::Solid(c) => assert!((c[1] - 1.0).abs() < 1e-9),
            other => panic!("expected solid, got {other:?}"),
        }
    }

    #[test]
    fn a_vertical_gradient_is_uniform_along_rows() {
        let bg = Background::Gradient {
            stops: vec![
                GradientStop::new(0.0, RGBAColor::WHITE),
                GradientStop::new(1.0, RGBAColor::BLACK),
            ],
            angle_degrees: 90.0,
        };
        let px = BackgroundRasterizer::rgba(&bg, 16, 32);
        let top = &px[0..3];
        let bottom = &px[((31 * 16) * 4)..((31 * 16) * 4) + 3];
        assert!(top[0] > 240, "{top:?}");
        assert!(bottom[0] < 15, "{bottom:?}");
    }

    #[test]
    fn a_mesh_blends_its_colours_smoothly() {
        let bg = Background::Mesh {
            points: vec![
                MeshPoint::new(0.0, 0.0, red()),
                MeshPoint::new(1.0, 0.0, blue()),
            ],
            warp: 0.0,
        };
        let px = BackgroundRasterizer::rgba(&bg, 320, 180);
        let at = |x: usize, y: usize| -> [u8; 3] {
            let i = (y * 320 + x) * 4;
            [px[i], px[i + 1], px[i + 2]]
        };
        let left = at(1, 1);
        let right = at(318, 1);
        assert!(left[0] > 200 && left[2] < 80, "{left:?}");
        assert!(right[2] > 200 && right[0] < 80, "{right:?}");
        // No hard steps along the top row.
        let mut max_step = 0_i32;
        for x in 1..320 {
            let a = at(x, 2);
            let b = at(x - 1, 2);
            let la = (i32::from(a[0]) + i32::from(a[1]) + i32::from(a[2])) / 3;
            let lb = (i32::from(b[0]) + i32::from(b[1]) + i32::from(b[2])) / 3;
            max_step = max_step.max((la - lb).abs());
        }
        assert!(max_step <= 6, "max step {max_step}");
    }

    #[test]
    fn a_legacy_wallpaper_draws_like_its_migrated_mesh() {
        for name in background_presets::LEGACY_WALLPAPER_NAMES {
            let legacy = Background::Wallpaper {
                name: (*name).to_owned(),
            };
            let mesh = background_presets::migrated_wallpaper(name);
            assert_eq!(
                BackgroundRasterizer::rgba(&legacy, 32, 18),
                BackgroundRasterizer::rgba(&mesh, 32, 18),
                "{name}"
            );
        }
    }

    #[test]
    fn an_image_background_rasterizes_as_the_grey_fallback() {
        let px = BackgroundRasterizer::rgba(
            &Background::Image {
                file: "nope.png".to_owned(),
            },
            2,
            2,
        );
        assert_eq!(&px[0..3], &[128, 128, 128]);
    }

    #[test]
    fn every_built_in_gradient_produces_a_visible_ramp() {
        for preset in background_presets::gradients() {
            let px = BackgroundRasterizer::rgba(&preset.background, 64, 36);
            // Corner to opposite corner, so a vertical or diagonal preset is not judged flat along
            // a single row.
            let first = &px[0..3];
            let last_index = (36 * 64 - 1) * 4;
            let last = &px[last_index..last_index + 3];
            assert_ne!(first, last, "{} is flat", preset.id);
        }
    }

    #[test]
    fn a_non_finite_warp_cannot_poison_a_mesh() {
        let shading = BackgroundRasterizer::shading(
            &Background::Mesh {
                points: vec![
                    MeshPoint::new(0.0, 0.0, red()),
                    MeshPoint::new(1.0, 1.0, blue()),
                ],
                warp: f64::NAN,
            },
            8.0,
            8.0,
        );
        match shading {
            BackgroundShading::Mesh { warp, .. } => assert_eq!(warp, 0.0),
            other => panic!("expected mesh, got {other:?}"),
        }
        let c = shading.linear_color([4.0, 4.0], [8.0, 8.0]);
        assert!(c.iter().all(|v| v.is_finite()), "{c:?}");
    }
}
