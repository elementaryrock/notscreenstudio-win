//! Pixel tests on the real GPU, ported name-for-name from upstream
//! `Tests/NSSRenderTests/MetalRendererTests.swift`. Every test skips cleanly when the machine has no
//! usable adapter (upstream: `.enabled(if: GPU.available)`).

mod support;

use nss_core::{
    AspectRatio, Background, CameraBorder, CameraBubble, CameraShape, CameraState, CursorState,
    ExportResolution, GradientStop, NormalizedPoint, PixelSize, RGBAColor, RippleState, Shadow,
    Style,
};
use nss_render::layout::RenderLayout;
use nss_render::renderer::Renderer;

use support::*;

const W: u32 = 1920;
const H: u32 = 1080;

fn layout_for(style: &Style, size: (u32, u32), camera: CameraState) -> RenderLayout {
    RenderLayout::new(
        PixelSize::new(size.0, size.1),
        PixelSize::new(1920, 1080),
        style,
        camera,
    )
}

#[test]
fn corners_show_background_and_centre_shows_screen() {
    let mut r = need_gpu!();
    let screen = filled(1920, 1080, GREEN);
    let out = render(
        &mut r,
        &screen,
        None,
        &hidden_cursor(),
        &plain_style(0.1, 0.0),
        PixelSize::new(W, H),
    );
    for (x, y) in [(0, 0), (W - 1, 0), (0, H - 1), (W - 1, H - 1)] {
        let p = pixel(&out, x, y);
        assert!(
            p.near(Rgb { r: 255, g: 0, b: 0 }, 3),
            "corner {x},{y} = {p}"
        );
    }
    let c = pixel(&out, W / 2, H / 2);
    assert!(c.near(Rgb { r: 0, g: 255, b: 0 }, 1), "centre {c}");
}

#[test]
fn rounded_corner_is_anti_aliased() {
    let mut r = need_gpu!();
    let screen = filled(1920, 1080, GREEN);
    let style = plain_style(0.1, 0.05);
    let out = render(
        &mut r,
        &screen,
        None,
        &hidden_cursor(),
        &style,
        PixelSize::new(W, H),
    );

    let l = layout_for(&style, (W, H), CameraState::IDENTITY);
    let rad = l.corner_radius;
    let cx = l.content_rect.min_x() + rad;
    let cy = l.content_rect.min_y() + rad;

    let outside = pixel(
        &out,
        l.content_rect.min_x() as u32 + 1,
        l.content_rect.min_y() as u32 + 1,
    );
    assert!(
        outside.near(Rgb { r: 255, g: 0, b: 0 }, 3),
        "corner pixel {outside}"
    );

    let edge_screen = pixel(&out, (cx + 10.0) as u32, l.content_rect.min_y() as u32 + 1);
    assert!(
        edge_screen.near(Rgb { r: 0, g: 255, b: 0 }, 1),
        "straight edge {edge_screen}"
    );

    // The pixel whose centre sits closest to the 45-degree point of the arc must be a blend.
    let diag = (
        cx - rad * std::f64::consts::FRAC_1_SQRT_2,
        cy - rad * std::f64::consts::FRAC_1_SQRT_2,
    );
    let mut best = (0_u32, 0_u32, f64::INFINITY);
    for y in (diag.1 as i64 - 3)..=(diag.1 as i64 + 3) {
        for x in (diag.0 as i64 - 3)..=(diag.0 as i64 + 3) {
            let px = x as f64 + 0.5;
            let py = y as f64 + 0.5;
            let d = ((cx - px).powi(2) + (cy - py).powi(2)).sqrt() - rad;
            if d.abs() < best.2.abs() {
                best = (x.max(0) as u32, y.max(0) as u32, d);
            }
        }
    }
    assert!(
        best.2.abs() < 0.2,
        "closest pixel is {} from the arc",
        best.2
    );
    let edge = pixel(&out, best.0, best.1);
    assert!(
        edge.r > 100 && edge.r < 240 && edge.g > 100 && edge.g < 240,
        "edge pixel {edge}"
    );
}

#[test]
fn drop_shadow_darkens_background_below_content() {
    let mut r = need_gpu!();
    let screen = filled(1920, 1080, GREEN);
    let mut style = plain_style(0.1, 0.02);
    style.background = Background::Solid(RGBAColor::WHITE);
    style.shadow = Shadow {
        radius: 0.04,
        opacity: 0.6,
        offset_x: 0.0,
        offset_y: 0.015,
    };
    let out = render(
        &mut r,
        &screen,
        None,
        &hidden_cursor(),
        &style,
        PixelSize::new(W, H),
    );
    let l = layout_for(&style, (W, H), CameraState::IDENTITY);
    let below = pixel(&out, W / 2, l.content_rect.max_y() as u32 + 6);
    let far = pixel(&out, 2, 2);
    assert!(below.luma() < far.luma() - 40, "below {below} far {far}");
    assert!(
        far.near(
            Rgb {
                r: 255,
                g: 255,
                b: 255
            },
            2
        ),
        "far {far}"
    );
}

#[test]
fn renders_every_aspect_ratio() {
    let mut r = need_gpu!();
    for aspect in AspectRatio::ALL {
        let size = RenderLayout::output_size_for(aspect, ExportResolution::P1080);
        let mut style = plain_style(0.1, 0.0);
        style.aspect_ratio = aspect;
        let out = render(
            &mut r,
            &filled(1920, 1080, GREEN),
            None,
            &hidden_cursor(),
            &style,
            size,
        );
        assert_eq!((out.width, out.height), (size.width, size.height));
        assert!(pixel(&out, 1, 1).near(Rgb { r: 255, g: 0, b: 0 }, 3));
        let c = pixel(&out, size.width / 2, size.height / 2);
        assert!(
            c.near(Rgb { r: 0, g: 255, b: 0 }, 1),
            "{aspect:?} centre {c}"
        );
    }
}

#[test]
fn downscale_averages_the_footprint_in_linear_light() {
    let mut r = need_gpu!();
    // Stripes of period 3 (white, black, black), shrunk 3x, must average to 1/3 linear = sRGB ~156;
    // a single bilinear tap cannot produce that.
    let src = make(1920, 1080, |x, _| {
        if x % 3 == 0 {
            RGBAColor::WHITE
        } else {
            RGBAColor::BLACK
        }
    });
    let out = render(
        &mut r,
        &src,
        None,
        &hidden_cursor(),
        &plain_style(0.0, 0.0),
        PixelSize::new(640, 360),
    );
    for x in [100, 320, 500] {
        let p = pixel(&out, x, 180);
        assert!((p.g - 156).abs() <= 6, "pixel {x} = {p}");
    }
}

#[test]
fn camera_zoom_crops_the_source() {
    let mut r = need_gpu!();
    let src = make(1920, 1080, |x, _| if x < 960 { BLUE } else { GREEN });
    let mut state = hidden_cursor();
    state.camera = CameraState::new(2.0, NormalizedPoint::new(0.75, 0.5));
    let out = render(
        &mut r,
        &src,
        None,
        &state,
        &plain_style(0.0, 0.0),
        PixelSize::new(W, H),
    );
    let left = pixel(&out, 20, H / 2);
    let right = pixel(&out, W - 20, H / 2);
    assert!(left.near(Rgb { r: 0, g: 255, b: 0 }, 1), "left {left}");
    assert!(right.near(Rgb { r: 0, g: 255, b: 0 }, 1), "right {right}");
}

#[test]
fn ripple_draws_a_ring() {
    let mut r = need_gpu!();
    let grey = RGBAColor::new(0.2, 0.2, 0.2, 1.0);
    let screen = filled(1920, 1080, grey);
    let style = plain_style(0.1, 0.0);
    let progress = 0.3;
    let mut state = hidden_cursor();
    state.cursor.ripples = vec![RippleState {
        position: NormalizedPoint::CENTER,
        progress,
    }];
    let size = PixelSize::new(W, H);
    let out = render(&mut r, &screen, None, &state, &style, size);

    let l = layout_for(&style, (W, H), CameraState::IDENTITY);
    let c = l.output_point(NormalizedPoint::CENTER);
    let rad = l.ripple_radius(progress);
    let ring = pixel(&out, (c.x + rad) as u32, c.y as u32);
    let inside = pixel(&out, (c.x + rad * 0.4) as u32, c.y as u32);
    let outside = pixel(&out, (c.x + l.ripple_max_radius() * 1.6) as u32, c.y as u32);
    assert!(
        ring.luma() > inside.luma() + 80,
        "ring {ring} inside {inside}"
    );
    assert!((outside.luma() - 51).abs() <= 2, "outside {outside}");

    // Ripples off: no ring.
    let mut off = style;
    off.cursor.click_ripples = false;
    let out = render(&mut r, &screen, None, &state, &off, size);
    let ring = pixel(&out, (c.x + rad) as u32, c.y as u32);
    assert!(
        (ring.luma() - 51).abs() <= 2,
        "ring with ripples off {ring}"
    );
}

fn cursor_scene(
    r: &mut Renderer,
    cursor: CursorState,
    motion_blur: f64,
    screen_color: RGBAColor,
) -> (nss_render::Image, (f64, f64), f64) {
    let screen = filled(1920, 1080, screen_color);
    let mut style = plain_style(0.1, 0.0);
    style.cursor.size = 4.0;
    style.cursor.motion_blur = motion_blur;
    let mut state = hidden_cursor();
    state.cursor = cursor.clone();
    let out = render(r, &screen, None, &state, &style, PixelSize::new(W, H));
    let l = layout_for(&style, (W, H), CameraState::IDENTITY);
    let tip = l.output_point(cursor.position);
    let unit = l.cursor_pixels_per_point(&style.cursor);
    (out, (tip.x, tip.y), unit)
}

/// The darkest and brightest pixels in a `points`-square window starting at the cursor tip.
///
/// The arrow art itself is ours (Windows has no `NSCursor`), so these tests ask "is a dark body with a
/// bright outline drawn at the hotspot?" rather than pinning pixels of the macOS system arrow.
fn extremes(out: &nss_render::Image, tip: (f64, f64), unit: f64, points: f64) -> (Rgb, Rgb) {
    let mut dark = Rgb {
        r: 255,
        g: 255,
        b: 255,
    };
    let mut bright = Rgb { r: 0, g: 0, b: 0 };
    let span = (points * unit) as i64;
    for dy in 0..span {
        for dx in 0..span {
            let p = pixel(out, (tip.0 as i64 + dx) as u32, (tip.1 as i64 + dy) as u32);
            if p.luma() < dark.luma() {
                dark = p;
            }
            if p.luma() > bright.luma() {
                bright = p;
            }
        }
    }
    (dark, bright)
}

#[test]
fn cursor_is_drawn_and_respects_visibility() {
    let mut r = need_gpu!();
    let grey = RGBAColor::new(0.5, 0.5, 0.5, 1.0);
    let (out, tip, unit) = cursor_scene(&mut r, CursorState::default(), 0.4, grey);
    let (dark, bright) = extremes(&out, tip, unit, 12.0);
    assert!(dark.luma() < 15, "the arrow body is not dark: {dark}");
    assert!(
        bright.luma() > 230,
        "the arrow outline is not bright: {bright}"
    );

    let hidden = CursorState {
        visible: false,
        ..CursorState::default()
    };
    let (hidden_out, _, _) = cursor_scene(&mut r, hidden, 0.4, grey);
    let (hidden_dark, hidden_bright) = extremes(&hidden_out, tip, unit, 12.0);
    // Nothing but the flat background: one luma, and no bright outline.
    assert!(
        (hidden_dark.luma() - 128).abs() <= 2,
        "hidden cursor {hidden_dark}"
    );
    assert!(
        (hidden_bright.luma() - 128).abs() <= 2,
        "hidden outline {hidden_bright}"
    );

    let half = CursorState {
        opacity: 0.5,
        ..CursorState::default()
    };
    let (half_out, _, _) = cursor_scene(&mut r, half, 0.4, grey);
    let (half_dark, _) = extremes(&half_out, tip, unit, 12.0);
    // Black at 50% over grey, blended in linear light.
    assert!(
        (70..=115).contains(&half_dark.luma()),
        "half-faded body {}",
        half_dark.luma()
    );
}

#[test]
fn cursor_motion_blur_smears_along_velocity() {
    let mut r = need_gpu!();
    let (sharp, tip, unit) = cursor_scene(&mut r, CursorState::default(), 0.4, RGBAColor::WHITE);
    let moving = CursorState {
        velocity: NormalizedPoint::new(3.0, 0.0),
        ..CursorState::default()
    };
    let (blurred, _, _) = cursor_scene(&mut r, moving.clone(), 1.0, RGBAColor::WHITE);

    let (sharp_dark, _) = extremes(&sharp, tip, unit, 12.0);
    let (blurred_dark, _) = extremes(&blurred, tip, unit, 12.0);
    assert!(
        blurred_dark.luma() > sharp_dark.luma() + 40,
        "smearing did not lighten the body: {} -> {}",
        sharp_dark.luma(),
        blurred_dark.luma()
    );

    // A pixel 50 px ahead of the arrow is white when sharp and smeared when blurred.
    let ahead = (
        (tip.0 + 3.0 * unit + 50.0) as u32,
        (tip.1 + 8.0 * unit) as u32,
    );
    assert!(pixel(&sharp, ahead.0, ahead.1).luma() >= 253);
    assert!(
        pixel(&blurred, ahead.0, ahead.1).luma() < 245,
        "ahead {}",
        pixel(&blurred, ahead.0, ahead.1)
    );

    // motionBlur 0 means no blur even when moving.
    let (no_blur, _, _) = cursor_scene(&mut r, moving, 0.0, RGBAColor::WHITE);
    assert_eq!(bytes(&no_blur), bytes(&sharp));
}

/// Which shape each case draws and where the camera must (not) show.
struct ShapeCase {
    name: &'static str,
    shape: CameraShape,
    roundness: f64,
    inside: (bool, bool),
}

#[test]
fn bubble_is_masked_to_its_shape() {
    let mut r = need_gpu!();
    let cases = [
        ShapeCase {
            name: "circle",
            shape: CameraShape::Circle,
            roundness: 0.5,
            inside: (false, false),
        },
        ShapeCase {
            name: "squircle",
            shape: CameraShape::Square,
            roundness: 0.6,
            inside: (true, false),
        },
        ShapeCase {
            name: "roundedRect",
            shape: CameraShape::Square,
            roundness: 0.5,
            inside: (true, false),
        },
        ShapeCase {
            name: "rectangle",
            shape: CameraShape::Wide,
            roundness: 0.0,
            inside: (true, true),
        },
    ];
    let screen = filled(1920, 1080, GREEN);
    let cam = filled(640, 480, BLUE);
    for case in cases {
        let mut style = plain_style(0.0, 0.0);
        style.camera = CameraBubble {
            visible: true,
            shape: case.shape,
            position: NormalizedPoint::CENTER,
            size: 0.3,
            border: None,
            mirror: false,
            roundness: case.roundness,
            ..CameraBubble::default()
        };
        let out = render(
            &mut r,
            &screen,
            Some(&cam),
            &hidden_cursor(),
            &style,
            PixelSize::new(W, H),
        );
        let l = layout_for(&style, (W, H), CameraState::IDENTITY);
        let rc = l.camera_bubble_rect(&style.camera);
        let at = |f: f64| {
            pixel(
                &out,
                (rc.mid_x() + f * rc.width() / 2.0) as u32,
                (rc.mid_y() + f * rc.height() / 2.0) as u32,
            )
        };
        let blue = Rgb { r: 0, g: 0, b: 255 };
        let green = Rgb { r: 0, g: 255, b: 0 };
        assert!(at(0.0).near(blue, 1), "{} centre {}", case.name, at(0.0));
        assert!(at(0.5).near(blue, 1), "{} at 0.5 {}", case.name, at(0.5));
        let want_mid = if case.inside.0 { blue } else { green };
        let want_corner = if case.inside.1 { blue } else { green };
        assert!(
            at(0.8).near(want_mid, 1),
            "{} at 0.8 = {}",
            case.name,
            at(0.8)
        );
        assert!(
            at(0.97).near(want_corner, 1),
            "{} at 0.97 = {}",
            case.name,
            at(0.97)
        );
    }
}

#[test]
fn bubble_border_and_mirror() {
    let mut r = need_gpu!();
    let screen = filled(1920, 1080, GREEN);
    let cam = make(640, 480, |x, _| if x < 320 { BLUE } else { RED });
    let mut style = plain_style(0.0, 0.0);
    style.camera = CameraBubble {
        visible: true,
        shape: CameraShape::Circle,
        position: NormalizedPoint::CENTER,
        size: 0.3,
        border: Some(CameraBorder {
            width: 0.01,
            color: RGBAColor::WHITE,
        }),
        mirror: false,
        ..CameraBubble::default()
    };
    let size = PixelSize::new(W, H);
    let out = render(&mut r, &screen, Some(&cam), &hidden_cursor(), &style, size);
    let l = layout_for(&style, (W, H), CameraState::IDENTITY);
    let rc = l.camera_bubble_rect(&style.camera);
    let left = (rc.mid_x() - rc.width() / 4.0, rc.mid_y());
    let p = pixel(&out, left.0 as u32, left.1 as u32);
    assert!(p.near(Rgb { r: 0, g: 0, b: 255 }, 1), "left of bubble {p}");
    // Border: 0.01 * 1080 = 10.8 px inside the edge.
    let border = pixel(&out, rc.mid_x() as u32, rc.min_y() as u32 + 5);
    assert!(
        border.near(
            Rgb {
                r: 255,
                g: 255,
                b: 255
            },
            2
        ),
        "border {border}"
    );

    style.camera.mirror = true;
    let out = render(&mut r, &screen, Some(&cam), &hidden_cursor(), &style, size);
    let p = pixel(&out, left.0 as u32, left.1 as u32);
    assert!(p.near(Rgb { r: 255, g: 0, b: 0 }, 1), "mirrored {p}");

    style.camera.visible = false;
    let out = render(&mut r, &screen, Some(&cam), &hidden_cursor(), &style, size);
    let p = pixel(&out, left.0 as u32, left.1 as u32);
    assert!(p.near(Rgb { r: 0, g: 255, b: 0 }, 1), "hidden bubble {p}");
}

#[test]
fn linear_gradient_runs_along_its_angle() {
    let mut r = need_gpu!();
    let size = PixelSize::new(W, H);
    let mut style = plain_style(0.2, 0.0);
    style.background = Background::Gradient {
        stops: vec![GradientStop::new(0.0, RED), GradientStop::new(1.0, BLUE)],
        angle_degrees: 0.0,
    };
    let screen = filled(64, 36, GREEN);
    let out = render(&mut r, &screen, None, &hidden_cursor(), &style, size);
    let left = pixel(&out, 1, H / 2);
    let right = pixel(&out, W - 2, H / 2);
    assert!(left.r > 240 && left.b < 20, "left {left}");
    assert!(right.b > 240 && right.r < 20, "right {right}");

    // Vertical gradient: top and bottom differ, left and right of a row agree.
    style.background = Background::Gradient {
        stops: vec![
            GradientStop::new(0.0, RGBAColor::WHITE),
            GradientStop::new(1.0, RGBAColor::BLACK),
        ],
        angle_degrees: 90.0,
    };
    let out = render(&mut r, &screen, None, &hidden_cursor(), &style, size);
    assert!(pixel(&out, 5, 1).luma() > 240, "top");
    assert!(pixel(&out, 5, H - 2).luma() < 15, "bottom");
    let a = pixel(&out, 5, 30).luma();
    let b = pixel(&out, W - 5, 30).luma();
    assert!((a - b).abs() <= 2, "row is not flat: {a} vs {b}");
}

#[test]
fn wallpapers_render() {
    let mut r = need_gpu!();
    for name in nss_core::background_presets::LEGACY_WALLPAPER_NAMES {
        let mut style = plain_style(0.2, 0.0);
        style.background = Background::Wallpaper {
            name: (*name).to_owned(),
        };
        let out = render(
            &mut r,
            &filled(64, 36, GREEN),
            None,
            &hidden_cursor(),
            &style,
            PixelSize::new(960, 540),
        );
        assert_ne!(pixel(&out, 1, 1), pixel(&out, 958, 538), "{name} is flat");
        let centre = pixel(&out, 480, 270);
        assert!(
            centre.near(Rgb { r: 0, g: 255, b: 0 }, 1),
            "{name} centre {centre}"
        );
    }
}

#[test]
fn image_background_is_loaded_and_aspect_filled() {
    let mut r = need_gpu!();
    let dir = ScratchDir::new("image-fill");
    // 200x100: left half yellow, right half cyan. Aspect-filled into 1:1 the centre column stays.
    let url = dir.join("bg.png");
    write_png(&url, 200, 100, |x, _| {
        if x < 100 {
            (255, 255, 0)
        } else {
            (0, 255, 255)
        }
    });
    let mut style = plain_style(0.3, 0.0);
    style.background = Background::Image {
        file: "bg.png".to_owned(),
    };
    r.set_asset_base_url(Some(dir.0.clone()));
    let size = PixelSize::new(600, 600);
    let out = render(
        &mut r,
        &filled(64, 36, GREEN),
        None,
        &hidden_cursor(),
        &style,
        size,
    );
    let left = pixel(&out, 2, 300);
    let right = pixel(&out, 597, 300);
    assert!(
        left.near(
            Rgb {
                r: 255,
                g: 255,
                b: 0
            },
            2
        ),
        "left {left}"
    );
    assert!(
        right.near(
            Rgb {
                r: 0,
                g: 255,
                b: 255
            },
            2
        ),
        "right {right}"
    );

    // A missing file falls back to grey instead of failing.
    style.background = Background::Image {
        file: "nope.png".to_owned(),
    };
    let out = render(
        &mut r,
        &filled(64, 36, GREEN),
        None,
        &hidden_cursor(),
        &style,
        size,
    );
    let p = pixel(&out, 2, 2);
    assert!(
        p.near(
            Rgb {
                r: 128,
                g: 128,
                b: 128
            },
            3
        ),
        "missing image {p}"
    );
}

#[test]
fn image_background_reloads_when_the_file_changes_or_appears() {
    let mut r = need_gpu!();
    let dir = ScratchDir::new("image-reload");
    let url = dir.join("bg.png");
    let size = PixelSize::new(320, 180);
    let mut style = plain_style(0.3, 0.0);
    style.background = Background::Image {
        file: "bg.png".to_owned(),
    };
    r.set_asset_base_url(Some(dir.0.clone()));
    let screen = filled(64, 36, GREEN);

    // Rendered before the file exists: grey fallback, and the miss must not stick.
    let out = render(&mut r, &screen, None, &hidden_cursor(), &style, size);
    let p = pixel(&out, 2, 2);
    assert!(
        p.near(
            Rgb {
                r: 128,
                g: 128,
                b: 128
            },
            3
        ),
        "before the file exists {p}"
    );

    write_png(&url, 32, 18, |_, _| (255, 255, 0));
    let out = render(&mut r, &screen, None, &hidden_cursor(), &style, size);
    let p = pixel(&out, 2, 2);
    assert!(
        p.near(
            Rgb {
                r: 255,
                g: 255,
                b: 0
            },
            2
        ),
        "after it appears {p}"
    );

    // Rewritten with new pixels and a different size, so the stamp changes even on a filesystem
    // with coarse timestamps.
    write_png(&url, 64, 36, |_, _| (0, 255, 255));
    let out = render(&mut r, &screen, None, &hidden_cursor(), &style, size);
    let p = pixel(&out, 2, 2);
    assert!(
        p.near(
            Rgb {
                r: 0,
                g: 255,
                b: 255
            },
            2
        ),
        "after a rewrite {p}"
    );

    // Same size, explicit invalidation.
    write_png(&url, 64, 36, |_, _| (255, 0, 255));
    r.invalidate_background_image();
    let out = render(&mut r, &screen, None, &hidden_cursor(), &style, size);
    let p = pixel(&out, 2, 2);
    assert!(
        p.near(
            Rgb {
                r: 255,
                g: 0,
                b: 255
            },
            2
        ),
        "after invalidate {p}"
    );
}

#[test]
fn render_is_deterministic() {
    let mut a = need_gpu!();
    let screen = make(1280, 800, |x, y| {
        RGBAColor::new(
            f64::from(x % 256) / 255.0,
            f64::from(y % 256) / 255.0,
            0.4,
            1.0,
        )
    });
    let cam = filled(640, 480, BLUE);
    let mut state = hidden_cursor();
    state.camera = CameraState::new(1.7, NormalizedPoint::new(0.4, 0.6));
    state.cursor = CursorState {
        visible: true,
        position: NormalizedPoint::new(0.45, 0.55),
        velocity: NormalizedPoint::new(0.8, -0.3),
        ripples: vec![RippleState {
            position: NormalizedPoint::new(0.42, 0.58),
            progress: 0.4,
        }],
        ..CursorState::default()
    };
    let size = PixelSize::new(1280, 720);
    let style = Style::default();
    let first = render(&mut a, &screen, Some(&cam), &state, &style, size);
    let second = render(&mut a, &screen, Some(&cam), &state, &style, size);
    assert_eq!(bytes(&first), bytes(&second), "the same renderer drifted");

    // A second renderer (preview vs export) must produce the same bytes.
    if let Ok(mut b) = Renderer::new() {
        let other = render(&mut b, &screen, Some(&cam), &state, &style, size);
        assert_eq!(bytes(&first), bytes(&other), "two renderers disagree");
    }
}

#[test]
fn benchmark_4k_frame() {
    let mut r = need_gpu!();
    let screen = make(3840, 2160, |x, y| {
        RGBAColor::new(
            f64::from(x & 255) / 255.0,
            f64::from(y & 255) / 255.0,
            0.5,
            1.0,
        )
    });
    let cam = filled(1920, 1080, BLUE);
    let size = PixelSize::new(3840, 2160);
    let style = Style::default();
    let mut state = hidden_cursor();
    state.camera = CameraState::new(1.6, NormalizedPoint::new(0.4, 0.4));
    state.cursor = CursorState {
        visible: true,
        position: NormalizedPoint::new(0.42, 0.41),
        velocity: NormalizedPoint::new(1.5, 0.6),
        ripples: (0..3)
            .map(|i| RippleState {
                position: NormalizedPoint::new(0.4 + 0.02 * f64::from(i), 0.4),
                progress: 0.2 * f64::from(i + 1),
            })
            .collect(),
        ..CursorState::default()
    };
    for _ in 0..5 {
        let _ = render(&mut r, &screen, Some(&cam), &state, &style, size);
    }
    let mut times = Vec::new();
    for _ in 0..30 {
        let t0 = std::time::Instant::now();
        let _ = render(&mut r, &screen, Some(&cam), &state, &style, size);
        times.push(t0.elapsed().as_secs_f64() * 1000.0);
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let best = times[0];
    let median = times[times.len() / 2];
    println!(
        "Renderer 4K frame: best {best:.2} ms, median {median:.2} ms on {}",
        r.adapter_name()
    );
    // Wall-clock budgets are flaky on loaded machines, so only assert when asked to
    // (`NSS_BENCH=1 cargo test --release --test metal_renderer benchmark_4k_frame -- --nocapture`).
    if std::env::var("NSS_BENCH").as_deref() == Ok("1") {
        assert!(
            median < 8.0,
            "median 4K frame {median:.2} ms over the 8 ms budget"
        );
    }
}
