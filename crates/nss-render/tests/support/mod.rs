//! Shared helpers for the render tests: a pixel readback value, buffer builders, a plain style and a
//! renderer that skips the test when the machine has no usable GPU adapter.
//!
//! Port of upstream `Tests/NSSRenderTests/TestSupport.swift` (`RGB`, `TB`, `GPU`).

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use nss_core::{CursorState, FrameState, PixelSize, RGBAColor, Style};
use nss_render::frame::Image;
use nss_render::renderer::{FrameRenderer, Renderer};

pub const RED: RGBAColor = RGBAColor::new(1.0, 0.0, 0.0, 1.0);
pub const GREEN: RGBAColor = RGBAColor::new(0.0, 1.0, 0.0, 1.0);
pub const BLUE: RGBAColor = RGBAColor::new(0.0, 0.0, 1.0, 1.0);

/// An 8-bit RGB triple, the unit every assertion in these tests speaks in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: i32,
    pub g: i32,
    pub b: i32,
}

impl Rgb {
    pub fn near(self, other: Rgb, tol: i32) -> bool {
        (self.r - other.r).abs() <= tol
            && (self.g - other.g).abs() <= tol
            && (self.b - other.b).abs() <= tol
    }

    pub fn luma(self) -> i32 {
        (self.r + self.g + self.b) / 3
    }
}

impl std::fmt::Display for Rgb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "({}, {}, {})", self.r, self.g, self.b)
    }
}

/// A solid-colour source image.
pub fn filled(w: u32, h: u32, c: RGBAColor) -> Image {
    Image::filled(w, h, c)
}

/// A source image from a per-pixel function.
pub fn make(w: u32, h: u32, f: impl Fn(u32, u32) -> RGBAColor) -> Image {
    Image::from_fn(w, h, f)
}

/// One pixel of a frame as 8-bit RGB.
pub fn pixel(img: &Image, x: u32, y: u32) -> Rgb {
    let p = img.pixel(x, y);
    Rgb {
        r: i32::from(p[0]),
        g: i32::from(p[1]),
        b: i32::from(p[2]),
    }
}

/// The whole frame, RGBA, for byte comparisons.
pub fn bytes(img: &Image) -> Vec<u8> {
    img.pixels.clone()
}

/// Upstream's `TB.plainStyle`: solid red background, no shadow, no camera bubble, no ripples.
pub fn plain_style(padding: f64, corner: f64) -> Style {
    let mut s = Style {
        background: nss_core::Background::Solid(RED),
        padding,
        corner_radius: corner,
        ..Style::default()
    };
    s.shadow.opacity = 0.0;
    s.camera.visible = false;
    s.camera.border = None;
    s
}

/// A frame state with the cursor hidden, so a test only sees what it is testing.
pub fn hidden_cursor() -> FrameState {
    let mut s = FrameState::new(0.0);
    s.cursor = CursorState {
        visible: false,
        ..CursorState::default()
    };
    s
}

/// A renderer, or `None` when this machine has no adapter: the GPU suites skip in that case (the
/// same shape as upstream's `@Suite(.enabled(if: GPU.available))`).
pub fn renderer() -> Option<Renderer> {
    match Renderer::new() {
        Ok(r) => Some(r),
        Err(e) => {
            eprintln!("skipping: no GPU adapter ({e})");
            None
        }
    }
}

/// `renderer()` and an early return, so one line enables a test.
#[macro_export]
macro_rules! need_gpu {
    () => {
        match $crate::support::renderer() {
            Some(r) => r,
            None => return,
        }
    };
}

/// A unique scratch directory for a test, removed by [`ScratchDir`]'s `Drop`.
pub struct ScratchDir(pub PathBuf);

impl ScratchDir {
    pub fn new(tag: &str) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "nss-render-{tag}-{}-{}-{}",
            std::process::id(),
            nanos,
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        Self(dir)
    }

    pub fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Writes an RGBA PNG, the shape the image-background tests need.
pub fn write_png(path: &Path, width: u32, height: u32, f: impl Fn(u32, u32) -> (u8, u8, u8)) {
    let mut buf = image::RgbaImage::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let (r, g, b) = f(x, y);
            buf.put_pixel(x, y, image::Rgba([r, g, b, 255]));
        }
    }
    buf.save(path).expect("write png");
}

/// Renders `screen` into an image of `size` through `r`.
pub fn render(
    r: &mut Renderer,
    screen: &Image,
    camera: Option<&Image>,
    state: &FrameState,
    style: &Style,
    size: PixelSize,
) -> Image {
    r.render(screen, camera, state, style, size)
        .expect("render failed")
}
