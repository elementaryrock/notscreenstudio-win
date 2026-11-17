//! wgpu single-pass compositor, render layout, cursor atlas and background rasterizer.
//!
//! Replaces upstream NSSRender. See `PLAN.md` §5 Stage 4 and Appendix A.

#![forbid(unsafe_code)]

pub mod cursor_atlas;
pub mod frame_pool;
pub mod image_blur;
pub mod layout;
pub mod passthrough;
pub mod rasterizer;
pub mod renderer;
pub mod shaders;
