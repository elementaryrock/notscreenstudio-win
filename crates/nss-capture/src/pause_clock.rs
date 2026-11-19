//! Pauses are cut out of the recording rather than left as gaps.
//!
//! Port of upstream `NSSCapture/PauseClock.swift`. `PauseTimeline` is a pure value type so the
//! timestamp maths is unit-tested; `PauseClock` is the shared, lock-protected copy the screen,
//! microphone and camera recorders all consult.
//!
//! The rule, unchanged from upstream: a sample that starts inside a pause is dropped, and every
//! later sample moves earlier by the total paused time before it. So the file has no gap, no clock
//! jump, and nothing that happened while paused is in it.

use std::sync::Mutex;

use nss_core::{ClickEvent, ClickKind, CursorTrack, MediaTime, MouseButton};

/// One stretch of host-clock time that was paused.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PauseInterval {
    pub start: f64,
    /// `None` while the pause is still open.
    pub end: Option<f64>,
}

impl PauseInterval {
    #[must_use]
    pub fn new(start: f64, end: Option<f64>) -> Self {
        Self { start, end }
    }

    fn end_or_infinite(&self) -> f64 {
        self.end.unwrap_or(f64::INFINITY)
    }
}

/// The stretches of host-clock time that were paused.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PauseTimeline {
    intervals: Vec<PauseInterval>,
}

impl PauseTimeline {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn intervals(&self) -> &[PauseInterval] {
        &self.intervals
    }

    /// True while a pause is open.
    #[must_use]
    pub fn is_paused(&self) -> bool {
        self.intervals.last().is_some_and(|i| i.end.is_none())
    }

    /// Opens a pause at `t`. Ignored if already paused.
    pub fn pause(&mut self, t: f64) {
        if self.is_paused() {
            return;
        }
        self.intervals.push(PauseInterval::new(t, None));
    }

    /// Closes the open pause at `t`. Ignored if not paused.
    pub fn resume(&mut self, t: f64) {
        if !self.is_paused() {
            return;
        }
        if let Some(last) = self.intervals.last_mut() {
            last.end = Some(t.max(last.start));
        }
    }

    /// True when `t` falls inside a pause (start inclusive, end exclusive).
    #[must_use]
    pub fn is_paused_at(&self, t: f64) -> bool {
        self.intervals
            .iter()
            .any(|i| t >= i.start && t < i.end_or_infinite())
    }

    /// Total paused time strictly before `t`. An open pause counts up to `t`.
    #[must_use]
    pub fn paused_duration_before(&self, t: f64) -> f64 {
        self.intervals
            .iter()
            .map(|i| (t.min(i.end_or_infinite()) - i.start).max(0.0))
            .sum()
    }

    /// `t` moved onto the pause-free timeline. A time inside a pause collapses to the start of it.
    #[must_use]
    pub fn shift(&self, t: f64) -> MediaTime {
        t - self.paused_duration_before(t)
    }

    /// The pause-free time for a sample starting at `t`, or `None` if it starts inside a pause.
    #[must_use]
    pub fn map(&self, t: f64) -> Option<MediaTime> {
        if self.is_paused_at(t) {
            None
        } else {
            Some(self.shift(t))
        }
    }

    /// Total paused time up to `t`.
    #[must_use]
    pub fn total_paused_until(&self, t: f64) -> f64 {
        self.paused_duration_before(t)
    }

    /// A cursor track (times relative to `origin`, a host time) with the paused stretches removed,
    /// matching the video. Samples inside a pause are dropped.
    ///
    /// Clicks made during a pause never happened in the recording: a press that starts inside a pause
    /// is dropped together with its release even if that lands after the resume. A press from before
    /// the pause whose release falls inside it keeps the release, collapsed to the pause start, so the
    /// button is not left held down for the rest of the recording.
    #[must_use]
    pub fn removing_pauses(&self, track: &CursorTrack, origin: f64) -> CursorTrack {
        if self.intervals.is_empty() {
            return track.clone();
        }
        let mut out = track.clone();
        out.samples = track
            .samples
            .iter()
            .filter_map(|s| {
                let mapped = self.map(origin + s.t)?;
                let mut s = *s;
                s.t = (mapped - origin).max(0.0);
                Some(s)
            })
            .collect();

        let mut dropping_release: Vec<MouseButton> = Vec::new();
        let mut clicks = Vec::new();
        for c in &track.clicks {
            let mut c: ClickEvent = *c;
            match c.kind {
                ClickKind::Down => {
                    if self.is_paused_at(origin + c.t) {
                        if !dropping_release.contains(&c.button) {
                            dropping_release.push(c.button);
                        }
                        continue;
                    }
                    dropping_release.retain(|b| *b != c.button);
                }
                ClickKind::Up => {
                    if let Some(i) = dropping_release.iter().position(|b| *b == c.button) {
                        dropping_release.remove(i);
                        continue;
                    }
                }
            }
            c.t = (self.shift(origin + c.t) - origin).max(0.0);
            clicks.push(c);
        }
        out.clicks = clicks;

        out.shapes = track
            .shapes
            .iter()
            .map(|c| {
                let mut c = *c;
                c.t = (self.shift(origin + c.t) - origin).max(0.0);
                c
            })
            .collect();
        out
    }
}

/// Thread-safe [`PauseTimeline`] shared by the screen, microphone and camera recorders of a session.
#[derive(Debug, Default)]
pub struct PauseClock {
    timeline: Mutex<PauseTimeline>,
}

impl PauseClock {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn snapshot(&self) -> PauseTimeline {
        self.guard().clone()
    }

    #[must_use]
    pub fn is_paused(&self) -> bool {
        self.guard().is_paused()
    }

    pub fn pause(&self, t: f64) {
        self.guard().pause(t);
    }

    pub fn resume(&self, t: f64) {
        self.guard().resume(t);
    }

    #[must_use]
    pub fn shift(&self, t: f64) -> MediaTime {
        self.guard().shift(t)
    }

    /// The sample's time stamp moved onto the pause-free timeline, or `None` when it starts inside a
    /// pause and must be dropped. Untouched samples are reported as unchanged, so the caller can skip
    /// rewriting them.
    #[must_use]
    pub fn retime(&self, pts: f64) -> Option<f64> {
        let timeline = self.guard();
        if timeline.intervals().is_empty() {
            return Some(pts);
        }
        timeline.map(pts)
    }

    fn guard(&self) -> std::sync::MutexGuard<'_, PauseTimeline> {
        self.timeline.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Holds the newest frame that arrived while paused, so one frame can be written at the pause start.
///
/// Without this, the recording would jump from the last pre-pause frame straight to the first
/// post-pause one, showing a stale picture for the length of the pause in the viewer's head. With it,
/// the paused screen is shown once, at the instant the pause began.
#[derive(Debug, Default)]
pub struct PausedFrameHold<T> {
    frame: Option<T>,
}

impl<T> PausedFrameHold<T> {
    #[must_use]
    pub fn new() -> Self {
        Self { frame: None }
    }

    pub fn hold(&mut self, frame: T) {
        self.frame = Some(frame);
    }

    /// Returns the held frame once and clears it.
    pub fn take(&mut self) -> Option<T> {
        self.frame.take()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.frame.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nss_core::{CursorSample, CursorShape, CursorShapeChange};

    #[test]
    fn no_pauses_changes_nothing() {
        let t = PauseTimeline::new();
        assert_eq!(t.shift(5.0), 5.0);
        assert_eq!(t.map(5.0), Some(5.0));
        assert!(!t.is_paused());
    }

    #[test]
    fn samples_inside_a_pause_are_dropped() {
        let mut t = PauseTimeline::new();
        t.pause(10.0);
        assert!(t.is_paused());
        assert_eq!(t.map(10.0), None);
        assert_eq!(t.map(50.0), None, "an open pause drops everything after it");
        t.resume(14.0);
        assert!(!t.is_paused());
        assert_eq!(t.map(12.0), None);
        assert_eq!(
            t.map(14.0),
            Some(10.0),
            "the first sample after resume follows the last one before the pause"
        );
        assert_eq!(t.map(9.99), Some(9.99));
    }

    #[test]
    fn later_samples_move_earlier_by_the_total_paused_time() {
        let mut t = PauseTimeline::new();
        t.pause(10.0);
        t.resume(14.0); // 4 s
        t.pause(20.0);
        t.resume(23.0); // 3 s
        assert_eq!(t.shift(15.0), 11.0);
        assert_eq!(t.shift(30.0), 23.0);
        assert_eq!(t.map(21.0), None);
        assert_eq!(t.map(23.0), Some(16.0));
    }

    #[test]
    fn shifted_times_never_go_backwards() {
        let mut t = PauseTimeline::new();
        t.pause(1.0);
        t.resume(2.0);
        t.pause(3.0);
        t.resume(3.5);
        let mut last = f64::NEG_INFINITY;
        for i in 0..100 {
            let x = f64::from(i) * 0.06;
            if let Some(m) = t.map(x) {
                assert!(m >= last, "map({x}) = {m} went backwards from {last}");
                last = m;
            }
        }
    }

    #[test]
    fn end_time_while_paused_is_the_start_of_the_pause() {
        let mut t = PauseTimeline::new();
        t.pause(10.0);
        t.resume(12.0);
        t.pause(20.0);
        assert_eq!(t.shift(25.0), 18.0); // 25 minus the 2 s earlier pause
        assert_eq!(t.paused_duration_before(25.0), 7.0);
    }

    #[test]
    fn double_pause_and_resume_are_ignored() {
        let mut t = PauseTimeline::new();
        t.resume(1.0);
        t.pause(2.0);
        t.pause(3.0);
        t.resume(4.0);
        t.resume(5.0);
        assert_eq!(t.intervals(), [PauseInterval::new(2.0, Some(4.0))]);
    }

    #[test]
    fn a_resume_before_the_pause_does_not_go_backwards() {
        let mut t = PauseTimeline::new();
        t.pause(10.0);
        t.resume(4.0);
        assert_eq!(t.intervals()[0].end, Some(10.0));
    }

    #[test]
    fn cursor_track_loses_paused_stretches_and_stays_in_step_with_the_video() {
        let mut t = PauseTimeline::new();
        t.pause(102.0);
        t.resume(105.0); // origin 100: paused from t = 2 to t = 5

        let click = |t: f64, kind: ClickKind| ClickEvent::new(t, 0.0, 0.0, MouseButton::Left, kind);
        let track = CursorTrack {
            samples: [0.0, 1.0, 2.5, 4.0, 5.0, 6.0]
                .iter()
                .map(|x| CursorSample::new(*x, x / 10.0, 0.0))
                .collect(),
            clicks: vec![
                click(1.0, ClickKind::Down),
                click(3.0, ClickKind::Up), // release inside: kept, collapsed
                click(3.2, ClickKind::Down),
                click(3.4, ClickKind::Up), // fully inside: dropped
                click(4.0, ClickKind::Down),
                click(5.5, ClickKind::Up), // down inside, up after: both dropped
                click(6.0, ClickKind::Down),
                click(6.5, ClickKind::Up),
            ],
            shapes: vec![
                CursorShapeChange::new(3.0, CursorShape::IBeam),
                CursorShapeChange::new(6.0, CursorShape::Arrow),
            ],
            ..CursorTrack::default()
        };

        let out = t.removing_pauses(&track, 100.0);
        assert_eq!(
            out.samples.iter().map(|s| s.t).collect::<Vec<_>>(),
            [0.0, 1.0, 2.0, 3.0],
            "5 -> 2, 6 -> 3; 2.5 and 4 dropped"
        );
        assert_eq!(
            out.clicks.iter().map(|c| c.t).collect::<Vec<_>>(),
            [1.0, 2.0, 3.0, 3.5]
        );
        assert_eq!(
            out.clicks.iter().map(|c| c.kind).collect::<Vec<_>>(),
            [
                ClickKind::Down,
                ClickKind::Up,
                ClickKind::Down,
                ClickKind::Up
            ]
        );
        assert_eq!(
            out.shapes.iter().map(|c| c.t).collect::<Vec<_>>(),
            [2.0, 3.0]
        );
    }

    #[test]
    fn a_track_without_pauses_is_returned_unchanged() {
        let t = PauseTimeline::new();
        let mut track = CursorTrack::default();
        track.samples.push(CursorSample::new(1.0, 0.5, 0.5));
        let out = t.removing_pauses(&track, 0.0);
        assert_eq!(out, track);
    }

    #[test]
    fn the_clock_shares_one_timeline() {
        let clock = PauseClock::new();
        assert_eq!(clock.retime(12.0), Some(12.0));
        clock.pause(10.0);
        clock.resume(14.0);
        assert_eq!(clock.retime(12.0), None);
        assert_eq!(clock.retime(9.0), Some(9.0));
        assert_eq!(clock.retime(20.0), Some(16.0));
        assert_eq!(clock.shift(15.0), 11.0);
        assert!(!clock.is_paused());
        assert_eq!(clock.snapshot().intervals().len(), 1);
    }

    #[test]
    fn the_paused_frame_hold_keeps_only_the_newest_and_hands_it_back_once() {
        let mut hold = PausedFrameHold::new();
        assert!(hold.is_empty());
        assert_eq!(hold.take(), None);
        hold.hold(1);
        hold.hold(2);
        hold.hold(3);
        assert_eq!(hold.take(), Some(3));
        assert_eq!(hold.take(), None);
    }

    #[test]
    fn a_held_frame_lands_at_the_pause_start_on_the_shifted_timeline() {
        let mut timeline = PauseTimeline::new();
        timeline.pause(10.0);
        timeline.resume(14.0);
        // The held frame is stamped at the resume time (14), which maps to the pause start (10).
        assert_eq!(timeline.map(14.0), Some(10.0));
    }
}
