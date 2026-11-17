//! Cursor rendering on the GPU: placement at the hotspot, one image per shape, and the press squish.
//!
//! Port of the cursor half of upstream `Tests/NSSRenderTests/Round2RenderTests.swift`. The arrow art
//! is ours (Windows has no `NSCursor`), so the assertions are about geometry and darkness rather
//! than the macOS system arrow's exact pixels.

mod support;

use nss_core::{CursorShape, CursorState, NormalizedPoint, PixelSize, RGBAColor, Style};
use nss_render::layout::RenderLayout;
use nss_render::renderer::Renderer;

use support::*;

const W: u32 = 1920;
const H: u32 = 1080;

fn cursor_frame(
    r: &mut Renderer,
    cursor: CursorState,
    motion_blur: f64,
    screen: RGBAColor,
) -> (nss_render::Image, RenderLayout, Style) {
    let mut style = plain_style(0.1, 0.0);
    style.cursor.size = 2.0;
    style.cursor.motion_blur = motion_blur;
    let mut state = hidden_cursor();
    state.cursor = cursor;
    let out = render(
        r,
        &filled(1920, 1080, screen),
        None,
        &state,
        &style,
        PixelSize::new(W, H),
    );
    let l = RenderLayout::new(
        PixelSize::new(W, H),
        PixelSize::new(1920, 1080),
        &style,
        nss_core::CameraState::IDENTITY,
    );
    (out, l, style)
}

/// Bounding box `(min_x, min_y, max_x, max_y)` of pixels darker than `luma` within `radius` of `c`.
fn dark_box(
    img: &nss_render::Image,
    c: (f64, f64),
    radius: i64,
    luma: i32,
) -> Option<(i64, i64, i64, i64)> {
    let mut min = (i64::MAX, i64::MAX);
    let mut max = (i64::MIN, i64::MIN);
    for y in (c.1 as i64 - radius).max(0)..(c.1 as i64 + radius).min(i64::from(H)) {
        for x in (c.0 as i64 - radius).max(0)..(c.0 as i64 + radius).min(i64::from(W)) {
            if pixel(img, x as u32, y as u32).luma() < luma {
                min = (min.0.min(x), min.1.min(y));
                max = (max.0.max(x), max.1.max(y));
            }
        }
    }
    (min.0 != i64::MAX).then_some((min.0, min.1, max.0, max.1))
}

#[test]
fn arrow_tip_sits_on_the_cursor_position() {
    let mut r = need_gpu!();
    let pos = NormalizedPoint::new(0.4, 0.4);
    let (out, l, style) = cursor_frame(
        &mut r,
        CursorState {
            position: pos,
            ..CursorState::default()
        },
        0.4,
        RGBAColor::new(0.5, 0.5, 0.5, 1.0),
    );
    let tip = l.output_point(pos);
    let (min_x, min_y, _, _) =
        dark_box(&out, (tip.x, tip.y), 200, 60).expect("nothing dark near the hotspot");
    let ppp = l.cursor_pixels_per_point(&style.cursor);
    // The dark body starts at the hotspot, inside a white outline of about 1.5 pt.
    assert!(
        (min_x as f64 - tip.x).abs() < 2.5 * ppp && (min_y as f64 - tip.y).abs() < 2.5 * ppp,
        "dark box starts at {min_x},{min_y} but the tip is at {tip:?} (ppp {ppp})"
    );
    assert!(min_x as f64 >= tip.x - 1.0 && min_y as f64 >= tip.y - 1.0);
}

#[test]
fn every_shape_draws_its_own_image_and_unknown_is_the_arrow() {
    let mut r = need_gpu!();
    let pos = NormalizedPoint::new(0.5, 0.5);
    let grey = RGBAColor::new(0.5, 0.5, 0.5, 1.0);

    let (blank, l, _) = cursor_frame(
        &mut r,
        CursorState {
            visible: false,
            position: pos,
            ..CursorState::default()
        },
        0.4,
        grey,
    );
    let blank_bytes = bytes(&blank);
    let hot = l.output_point(pos);

    let mut images: Vec<(CursorShape, Vec<u8>)> = Vec::new();
    for shape in CursorShape::ALL {
        let (out, _, _) = cursor_frame(
            &mut r,
            CursorState {
                position: pos,
                shape,
                ..CursorState::default()
            },
            0.4,
            grey,
        );
        let drawn = bytes(&out);
        assert_ne!(drawn, blank_bytes, "{shape:?} drew nothing");

        // Some pixel within 60 px of the hotspot changed.
        let mut changed = false;
        let mut y = hot.y as i64 - 60;
        while y < hot.y as i64 + 60 && !changed {
            let mut x = hot.x as i64 - 60;
            while x < hot.x as i64 + 60 {
                if pixel(&out, x.max(0) as u32, y.max(0) as u32)
                    != pixel(&blank, x.max(0) as u32, y.max(0) as u32)
                {
                    changed = true;
                    break;
                }
                x += 2;
            }
            y += 2;
        }
        assert!(changed, "{shape:?} not drawn at its hotspot");
        images.push((shape, drawn));
    }

    let find = |shape: CursorShape| {
        images
            .iter()
            .find(|(s, _)| *s == shape)
            .map(|(_, b)| b.clone())
            .expect("every shape was rendered")
    };
    assert_eq!(find(CursorShape::Unknown), find(CursorShape::Arrow));
    let atlas = nss_render::CursorAtlas::build();
    for shape in CursorShape::ALL {
        if shape == CursorShape::Arrow || shape == CursorShape::Unknown {
            continue;
        }
        if atlas.cell_opt(shape).is_none() {
            continue;
        }
        assert_ne!(
            find(shape),
            find(CursorShape::Arrow),
            "{shape:?} looks like the arrow"
        );
    }
}

#[test]
fn press_squish_scales_around_the_hotspot() {
    let mut r = need_gpu!();
    let pos = NormalizedPoint::new(0.5, 0.5);
    for shape in [CursorShape::Arrow, CursorShape::PointingHand] {
        let (normal, l, style) = cursor_frame(
            &mut r,
            CursorState {
                position: pos,
                shape,
                ..CursorState::default()
            },
            0.4,
            RGBAColor::new(0.5, 0.5, 0.5, 1.0),
        );
        let (pressed, _, _) = cursor_frame(
            &mut r,
            CursorState {
                position: pos,
                shape,
                press_scale: 0.5,
                ..CursorState::default()
            },
            0.4,
            RGBAColor::new(0.5, 0.5, 0.5, 1.0),
        );
        let hot = l.output_point(pos);
        let a = dark_box(&normal, (hot.x, hot.y), 200, 60).expect("no normal image");
        let b = dark_box(&pressed, (hot.x, hot.y), 200, 60).expect("no pressed image");
        let (aw, ah) = (a.2 - a.0 + 1, a.3 - a.1 + 1);
        let (bw, bh) = (b.2 - b.0 + 1, b.3 - b.1 + 1);
        assert!(
            ((bw as f64 / aw as f64) - 0.5).abs() < 0.15
                && ((bh as f64 / ah as f64) - 0.5).abs() < 0.15,
            "{shape:?}: {aw}x{ah} -> {bw}x{bh}"
        );
        // Every edge moves halfway toward the hotspot.
        let ppp = l.cursor_pixels_per_point(&style.cursor);
        for (e0, e1, h) in [
            (a.0 as f64, b.0 as f64, hot.x),
            (a.2 as f64, b.2 as f64, hot.x),
            (a.1 as f64, b.1 as f64, hot.y),
            (a.3 as f64, b.3 as f64, hot.y),
        ] {
            assert!(
                ((e1 - h) - 0.5 * (e0 - h)).abs() < 1.5 * ppp,
                "{shape:?}: edge {e0} -> {e1} around {h}"
            );
        }

        let (same, _, _) = cursor_frame(
            &mut r,
            CursorState {
                position: pos,
                shape,
                press_scale: 1.0,
                ..CursorState::default()
            },
            0.4,
            RGBAColor::new(0.5, 0.5, 0.5, 1.0),
        );
        assert_eq!(
            bytes(&same),
            bytes(&normal),
            "{shape:?}: press_scale 1 changed the image"
        );
    }
}
