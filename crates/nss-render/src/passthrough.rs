//! CPU fallback for machines without a usable adapter: clears the frame to the style's background
//! colour.
//!
//! Port of upstream `NSSRender/PassthroughRenderer.swift`. It draws no screen, cursor or camera, so
//! it is only ever used by the shell when `Renderer::new()` reports no GPU (upstream does the same
//! with `PassthroughRenderer()` on Macs without Metal).

use nss_core::{FrameState, Style};

use crate::frame::{Frame, Image};
use crate::renderer::{FrameRenderer, RenderError};

/// Fills every pixel with `style`'s fallback background colour.
#[derive(Debug, Default, Clone, Copy)]
pub struct PassthroughRenderer;

impl PassthroughRenderer {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl FrameRenderer for PassthroughRenderer {
    fn render(
        &mut self,
        _screen: &Image,
        _camera: Option<&Image>,
        _state: &FrameState,
        style: &Style,
        output_size: nss_core::PixelSize,
    ) -> Result<Frame, RenderError> {
        let c = style.background.fallback_color();
        Ok(Image::filled(
            output_size.width.max(1),
            output_size.height.max(1),
            c,
        ))
    }

    fn name(&self) -> &'static str {
        "passthrough"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nss_core::{Background, FrameState, PixelSize, RGBAColor};

    #[test]
    fn passthrough_clears_to_the_background_colour() {
        let screen = Image::filled(16, 16, RGBAColor::WHITE);
        let style = Style {
            background: Background::Solid(RGBAColor::new(1.0, 0.0, 0.0, 1.0)),
            ..Style::default()
        };
        let out = PassthroughRenderer::new()
            .render(
                &screen,
                None,
                &FrameState::new(0.0),
                &style,
                PixelSize::new(32, 18),
            )
            .unwrap();
        assert_eq!((out.width, out.height), (32, 18));
        assert_eq!(out.pixel(0, 0), [255, 0, 0, 255]);
        assert_eq!(out.pixel(31, 17), [255, 0, 0, 255]);
    }

    #[test]
    fn passthrough_uses_a_gradient_s_first_stop() {
        let screen = Image::filled(4, 4, RGBAColor::WHITE);
        let style = Style {
            background: Background::Gradient {
                stops: vec![
                    nss_core::GradientStop::new(0.0, RGBAColor::new(0.0, 0.0, 1.0, 1.0)),
                    nss_core::GradientStop::new(1.0, RGBAColor::WHITE),
                ],
                angle_degrees: 0.0,
            },
            ..Style::default()
        };
        let out = PassthroughRenderer::new()
            .render(
                &screen,
                None,
                &FrameState::new(0.0),
                &style,
                PixelSize::new(4, 4),
            )
            .unwrap();
        assert_eq!(out.pixel(1, 1), [0, 0, 255, 255]);
    }
}
