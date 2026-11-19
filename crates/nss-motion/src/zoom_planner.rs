//! Turning clicks and manual `ZoomBlock`s into non-overlapping `ZoomSegment`s.
//!
//! Port of upstream `NSSMotion/ZoomPlanner.swift`. The clustering rules here define when the
//! camera zooms out and back in versus pans, so they are user-visible behaviour, not internals
//! (PLAN.md Appendix B).

use nss_core::{
    ClickEvent, ClickKind, CursorSample, CursorTrack, MediaTime, MouseButton, NormalizedPoint,
    Timeline, ZoomBlock, ZoomFocus,
};
use uuid::Uuid;

use crate::motion_tuning::MotionTuning;

/// Where a segment came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoomOrigin {
    Auto,
    Manual(Uuid),
}

/// One resolved zoom interval: an auto-zoom cluster or a manual block, after overlap resolution.
/// Segments returned by [`ZoomPlanner::segments`] are sorted and never overlap.
#[derive(Debug, Clone, PartialEq)]
pub struct ZoomSegment {
    pub start: MediaTime,
    pub end: MediaTime,
    /// Target magnification (already clamped to `1..=max_zoom_scale`).
    pub scale: f64,
    /// Focus at the start of the segment (the first click, for auto segments).
    pub initial_focus: NormalizedPoint,
    /// When set, the focus follows the cursor (with a dead zone) from this time on.
    /// `None` = fixed focus (`ZoomFocus::Manual`).
    pub follow_from: Option<MediaTime>,
    pub origin: ZoomOrigin,
    /// Zoom spring response (seconds) while easing in, from `start`.
    pub ease_in: f64,
    /// Zoom spring response (seconds) while easing back out to 1x, from `end`.
    pub ease_out: f64,
}

impl ZoomSegment {
    pub fn new(
        start: MediaTime,
        end: MediaTime,
        scale: f64,
        initial_focus: NormalizedPoint,
        follow_from: Option<MediaTime>,
        origin: ZoomOrigin,
    ) -> Self {
        Self {
            start,
            end,
            scale,
            initial_focus,
            follow_from,
            origin,
            ease_in: ZoomBlock::DEFAULT_EASE,
            ease_out: ZoomBlock::DEFAULT_EASE,
        }
    }

    #[must_use]
    pub fn contains(&self, t: MediaTime) -> bool {
        t >= self.start && t < self.end
    }
}

/// A mouse-down paired with the time its button was released (`t` when there is no matching up).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Press {
    down: ClickEvent,
    release: MediaTime,
}

/// A cluster of presses that share one zoom segment.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Cluster {
    first: ClickEvent,
    last: ClickEvent,
    release: MediaTime,
    /// The next cluster starts because of distance, not time.
    spatial_break_after: bool,
}

/// Turns clicks and manual [`ZoomBlock`]s into non-overlapping [`ZoomSegment`]s.
pub struct ZoomPlanner;

impl ZoomPlanner {
    /// Safety clamp for stored ease durations (spring responses). The inspector offers a narrower
    /// range; this only guards hand-edited or corrupt project files.
    pub const EASE_CLAMP_RANGE: std::ops::RangeInclusive<f64> = 0.05..=5.0;

    /// Resolve every zoom for `timeline` into segments.
    ///
    /// `source_aspect` (width / height) makes click distances isotropic, so a 0.3 horizontal move
    /// and a 0.3 vertical move cluster the same way regardless of how wide the screen is.
    #[must_use]
    pub fn segments(
        track: &CursorTrack,
        timeline: &Timeline,
        duration: MediaTime,
        source_aspect: f64,
        tuning: &MotionTuning,
    ) -> Vec<ZoomSegment> {
        let manual = Self::manual_segments(track, &timeline.zoom_blocks, duration, tuning);
        if !timeline.auto_zoom_enabled {
            return manual;
        }
        let auto = Self::auto_segments(track, duration, source_aspect, tuning);
        let mut result = manual.clone();
        for seg in auto {
            result.extend(Self::subtract(&manual, &seg, track, tuning));
        }
        result.sort_by(|a, b| {
            a.start
                .partial_cmp(&b.start)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        result
    }

    // MARK: Auto

    /// Pairs each mouse-down with the next mouse-up of the same button, so drags count as one press.
    fn presses(clicks: &[ClickEvent], duration: MediaTime) -> Vec<Press> {
        let mut events: Vec<ClickEvent> = clicks
            .iter()
            .copied()
            .filter(|c| {
                c.t.is_finite()
                    && c.x.is_finite()
                    && c.y.is_finite()
                    && c.t >= 0.0
                    && c.t <= duration
            })
            .collect();
        events.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));

        let mut out: Vec<Press> = Vec::new();
        let mut open: Vec<(MouseButton, usize)> = Vec::new();
        for e in events {
            match e.kind {
                ClickKind::Down => {
                    open.retain(|(b, _)| *b != e.button);
                    open.push((e.button, out.len()));
                    out.push(Press {
                        down: e,
                        release: e.t,
                    });
                }
                ClickKind::Up => {
                    if let Some(pos) = open.iter().position(|(b, _)| *b == e.button) {
                        let (_, i) = open.remove(pos);
                        out[i].release = e.t;
                    }
                }
            }
        }
        out
    }

    fn auto_segments(
        track: &CursorTrack,
        duration: MediaTime,
        source_aspect: f64,
        tuning: &MotionTuning,
    ) -> Vec<ZoomSegment> {
        let presses = Self::presses(&track.clicks, duration);
        if presses.is_empty() {
            return Vec::new();
        }
        let aspect = if source_aspect.is_finite() && source_aspect > 0.0 {
            source_aspect
        } else {
            16.0 / 9.0
        };

        // 1. Cluster presses in time (gap from the previous release) and space.
        let mut clusters: Vec<Cluster> = Vec::new();
        for p in &presses {
            let c = p.down;
            if let Some(mut cur) = clusters.last().copied() {
                let dt = c.t - cur.release;
                let dx = (c.x - cur.last.x) * aspect;
                let dy = c.y - cur.last.y;
                let near = (dx * dx + dy * dy).sqrt() <= tuning.cluster_max_distance;
                if dt <= tuning.cluster_max_gap && (near || dt <= tuning.cluster_pan_gap) {
                    cur.last = c;
                    cur.release = cur.release.max(p.release);
                    *clusters.last_mut().unwrap() = cur;
                    continue;
                }
                if dt <= tuning.cluster_max_gap {
                    clusters.last_mut().unwrap().spatial_break_after = true;
                }
            }
            clusters.push(Cluster {
                first: c,
                last: c,
                release: p.release,
                spatial_break_after: false,
            });
        }

        // 2. Each cluster -> interval: lead-in before the first click, hold after the last
        //    release while the cursor keeps moving.
        let mut segs: Vec<(ZoomSegment, MediaTime, bool)> = clusters
            .iter()
            .map(|cl| {
                let seg = ZoomSegment {
                    start: cl.first.t - tuning.auto_zoom_lead_in,
                    end: Self::hold_end(cl.release, track, tuning),
                    scale: Self::clamp_scale(tuning.auto_zoom_scale, tuning),
                    initial_focus: cl.first.point().clamped(),
                    follow_from: Some(cl.first.t),
                    origin: ZoomOrigin::Auto,
                    ease_in: tuning.zoom_spring.response,
                    ease_out: tuning.zoom_spring.response,
                };
                (seg, cl.release, cl.spatial_break_after)
            })
            .collect();

        // 3. Spatial splits: shorten the earlier hold to leave room to zoom out and back in. If
        //    there is no room the segments merge below and the camera pans instead.
        for i in 0..segs.len().saturating_sub(1) {
            if !segs[i].2 {
                continue;
            }
            let latest = segs[i + 1].0.start - tuning.min_zoom_out_gap;
            if latest >= segs[i].1 + tuning.auto_zoom_min_hold {
                segs[i].0.end = segs[i].0.end.min(latest);
            }
        }

        // 4. Merge segments whose gap is too short to zoom out and back in.
        let mut merged: Vec<ZoomSegment> = Vec::new();
        for (s, _, _) in segs {
            match merged.last_mut() {
                Some(prev) if s.start - prev.end < tuning.min_zoom_out_gap => {
                    prev.end = prev.end.max(s.end);
                }
                _ => merged.push(s),
            }
        }

        // 5. Clamp to the recording.
        merged
            .into_iter()
            .filter_map(|mut s| {
                s.start = s.start.max(0.0);
                s.end = s.end.min(duration);
                (s.end - s.start >= tuning.min_segment_duration).then_some(s)
            })
            .collect()
    }

    /// End of the hold after the last release at `t0`: `auto_zoom_hold` after the cursor last
    /// moved, capped at `t0 + auto_zoom_max_hold`.
    fn hold_end(t0: MediaTime, track: &CursorTrack, tuning: &MotionTuning) -> MediaTime {
        let cap = t0 + tuning.auto_zoom_max_hold.max(tuning.auto_zoom_hold);
        let mut last_move = t0;
        let samples = &track.samples;
        let mut i = Self::first_index_after(samples, t0).saturating_sub(1);
        let mut prev: Option<CursorSample> = None;
        while i < samples.len() {
            let s = samples[i];
            if s.t > cap || s.t - last_move > tuning.auto_zoom_hold {
                break;
            }
            if let Some(p) = prev {
                if s.t > t0 && (s.x - p.x).abs() + (s.y - p.y).abs() > tuning.activity_epsilon {
                    last_move = s.t;
                }
            }
            prev = Some(s);
            i += 1;
        }
        (last_move + tuning.auto_zoom_hold).min(cap)
    }

    // MARK: Manual

    fn manual_segments(
        track: &CursorTrack,
        blocks: &[ZoomBlock],
        duration: MediaTime,
        tuning: &MotionTuning,
    ) -> Vec<ZoomSegment> {
        let mut sorted: Vec<&ZoomBlock> = blocks
            .iter()
            .filter(|b| b.start.is_finite() && b.end.is_finite() && b.end > b.start)
            .collect();
        sorted.sort_by(|a, b| {
            a.start
                .partial_cmp(&b.start)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut segs: Vec<ZoomSegment> = Vec::new();
        for b in sorted {
            let start = b.start.max(0.0);
            let end = if duration > 0.0 {
                b.end.min(duration)
            } else {
                b.end
            };
            if end <= start {
                continue;
            }
            let mut seg = match b.focus {
                ZoomFocus::Manual(p) => {
                    let safe_focus = if p.x.is_finite() && p.y.is_finite() {
                        p.clamped()
                    } else {
                        NormalizedPoint::CENTER
                    };
                    ZoomSegment::new(
                        start,
                        end,
                        Self::clamp_scale(b.scale, tuning),
                        safe_focus,
                        None,
                        ZoomOrigin::Manual(b.id),
                    )
                }
                ZoomFocus::Auto {} => {
                    // Follow the cursor. A click in the block's lead-in is where the action is:
                    // aim there first and start following from the click.
                    if let Some(c) = Self::lead_in_click(track, start, end, tuning) {
                        ZoomSegment::new(
                            start,
                            end,
                            Self::clamp_scale(b.scale, tuning),
                            c.point().clamped(),
                            Some(c.t),
                            ZoomOrigin::Manual(b.id),
                        )
                    } else {
                        let p = track
                            .raw_position(start)
                            .map_or(NormalizedPoint::CENTER, NormalizedPoint::clamped);
                        ZoomSegment::new(
                            start,
                            end,
                            Self::clamp_scale(b.scale, tuning),
                            p,
                            Some(start),
                            ZoomOrigin::Manual(b.id),
                        )
                    }
                }
            };
            seg.ease_in = Self::clamp_ease(b.ease_in);
            seg.ease_out = Self::clamp_ease(b.ease_out);

            // A later-starting block takes over only where it overlaps: earlier blocks keep the
            // parts before and after it (for example a short block nested inside a long one).
            let mut next: Vec<ZoomSegment> = Vec::new();
            for prev in segs {
                next.extend(
                    Self::cut(&prev, seg.start, seg.end, track)
                        .into_iter()
                        .filter(|s| s.end > s.start),
                );
            }
            next.push(seg);
            next.sort_by(|a, b| {
                a.start
                    .partial_cmp(&b.start)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            segs = next;
        }
        segs
    }

    /// Cut every manual interval out of an auto segment; manual blocks win where they overlap.
    fn subtract(
        manual: &[ZoomSegment],
        seg: &ZoomSegment,
        track: &CursorTrack,
        tuning: &MotionTuning,
    ) -> Vec<ZoomSegment> {
        let mut pieces = vec![seg.clone()];
        for m in manual
            .iter()
            .filter(|m| m.end > seg.start && m.start < seg.end)
        {
            pieces = pieces
                .into_iter()
                .flat_map(|p| Self::cut(&p, m.start, m.end, track))
                .collect();
        }
        pieces
            .into_iter()
            .filter(|p| p.end - p.start >= tuning.min_segment_duration)
            .collect()
    }

    /// The parts of `seg` outside `[start, end)`: up to one piece before and one after. A
    /// cursor-following piece that resumes after the cut is re-anchored on the cursor there.
    fn cut(
        seg: &ZoomSegment,
        start: MediaTime,
        end: MediaTime,
        track: &CursorTrack,
    ) -> Vec<ZoomSegment> {
        if end <= seg.start || start >= seg.end {
            return vec![seg.clone()];
        }
        let mut out = Vec::new();
        if start > seg.start {
            let mut a = seg.clone();
            a.end = start;
            out.push(a);
        }
        if end < seg.end {
            let mut b = seg.clone();
            b.start = end;
            if let Some(f) = b.follow_from {
                if b.start > f {
                    b.initial_focus = track
                        .raw_position(b.start)
                        .map_or(b.initial_focus, NormalizedPoint::clamped);
                }
            }
            out.push(b);
        }
        out
    }

    /// First mouse-down in `[start, start + auto_zoom_lead_in]` (and before `end`), if any.
    fn lead_in_click(
        track: &CursorTrack,
        start: MediaTime,
        end: MediaTime,
        tuning: &MotionTuning,
    ) -> Option<ClickEvent> {
        let limit = end.min(start + tuning.auto_zoom_lead_in.max(0.0) + 1e-6);
        track
            .clicks
            .iter()
            .copied()
            .filter(|c| {
                c.kind == ClickKind::Down
                    && c.t.is_finite()
                    && c.x.is_finite()
                    && c.y.is_finite()
                    && c.t >= start
                    && c.t <= limit
            })
            .min_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal))
    }

    // MARK: Stored auto-zoom blocks

    /// Auto-zooms from clicks as stored [`ZoomBlock`]s (`is_auto`, follow-cursor focus, default
    /// ease). Parts that overlap `existing` blocks are cut away, and pieces shorter than
    /// [`MotionTuning::min_segment_duration`] are dropped, so the result never overlaps `existing`.
    #[must_use]
    pub fn auto_zoom_blocks(
        track: &CursorTrack,
        duration: MediaTime,
        source_aspect: f64,
        existing: &[ZoomBlock],
        tuning: &MotionTuning,
    ) -> Vec<ZoomBlock> {
        if !duration.is_finite() || duration <= 0.0 {
            return Vec::new();
        }
        let segs = Self::auto_segments(track, duration, source_aspect, tuning);
        let taken: Vec<&ZoomBlock> = existing
            .iter()
            .filter(|b| b.start.is_finite() && b.end.is_finite() && b.end > b.start)
            .collect();

        let mut out: Vec<ZoomBlock> = Vec::new();
        for seg in segs {
            let mut pieces: Vec<(MediaTime, MediaTime)> = vec![(seg.start, seg.end)];
            for b in &taken {
                pieces = pieces
                    .into_iter()
                    .flat_map(|(a, z)| {
                        if b.end <= a || b.start >= z {
                            return vec![(a, z)];
                        }
                        let mut r = Vec::new();
                        if b.start > a {
                            r.push((a, b.start));
                        }
                        if b.end < z {
                            r.push((b.end, z));
                        }
                        r
                    })
                    .collect();
            }
            for (a, b) in pieces {
                if b - a >= tuning.min_segment_duration - 1e-9 {
                    out.push(ZoomBlock {
                        id: Uuid::new_v4(),
                        start: a,
                        end: b,
                        scale: seg.scale,
                        focus: ZoomFocus::Auto {},
                        is_auto: true,
                        ease_in: tuning.zoom_spring.response,
                        ease_out: tuning.zoom_spring.response,
                    });
                }
            }
        }
        out.sort_by(|a, b| {
            a.start
                .partial_cmp(&b.start)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        out
    }

    // MARK: Helpers

    /// Clamp a stored ease duration into [`Self::EASE_CLAMP_RANGE`]; non-finite falls back to
    /// [`ZoomBlock::DEFAULT_EASE`].
    #[must_use]
    pub fn clamp_ease(e: f64) -> f64 {
        if !e.is_finite() {
            return ZoomBlock::DEFAULT_EASE;
        }
        e.clamp(
            *Self::EASE_CLAMP_RANGE.start(),
            *Self::EASE_CLAMP_RANGE.end(),
        )
    }

    fn clamp_scale(s: f64, tuning: &MotionTuning) -> f64 {
        if !s.is_finite() {
            return 1.0;
        }
        s.clamp(1.0, tuning.max_zoom_scale.max(1.0))
    }

    /// Index of the first sample with `t > time` (`samples.len()` if none).
    fn first_index_after(samples: &[CursorSample], time: MediaTime) -> usize {
        let (mut lo, mut hi) = (0usize, samples.len());
        while lo < hi {
            let mid = (lo + hi) / 2;
            if samples[mid].t <= time {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{click, make_samples};

    #[test]
    fn clamp_ease_guards_the_ends() {
        assert_eq!(ZoomPlanner::clamp_ease(f64::NAN), ZoomBlock::DEFAULT_EASE);
        assert_eq!(
            ZoomPlanner::clamp_ease(0.0),
            *ZoomPlanner::EASE_CLAMP_RANGE.start()
        );
        assert_eq!(
            ZoomPlanner::clamp_ease(100.0),
            *ZoomPlanner::EASE_CLAMP_RANGE.end()
        );
        assert_eq!(ZoomPlanner::clamp_ease(0.4), 0.4);
    }

    #[test]
    fn presses_pair_with_the_matching_button_only() {
        let clicks = [
            click(1.0, ClickKind::Down, 0.5, 0.5),
            click(1.5, ClickKind::Up, 0.5, 0.5),
            click(2.0, ClickKind::Down, 0.5, 0.5),
            ClickEvent::new(3.0, 0.5, 0.5, MouseButton::Right, ClickKind::Up),
        ];
        let p = ZoomPlanner::presses(&clicks, 10.0);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].release, 1.5);
        assert_eq!(p[1].release, 2.0, "a right-up does not close a left-down");
    }

    #[test]
    fn lead_in_click_finds_only_down_events_in_the_window() {
        let t = MotionTuning::default();
        let track = CursorTrack {
            clicks: vec![
                click(4.4, ClickKind::Up, 0.5, 0.5),
                click(4.8, ClickKind::Down, 0.2, 0.3),
                click(6.0, ClickKind::Down, 0.9, 0.9),
            ],
            ..CursorTrack::default()
        };
        let c = ZoomPlanner::lead_in_click(&track, 4.5, 9.0, &t).unwrap();
        assert_eq!((c.t, c.x), (4.8, 0.2));
    }

    #[test]
    fn segments_are_sorted_and_do_not_overlap() {
        let t = MotionTuning::default();
        let track = CursorTrack {
            samples: make_samples(20.0, 120.0, |tt| NormalizedPoint::new(0.2 + 0.03 * tt, 0.5)),
            clicks: vec![
                click(2.0, ClickKind::Down, 0.2, 0.2),
                click(9.0, ClickKind::Down, 0.8, 0.8),
            ],
            ..CursorTrack::default()
        };
        let manual = ZoomBlock {
            focus: ZoomFocus::Manual(NormalizedPoint::new(0.5, 0.5)),
            ..ZoomBlock::new(5.0, 7.0)
        };
        let timeline = Timeline {
            zoom_blocks: vec![manual.clone()],
            ..Timeline::default()
        };
        let segs = ZoomPlanner::segments(&track, &timeline, 20.0, 16.0 / 9.0, &t);
        assert!(!segs.is_empty());
        for w in segs.windows(2) {
            assert!(
                w[0].end <= w[1].start + 1e-9,
                "{:?} overlaps {:?}",
                w[0],
                w[1]
            );
        }
        for s in &segs {
            assert!(s.end > s.start);
            assert!(s.scale >= 1.0 && s.scale <= t.max_zoom_scale);
        }
        assert!(segs
            .iter()
            .any(|s| s.origin == ZoomOrigin::Manual(manual.id)));
    }

    #[test]
    fn auto_zoom_blocks_avoid_existing_blocks() {
        // Mirrors upstream StoredZoomTests.autoBlocksAvoidExistingBlocks.
        let t = MotionTuning::default();
        let track = CursorTrack {
            samples: make_samples(20.0, 120.0, |tt| {
                NormalizedPoint::new(0.3 + 0.2 * tt.sin(), 0.4 + 0.1 * (tt * 0.7).cos())
            }),
            clicks: vec![
                click(2.0, ClickKind::Down, 0.3, 0.3),
                click(2.6, ClickKind::Down, 0.32, 0.31),
                click(8.0, ClickKind::Down, 0.8, 0.7),
                click(14.0, ClickKind::Down, 0.2, 0.8),
            ],
            ..CursorTrack::default()
        };
        let manual = ZoomBlock::new(1.0, 2.5);
        let blocks = ZoomPlanner::auto_zoom_blocks(&track, 20.0, 16.0 / 9.0, &[manual], &t);
        for b in &blocks {
            assert!(
                b.end <= 1.0 || b.start >= 2.5,
                "{:?} overlaps the manual block",
                b
            );
            assert!(b.end - b.start >= t.min_segment_duration - 1e-9);
            assert!(b.is_auto);
        }
        assert!(blocks.iter().any(|b| (b.start - 2.5).abs() < 1e-9));
        assert!(ZoomPlanner::auto_zoom_blocks(&track, 0.0, 1.7, &[], &t).is_empty());
    }
}
