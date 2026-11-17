//! Pure geometry of one rendered frame, in output pixels (origin top-left, y down).
//!
//! Port of upstream `NSSRender/RenderLayout.swift`. Every shader uniform is derived from this type,
//! so the editor can hit-test the screen content, the cursor and the camera bubble exactly where the
//! GPU draws them — and, more importantly for this port, the preview and the export cannot disagree
//! about geometry because they read it from the same place.

use nss_core::{
    CameraBubble, CameraBubbleGeometry, CameraState, CursorStyle, ExportResolution,
    NormalizedPoint, PixelSize, Point2, Rect, Style,
};

/// Which point of a full-size bubble stays fixed while it shrinks, per axis.
///
/// 0 (left/top edge) in the first third, 1 (right/bottom edge) in the last third, a linear blend
/// through the middle third. Continuous, so dragging the bubble while zoomed never makes it jump.
#[must_use]
pub fn bubble_anchor(position: f64) -> f64 {
    if !position.is_finite() {
        return 0.5;
    }
    ((position - 1.0 / 3.0) * 3.0).clamp(0.0, 1.0)
}

/// The camera the renderer actually uses.
///
/// Scale is at least 1 and the centre is kept `0.5 / scale` away from every source edge — the same
/// rule as the motion model, so the content can never show a gap or a stretched edge even when a
/// caller passes a camera outside that range. Non-finite values fall back to identity per component
/// instead of poisoning every uniform with NaN.
#[must_use]
pub fn clamped_camera(camera: CameraState) -> CameraState {
    // Deliberately the same function the motion model and its guarantee test use
    // (`nss_motion::clamped_to_source`, which nudges by one ULP so the viewport is never a hair
    // outside the source). Duplicating the maths here is how preview and export would drift.
    nss_motion::clamped_to_source(camera.scale, camera.center)
}

/// Geometry of one frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderLayout {
    pub output_size: PixelSize,
    pub source_size: PixelSize,
    /// Already clamped (see [`clamped_camera`]).
    pub camera: CameraState,
    /// Where the (rounded, shadowed) screen content sits in the output.
    pub content_rect: Rect,
    /// Corner radius of the content in output pixels (already clamped to half the shorter side).
    pub corner_radius: f64,
}

impl RenderLayout {
    #[must_use]
    pub fn new(
        output_size: PixelSize,
        source_size: PixelSize,
        style: &Style,
        camera: CameraState,
    ) -> Self {
        let camera = clamped_camera(camera);
        let w = f64::from(output_size.width.max(1));
        let h = f64::from(output_size.height.max(1));
        let pad = style.padding.max(0.0) * h;
        let avail_w = (w - 2.0 * pad).max(1.0);
        let avail_h = (h - 2.0 * pad).max(1.0);
        let src_aspect = if source_size.width > 0 && source_size.height > 0 {
            source_size.aspect()
        } else {
            16.0 / 9.0
        };
        let (cw, ch) = if src_aspect > avail_w / avail_h {
            (avail_w, avail_w / src_aspect)
        } else {
            (avail_h * src_aspect, avail_h)
        };
        let content_rect = Rect::new((w - cw) / 2.0, (h - ch) / 2.0, cw, ch);
        let corner_radius = (style.corner_radius.max(0.0) * h).min(cw.min(ch) / 2.0);
        Self {
            output_size,
            source_size,
            camera,
            content_rect,
            corner_radius,
        }
    }

    /// Output pixel size for an aspect ratio and export resolution.
    #[must_use]
    pub fn output_size_for(
        aspect: nss_core::AspectRatio,
        resolution: ExportResolution,
    ) -> PixelSize {
        aspect.output_size(resolution)
    }

    // MARK: Camera transform

    /// Maps a normalized SOURCE point through the camera (zoom/pan) to output pixels.
    #[must_use]
    pub fn output_point(&self, source: NormalizedPoint) -> Point2 {
        let ux = (source.x - self.camera.center.x) * self.camera.scale + 0.5;
        let uy = (source.y - self.camera.center.y) * self.camera.scale + 0.5;
        Point2::new(
            self.content_rect.origin.x + ux * self.content_rect.size.width,
            self.content_rect.origin.y + uy * self.content_rect.size.height,
        )
    }

    /// Inverse of [`Self::output_point`].
    #[must_use]
    pub fn source_point(&self, output: Point2) -> NormalizedPoint {
        let ux = (output.x - self.content_rect.origin.x) / self.content_rect.size.width;
        let uy = (output.y - self.content_rect.origin.y) / self.content_rect.size.height;
        NormalizedPoint::new(
            self.camera.center.x + (ux - 0.5) / self.camera.scale,
            self.camera.center.y + (uy - 0.5) / self.camera.scale,
        )
    }

    /// Source texels covered by one output pixel along x (greater than 1 means downscaling).
    #[must_use]
    pub fn source_texels_per_output_pixel(&self) -> f64 {
        f64::from(self.source_size.width) / (self.content_rect.size.width * self.camera.scale)
    }

    /// A size that tracks the screen content independent of its aspect (geometric mean of the sides).
    #[must_use]
    pub fn content_scale(&self) -> f64 {
        (self.content_rect.size.width * self.content_rect.size.height).sqrt()
    }

    // MARK: Cursor and ripples

    /// Height of the drawn cursor arrow in output pixels; scales with the content, the zoom and
    /// [`CursorStyle::size`].
    #[must_use]
    pub fn cursor_height(&self, style: &CursorStyle) -> f64 {
        0.0165 * self.content_scale() * self.camera.scale * style.size.max(0.0)
    }

    /// Output pixels per cursor point (the unit of the cursor images and hotspots) at normal size,
    /// before `CursorState::press_scale`. The 19 pt reference is the height of the arrow with its
    /// outline, matching upstream's `NSCursor` sizing.
    #[must_use]
    pub fn cursor_pixels_per_point(&self, style: &CursorStyle) -> f64 {
        self.cursor_height(style) / 19.0
    }

    /// Output-pixel blur vector for the cursor (full shutter length, centred on the position).
    #[must_use]
    pub fn cursor_blur_vector(&self, velocity: NormalizedPoint, style: &CursorStyle) -> Point2 {
        let shutter = style.motion_blur.clamp(0.0, 1.0) / 30.0;
        let mut dx = velocity.x * self.content_rect.size.width * self.camera.scale * shutter;
        let mut dy = velocity.y * self.content_rect.size.height * self.camera.scale * shutter;
        let len = (dx * dx + dy * dy).sqrt();
        let cap = 3.0 * self.cursor_height(style);
        if len > cap && len > 0.0 {
            dx *= cap / len;
            dy *= cap / len;
        }
        Point2::new(dx, dy)
    }

    /// Largest ripple radius in output pixels.
    #[must_use]
    pub fn ripple_max_radius(&self) -> f64 {
        0.03 * self.content_scale() * self.camera.scale
    }

    /// Ring radius in output pixels for `progress` 0...1 (ease-out expansion).
    #[must_use]
    pub fn ripple_radius(&self, progress: f64) -> f64 {
        let p = progress.clamp(0.0, 1.0);
        let e = 1.0 - (1.0 - p).powi(3);
        self.ripple_max_radius() * (0.15 + 0.85 * e)
    }

    /// Ring opacity for `progress` 0...1.
    #[must_use]
    pub fn ripple_opacity(&self, progress: f64) -> f64 {
        (1.0 - progress.clamp(0.0, 1.0)).powf(1.5)
    }

    /// Ring stroke width in output pixels.
    #[must_use]
    pub fn ripple_thickness(&self) -> f64 {
        (self.ripple_max_radius() * 0.11).max(1.5)
    }

    // MARK: Camera bubble

    /// Bubble size multiplier for this layout's camera zoom.
    #[must_use]
    pub fn camera_bubble_size_multiplier(&self, bubble: &CameraBubble) -> f64 {
        let m = bubble.size_multiplier(self.camera.scale);
        if m.is_finite() {
            m.max(0.0)
        } else {
            1.0
        }
    }

    /// Bubble frame at full size (camera scale 1), centred on `bubble.position`.
    #[must_use]
    pub fn camera_bubble_full_rect(&self, bubble: &CameraBubble) -> Rect {
        let w = f64::from(self.output_size.width);
        let h = f64::from(self.output_size.height);
        let l = bubble.layout(bubble.size.max(0.0) * w);
        Rect::new(
            bubble.position.x * w - l.width / 2.0,
            bubble.position.y * h - l.height / 2.0,
            l.width,
            l.height,
        )
    }

    /// Bubble frame as drawn at this layout's camera zoom. At scale 1 it is centred on
    /// `bubble.position`; when zoomed in it shrinks by [`Self::camera_bubble_size_multiplier`]
    /// toward its anchor, so a bubble in a corner stays tucked into that corner.
    #[must_use]
    pub fn camera_bubble_rect(&self, bubble: &CameraBubble) -> Rect {
        let full = self.camera_bubble_full_rect(bubble);
        let m = self.camera_bubble_size_multiplier(bubble);
        let w = full.size.width * m;
        let h = full.size.height * m;
        let ax = bubble_anchor(bubble.position.x);
        let ay = bubble_anchor(bubble.position.y);
        Rect::new(
            full.origin.x + ax * (full.size.width - w),
            full.origin.y + ay * (full.size.height - h),
            w,
            h,
        )
    }

    /// Corner radius of the bubble in output pixels.
    #[must_use]
    pub fn camera_bubble_corner_radius(&self, bubble: &CameraBubble, rect: Rect) -> f64 {
        CameraBubbleGeometry::layout(bubble.shape, bubble.roundness, rect.size.width).corner_radius
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nss_core::{AspectRatio, CameraShape, RGBAColor};

    /// Whole-rect containment (upstream uses `CGRect.contains(CGRect)`).
    fn rect_contains(outer: nss_core::Rect, inner: nss_core::Rect) -> bool {
        inner.min_x() >= outer.min_x()
            && inner.min_y() >= outer.min_y()
            && inner.max_x() <= outer.max_x()
            && inner.max_y() <= outer.max_y()
    }

    fn out(size: PixelSize, style: &Style, camera: CameraState) -> RenderLayout {
        RenderLayout::new(size, PixelSize::new(1920, 1080), style, camera)
    }

    #[test]
    fn output_sizes_match_the_aspect_ratio() {
        assert_eq!(
            RenderLayout::output_size_for(AspectRatio::R16x9, ExportResolution::P1080),
            PixelSize::new(1920, 1080)
        );
        assert_eq!(
            RenderLayout::output_size_for(AspectRatio::R16x9, ExportResolution::P2160),
            PixelSize::new(3840, 2160)
        );
        assert_eq!(
            RenderLayout::output_size_for(AspectRatio::R1x1, ExportResolution::P2160),
            PixelSize::new(2160, 2160)
        );
        assert_eq!(
            RenderLayout::output_size_for(AspectRatio::R9x16, ExportResolution::P1080),
            PixelSize::new(1080, 1920)
        );
    }

    #[test]
    fn content_fits_inside_the_padding_keeping_the_source_aspect() {
        for aspect in AspectRatio::ALL {
            let size = aspect.output_size(ExportResolution::P1080);
            let style = Style {
                padding: 0.08,
                ..Style::default()
            };
            let l = RenderLayout::new(
                size,
                PixelSize::new(2880, 1800),
                &style,
                CameraState::IDENTITY,
            );
            let pad = 0.08 * f64::from(size.height);
            let r = l.content_rect;
            assert!(r.min_x() >= pad - 0.001 && r.min_y() >= pad - 0.001);
            assert!(r.max_x() <= f64::from(size.width) - pad + 0.001);
            assert!(r.max_y() <= f64::from(size.height) - pad + 0.001);
            assert!((r.size.width / r.size.height - 1.6).abs() < 1e-9);
            assert!((r.mid_x() - f64::from(size.width) / 2.0).abs() < 1e-9);
            assert!((r.mid_y() - f64::from(size.height) / 2.0).abs() < 1e-9);
            assert!((r.min_x() - pad).abs() < 0.001 || (r.min_y() - pad).abs() < 0.001);
        }
    }

    #[test]
    fn camera_transform_round_trips_through_the_content_centre() {
        let cam = CameraState {
            scale: 2.5,
            center: NormalizedPoint::new(0.3, 0.7),
        };
        let l = out(PixelSize::new(1920, 1080), &Style::default(), cam);
        let centre = l.output_point(cam.center);
        assert!((centre.x - l.content_rect.mid_x()).abs() < 1e-9);
        assert!((centre.y - l.content_rect.mid_y()).abs() < 1e-9);
        let p = NormalizedPoint::new(0.31, 0.66);
        let back = l.source_point(l.output_point(p));
        assert!((back.x - p.x).abs() < 1e-9 && (back.y - p.y).abs() < 1e-9);
    }

    #[test]
    fn non_finite_cameras_fall_back_to_identity() {
        let bad = CameraState {
            scale: f64::NAN,
            center: NormalizedPoint::new(f64::INFINITY, f64::NEG_INFINITY),
        };
        let l = out(PixelSize::new(1920, 1080), &Style::default(), bad);
        assert_eq!(l.camera, CameraState::IDENTITY);
        let p = l.output_point(NormalizedPoint::CENTER);
        assert!(p.x.is_finite() && p.y.is_finite());
    }

    #[test]
    fn valid_cameras_are_untouched_by_the_clamp() {
        let cam = CameraState {
            scale: 2.5,
            center: NormalizedPoint::new(0.3, 0.7),
        };
        assert_eq!(clamped_camera(cam), cam);
        assert_eq!(clamped_camera(CameraState::IDENTITY), CameraState::IDENTITY);
    }

    #[test]
    fn the_clamp_keeps_the_viewport_inside_the_source() {
        let cases = [
            CameraState {
                scale: 0.5,
                center: NormalizedPoint::CENTER,
            },
            CameraState {
                scale: 2.0,
                center: NormalizedPoint::new(0.0, 1.0),
            },
            CameraState {
                scale: 3.0,
                center: NormalizedPoint::new(-0.4, 1.7),
            },
            CameraState {
                scale: 1.0001,
                center: NormalizedPoint::new(0.9, 0.1),
            },
        ];
        for cam in cases {
            let c = clamped_camera(cam);
            assert!(c.scale >= 1.0);
            let half = 0.5 / c.scale;
            assert!(c.center.x - half >= -1e-12 && c.center.x + half <= 1.0 + 1e-12);
            assert!(c.center.y - half >= -1e-12 && c.center.y + half <= 1.0 + 1e-12);
            let l = out(PixelSize::new(1920, 1080), &Style::default(), cam);
            for p in [
                Point2::new(l.content_rect.min_x(), l.content_rect.min_y()),
                Point2::new(l.content_rect.max_x(), l.content_rect.max_y()),
            ] {
                let s = l.source_point(p);
                // The guarantee (ULP-exact) lives in `nss_motion::clamped_to_source`; mapping the
                // content rect back through it can still round by one ULP.
                assert!(
                    (-1e-9..=1.0 + 1e-9).contains(&s.x) && (-1e-9..=1.0 + 1e-9).contains(&s.y),
                    "{cam:?} -> {s:?}"
                );
            }
        }
    }

    #[test]
    fn ripples_grow_and_fade() {
        let l = out(
            PixelSize::new(1920, 1080),
            &Style::default(),
            CameraState::IDENTITY,
        );
        assert!(l.ripple_radius(0.0) < l.ripple_radius(0.5));
        assert!(l.ripple_radius(0.5) < l.ripple_radius(1.0));
        assert!((l.ripple_opacity(0.0) - 1.0).abs() < 1e-12);
        assert!(l.ripple_opacity(1.0).abs() < 1e-12);
    }

    #[test]
    fn cursor_pixels_per_point_scale_with_zoom_and_size() {
        let mut style = CursorStyle::default();
        let base = out(
            PixelSize::new(1920, 1080),
            &Style::default(),
            CameraState::IDENTITY,
        )
        .cursor_pixels_per_point(&style);
        let l = out(
            PixelSize::new(1920, 1080),
            &Style::default(),
            CameraState::IDENTITY,
        );
        assert!((base * 19.0 - l.cursor_height(&style)).abs() < 1e-9);
        let zoomed = out(
            PixelSize::new(1920, 1080),
            &Style::default(),
            CameraState {
                scale: 2.0,
                center: NormalizedPoint::CENTER,
            },
        );
        assert!((zoomed.cursor_pixels_per_point(&style) - 2.0 * base).abs() < 1e-9);
        style.size *= 2.0;
        assert!((l.cursor_pixels_per_point(&style) - 2.0 * base).abs() < 1e-9);
    }

    #[test]
    fn bubble_anchor_is_continuous_and_picks_the_near_side() {
        assert!(bubble_anchor(0.05).abs() < 1e-12);
        assert!(bubble_anchor(0.3).abs() < 1e-12);
        assert!((bubble_anchor(0.5) - 0.5).abs() < 1e-12);
        assert!((bubble_anchor(0.7) - 1.0).abs() < 1e-12);
        assert!((bubble_anchor(0.95) - 1.0).abs() < 1e-12);
        assert!((bubble_anchor(f64::NAN) - 0.5).abs() < 1e-12);
        let mut prev = bubble_anchor(0.0);
        for i in 1..=1000 {
            let a = bubble_anchor(f64::from(i) / 1000.0);
            assert!(a >= prev && a - prev < 0.004, "jump at {i}: {prev} -> {a}");
            prev = a;
        }
    }

    #[test]
    fn bubble_rect_follows_its_shape() {
        let l = out(
            PixelSize::new(1920, 1080),
            &Style::default(),
            CameraState::IDENTITY,
        );
        let mut b = CameraBubble {
            shape: CameraShape::Circle,
            position: NormalizedPoint::new(0.5, 0.5),
            size: 0.2,
            ..CameraBubble::default()
        };
        let circle = l.camera_bubble_rect(&b);
        assert!((circle.size.width - 384.0).abs() < 1e-9);
        assert!((circle.size.height - 384.0).abs() < 1e-9);
        assert!((circle.mid_x() - 960.0).abs() < 1e-9 && (circle.mid_y() - 540.0).abs() < 1e-9);
        b.shape = CameraShape::Wide;
        let rect = l.camera_bubble_rect(&b);
        assert!((rect.size.width - 384.0).abs() < 1e-9);
        assert!((rect.size.height - 216.0).abs() < 1e-9);
    }

    #[test]
    fn bubble_is_full_size_when_not_zoomed() {
        let b = CameraBubble::default();
        let l = out(
            PixelSize::new(1920, 1080),
            &Style::default(),
            CameraState::IDENTITY,
        );
        assert!((l.camera_bubble_size_multiplier(&b) - 1.0).abs() < 1e-12);
        assert_eq!(l.camera_bubble_rect(&b), l.camera_bubble_full_rect(&b));
    }

    #[test]
    fn bubble_shrinks_into_its_corner() {
        for scale in [1.0, 1.3, 1.7, 2.0, 3.5] {
            let b = CameraBubble {
                shape: CameraShape::Wide,
                position: NormalizedPoint::new(0.88, 0.82),
                size: 0.2,
                zoomed_scale: 0.5,
                ..CameraBubble::default()
            };
            let l = out(
                PixelSize::new(1920, 1080),
                &Style::default(),
                CameraState {
                    scale,
                    center: NormalizedPoint::CENTER,
                },
            );
            let full = l.camera_bubble_full_rect(&b);
            let r = l.camera_bubble_rect(&b);
            let m = nss_core::CameraBubbleGeometry::size_multiplier(scale, 0.5);
            assert!((l.camera_bubble_size_multiplier(&b) - m).abs() < 1e-12);
            assert!((r.size.width - full.size.width * m).abs() < 1e-9);
            assert!((r.size.height - full.size.height * m).abs() < 1e-9);
            // The bottom-right corner stays put.
            assert!((r.max_x() - full.max_x()).abs() < 1e-9);
            assert!((r.max_y() - full.max_y()).abs() < 1e-9);
            assert!(rect_contains(full, r));
        }
    }

    #[test]
    fn bubble_shrink_anchors_per_axis() {
        let l = out(
            PixelSize::new(1920, 1080),
            &Style::default(),
            CameraState {
                scale: 2.0,
                center: NormalizedPoint::CENTER,
            },
        );
        let mut b = CameraBubble {
            position: NormalizedPoint::new(0.1, 0.15),
            size: 0.2,
            ..CameraBubble::default()
        };
        let full = l.camera_bubble_full_rect(&b);
        let r = l.camera_bubble_rect(&b);
        assert!((r.min_x() - full.min_x()).abs() < 1e-9);
        assert!((r.min_y() - full.min_y()).abs() < 1e-9);
        b.position = NormalizedPoint::new(0.5, 0.9);
        let full = l.camera_bubble_full_rect(&b);
        let r = l.camera_bubble_rect(&b);
        assert!((r.mid_x() - full.mid_x()).abs() < 1e-9);
        assert!((r.max_y() - full.max_y()).abs() < 1e-9);
        b.zoomed_scale = 1.0;
        assert_eq!(l.camera_bubble_rect(&b), l.camera_bubble_full_rect(&b));
    }

    #[test]
    fn bubble_corner_radius_comes_from_the_shared_geometry() {
        for shape in CameraShape::ALL {
            let b = CameraBubble {
                shape,
                size: 0.2,
                roundness: 0.4,
                ..CameraBubble::default()
            };
            let l = out(
                PixelSize::new(1920, 1080),
                &Style::default(),
                CameraState {
                    scale: 2.0,
                    center: NormalizedPoint::CENTER,
                },
            );
            let r = l.camera_bubble_rect(&b);
            let g = CameraBubbleGeometry::layout(shape, b.roundness, r.size.width);
            assert!((r.size.height - g.height).abs() < 1e-9);
            assert!(
                (r.size.width / r.size.height - CameraBubbleGeometry::aspect(shape)).abs() < 1e-9
            );
            assert!((l.camera_bubble_corner_radius(&b, r) - g.corner_radius).abs() < 1e-12);
            if shape == CameraShape::Circle {
                assert!((g.corner_radius - r.size.width / 2.0).abs() < 1e-9);
            }
        }
    }

    #[test]
    fn a_zero_padding_style_fills_the_whole_output() {
        let style = Style {
            background: nss_core::Background::Solid(RGBAColor::BLACK),
            padding: 0.0,
            corner_radius: 0.0,
            ..Style::default()
        };
        let l = RenderLayout::new(
            PixelSize::new(640, 360),
            PixelSize::new(1920, 1080),
            &style,
            CameraState::IDENTITY,
        );
        assert!((l.content_rect.size.width - 640.0).abs() < 1e-9);
        assert!((l.content_rect.size.height - 360.0).abs() < 1e-9);
        assert!(l.content_rect.origin.x.abs() < 1e-9 && l.content_rect.origin.y.abs() < 1e-9);
    }
}
