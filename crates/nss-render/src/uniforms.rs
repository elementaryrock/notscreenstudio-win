//! The CPU half of the shader contract: turning a layout, a frame state and a style into the flat
//! slot array the fragment shader reads.
//!
//! Port of upstream `MetalRenderer.writeUniforms`. Everything here is pure arithmetic on purpose:
//! it is what the GPU is *told* to do, so it can be tested without an adapter, and the pixel tests
//! only have to check that the shader then does what these numbers say.

use nss_core::{
    CameraState, CursorShape, CursorStyle, CursorTrack, FrameState, NormalizedPoint, PixelSize,
    RGBAColor, Style,
};

use crate::color::linear;
use crate::cursor_atlas::{Cell, CursorAtlas, MAX_LOD, SCALE};
use crate::layout::RenderLayout;
use crate::rasterizer::{BackgroundRasterizer, BackgroundShading};
use crate::shaders::{Slot, MAX_RIPPLES, MAX_STOPS};

/// A ready-to-upload uniform block.
#[derive(Debug, Clone, PartialEq)]
pub struct Uniforms {
    #[allow(clippy::type_complexity)]
    slots: Vec<[f32; 4]>,
}

impl Default for Uniforms {
    fn default() -> Self {
        Self {
            slots: vec![[0.0; 4]; Slot::COUNT],
        }
    }
}

impl Uniforms {
    #[must_use]
    pub fn get(&self, slot: Slot) -> [f32; 4] {
        self.slots[slot.index()]
    }

    /// Slot `slot` plus `offset` (for the stop/loc/ripple arrays).
    #[must_use]
    pub fn get_at(&self, slot: Slot, offset: usize) -> [f32; 4] {
        self.slots[slot.index() + offset]
    }

    #[must_use]
    pub fn slots(&self) -> &[[f32; 4]] {
        &self.slots
    }

    /// The bytes to upload, appended to `out` (which the renderer reuses per frame).
    pub fn write_bytes(&self, out: &mut Vec<u8>) {
        out.clear();
        out.reserve(self.slots.len() * 16);
        for slot in &self.slots {
            for v in slot {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
    }

    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.slots.len() * 16);
        self.write_bytes(&mut out);
        out
    }

    fn set(&mut self, slot: Slot, x: f64, y: f64, z: f64, w: f64) {
        self.slots[slot.index()] = [x as f32, y as f32, z as f32, w as f32];
    }

    fn set_at(&mut self, slot: Slot, offset: usize, x: f64, y: f64, z: f64, w: f64) {
        self.slots[slot.index() + offset] = [x as f32, y as f32, z as f32, w as f32];
    }
}

/// Atlas mip level for a cursor drawn at `ppp` output pixels per point (the atlas holds
/// [`SCALE`] pixels per point at level 0), clamped to the levels the padding protects.
#[must_use]
pub fn cursor_lod(pixels_per_point: f64) -> f64 {
    if !pixels_per_point.is_finite() || pixels_per_point <= 0.0 {
        return 0.0;
    }
    (SCALE / pixels_per_point).log2().clamp(0.0, MAX_LOD)
}

/// Taps per axis for the footprint filter: enough to cover the source texels under one output pixel.
#[must_use]
pub fn taps(texels_per_pixel: f64) -> i32 {
    if !texels_per_pixel.is_finite() || texels_per_pixel <= 1.05 {
        return 1;
    }
    (texels_per_pixel.ceil() as i32).min(6)
}

/// Everything the uniform writer needs that does not come from the layout/state/style triple.
pub struct Scene<'a> {
    pub layout: &'a RenderLayout,
    pub state: &'a FrameState,
    pub style: &'a Style,
    /// Present when a camera frame was supplied *and* the bubble is visible.
    pub camera: Option<PixelSize>,
    /// The decoded background image's size, when the background is an image that loaded.
    pub image: Option<PixelSize>,
    /// Mip level to sample the background image at (computed from the footprint).
    pub image_lod: f32,
    pub atlas: &'a CursorAtlas,
    /// Triangular dither on background pixels. On for the preview and the export alike.
    pub dither: bool,
}

fn finite_or(v: f64, fallback: f64) -> f64 {
    if v.is_finite() {
        v
    } else {
        fallback
    }
}

/// Builds the uniform block for one frame.
#[must_use]
pub fn write(scene: &Scene<'_>) -> Uniforms {
    let layout = scene.layout;
    let style = scene.style;
    let state = scene.state;
    let mut u = Uniforms::default();

    let w = f64::from(layout.output_size.width);
    let h = f64::from(layout.output_size.height);
    let rect = layout.content_rect;
    let cam = layout.camera;
    u.set(Slot::Out, w, h, if scene.dither { 1.0 } else { 0.0 }, 0.0);
    u.set(
        Slot::Content,
        rect.min_x(),
        rect.min_y(),
        rect.size.width,
        rect.size.height,
    );
    u.set(
        Slot::Content2,
        layout.corner_radius,
        cam.scale,
        cam.center.x,
        cam.center.y,
    );
    let duv_x = 1.0 / (rect.size.width * cam.scale);
    let duv_y = 1.0 / (rect.size.height * cam.scale);
    u.set(
        Slot::ScreenSs,
        duv_x,
        duv_y,
        f64::from(taps(layout.source_texels_per_output_pixel())),
        0.0,
    );

    let sh = &style.shadow;
    u.set(
        Slot::Shadow,
        finite_or(sh.radius, 0.0).max(0.0) * h / 2.0,
        finite_or(sh.opacity, 0.0).clamp(0.0, 1.0),
        finite_or(sh.offset_x, 0.0) * h,
        finite_or(sh.offset_y, 0.0) * h,
    );

    write_background(
        &mut u,
        &BackgroundRasterizer::shading(&style.background, w, h),
        scene.image,
        w,
        h,
        if scene.image_lod.is_finite() {
            scene.image_lod.max(0.0)
        } else {
            0.0
        },
    );

    // Cursor: the image for `shape`, hotspot on the position, squished around the hotspot.
    let cs = &state.cursor;
    let alpha = if cs.visible && cs.opacity.is_finite() {
        cs.opacity.clamp(0.0, 1.0)
    } else {
        0.0
    };
    if alpha > 0.0 && cs.position.x.is_finite() && cs.position.y.is_finite() {
        let cell = scene.atlas.cell(cs.shape);
        let press = if cs.press_scale.is_finite() {
            cs.press_scale.clamp(0.05, 4.0)
        } else {
            1.0
        };
        let ppp = layout.cursor_pixels_per_point(&style.cursor) * press;
        if ppp > 0.0 {
            let hot = layout.output_point(cs.position);
            let blur = layout.cursor_blur_vector(cs.velocity, &style.cursor);
            let len = (blur.x * blur.x + blur.y * blur.y).sqrt();
            let samples = if len < 1.0 {
                1
            } else {
                ((len / 2.0).ceil() as i32 + 1).min(16)
            };
            let (bx, by) = if samples == 1 {
                (0.0, 0.0)
            } else {
                (blur.x, blur.y)
            };
            u.set(Slot::Cursor, hot.x, hot.y, ppp, alpha);
            u.set(
                Slot::CursorBlur,
                bx,
                by,
                f64::from(samples),
                cursor_lod(ppp),
            );
            // Image bounds in output px, grown by the shader's 2 pt margin, half the blur and 1 px.
            let mx = 2.0 * ppp + 1.0 + bx.abs() / 2.0;
            let my = 2.0 * ppp + 1.0 + by.abs() / 2.0;
            let min_x = hot.x - cell.hotspot.x * ppp - mx;
            let max_x = hot.x + (cell.size.width - cell.hotspot.x) * ppp + mx;
            let min_y = hot.y - cell.hotspot.y * ppp - my;
            let max_y = hot.y + (cell.size.height - cell.hotspot.y) * ppp + my;
            u.set(Slot::CursorBox, min_x, min_y, max_x, max_y);
            let aw = f64::from(scene.atlas.width);
            let ah = f64::from(scene.atlas.height);
            u.set(
                Slot::CursorCell,
                f64::from(cell.x) / aw,
                f64::from(cell.y) / ah,
                f64::from(cell.width) / aw,
                f64::from(cell.height) / ah,
            );
            u.set(
                Slot::CursorCell2,
                cell.hotspot.x,
                cell.hotspot.y,
                cell.size.width,
                cell.size.height,
            );
        }
    }

    // Ripples.
    if style.cursor.click_ripples {
        let mut n = 0;
        for r in &cs.ripples {
            if n >= MAX_RIPPLES
                || !(0.0..1.0).contains(&r.progress)
                || !r.position.x.is_finite()
                || !r.position.y.is_finite()
            {
                continue;
            }
            let p = layout.output_point(r.position);
            u.set_at(
                Slot::Ripple0,
                n,
                p.x,
                p.y,
                layout.ripple_radius(r.progress),
                layout.ripple_opacity(r.progress),
            );
            n += 1;
        }
        u.set(
            Slot::RippleParams,
            n as f64,
            layout.ripple_thickness(),
            0.0,
            0.0,
        );
    }

    // Camera bubble: shared geometry, shrunk toward its corner/edge while zoomed. Border and shadow
    // scale with the bubble.
    let bubble = &style.camera;
    if let Some(cam_size) = scene.camera {
        if bubble.visible && bubble.size.is_finite() && bubble.size > 0.0 {
            let r = layout.camera_bubble_rect(bubble);
            let k = layout.camera_bubble_size_multiplier(bubble);
            if r.size.width >= 0.5 && r.size.height >= 0.5 {
                let border = bubble
                    .border
                    .as_ref()
                    .map_or(0.0, |b| finite_or(b.width, 0.0).max(0.0) * h * k);
                u.set(
                    Slot::Bubble,
                    r.mid_x(),
                    r.mid_y(),
                    r.size.width / 2.0,
                    r.size.height / 2.0,
                );
                u.set(
                    Slot::Bubble2,
                    0.0,
                    layout.camera_bubble_corner_radius(bubble, r),
                    border,
                    1.0,
                );
                // Aspect-fill: crop the camera to the bubble aspect.
                let ba = r.size.width / r.size.height.max(1e-6);
                let ca = cam_size.aspect();
                let sx = if ca > ba { ba / ca } else { 1.0 };
                let sy = if ca > ba { 1.0 } else { ca / ba };
                let texels_per_px = f64::from(cam_size.width) * sx / r.size.width.max(1.0);
                u.set(
                    Slot::BubbleUv,
                    sx,
                    sy,
                    if bubble.mirror { 1.0 } else { 0.0 },
                    f64::from(taps(texels_per_px)),
                );
                u.set(
                    Slot::BubbleSs,
                    sx / r.size.width.max(1.0),
                    sy / r.size.height.max(1.0),
                    0.0,
                    0.0,
                );
                if let Some(border) = &bubble.border {
                    let lin = linear(border.color);
                    u.set(
                        Slot::BorderColor,
                        lin[0],
                        lin[1],
                        lin[2],
                        finite_or(border.color.a, 1.0).clamp(0.0, 1.0),
                    );
                }
                let bs = &style.shadow;
                u.set(
                    Slot::BubbleShadow,
                    finite_or(bs.radius, 0.0).max(0.0) * h / 2.0 * k,
                    finite_or(bs.opacity, 0.0).clamp(0.0, 1.0),
                    finite_or(bs.offset_x, 0.0) * h * k,
                    finite_or(bs.offset_y, 0.0) * h * k,
                );
            }
        }
    }

    u
}

fn write_background(
    u: &mut Uniforms,
    shading: &BackgroundShading,
    image: Option<PixelSize>,
    w: f64,
    h: f64,
    lod: f32,
) {
    fn solid(u: &mut Uniforms, c: [f64; 4]) {
        u.set(Slot::Bg, 0.0, 0.0, 0.0, 0.0);
        u.set(Slot::Solid, c[0], c[1], c[2], c[3]);
    }
    match shading {
        BackgroundShading::Solid(c) => solid(u, *c),
        BackgroundShading::Linear {
            stops,
            locations,
            dir,
            inv_length,
        } => {
            u.set(Slot::Bg, 1.0, stops.len() as f64, 0.0, 0.0);
            u.set(Slot::BgGrad, dir[0], dir[1], *inv_length, 0.0);
            for (i, s) in stops.iter().enumerate() {
                u.set_at(Slot::Stop0, i, s[0], s[1], s[2], s[3]);
                u.set_at(Slot::Loc0, i, locations[i], 0.0, 0.0, 0.0);
            }
        }
        BackgroundShading::Mesh {
            colors,
            positions,
            warp,
        } => {
            u.set(Slot::Bg, 3.0, colors.len() as f64, 0.0, 0.0);
            u.set(Slot::Wallpaper, *warp, 0.0, 0.0, 0.0);
            for (i, c) in colors.iter().enumerate() {
                u.set_at(Slot::Stop0, i, c[0], c[1], c[2], 1.0);
                u.set_at(Slot::Loc0, i, positions[i][0], positions[i][1], 0.0, 0.0);
            }
        }
        BackgroundShading::Image => match image {
            None => {
                let g = linear(BackgroundRasterizer::MISSING_IMAGE_GREY);
                solid(u, [g[0], g[1], g[2], 1.0]);
            }
            Some(size) => {
                let oa = w / h;
                let ia = f64::from(size.width) / f64::from(size.height.max(1));
                let sx = if ia > oa { oa / ia } else { 1.0 };
                let sy = if ia > oa { 1.0 } else { ia / oa };
                u.set(Slot::Bg, 2.0, 0.0, 0.0, 0.0);
                u.set(Slot::BgImage, sx, sy, f64::from(lod), 0.0);
            }
        },
    }
}

/// The number of stops a gradient will actually use, after sorting and clamping.
#[must_use]
pub fn gradient_stop_count(style: &Style) -> usize {
    match &style.background {
        nss_core::Background::Gradient { stops, .. } => stops.len().min(MAX_STOPS),
        nss_core::Background::Mesh { points, .. } => points.len().min(MAX_STOPS),
        _ => 0,
    }
}

/// Convenience used by tests and the exporter: which cursor cell a shape resolves to.
#[must_use]
pub fn cursor_cell(atlas: &CursorAtlas, shape: CursorShape) -> Cell {
    atlas.cell(shape)
}

/// The camera state the renderer uses for a state (already clamped, for hit-testing callers).
#[must_use]
pub fn clamped(state: &FrameState) -> CameraState {
    crate::layout::clamped_camera(state.camera)
}

/// A cursor style with only the fields the renderer reads, for callers that build one ad hoc.
#[must_use]
pub fn cursor_style(size: f64, motion_blur: f64, ripples: bool) -> CursorStyle {
    CursorStyle {
        size,
        motion_blur,
        click_ripples: ripples,
        ..CursorStyle::default()
    }
}

/// A normalized point helper so callers do not have to import `nss_core` just for this.
#[must_use]
pub fn point(x: f64, y: f64) -> NormalizedPoint {
    NormalizedPoint::new(x, y)
}

/// A colour helper with the same purpose.
#[must_use]
pub fn rgba(r: f64, g: f64, b: f64, a: f64) -> RGBAColor {
    RGBAColor::new(r, g, b, a)
}

/// Cursor tracks are not the renderer's business, but the type appears in the public API surface of
/// the crate's doc examples; keep it re-exported here so `use nss_render::uniforms::*` is enough.
pub type CursorTrackRef = CursorTrack;

#[cfg(test)]
mod tests {
    use super::*;
    use nss_core::{
        Background, CameraBubble, CameraShape, GradientStop, MeshPoint, RippleState, Shadow,
    };

    fn style_plain() -> Style {
        Style {
            background: Background::Solid(RGBAColor::new(1.0, 0.0, 0.0, 1.0)),
            padding: 0.1,
            corner_radius: 0.0,
            shadow: Shadow {
                opacity: 0.0,
                ..Shadow::default()
            },
            camera: CameraBubble {
                visible: false,
                border: None,
                ..CameraBubble::default()
            },
            ..Style::default()
        }
    }

    fn layout(style: &Style, camera: CameraState) -> RenderLayout {
        RenderLayout::new(
            PixelSize::new(1920, 1080),
            PixelSize::new(1920, 1080),
            style,
            camera,
        )
    }

    fn build(style: &Style, state: &FrameState) -> Uniforms {
        let atlas = CursorAtlas::build();
        let l = layout(style, state.camera);
        write(&Scene {
            layout: &l,
            state,
            style,
            camera: None,
            image: None,
            image_lod: 0.0,
            atlas: &atlas,
            dither: true,
        })
    }

    #[test]
    fn the_output_and_content_slots_describe_the_layout() {
        let style = style_plain();
        let state = FrameState::new(0.0);
        let u = build(&style, &state);
        assert_eq!(u.get(Slot::Out), [1920.0, 1080.0, 1.0, 0.0]);
        let l = layout(&style, CameraState::IDENTITY);
        let c = u.get(Slot::Content);
        assert!((c[0] - l.content_rect.min_x() as f32).abs() < 1e-3);
        assert!((c[2] - l.content_rect.size.width as f32).abs() < 1e-3);
        let c2 = u.get(Slot::Content2);
        assert!((c2[1] - 1.0).abs() < 1e-6, "camera scale");
        assert!((c2[2] - 0.5).abs() < 1e-6 && (c2[3] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn a_solid_background_reaches_the_solid_slot_in_linear_light() {
        let style = style_plain();
        let u = build(&style, &FrameState::new(0.0));
        assert_eq!(u.get(Slot::Bg)[0], 0.0, "kind solid");
        let s = u.get(Slot::Solid);
        assert!(
            (s[0] - 1.0).abs() < 1e-6 && s[1] == 0.0 && s[2] == 0.0 && (s[3] - 1.0).abs() < 1e-6
        );
    }

    #[test]
    fn gradient_stops_are_clamped_to_eight_and_sorted() {
        let mut stops: Vec<GradientStop> = (0..12)
            .map(|i| {
                GradientStop::new(
                    f64::from(i) / 11.0,
                    RGBAColor::new(0.1 * f64::from(i), 0.2, 0.3, 1.0),
                )
            })
            .collect();
        stops.reverse();
        let style = Style {
            background: Background::Gradient {
                stops,
                angle_degrees: 0.0,
            },
            ..style_plain()
        };
        let u = build(&style, &FrameState::new(0.0));
        assert_eq!(u.get(Slot::Bg)[0], 1.0);
        assert_eq!(u.get(Slot::Bg)[1] as usize, MAX_STOPS);
        // Sorted ascending by location.
        let locs: Vec<f32> = (0..MAX_STOPS).map(|i| u.get_at(Slot::Loc0, i)[0]).collect();
        assert!(locs.windows(2).all(|w| w[0] <= w[1]), "{locs:?}");
        // dir is the unit x axis for 0 degrees, invLength = 1 / width.
        let g = u.get(Slot::BgGrad);
        assert!((g[0] - 1.0).abs() < 1e-6 && g[1].abs() < 1e-6);
        assert!((g[2] - 1.0 / 1920.0).abs() < 1e-9);
    }

    #[test]
    fn a_vertical_gradient_runs_down_the_image() {
        let style = Style {
            background: Background::Gradient {
                stops: vec![
                    GradientStop::new(0.0, RGBAColor::WHITE),
                    GradientStop::new(1.0, RGBAColor::BLACK),
                ],
                angle_degrees: 90.0,
            },
            ..style_plain()
        };
        let u = build(&style, &FrameState::new(0.0));
        let g = u.get(Slot::BgGrad);
        assert!(g[0].abs() < 1e-6 && (g[1] - 1.0).abs() < 1e-6, "{g:?}");
        assert!((g[2] - 1.0 / 1080.0).abs() < 1e-9);
    }

    #[test]
    fn mesh_points_reach_the_stop_and_loc_arrays() {
        let style = Style {
            background: Background::Mesh {
                points: vec![
                    MeshPoint::new(0.0, 0.0, RGBAColor::new(1.0, 0.0, 0.0, 1.0)),
                    MeshPoint::new(1.0, 1.0, RGBAColor::new(0.0, 0.0, 1.0, 1.0)),
                ],
                warp: 0.06,
            },
            ..style_plain()
        };
        let u = build(&style, &FrameState::new(0.0));
        assert_eq!(u.get(Slot::Bg)[0], 3.0);
        assert_eq!(u.get(Slot::Bg)[1] as usize, 2);
        assert!((u.get(Slot::Wallpaper)[0] - 0.06).abs() < 1e-6);
        assert_eq!(u.get_at(Slot::Stop0, 0)[3], 1.0);
        let loc1 = u.get_at(Slot::Loc0, 1);
        assert!((loc1[0] - 1.0).abs() < 1e-6 && (loc1[1] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn an_image_background_aspect_fills_and_a_missing_one_is_grey() {
        let style = Style {
            background: Background::Image {
                file: "bg.png".to_owned(),
            },
            ..style_plain()
        };
        let atlas = CursorAtlas::build();
        let l = layout(&style, CameraState::IDENTITY);
        let state = FrameState::new(0.0);
        // 200x100 image into a 16:9 output: the width is cropped.
        let u = write(&Scene {
            layout: &l,
            state: &state,
            style: &style,
            camera: None,
            image: Some(PixelSize::new(200, 100)),
            image_lod: 1.5,
            atlas: &atlas,
            dither: true,
        });
        assert_eq!(u.get(Slot::Bg)[0], 2.0);
        let s = u.get(Slot::BgImage);
        assert!((s[0] - 16.0 / 9.0 / 2.0).abs() < 1e-6, "{s:?}");
        assert!(s[1] == 1.0, "{s:?}");
        assert!((s[2] - 1.5).abs() < 1e-6);

        // Missing image: mid grey, not an error.
        let u = write(&Scene {
            layout: &l,
            state: &state,
            style: &style,
            camera: None,
            image: None,
            image_lod: 0.0,
            atlas: &atlas,
            dither: true,
        });
        assert_eq!(u.get(Slot::Bg)[0], 0.0, "falls back to a solid");
        let s = u.get(Slot::Solid);
        // linear(0.5) is about 0.2140 for all three channels.
        assert!((s[0] - 0.214_041).abs() < 1e-4, "{s:?}");
    }

    #[test]
    fn a_hidden_or_transparent_cursor_writes_nothing() {
        for cursor in [
            nss_core::CursorState {
                visible: false,
                ..Default::default()
            },
            nss_core::CursorState {
                opacity: 0.0,
                ..Default::default()
            },
            nss_core::CursorState {
                opacity: f64::NAN,
                ..Default::default()
            },
        ] {
            let mut state = FrameState::new(0.0);
            state.cursor = cursor;
            let u = build(&style_plain(), &state);
            assert_eq!(u.get(Slot::Cursor)[3], 0.0);
            assert_eq!(u.get(Slot::CursorBox), [0.0; 4], "no bounding box either");
        }
    }

    #[test]
    fn the_cursor_hotspot_sits_on_the_position_and_the_box_wraps_the_image() {
        let style = Style {
            cursor: CursorStyle {
                size: 4.0,
                motion_blur: 0.4,
                ..CursorStyle::default()
            },
            ..style_plain()
        };
        let mut state = FrameState::new(0.0);
        state.cursor.position = NormalizedPoint::new(0.4, 0.6);
        state.cursor.velocity = NormalizedPoint::ZERO;
        let u = build(&style, &state);
        let l = layout(&style, CameraState::IDENTITY);
        let hot = l.output_point(state.cursor.position);
        let c = u.get(Slot::Cursor);
        assert!((c[0] - hot.x as f32).abs() < 1e-3 && (c[1] - hot.y as f32).abs() < 1e-3);
        let ppp = l.cursor_pixels_per_point(&style.cursor);
        assert!((c[2] - ppp as f32).abs() < 1e-4);
        assert_eq!(c[3], 1.0);
        // A still cursor gets one sample and no blur.
        let blur = u.get(Slot::CursorBlur);
        assert_eq!(blur[0], 0.0);
        assert_eq!(blur[1], 0.0);
        assert_eq!(blur[2], 1.0);
        let b = u.get(Slot::CursorBox);
        assert!(
            b[0] < c[0] && b[1] < c[1] && b[2] > c[0] && b[3] > c[1],
            "{b:?}"
        );
        // The cell's uv rectangle is inside the atlas.
        let cell = u.get(Slot::CursorCell);
        assert!(cell[0] > 0.0 && cell[0] < 1.0 && cell[1] > 0.0 && cell[1] < 1.0);
        assert!(cell[0] + cell[2] <= 1.0001 && cell[1] + cell[3] <= 1.0001);
        let c2 = u.get(Slot::CursorCell2);
        assert!(c2[2] > 0.0 && c2[3] > 0.0, "image size in points");
    }

    #[test]
    fn press_squish_scales_the_cursor_and_motion_blur_adds_samples() {
        let style = Style {
            cursor: CursorStyle {
                size: 4.0,
                motion_blur: 1.0,
                ..CursorStyle::default()
            },
            ..style_plain()
        };
        let mut state = FrameState::new(0.0);
        state.cursor.press_scale = 0.5;
        let squished = build(&style, &state).get(Slot::Cursor)[2];
        state.cursor.press_scale = 1.0;
        let normal = build(&style, &state).get(Slot::Cursor)[2];
        assert!((squished / normal - 0.5).abs() < 1e-6);

        state.cursor.velocity = NormalizedPoint::new(3.0, 0.0);
        let u = build(&style, &state);
        let blur = u.get(Slot::CursorBlur);
        assert!(blur[0] > 0.0, "blur vector {blur:?}");
        assert!(blur[2] >= 2.0 && blur[2] <= 16.0, "samples {blur:?}");
        // The box grows to cover the blur.
        let cell_box = u.get(Slot::CursorBox);
        assert!(cell_box[3] - cell_box[1] > 0.0);
    }

    #[test]
    fn a_bad_press_scale_cannot_break_the_cursor() {
        let style = style_plain();
        for press in [f64::NAN, f64::INFINITY, -5.0, 100.0] {
            let mut state = FrameState::new(0.0);
            state.cursor.press_scale = press;
            let ppp = build(&style, &state).get(Slot::Cursor)[2];
            assert!(ppp.is_finite() && ppp > 0.0, "press {press} gave ppp {ppp}");
        }
    }

    #[test]
    fn ripples_are_capped_and_skipped_outside_their_lifetime() {
        let mut state = FrameState::new(0.0);
        state.cursor.ripples = (0..12)
            .map(|i| RippleState::new(NormalizedPoint::CENTER, f64::from(i) / 14.0))
            .collect();
        let u = build(&style_plain(), &state);
        assert_eq!(u.get(Slot::RippleParams)[0] as usize, MAX_RIPPLES);
        // progress >= 1 is not drawn.
        state.cursor.ripples = vec![RippleState::new(NormalizedPoint::CENTER, 1.0)];
        assert_eq!(
            build(&style_plain(), &state).get(Slot::RippleParams)[0],
            0.0
        );
        // A ring widens as it ages.
        state.cursor.ripples = vec![
            RippleState::new(NormalizedPoint::CENTER, 0.0),
            RippleState::new(NormalizedPoint::CENTER, 0.5),
        ];
        let u = build(&style_plain(), &state);
        assert!(u.get_at(Slot::Ripple0, 1)[2] > u.get_at(Slot::Ripple0, 0)[2]);
        assert!(u.get_at(Slot::Ripple0, 1)[3] < u.get_at(Slot::Ripple0, 0)[3]);
        // Off means off, even with live ripples.
        let style = Style {
            cursor: CursorStyle {
                click_ripples: false,
                ..CursorStyle::default()
            },
            ..style_plain()
        };
        assert_eq!(build(&style, &state).get(Slot::RippleParams)[0], 0.0);
    }

    #[test]
    fn the_bubble_needs_a_camera_frame_and_is_scaled_while_zoomed() {
        let style = Style {
            camera: CameraBubble {
                visible: true,
                shape: CameraShape::Circle,
                position: NormalizedPoint::new(0.85, 0.8),
                size: 0.2,
                border: None,
                zoomed_scale: 0.5,
                ..CameraBubble::default()
            },
            ..style_plain()
        };
        let mut state = FrameState::new(0.0);
        state.camera = CameraState {
            scale: 2.0,
            center: NormalizedPoint::CENTER,
        };
        let atlas = CursorAtlas::build();
        let l = layout(&style, state.camera);

        // No camera frame: the bubble is not drawn at all.
        let u = write(&Scene {
            layout: &l,
            state: &state,
            style: &style,
            camera: None,
            image: None,
            image_lod: 0.0,
            atlas: &atlas,
            dither: true,
        });
        assert_eq!(u.get(Slot::Bubble2)[3], 0.0);

        let u = write(&Scene {
            layout: &l,
            state: &state,
            style: &style,
            camera: Some(PixelSize::new(640, 480)),
            image: None,
            image_lod: 0.0,
            atlas: &atlas,
            dither: true,
        });
        assert_eq!(u.get(Slot::Bubble2)[3], 1.0);
        let full = l.camera_bubble_full_rect(&style.camera);
        let r = l.camera_bubble_rect(&style.camera);
        let b = u.get(Slot::Bubble);
        assert!((b[2] - (r.size.width / 2.0) as f32).abs() < 1e-3);
        assert!(
            (b[2] - (full.size.width / 4.0) as f32).abs() < 1e-3,
            "shrunk to half"
        );
        // Aspect fill of a 4:3 camera into a 1:1 bubble crops the width.
        let buv = u.get(Slot::BubbleUv);
        assert!(buv[0] < 1.0 && (buv[1] - 1.0).abs() < 1e-6, "{buv:?}");
    }

    #[test]
    fn the_bubble_border_band_and_shadow_scale_with_the_bubble() {
        let style = Style {
            camera: CameraBubble {
                visible: true,
                shape: CameraShape::Square,
                size: 0.2,
                border: Some(nss_core::CameraBorder {
                    width: 0.01,
                    color: RGBAColor::WHITE,
                }),
                zoomed_scale: 0.5,
                ..CameraBubble::default()
            },
            shadow: Shadow {
                radius: 0.03,
                opacity: 0.6,
                offset_x: 0.0,
                offset_y: 0.02,
            },
            ..style_plain()
        };
        let mut state = FrameState::new(0.0);
        state.camera = CameraState {
            scale: 2.0,
            center: NormalizedPoint::CENTER,
        };
        let atlas = CursorAtlas::build();
        let l = layout(&style, state.camera);
        let u = write(&Scene {
            layout: &l,
            state: &state,
            style: &style,
            camera: Some(PixelSize::new(640, 480)),
            image: None,
            image_lod: 0.0,
            atlas: &atlas,
            dither: false,
        });
        let k = l.camera_bubble_size_multiplier(&style.camera);
        assert!((k - 0.5).abs() < 1e-9);
        let b2 = u.get(Slot::Bubble2);
        assert!(
            (b2[2] - (0.01 * 1080.0 * k) as f32).abs() < 1e-3,
            "border {}",
            b2[2]
        );
        let bs = u.get(Slot::BubbleShadow);
        assert!((bs[0] - (0.03 * 1080.0 / 2.0 * k) as f32).abs() < 1e-3);
        assert!((bs[3] - (0.02 * 1080.0 * k) as f32).abs() < 1e-3);
        let bc = u.get(Slot::BorderColor);
        assert!((bc[0] - 1.0).abs() < 1e-6 && (bc[3] - 1.0).abs() < 1e-6);
        // The dither switch really reaches the shader.
        assert_eq!(u.get(Slot::Out)[2], 0.0);
    }

    #[test]
    fn the_footprint_filter_picks_up_to_six_taps() {
        assert_eq!(taps(0.5), 1);
        assert_eq!(taps(1.0), 1);
        assert_eq!(taps(2.0), 2);
        assert_eq!(taps(2.6), 3);
        assert_eq!(taps(40.0), 6);
        assert_eq!(taps(f64::NAN), 1);
    }

    #[test]
    fn cursor_lod_follows_minification() {
        assert_eq!(cursor_lod(SCALE), 0.0);
        assert_eq!(cursor_lod(40.0), 0.0);
        assert!((cursor_lod(SCALE / 4.0) - 2.0).abs() < 1e-9);
        assert_eq!(cursor_lod(0.01), MAX_LOD);
        assert_eq!(cursor_lod(f64::NAN), 0.0);
    }

    #[test]
    fn the_uniform_block_serialises_to_sixteen_bytes_per_slot() {
        let u = build(&style_plain(), &FrameState::new(0.0));
        let bytes = u.to_bytes();
        assert_eq!(bytes.len(), Slot::COUNT * 16);
        // Little-endian f32 for the first slot's W.
        let w = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        assert!((w - 1920.0).abs() < 1e-6);
    }
}
