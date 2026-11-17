//! Shared models, conventions and contracts.
//!
//! Port of upstream `NSSCore`. See `PLAN.md` §5 Stage 1 and Appendix A.
//!
//! # Conventions (inherited verbatim from upstream `Basics.swift`)
//!
//! * All times are [`MediaTime`]: seconds (`f64`) from the first screen frame.
//! * All positions are normalised `0...1` in SOURCE space, origin **top-left**,
//!   x to the right, y downward.
//! * Motion is a pure function of time: no wall-clock state anywhere.
//!
//! This crate is platform-neutral by rule. It must never depend on `windows`,
//! `wgpu` or `tauri`.

#![forbid(unsafe_code)]

pub mod background_presets;
pub mod basics;
pub mod cursor;
pub mod frame_state;
pub mod project;
pub mod protocols;
pub mod recording;
pub mod style;
pub mod timeline;

pub use basics::{MediaTime, NormalizedPoint, NormalizedSize, PixelSize, RGBAColor};
pub use protocols::{NssError, Result};
