//! wgpu single-pass compositor, render layout, cursor atlas and background rasterizer.
//!
//! Replaces upstream `NSSRender` (Metal). See `PLAN.md` §5 Stage 4, `docs/ADR/0011-renderer-parity.md`
//! and `_upstream/Sources/NSSRender/RENDERING.md` for the frame model.

#![forbid(unsafe_code)]

pub mod color;
pub mod cursor_atlas;
pub mod frame;
pub mod image_blur;
pub mod layout;
pub mod passthrough;
pub mod rasterizer;
pub mod renderer;
pub mod shaders;
pub mod uniforms;

pub use cursor_atlas::{Cell, CursorAtlas};
pub use frame::{Frame, FrameError, FrameSource, Image, SolidColorFrameSource};
pub use layout::{bubble_anchor, clamped_camera, RenderLayout};
pub use passthrough::PassthroughRenderer;
pub use rasterizer::{BackgroundRasterizer, BackgroundShading};
pub use renderer::{is_supported, FrameRenderer, RenderError, Renderer};
pub use uniforms::{cursor_lod, taps, Uniforms};
