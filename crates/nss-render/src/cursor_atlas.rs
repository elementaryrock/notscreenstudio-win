//! A sprite sheet of the cursor images, rasterized once at 8 pixels per point.
//!
//! Port of upstream `NSSRender/CursorAtlas.swift`, with one substitution that ADR 0011 records:
//! upstream draws the real macOS `NSCursor` bitmaps, and Windows has no equivalent to hand us
//! (`LoadCursor` returns monochrome resources, and the DWM-composited pointer is not inspectable),
//! so every shape is **drawn here** as vector geometry at 8x. The contract the shader depends on is
//! identical either way: linear-light premultiplied pixels with a full mip chain, plus a cell table
//! carrying each image's size in points and its hotspot.
//!
//! Pixels are linear-light and premultiplied so the GPU can filter and mipmap them correctly and the
//! shader can composite them straight into its linear-light frame.

use nss_core::{CursorShape, Point2, Size2};

/// Atlas pixels per cursor point. The vector art is resolution-independent, so this only has to be
/// big enough to keep the arrow crisp at 4K zoom; 8x matches upstream's atlas resolution.
pub const SCALE: f64 = 8.0;
/// Clear border around every cell, in atlas pixels. Keeps bilinear and mipmap taps from bleeding
/// between neighbours up to [`MAX_LOD`] (32 px is 2 px at level 4).
pub const PADDING: u32 = 32;
/// Coarsest mip level the shader samples.
pub const MAX_LOD: f64 = 4.0;
/// Atlas rows wrap at this width.
pub const MAX_WIDTH: u32 = 2048;

/// One image in the atlas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cell {
    /// Image area in atlas pixels at mip level 0 (origin top-left). Surrounded by [`PADDING`].
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    /// Cursor image size in points.
    pub size: Size2,
    /// Hotspot in points from the image's top-left corner.
    pub hotspot: Point2,
    /// True when the pixels came from `NSCursor` upstream; always false here (see module docs).
    pub is_system_image: bool,
}

/// A mip level of the atlas: size and linear premultiplied RGBA pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct MipLevel {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<f32>,
}

/// The baked atlas.
#[derive(Debug, Clone, PartialEq)]
pub struct CursorAtlas {
    pub width: u32,
    pub height: u32,
    /// Level 0 first, each level half the previous (never smaller than 1x1).
    pub mips: Vec<MipLevel>,
    pub cells: Vec<(CursorShape, Cell)>,
}

impl CursorAtlas {
    /// Cell for `shape`. Shapes without an image (including [`CursorShape::Unknown`]) use the arrow.
    #[must_use]
    pub fn cell(&self, shape: CursorShape) -> Cell {
        self.cell_opt(shape)
            .or_else(|| self.cell_opt(CursorShape::Arrow))
            .expect("the atlas always contains an arrow")
    }

    #[must_use]
    pub fn cell_opt(&self, shape: CursorShape) -> Option<Cell> {
        self.cells
            .iter()
            .find(|(s, _)| *s == shape)
            .map(|(_, c)| *c)
    }

    /// Number of mip levels.
    #[must_use]
    pub fn mip_levels(&self) -> u32 {
        self.mips.len() as u32
    }

    /// Level-0 pixels, linear premultiplied RGBA.
    #[must_use]
    pub fn pixels(&self) -> &[f32] {
        &self.mips[0].pixels
    }

    /// Rasterizes every [`CursorShape`] that has an image. Pure CPU work, safe on any thread.
    #[must_use]
    pub fn build() -> Self {
        let sources: Vec<(CursorShape, Raster)> = CursorShape::ALL
            .iter()
            .filter(|s| **s != CursorShape::Unknown)
            .map(|s| (*s, draw(*s)))
            .collect();

        // Shelf packing, same rule as upstream: rows up to MAX_WIDTH.
        let mut cells: Vec<(CursorShape, Cell)> = Vec::with_capacity(sources.len());
        let mut x = 0_u32;
        let mut y = 0_u32;
        let mut row_height = 0_u32;
        let mut atlas_width = 0_u32;
        for (shape, raster) in &sources {
            let w = raster.width;
            let h = raster.height;
            let slot_w = w + 2 * PADDING;
            let slot_h = h + 2 * PADDING;
            if x > 0 && x + slot_w > MAX_WIDTH {
                y += row_height;
                x = 0;
                row_height = 0;
            }
            cells.push((
                *shape,
                Cell {
                    x: x + PADDING,
                    y: y + PADDING,
                    width: w,
                    height: h,
                    size: raster.size,
                    hotspot: raster.hotspot,
                    is_system_image: false,
                },
            ));
            x += slot_w;
            row_height = row_height.max(slot_h);
            atlas_width = atlas_width.max(x);
        }
        let atlas_height = (y + row_height).max(1);
        let atlas_width = atlas_width.max(1);

        let mut pixels = vec![0.0_f32; (atlas_width * atlas_height * 4) as usize];
        for ((_, cell), (_, raster)) in cells.iter().zip(sources.iter()) {
            for row in 0..raster.height {
                let dst = (((cell.y + row) * atlas_width + cell.x) * 4) as usize;
                let src = (row * raster.width * 4) as usize;
                let len = (raster.width * 4) as usize;
                pixels[dst..dst + len].copy_from_slice(&raster.pixels[src..src + len]);
            }
        }

        let mips = mip_chain(atlas_width, atlas_height, pixels);
        Self {
            width: atlas_width,
            height: atlas_height,
            mips,
            cells,
        }
    }
}

/// A rasterized cursor image in linear premultiplied RGBA.
#[derive(Debug, Clone)]
struct Raster {
    width: u32,
    height: u32,
    size: Size2,
    hotspot: Point2,
    pixels: Vec<f32>,
}

/// Box-filter mip chain, in linear light (the pixels are already linear, so a plain average of
/// premultiplied values is the correct filter).
fn mip_chain(width: u32, height: u32, level0: Vec<f32>) -> Vec<MipLevel> {
    let mut levels = vec![MipLevel {
        width,
        height,
        pixels: level0,
    }];
    loop {
        let prev = levels.last().expect("non-empty");
        if prev.width == 1 && prev.height == 1 {
            break;
        }
        let w = (prev.width / 2).max(1);
        let h = (prev.height / 2).max(1);
        let mut pixels = vec![0.0_f32; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let mut acc = [0.0_f32; 4];
                let mut n = 0.0_f32;
                for dy in 0..2u32 {
                    for dx in 0..2u32 {
                        let sx = (x * 2 + dx).min(prev.width - 1);
                        let sy = (y * 2 + dy).min(prev.height - 1);
                        let i = ((sy * prev.width + sx) * 4) as usize;
                        for (c, a) in acc.iter_mut().enumerate() {
                            *a += prev.pixels[i + c];
                        }
                        n += 1.0;
                    }
                }
                let o = ((y * w + x) * 4) as usize;
                for c in 0..4 {
                    pixels[o + c] = acc[c] / n;
                }
            }
        }
        levels.push(MipLevel {
            width: w,
            height: h,
            pixels,
        });
    }
    levels
}

// ---------------------------------------------------------------------------
// Geometry: a tiny vector rasterizer
// ---------------------------------------------------------------------------

/// A convex/concave primitive in points, origin at the image's top-left, y down.
#[derive(Debug, Clone)]
enum Prim {
    /// Rounded rectangle; `radius` 0 is a plain rectangle.
    Rect {
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
        radius: f64,
    },
    Circle {
        cx: f64,
        cy: f64,
        radius: f64,
    },
    Poly(Vec<(f64, f64)>),
    /// Stroked closed polyline of width `width` (round joins).
    Stroke {
        points: Vec<(f64, f64)>,
        width: f64,
        closed: bool,
    },
}

impl Prim {
    fn contains(&self, px: f64, py: f64) -> bool {
        match self {
            Prim::Rect {
                x0,
                y0,
                x1,
                y1,
                radius,
            } => {
                let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
                let (hx, hy) = ((x1 - x0) / 2.0, (y1 - y0) / 2.0);
                let r = radius.min(hx).min(hy).max(0.0);
                let qx = (px - cx).abs() - hx + r;
                let qy = (py - cy).abs() - hy + r;
                let d =
                    (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - r;
                d <= 0.0
            }
            Prim::Circle { cx, cy, radius } => {
                (px - cx).powi(2) + (py - cy).powi(2) <= radius * radius
            }
            Prim::Poly(points) => {
                let mut inside = false;
                let mut j = points.len() - 1;
                for i in 0..points.len() {
                    let (xi, yi) = points[i];
                    let (xj, yj) = points[j];
                    if (yi > py) != (yj > py) && px < (xj - xi) * (py - yi) / (yj - yi) + xi {
                        inside = !inside;
                    }
                    j = i;
                }
                inside
            }
            Prim::Stroke {
                points,
                width,
                closed,
            } => {
                let half = width / 2.0;
                let n = points.len();
                let last = if *closed { n } else { n - 1 };
                for i in 0..last.max(1) {
                    let a = points[i];
                    let b = points[(i + 1) % n];
                    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
                    let len2 = dx * dx + dy * dy;
                    let t = if len2 <= f64::EPSILON {
                        0.0
                    } else {
                        (((px - a.0) * dx + (py - a.1) * dy) / len2).clamp(0.0, 1.0)
                    };
                    let (qx, qy) = (a.0 + t * dx - px, a.1 + t * dy - py);
                    if qx * qx + qy * qy <= half * half {
                        return true;
                    }
                }
                false
            }
        }
    }
}

/// How to paint one cursor: a fill body, an optional white outline, and a soft shadow.
struct Art {
    /// Image size in points.
    size: (f64, f64),
    /// Hotspot in points from the top-left corner.
    hotspot: (f64, f64),
    /// Body: filled black.
    fill: Vec<Prim>,
    /// Outline: stroked white underneath the body.
    outline: Vec<Prim>,
    /// Extra decorations drawn in white on top of the body (badges, menu lines).
    accents: Vec<Prim>,
}

fn rot90(points: &[(f64, f64)], height: f64) -> Vec<(f64, f64)> {
    // (x, y) -> (height - y, x): a quarter turn clockwise of a `width x height` art box into
    // `height x width`, which is what the vertical I-beam and up/down resize cursors are.
    points.iter().map(|(x, y)| (height - y, *x)).collect()
}

fn arrow_art() -> Art {
    // Upstream's drawn fallback arrow: a 17x23 pt frame. The black body's apex sits on the hotspot
    // (the cursor position) and the white outline, 1.3 pt wide, reaches the very top-left corner of
    // the image, so the whole outline stays inside the frame instead of being clipped by it.
    const O: f64 = 1.3;
    let body: Vec<(f64, f64)> = [
        (0.0, 0.0),
        (0.0, 16.6),
        (4.1, 12.7),
        (6.9, 19.0),
        (9.5, 17.9),
        (6.8, 11.8),
        (12.0, 11.8),
    ]
    .iter()
    .map(|(x, y)| (x + O, y + O))
    .collect();
    let outline = Prim::Stroke {
        points: body.clone(),
        width: 2.6,
        closed: true,
    };
    Art {
        size: (17.0, 23.0),
        hotspot: (O, O),
        fill: vec![Prim::Poly(body)],
        outline: vec![outline],
        accents: Vec::new(),
    }
}

fn i_beam_art() -> Art {
    let body = vec![
        (0.0, 0.0),
        (9.0, 0.0),
        (9.0, 2.2),
        (5.6, 2.2),
        (5.6, 19.8),
        (9.0, 19.8),
        (9.0, 22.0),
        (0.0, 22.0),
        (0.0, 19.8),
        (3.4, 19.8),
        (3.4, 2.2),
        (0.0, 2.2),
    ];
    Art {
        size: (9.0, 22.0),
        hotspot: (4.5, 11.0),
        outline: vec![Prim::Stroke {
            points: body.clone(),
            width: 1.6,
            closed: true,
        }],
        fill: vec![Prim::Poly(body)],
        accents: Vec::new(),
    }
}

fn i_beam_vertical_art() -> Art {
    let mut a = i_beam_art();
    let h = a.size.1;
    let convert = |p: &Prim| match p {
        Prim::Poly(points) => Prim::Poly(rot90(points, h)),
        Prim::Stroke {
            points,
            width,
            closed,
        } => Prim::Stroke {
            points: rot90(points, h),
            width: *width,
            closed: *closed,
        },
        other => other.clone(),
    };
    a.fill = a.fill.iter().map(convert).collect();
    a.outline = a.outline.iter().map(convert).collect();
    a.size = (22.0, 9.0);
    a.hotspot = (11.0, 4.5);
    a
}

fn pointing_hand_art() -> Art {
    // Index finger up, thumb out to the left, three curled fingers. Recognisable at 20 pt.
    let mut fill = vec![
        Prim::Rect {
            x0: 7.4,
            y0: 0.0,
            x1: 11.6,
            y1: 13.0,
            radius: 2.0,
        },
        Prim::Rect {
            x0: 4.6,
            y0: 9.0,
            x1: 17.6,
            y1: 22.5,
            radius: 4.5,
        },
        Prim::Rect {
            x0: 3.6,
            y0: 12.0,
            x1: 7.0,
            y1: 18.0,
            radius: 1.8,
        },
        Prim::Circle {
            cx: 12.6,
            cy: 15.6,
            radius: 2.6,
        },
        Prim::Circle {
            cx: 15.4,
            cy: 17.8,
            radius: 2.4,
        },
    ];
    fill.shrink_to_fit();
    Art {
        size: (20.0, 24.0),
        hotspot: (8.2, 1.6),
        fill,
        outline: Vec::new(),
        accents: Vec::new(),
    }
}

fn open_hand_art() -> Art {
    let mut fill = vec![
        Prim::Rect {
            x0: 3.0,
            y0: 12.0,
            x1: 19.0,
            y1: 24.0,
            radius: 4.0,
        },
        Prim::Rect {
            x0: 3.2,
            y0: 0.0,
            x1: 6.4,
            y1: 14.0,
            radius: 1.6,
        },
        Prim::Rect {
            x0: 6.6,
            y0: 0.0,
            x1: 9.8,
            y1: 13.0,
            radius: 1.6,
        },
        Prim::Rect {
            x0: 10.0,
            y0: 0.0,
            x1: 13.2,
            y1: 13.0,
            radius: 1.6,
        },
        Prim::Rect {
            x0: 13.4,
            y0: 1.0,
            x1: 16.6,
            y1: 14.0,
            radius: 1.6,
        },
    ];
    fill.shrink_to_fit();
    Art {
        size: (22.0, 26.0),
        hotspot: (10.0, 3.0),
        fill,
        outline: Vec::new(),
        accents: Vec::new(),
    }
}

fn closed_hand_art() -> Art {
    Art {
        size: (20.0, 20.0),
        hotspot: (9.0, 4.0),
        fill: vec![
            Prim::Rect {
                x0: 2.0,
                y0: 6.0,
                x1: 18.0,
                y1: 19.0,
                radius: 5.0,
            },
            Prim::Circle {
                cx: 6.6,
                cy: 7.0,
                radius: 2.8,
            },
            Prim::Circle {
                cx: 10.4,
                cy: 6.0,
                radius: 2.9,
            },
            Prim::Circle {
                cx: 14.2,
                cy: 6.6,
                radius: 2.7,
            },
        ],
        outline: Vec::new(),
        accents: Vec::new(),
    }
}

fn crosshair_art() -> Art {
    Art {
        size: (19.0, 19.0),
        hotspot: (9.5, 9.5),
        fill: vec![
            Prim::Rect {
                x0: 0.0,
                y0: 8.0,
                x1: 19.0,
                y1: 10.6,
                radius: 0.6,
            },
            Prim::Rect {
                x0: 8.2,
                y0: 0.0,
                x1: 10.8,
                y1: 19.0,
                radius: 0.6,
            },
        ],
        outline: Vec::new(),
        accents: Vec::new(),
    }
}

fn double_arrow_art() -> Art {
    // Horizontal double arrow, 24x16 pt, hotspot in the middle.
    Art {
        size: (24.0, 16.0),
        hotspot: (12.0, 8.0),
        fill: vec![
            Prim::Rect {
                x0: 6.0,
                y0: 6.6,
                x1: 18.0,
                y1: 9.4,
                radius: 0.8,
            },
            Prim::Poly(vec![(0.0, 8.0), (7.0, 2.6), (7.0, 13.4)]),
            Prim::Poly(vec![(24.0, 8.0), (17.0, 2.6), (17.0, 13.4)]),
        ],
        outline: Vec::new(),
        accents: Vec::new(),
    }
}

fn double_arrow_vertical_art() -> Art {
    let mut a = double_arrow_art();
    let h = a.size.1;
    let convert = |p: &Prim| match p {
        Prim::Poly(points) => Prim::Poly(rot90(points, h)),
        Prim::Rect {
            x0,
            y0,
            x1,
            y1,
            radius: _,
        } => {
            let pts = rot90(&[(*x0, *y0), (*x1, *y0), (*x1, *y1), (*x0, *y1)], h);
            Prim::Poly(pts)
        }
        other => other.clone(),
    };
    a.fill = a.fill.iter().map(convert).collect();
    a.size = (16.0, 24.0);
    a.hotspot = (8.0, 12.0);
    a
}

fn single_arrow(keep_left: bool) -> Art {
    // Left-pointing arrow in a 13x16 pt box: arrowhead at [0, 7] and stem bar at [6, 13].
    let mut fill = vec![
        Prim::Poly(vec![(0.0, 8.0), (7.0, 2.6), (7.0, 13.4)]),
        Prim::Rect {
            x0: 6.0,
            y0: 6.6,
            x1: 13.0,
            y1: 9.4,
            radius: 0.8,
        },
    ];
    let hotspot = if keep_left {
        (2.0, 8.0)
    } else {
        // Mirror horizontally across x = 6.5 so the arrowhead tip lands at x = 13.0.
        fill = fill
            .iter()
            .map(|p| match p {
                Prim::Poly(points) => {
                    Prim::Poly(points.iter().map(|(x, y)| (13.0 - x, *y)).collect())
                }
                Prim::Rect {
                    x0,
                    y0,
                    x1,
                    y1,
                    radius,
                } => Prim::Rect {
                    x0: 13.0 - x1,
                    y0: *y0,
                    x1: 13.0 - x0,
                    y1: *y1,
                    radius: *radius,
                },
                other => other.clone(),
            })
            .collect();
        (11.0, 8.0)
    };
    Art {
        size: (13.0, 16.0),
        hotspot,
        fill,
        outline: Vec::new(),
        accents: Vec::new(),
    }
}

fn single_vertical_arrow(keep_top: bool) -> Art {
    let mut a = single_arrow(true);
    let h = a.size.1;
    // Rotate the 13x16 left arrow a quarter turn clockwise into 16x13: pointing up, bar below.
    a.fill = a
        .fill
        .iter()
        .map(|p| match p {
            Prim::Poly(points) => Prim::Poly(rot90(points, h)),
            Prim::Rect {
                x0,
                y0,
                x1,
                y1,
                radius: _,
            } => Prim::Poly(rot90(&[(*x0, *y0), (*x1, *y0), (*x1, *y1), (*x0, *y1)], h)),
            other => other.clone(),
        })
        .collect();
    a.size = (16.0, 13.0);
    a.hotspot = (8.0, 2.0);
    if !keep_top {
        a.fill = a
            .fill
            .iter()
            .map(|p| match p {
                Prim::Poly(points) => {
                    Prim::Poly(points.iter().map(|(x, y)| (*x, 13.0 - y)).collect())
                }
                other => other.clone(),
            })
            .collect();
        a.hotspot = (8.0, 11.0);
    }
    a
}

fn not_allowed_art() -> Art {
    let mut ring = Vec::new();
    let r = 8.0;
    for i in 0..64 {
        let a = f64::from(i) * std::f64::consts::TAU / 64.0;
        ring.push((9.0 + r * a.cos(), 9.0 + r * a.sin()));
    }
    Art {
        size: (18.0, 18.0),
        hotspot: (9.0, 9.0),
        fill: vec![Prim::Stroke {
            points: ring,
            width: 2.4,
            closed: true,
        }],
        outline: Vec::new(),
        accents: vec![Prim::Rect {
            x0: 4.0,
            y0: 7.8,
            x1: 14.0,
            y1: 10.2,
            radius: 0.6,
        }],
    }
}

fn drag_badge_art(link: bool) -> Art {
    let mut a = arrow_art();
    a.size = (24.0, 26.0);
    if link {
        a.accents.push(Prim::Rect {
            x0: 12.4,
            y0: 15.4,
            x1: 22.6,
            y1: 24.4,
            radius: 1.6,
        });
    } else {
        a.accents.push(Prim::Rect {
            x0: 15.4,
            y0: 16.6,
            x1: 21.0,
            y1: 22.2,
            radius: 0.8,
        });
        a.accents.push(Prim::Rect {
            x0: 17.2,
            y0: 14.8,
            x1: 19.2,
            y1: 24.0,
            radius: 0.6,
        });
    }
    a
}

fn contextual_menu_art() -> Art {
    let mut a = arrow_art();
    a.size = (22.0, 24.0);
    for i in 0..3 {
        let y = 14.0 + f64::from(i) * 3.2;
        a.accents.push(Prim::Rect {
            x0: 13.0,
            y0: y,
            x1: 21.0,
            y1: y + 1.8,
            radius: 0.6,
        });
    }
    a
}

fn draw(shape: CursorShape) -> Raster {
    let art = match shape {
        CursorShape::Arrow => arrow_art(),
        CursorShape::IBeam => i_beam_art(),
        CursorShape::IBeamVertical => i_beam_vertical_art(),
        CursorShape::PointingHand => pointing_hand_art(),
        CursorShape::OpenHand => open_hand_art(),
        CursorShape::ClosedHand => closed_hand_art(),
        CursorShape::Crosshair => crosshair_art(),
        CursorShape::ResizeLeftRight => double_arrow_art(),
        CursorShape::ResizeUpDown => double_arrow_vertical_art(),
        CursorShape::ResizeLeft => single_arrow(true),
        CursorShape::ResizeRight => single_arrow(false),
        CursorShape::ResizeUp => single_vertical_arrow(true),
        CursorShape::ResizeDown => single_vertical_arrow(false),
        CursorShape::NotAllowed => not_allowed_art(),
        CursorShape::DragCopy => drag_badge_art(false),
        CursorShape::DragLink => drag_badge_art(true),
        CursorShape::ContextualMenu => contextual_menu_art(),
        // Unknown draws the arrow, exactly like upstream's `cells[shape] ?? cells[.arrow]`.
        CursorShape::Unknown => arrow_art(),
    };
    rasterize(&art)
}

/// Anti-aliased rasterizer: 4x4 coverage per pixel, shadow, white outline, black body, accents.
fn rasterize(art: &Art) -> Raster {
    let w = (art.size.0 * SCALE).round().max(1.0) as u32;
    let h = (art.size.1 * SCALE).round().max(1.0) as u32;
    let n = (w * h) as usize;
    let mut fill = vec![0.0_f32; n];
    let mut stroke = vec![0.0_f32; n];
    let mut accent = vec![0.0_f32; n];
    const SUB: u32 = 4;
    let inv = 1.0 / f64::from(SUB * SUB);
    for y in 0..h {
        for x in 0..w {
            let mut cover = (0.0_f64, 0.0_f64, 0.0_f64);
            for sy in 0..SUB {
                for sx in 0..SUB {
                    let px = (f64::from(x) + (f64::from(sx) + 0.5) / f64::from(SUB)) / SCALE;
                    let py = (f64::from(y) + (f64::from(sy) + 0.5) / f64::from(SUB)) / SCALE;
                    if art.fill.iter().any(|p| p.contains(px, py)) {
                        cover.0 += inv;
                    }
                    if art.outline.iter().any(|p| p.contains(px, py)) {
                        cover.1 += inv;
                    }
                    if art.accents.iter().any(|p| p.contains(px, py)) {
                        cover.2 += inv;
                    }
                }
            }
            let i = (y * w + x) as usize;
            fill[i] = cover.0 as f32;
            stroke[i] = cover.1 as f32;
            accent[i] = cover.2 as f32;
        }
    }

    // Composite in the same order as upstream's fallback arrow: a soft shadow, then the white
    // outline, then the black body, then white accents. Premultiplied throughout.
    let shadow = blur_mask(&fill, w, h, (2.0 * SCALE) as f32, (1.1 * SCALE) as i32);
    let mut pixels = vec![0.0_f32; n * 4];
    for i in 0..n {
        let mut rgba = [0.0_f32; 4];
        over(&mut rgba, [0.0, 0.0, 0.0], shadow[i] * 0.35);
        over(&mut rgba, [1.0, 1.0, 1.0], stroke[i]);
        over(&mut rgba, [0.0, 0.0, 0.0], fill[i]);
        over(&mut rgba, [1.0, 1.0, 1.0], accent[i]);
        pixels[i * 4..i * 4 + 4].copy_from_slice(&rgba);
    }
    Raster {
        width: w,
        height: h,
        size: Size2::new(art.size.0, art.size.1),
        hotspot: Point2::new(art.hotspot.0, art.hotspot.1),
        pixels,
    }
}

/// Source-over of a straight-alpha colour at `alpha` onto a premultiplied buffer.
fn over(dst: &mut [f32; 4], rgb: [f32; 3], alpha: f32) {
    let a = alpha.clamp(0.0, 1.0);
    if a <= 0.0 {
        return;
    }
    let inv = 1.0 - a;
    dst[0] = rgb[0] * a + dst[0] * inv;
    dst[1] = rgb[1] * a + dst[1] * inv;
    dst[2] = rgb[2] * a + dst[2] * inv;
    dst[3] = a + dst[3] * inv;
}

/// Separable box blur of a mask, offset downward by `dy` pixels (the shadow sits below the shape).
fn blur_mask(mask: &[f32], w: u32, h: u32, radius: f32, dy: i32) -> Vec<f32> {
    let r = radius.round().max(0.0) as i32;
    let shifted = |x: i32, y: i32| -> f32 {
        let sx = x.clamp(0, w as i32 - 1);
        let sy = (y - dy).clamp(0, h as i32 - 1);
        mask[(sy as u32 * w + sx as u32) as usize]
    };
    let mut tmp = vec![0.0_f32; mask.len()];
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let mut acc = 0.0;
            let mut n = 0.0;
            for d in -r..=r {
                acc += shifted(x + d, y);
                n += 1.0;
            }
            tmp[(y as u32 * w + x as u32) as usize] = acc / n;
        }
    }
    let mut out = vec![0.0_f32; mask.len()];
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let mut acc = 0.0;
            let mut n = 0.0;
            for d in -r..=r {
                let sy = (y + d).clamp(0, h as i32 - 1);
                acc += tmp[(sy as u32 * w + x as u32) as usize];
                n += 1.0;
            }
            out[(y as u32 * w + x as u32) as usize] = acc / n;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_atlas_has_an_arrow_and_every_cell_is_sane() {
        let atlas = CursorAtlas::build();
        assert!(atlas.width > 0 && atlas.width <= MAX_WIDTH && atlas.height > 0);
        assert_eq!(
            atlas.pixels().len(),
            (atlas.width * atlas.height * 4) as usize
        );
        let arrow = atlas.cell(CursorShape::Arrow);
        assert_eq!(
            atlas.cell(CursorShape::Unknown),
            arrow,
            "unknown must draw the arrow"
        );

        for (shape, c) in &atlas.cells {
            assert!(
                c.hotspot.x >= 0.0
                    && c.hotspot.x <= c.size.width
                    && c.hotspot.y >= 0.0
                    && c.hotspot.y <= c.size.height,
                "{shape:?} hotspot {:?} outside {:?}",
                c.hotspot,
                c.size
            );
            assert!(c.x >= PADDING && c.y >= PADDING);
            assert!(c.x + c.width + PADDING <= atlas.width);
            assert!(c.y + c.height + PADDING <= atlas.height);
            assert!(
                (f64::from(c.width) - c.size.width * SCALE).abs() < 1.0,
                "{shape:?} width {} for {} pt",
                c.width,
                c.size.width
            );
            // The image has opaque pixels.
            let mut max_a: f32 = 0.0;
            for y in c.y..(c.y + c.height) {
                for x in c.x..(c.x + c.width) {
                    max_a = max_a.max(atlas.pixels()[((y * atlas.width + x) * 4 + 3) as usize]);
                }
            }
            assert!(max_a > 0.9, "{shape:?} is empty (max alpha {max_a})");
        }

        // Cells (with padding) never overlap.
        for i in 0..atlas.cells.len() {
            for j in (i + 1)..atlas.cells.len() {
                let a = atlas.cells[i].1;
                let b = atlas.cells[j].1;
                let (ax0, ay0) = (a.x - PADDING, a.y - PADDING);
                let (ax1, ay1) = (a.x + a.width + PADDING, a.y + a.height + PADDING);
                let (bx0, by0) = (b.x - PADDING, b.y - PADDING);
                let (bx1, by1) = (b.x + b.width + PADDING, b.y + b.height + PADDING);
                let overlaps = ax0 < bx1 && bx0 < ax1 && ay0 < by1 && by0 < ay1;
                assert!(
                    !overlaps,
                    "{:?} and {:?} overlap",
                    atlas.cells[i].0, atlas.cells[j].0
                );
            }
        }

        // The padding is clear.
        let top = arrow.y - PADDING;
        for x in arrow.x..(arrow.x + arrow.width) {
            assert_eq!(
                atlas.pixels()[((top * atlas.width + x) * 4 + 3) as usize],
                0.0
            );
        }
    }

    #[test]
    fn distinct_shapes_have_distinct_images() {
        let atlas = CursorAtlas::build();
        let bytes = |c: Cell| -> Vec<f32> {
            let mut out = Vec::with_capacity((c.width * c.height * 4) as usize);
            for y in c.y..(c.y + c.height) {
                for x in c.x..(c.x + c.width) {
                    let i = ((y * atlas.width + x) * 4) as usize;
                    out.extend_from_slice(&atlas.pixels()[i..i + 4]);
                }
            }
            out
        };
        let arrow = bytes(atlas.cell(CursorShape::Arrow));
        for (shape, cell) in &atlas.cells {
            if *shape == CursorShape::Arrow {
                continue;
            }
            assert_ne!(
                bytes(*cell),
                arrow,
                "{shape:?} looks exactly like the arrow"
            );
        }
    }

    #[test]
    fn the_arrow_hotspot_is_the_top_left_tip() {
        let atlas = CursorAtlas::build();
        let c = atlas.cell(CursorShape::Arrow);
        assert!((c.size.width - 17.0).abs() < 1e-9 && (c.size.height - 23.0).abs() < 1e-9);
        // The hotspot is the body's apex, with the outline reaching the image's top-left corner.
        assert!((c.hotspot.x - 1.3).abs() < 1e-9 && (c.hotspot.y - 1.3).abs() < 1e-9);
        // Black body near the hotspot, white outline just outside it.
        let at =
            |x: u32, y: u32| atlas.pixels()[(((c.y + y) * atlas.width + c.x + x) * 4) as usize];
        let tip_x = (c.hotspot.x * SCALE) as u32;
        let tip_y = (c.hotspot.y * SCALE) as u32;
        let body = at(tip_x + 8, tip_y + 30);
        assert!(body < 0.05, "body should be black, got {body}");
    }

    #[test]
    fn mips_halve_down_to_one_pixel_and_keep_the_colour() {
        let atlas = CursorAtlas::build();
        assert!(atlas.mip_levels() >= 5);
        let mut w = atlas.width;
        let mut h = atlas.height;
        for mip in &atlas.mips {
            assert_eq!((mip.width, mip.height), (w, h));
            assert_eq!(mip.pixels.len(), (w * h * 4) as usize);
            w = (w / 2).max(1);
            h = (h / 2).max(1);
        }
        let last = atlas.mips.last().unwrap();
        assert_eq!((last.width, last.height), (1, 1));
        // Averaging a mostly clear atlas gives a small but non-zero alpha.
        assert!(
            last.pixels[3] > 0.0 && last.pixels[3] < 0.5,
            "{:?}",
            last.pixels
        );
    }
}
