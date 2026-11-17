//! Background parity: the GPU must draw every built-in background exactly like the CPU rasterizer
//! (which is what the picker's thumbnails use), plus image loading, aspect fill and blur.
//!
//! Port of upstream `Tests/NSSRenderTests/BackgroundRenderTests.swift`.

mod support;

use nss_core::background_presets;
use nss_core::{Background, BackgroundBlur, MeshPoint, PixelSize, RGBAColor, Style};
use nss_render::rasterizer::BackgroundRasterizer;

use support::*;

const W: u32 = 320;
const H: u32 = 180;

fn render_bg(
    r: &mut nss_render::renderer::Renderer,
    style: &Style,
    size: (u32, u32),
) -> nss_render::Image {
    render(
        r,
        &filled(64, 36, GREEN),
        None,
        &hidden_cursor(),
        style,
        PixelSize::new(size.0, size.1),
    )
}

fn bg_style(background: Background, blur: f64) -> Style {
    let mut s = plain_style(0.3, 0.0);
    s.background = background;
    s.background_blur = blur;
    s
}

#[test]
fn gpu_matches_cpu_rasterizer() {
    let mut r = need_gpu!();
    for preset in background_presets::gradients() {
        let style = bg_style(preset.background.clone(), 0.0);
        let out = render_bg(&mut r, &style, (W, H));
        let reference = BackgroundRasterizer::rgba(&preset.background, W as usize, H as usize);
        let mut worst = 0_i32;
        // Rows inside the top padding band (the screen covers the middle).
        for y in (0..40).step_by(3) {
            for x in (0..W).step_by(7) {
                let g = pixel(&out, x, y);
                let i = ((y * W + x) * 4) as usize;
                worst = worst
                    .max((g.r - i32::from(reference[i])).abs())
                    .max((g.g - i32::from(reference[i + 1])).abs())
                    .max((g.b - i32::from(reference[i + 2])).abs());
            }
        }
        // Linear gradients are a few multiplies and match the CPU to the dither. Mesh gradients go
        // through `pow` and `sin` in f32 on the GPU and f64 on the CPU, so allow one more 8-bit step.
        let tolerance = if preset.id.starts_with("mesh.") { 3 } else { 2 };
        assert!(
            worst <= tolerance,
            "{}: GPU and CPU differ by {worst}",
            preset.id
        );
    }
}

#[test]
fn mesh_is_smooth_and_uses_its_colours() {
    let mut r = need_gpu!();
    let bg = Background::Mesh {
        points: vec![
            MeshPoint::new(0.0, 0.0, RED),
            MeshPoint::new(1.0, 0.0, BLUE),
        ],
        warp: 0.0,
    };
    let out = render_bg(&mut r, &bg_style(bg, 0.0), (W, H));
    let left = pixel(&out, 1, 1);
    assert!(left.r > 200 && left.b < 80, "left corner {left}");
    let right = pixel(&out, W - 2, 1);
    assert!(right.b > 200 && right.r < 80, "right corner {right}");
    // No hard steps along the top row.
    let mut max_step = 0;
    for x in 1..W {
        max_step = max_step.max((pixel(&out, x, 2).luma() - pixel(&out, x - 1, 2).luma()).abs());
    }
    assert!(max_step <= 6, "step of {max_step} in the mesh");
}

#[test]
fn legacy_wallpaper_draws_like_its_migrated_mesh() {
    let mut r = need_gpu!();
    for name in background_presets::LEGACY_WALLPAPER_NAMES {
        let legacy = render_bg(
            &mut r,
            &bg_style(
                Background::Wallpaper {
                    name: (*name).to_owned(),
                },
                0.0,
            ),
            (W, H),
        );
        let mesh = render_bg(
            &mut r,
            &bg_style(background_presets::migrated_wallpaper(name), 0.0),
            (W, H),
        );
        assert_eq!(bytes(&legacy), bytes(&mesh), "{name}");
    }
}

/// Luma spread along the top band: large for a sharp checkerboard, small when blurred.
fn spread(img: &nss_render::Image) -> i32 {
    let mut lo = 255;
    let mut hi = 0;
    for x in 20..100 {
        let l = pixel(img, x, 10).luma();
        lo = lo.min(l);
        hi = hi.max(l);
    }
    hi - lo
}

fn with_image(tag: &str, mut body: impl FnMut(&ScratchDir, &std::path::Path)) {
    let dir = ScratchDir::new(tag);
    let url = dir.join("checker.png");
    // 512x512 checkerboard of 8 px cells: left half yellow/black, right half cyan/black.
    write_png(&url, 512, 512, |x, y| {
        if (x / 8 + y / 8) % 2 != 0 {
            return (0, 0, 0);
        }
        if x < 256 {
            (255, 255, 0)
        } else {
            (0, 255, 255)
        }
    });
    body(&dir, &url);
}

#[test]
fn blur_smooths_images_and_zero_is_sharp() {
    let mut r = need_gpu!();
    with_image("blur", |dir, url| {
        r.set_asset_base_url(Some(dir.0.clone()));
        let image = Background::Image {
            file: url.file_name().unwrap().to_string_lossy().into_owned(),
        };
        let sharp = render_bg(&mut r, &bg_style(image.clone(), 0.0), (W, H));
        let zero = render_bg(&mut r, &bg_style(image.clone(), 0.004), (W, H));
        let blurred = render_bg(&mut r, &bg_style(image, 0.6), (W, H));
        assert_eq!(bytes(&zero), bytes(&sharp), "a blur below 1/100 is no blur");
        assert!(spread(&sharp) > 100, "the sharp checkerboard is flat");
        assert!(spread(&blurred) < 25, "blurred spread {}", spread(&blurred));
        // Going back to sharp after a blur uses the original texture again.
        let again = render_bg(
            &mut r,
            &bg_style(
                Background::Image {
                    file: url.file_name().unwrap().to_string_lossy().into_owned(),
                },
                0.0,
            ),
            (W, H),
        );
        assert_eq!(bytes(&again), bytes(&sharp));
    });
}

#[test]
fn blur_is_deterministic_across_renderers() {
    let Some(mut a) = renderer() else {
        return;
    };
    let Some(mut b) = renderer() else {
        return;
    };
    with_image("blur-determinism", |dir, url| {
        let image = Background::Image {
            file: url.file_name().unwrap().to_string_lossy().into_owned(),
        };
        a.set_asset_base_url(Some(dir.0.clone()));
        b.set_asset_base_url(Some(dir.0.clone()));
        for amount in [0.15, 0.5, 1.0] {
            let style = bg_style(image.clone(), amount);
            let first = render_bg(&mut a, &style, (W, H));
            let second = render_bg(&mut b, &style, (W, H));
            assert_eq!(bytes(&first), bytes(&second), "blur {amount}");
        }
    });
}

#[test]
fn blur_looks_the_same_at_every_output_size() {
    let mut r = need_gpu!();
    let dir = ScratchDir::new("blur-sizes");
    let url = dir.join("halves.png");
    write_png(&url, 640, 360, |x, _| {
        if x < 320 {
            (255, 40, 40)
        } else {
            (40, 40, 255)
        }
    });
    r.set_asset_base_url(Some(dir.0.clone()));
    let style = bg_style(
        Background::Image {
            file: "halves.png".to_owned(),
        },
        0.8,
    );
    let small = render_bg(&mut r, &style, (320, 180));
    let big = render_bg(&mut r, &style, (640, 360));
    let mut worst = 0;
    for x in (100..220).step_by(4) {
        let a = pixel(&small, x, 10);
        let b = pixel(&big, 2 * x, 20);
        worst = worst.max((a.r - b.r).abs()).max((a.b - b.b).abs());
    }
    assert!(
        worst <= 8,
        "preview-size and export-size blur differ by {worst}"
    );
    // And the boundary really is softened.
    let mid = pixel(&small, 160, 10);
    assert!(mid.r > 80 && mid.b > 80, "the edge is still sharp: {mid}");
}

#[test]
fn system_wallpaper_draws_like_the_same_image() {
    let mut r = need_gpu!();
    with_image("syswallpaper", |dir, url| {
        r.set_asset_base_url(Some(dir.0.clone()));
        let path = url.to_string_lossy().into_owned();
        let wallpaper = render_bg(
            &mut r,
            &bg_style(Background::SystemWallpaper { path: path.clone() }, 0.3),
            (W, H),
        );
        let image = render_bg(
            &mut r,
            &bg_style(Background::Image { file: path.clone() }, 0.3),
            (W, H),
        );
        assert_eq!(bytes(&wallpaper), bytes(&image));

        // A wallpaper that is missing draws the grey fallback and does not fail.
        let missing = render_bg(
            &mut r,
            &bg_style(
                Background::SystemWallpaper {
                    path: "C:/nonexistent/wall.heic".to_owned(),
                },
                0.0,
            ),
            (W, H),
        );
        let p = pixel(&missing, 2, 2);
        assert!(
            p.near(
                Rgb {
                    r: 128,
                    g: 128,
                    b: 128
                },
                3
            ),
            "missing wallpaper {p}"
        );
    });
}

#[test]
fn the_blur_threshold_is_the_documented_one() {
    // The quantisation happens first: 0.004 rounds to 0, so nothing blurs.
    assert_eq!(BackgroundBlur::quantized(0.004), 0.0);
    assert!(BackgroundBlur::quantized(0.6) > 0.0);
    assert!(std::hint::black_box(RGBAColor::WHITE) == RGBAColor::WHITE);
}
