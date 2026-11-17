//! Build and lookup cost for a long recording.
//!
//! Port of upstream `MotionPerformanceTests.tenMinuteBuildAndLookupAreFast`. Wall-clock budgets
//! are asserted only with `NSS_BENCH=1`: loaded machines and CI VMs are unreliable, and a flaky
//! perf gate teaches people to ignore the suite.

use nss_core::{
    ClickEvent, ClickKind, CursorSample, CursorStyle, CursorTrack, MotionModel, MouseButton,
    Timeline,
};
use nss_motion::{MotionTuning, SpringMotionModel};
use std::time::Instant;

#[test]
fn ten_minute_build_and_lookup_are_fast() {
    let duration = 600.0;
    let n = (duration * 120.0) as usize;
    let samples: Vec<CursorSample> = (0..=n)
        .map(|i| {
            let t = i as f64 / 120.0;
            CursorSample::new(
                t,
                0.5 + 0.45 * (t * 0.37).sin(),
                0.5 + 0.45 * (t * 0.53 + 1.0).sin(),
            )
        })
        .collect();
    let clicks: Vec<ClickEvent> = (0..)
        .map(|i| 1.0 + i as f64 * 1.7)
        .take_while(|t| *t < duration)
        .map(|t| {
            ClickEvent::new(
                t,
                0.5 + 0.4 * t.sin(),
                0.5 + 0.4 * t.cos(),
                MouseButton::Left,
                ClickKind::Down,
            )
        })
        .collect();
    let track = CursorTrack {
        samples,
        clicks,
        ..CursorTrack::default()
    };

    let clock = Instant::now();
    let model = SpringMotionModel::new(
        &track,
        &Timeline::default(),
        CursorStyle {
            hide_when_idle: true,
            ..CursorStyle::default()
        },
        duration,
        16.0 / 9.0,
        MotionTuning::standard(),
    );
    let build_secs = clock.elapsed().as_secs_f64();
    println!(
        "nss-motion: 10 min build took {:.1} ms",
        build_secs * 1000.0
    );

    let lookups = 36_000; // 10 minutes at 60 fps
    let clock = Instant::now();
    let mut sink = 0.0;
    for i in 0..lookups {
        sink += model.frame_state(i as f64 / 60.0).camera.scale;
    }
    let per_lookup = clock.elapsed().as_secs_f64() / lookups as f64;
    println!("nss-motion: lookup {:.3} us", per_lookup * 1e6);

    assert!(sink > 0.0);
    assert!(model.simulated_frame_count() > 60 * 600);

    if std::env::var("NSS_BENCH").as_deref() == Ok("1") {
        // Release builds are far under 100 ms; unoptimised test builds a few times slower.
        let budget = if cfg!(debug_assertions) { 1.0 } else { 0.1 };
        assert!(build_secs < budget, "build took {build_secs} s");
        assert!(per_lookup < 50e-6, "lookup took {} us", per_lookup * 1e6);
    }
}
