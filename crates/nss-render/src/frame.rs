//! Frames: the pixel containers that cross this crate's public API.
//!
//! Upstream trafficks in `CVPixelBuffer`s (IOSurface-backed 32BGRA) because Metal can wrap them
//! zero-copy. wgpu has no such requirement, so this port uses the simplest thing that can cross a
//! trait boundary and be written by any backend: tightly packed RGBA8 in **sRGB** encoding, exactly
//! what a `bgra8Unorm_srgb` view held upstream. ADR 0011 records the reasoning and the cost (one
//! upload per frame instead of a zero-copy bind).
//!
//! `Image` is an input (a decoded screen frame), `Frame` is the output. They have the same layout;
//! they are separate types so a caller cannot accidentally feed an output back in as a source.

use nss_core::{MediaTime, NormalizedPoint, PixelSize, RGBAColor};
use thiserror::Error;

/// Byte/Pixel format of an [`Image`]: 8 bits per channel, RGBA order, sRGB encoded, no row padding.
pub const BYTES_PER_PIXEL: usize = 4;

#[derive(Debug, Error)]
pub enum FrameError {
    #[error("image buffer is {actual} bytes, expected {expected} for {width}x{height}")]
    WrongLength {
        width: u32,
        height: u32,
        expected: usize,
        actual: usize,
    },
    #[error("image size {width}x{height} is not usable (both sides must be at least 1)")]
    Empty { width: u32, height: u32 },
}

/// A source image: RGBA8, sRGB, rows tightly packed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Image {
    /// Wraps existing pixels, checking the length.
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Result<Self, FrameError> {
        if width == 0 || height == 0 {
            return Err(FrameError::Empty { width, height });
        }
        let expected = width as usize * height as usize * BYTES_PER_PIXEL;
        if pixels.len() != expected {
            return Err(FrameError::WrongLength {
                width,
                height,
                expected,
                actual: pixels.len(),
            });
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    /// A solid sRGB colour, the shape most tests want.
    #[must_use]
    pub fn filled(width: u32, height: u32, color: RGBAColor) -> Self {
        let w = width.max(1);
        let h = height.max(1);
        let byte = |v: f64| ((v.clamp(0.0, 1.0) * 255.0) + 0.5) as u8;
        let px = [byte(color.r), byte(color.g), byte(color.b), byte(color.a)];
        let mut pixels = Vec::with_capacity(w as usize * h as usize * BYTES_PER_PIXEL);
        for _ in 0..(w as usize * h as usize) {
            pixels.extend_from_slice(&px);
        }
        Self {
            width: w,
            height: h,
            pixels,
        }
    }

    /// Builds an image from a per-pixel function (test and fixture helper).
    #[must_use]
    pub fn from_fn(width: u32, height: u32, f: impl Fn(u32, u32) -> RGBAColor) -> Self {
        let w = width.max(1);
        let h = height.max(1);
        let byte = |v: f64| ((v.clamp(0.0, 1.0) * 255.0) + 0.5) as u8;
        let mut pixels = Vec::with_capacity(w as usize * h as usize * BYTES_PER_PIXEL);
        for y in 0..h {
            for x in 0..w {
                let c = f(x, y);
                pixels.push(byte(c.r));
                pixels.push(byte(c.g));
                pixels.push(byte(c.b));
                pixels.push(byte(c.a));
            }
        }
        Self {
            width: w,
            height: h,
            pixels,
        }
    }

    #[must_use]
    pub fn size(&self) -> PixelSize {
        PixelSize::new(self.width, self.height)
    }

    #[must_use]
    pub fn aspect(&self) -> f64 {
        f64::from(self.width) / f64::from(self.height.max(1))
    }

    /// The RGBA of one pixel (`0` when out of bounds).
    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        if x >= self.width || y >= self.height {
            return [0; 4];
        }
        let i = ((y * self.width + x) as usize) * BYTES_PER_PIXEL;
        [
            self.pixels[i],
            self.pixels[i + 1],
            self.pixels[i + 2],
            self.pixels[i + 3],
        ]
    }
}

/// The rendered output: RGBA8, sRGB encoded, rows tightly packed.
pub type Frame = Image;

impl Image {
    /// Reordered to BGRA, the layout FFmpeg's `bgra` rawvideo input and the Windows compositor want.
    #[must_use]
    pub fn to_bgra(&self) -> Vec<u8> {
        let mut out = self.pixels.clone();
        for px in out.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        out
    }

    /// Rows as BGRA without reallocating: useful for a per-row encoder feed.
    pub fn write_bgra_rows(&self, out: &mut Vec<u8>) {
        out.clear();
        out.reserve(self.pixels.len());
        for px in self.pixels.chunks_exact(4) {
            out.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
        }
    }
}

/// Where a frame comes from: the recording, a decoder or a test double.
///
/// This is the trait ADR 0009 moved out of `nss-core` (upstream's version trafficks in
/// `CVPixelBuffer`). It is synchronous: callers own their own thread if they want pipelining, which
/// is what the exporter does.
pub trait FrameSource: Send {
    /// The screen frame at `t`.
    fn screen_frame(&mut self, t: MediaTime) -> Result<Option<Image>, FrameError>;

    /// The camera frame at `t`, when the recording has a camera track.
    fn camera_frame(&mut self, _t: MediaTime) -> Result<Option<Image>, FrameError> {
        Ok(None)
    }
}

/// A `FrameSource` that returns one solid-colour frame for every `t` (and for the camera when
/// `has_camera`). Useful for tests and as a placeholder until the real decoders land.
#[derive(Debug, Clone)]
pub struct SolidColorFrameSource {
    screen: Image,
    camera: Option<Image>,
}

impl SolidColorFrameSource {
    pub fn new(width: u32, height: u32, color: RGBAColor, has_camera: bool) -> Self {
        let screen = Image::filled(width, height, color);
        let camera = if has_camera {
            Some(Image::filled(width, height, color))
        } else {
            None
        };
        Self { screen, camera }
    }
}

impl FrameSource for SolidColorFrameSource {
    fn screen_frame(&mut self, _t: MediaTime) -> Result<Option<Image>, FrameError> {
        Ok(Some(self.screen.clone()))
    }

    fn camera_frame(&mut self, _t: MediaTime) -> Result<Option<Image>, FrameError> {
        Ok(self.camera.clone())
    }
}

/// A `FrameSource` that returns nothing: the export path uses it when a project has no recording.
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptyFrameSource;

impl FrameSource for EmptyFrameSource {
    fn screen_frame(&mut self, _t: MediaTime) -> Result<Option<Image>, FrameError> {
        Ok(None)
    }
}

/// Convenience for callers that have a normalized point and want a pixel: rounds to the nearest
/// pixel inside the image.
#[must_use]
pub fn point_to_pixel(p: NormalizedPoint, width: u32, height: u32) -> (u32, u32) {
    let x = (p.x * f64::from(width.max(1))).round();
    let y = (p.y * f64::from(height.max(1))).round();
    (
        x.clamp(0.0, f64::from(width.saturating_sub(1))) as u32,
        y.clamp(0.0, f64::from(height.saturating_sub(1))) as u32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_image_checks_its_buffer_length() {
        assert!(Image::new(2, 2, vec![0; 16]).is_ok());
        let err = Image::new(2, 2, vec![0; 15]).unwrap_err();
        assert!(err.to_string().contains("15"), "{err}");
        assert!(Image::new(0, 4, vec![]).is_err());
    }

    #[test]
    fn a_filled_image_is_its_colour() {
        let img = Image::filled(3, 2, RGBAColor::new(1.0, 0.0, 0.0, 1.0));
        assert_eq!(img.pixel(0, 0), [255, 0, 0, 255]);
        assert_eq!(img.pixel(2, 1), [255, 0, 0, 255]);
        assert_eq!(img.pixels.len(), 3 * 2 * 4);
    }

    #[test]
    fn bgra_reorders_the_channels_and_not_the_alpha() {
        let img = Image::filled(1, 1, RGBAColor::new(1.0, 0.5, 0.0, 0.25));
        let bgra = img.to_bgra();
        assert_eq!(bgra, vec![0, 128, 255, 64]);
        let mut rows = Vec::new();
        img.write_bgra_rows(&mut rows);
        assert_eq!(rows, bgra);
    }

    #[test]
    fn the_solid_source_returns_frames_for_every_time() {
        let mut src = SolidColorFrameSource::new(4, 4, RGBAColor::new(0.0, 1.0, 0.0, 1.0), false);
        let f = src.screen_frame(3.2).unwrap().unwrap();
        assert_eq!((f.width, f.height), (4, 4));
        assert_eq!(f.pixel(0, 0), [0, 255, 0, 255]);
        assert!(src.camera_frame(0.0).unwrap().is_none());
        let mut with_cam = SolidColorFrameSource::new(2, 2, RGBAColor::WHITE, true);
        assert!(with_cam.camera_frame(0.0).unwrap().is_some());
    }

    #[test]
    fn points_map_to_pixels_inside_the_image() {
        assert_eq!(
            point_to_pixel(NormalizedPoint::new(0.0, 0.0), 10, 10),
            (0, 0)
        );
        assert_eq!(
            point_to_pixel(NormalizedPoint::new(1.0, 1.0), 10, 10),
            (9, 9)
        );
        assert_eq!(
            point_to_pixel(NormalizedPoint::new(0.5, 0.5), 10, 10),
            (5, 5)
        );
    }
}
