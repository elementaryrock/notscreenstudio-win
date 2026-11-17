//! Fixtures shared by this crate's tests, mirroring the Swift test helpers in
//! `Tests/NSSMotionTests/NSSMotionTests.swift` (`makeSamples`, `click`, `model`).

use nss_core::{
    ClickEvent, ClickKind, CursorSample, CursorStyle, CursorTrack, MediaTime, MouseButton,
    NormalizedPoint, Timeline,
};

use crate::motion_tuning::MotionTuning;
use crate::spring_motion_model::SpringMotionModel;

/// `rate`-Hz samples from a position function over `[0, duration]`.
pub fn make_samples(
    duration: f64,
    rate: f64,
    f: impl Fn(f64) -> NormalizedPoint,
) -> Vec<CursorSample> {
    let n = (duration * rate) as usize;
    (0..=n)
        .map(|i| {
            let t = i as f64 / rate;
            let p = f(t);
            CursorSample::new(t, p.x, p.y)
        })
        .collect()
}

/// A left-button event at `t`.
pub fn click(t: MediaTime, kind: ClickKind, x: f64, y: f64) -> ClickEvent {
    ClickEvent::new(t, x, y, MouseButton::Left, kind)
}

/// Build a model with the standard tuning and a 16:9 source.
pub fn model(
    samples: Vec<CursorSample>,
    clicks: &[ClickEvent],
    timeline: Timeline,
    style: CursorStyle,
    duration: MediaTime,
) -> SpringMotionModel {
    SpringMotionModel::new(
        &CursorTrack {
            samples,
            clicks: clicks.to_vec(),
            ..CursorTrack::default()
        },
        &timeline,
        style,
        duration,
        16.0 / 9.0,
        MotionTuning::standard(),
    )
}
