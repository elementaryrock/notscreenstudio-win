//! Cursor sampling core (`CursorRecorder`), `NoopInputLogger`, and live `SystemInputLogger`.
//!
//! Port of upstream `NSSInput/CursorRecorder.swift`, `NSSInput/SystemInputLogger.swift`, and
//! `NSSInput/NoopInputLogger.swift`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use nss_core::{
    host_now, CaptureSource, ClickEvent, ClickKind, CursorSample, CursorTrack, InputLogger,
    MouseButton, NssError, Point2, Rect,
};
use parking_lot::Mutex;

use crate::geometry::{CoordinateMath, ScreenGeometryProvider, SystemScreenGeometry};
use crate::hook::MouseHookHandle;
use crate::schedule::SamplingSchedule;
use crate::shapes::{CursorShapeRecorder, CursorShapeSampler, SystemCursorShapeSampler};

/// Provider returning the current global top-left cursor location.
pub type PositionProvider = Arc<dyn Fn() -> Option<Point2> + Send + Sync>;
/// Monotonic host clock provider (seconds).
pub type ClockProvider = Arc<dyn Fn() -> f64 + Send + Sync>;

struct RecorderState {
    samples: Vec<CursorSample>,
    clicks: Vec<ClickEvent>,
    rect: Option<Rect>,
}

/// Hardware-free, lock-protected cursor logging core so tests can drive it with fake clocks.
pub struct CursorRecorder {
    source: CaptureSource,
    geometry: Arc<dyn ScreenGeometryProvider>,
    position: PositionProvider,
    clock: ClockProvider,
    state: Mutex<RecorderState>,
}

impl CursorRecorder {
    #[must_use]
    pub fn new(
        source: CaptureSource,
        geometry: Arc<dyn ScreenGeometryProvider>,
        schedule: SamplingSchedule,
        position: PositionProvider,
        clock: ClockProvider,
    ) -> Self {
        let initial_rect = geometry.global_rect(source);
        // Preallocate for ~10 minutes at the nominal rate (amortized growth after that).
        let cap = ((schedule.rate as usize) * 60 * 10).min(120_000);
        Self {
            source,
            geometry,
            position,
            clock,
            state: Mutex::new(RecorderState {
                samples: Vec::with_capacity(cap),
                clicks: Vec::new(),
                rect: initial_rect,
            }),
        }
    }

    /// How often the source frame should be re-read on its own low-rate thread:
    /// 30 Hz for a window that can move, 1 Hz for a display or area.
    #[must_use]
    pub fn refresh_interval(&self) -> Duration {
        if self.geometry.is_dynamic(self.source) {
            Duration::from_secs_f64(1.0 / 30.0)
        } else {
            Duration::from_secs(1)
        }
    }

    /// Re-reads the source frame. Keeps the last known rect when the source vanishes (e.g. a
    /// closed window) so in-flight ticks do not crash.
    pub fn refresh_rect(&self) {
        if let Some(r) = self.geometry.global_rect(self.source) {
            self.state.lock().rect = Some(r);
        }
    }

    /// Takes one sample using the cached rect (zero Win32 geometry lookups on the 120 Hz hot path).
    pub fn tick(&self) {
        let now = (self.clock)();
        let Some(p) = (self.position)() else {
            return;
        };
        let mut s = self.state.lock();
        if let Some(rect) = s.rect {
            if let Some(n) = CoordinateMath::normalize(p, rect) {
                s.samples.push(CursorSample::new(now, n.x, n.y));
            }
        }
    }

    /// Records a click at a global top-left point and monotonic host timestamp.
    pub fn click(&self, point: Point2, host_time: f64, button: MouseButton, kind: ClickKind) {
        let mut s = self.state.lock();
        if let Some(rect) = s.rect {
            if let Some(n) = CoordinateMath::normalize(point, rect) {
                s.clicks
                    .push(ClickEvent::new(host_time, n.x, n.y, button, kind));
            }
        }
    }

    #[must_use]
    pub fn sample_count(&self) -> usize {
        self.state.lock().samples.len()
    }

    /// Rebases all samples and clicks to `recording_start_host_time` and rounds for compact
    /// `cursor.json` output (`1e-4` s, `1e-5` normalized coords), matching upstream verbatim.
    ///
    /// Keeps the last sample at or before the origin clamped to `t = 0` so the initial pointer
    /// position is always known. Drops clicks before the origin.
    #[must_use]
    pub fn finish(&self, recording_start_host_time: f64, sample_rate: f64) -> CursorTrack {
        let (samples, clicks) = {
            let s = self.state.lock();
            (s.samples.clone(), s.clicks.clone())
        };

        let mut out = Vec::with_capacity(samples.len());
        let mut carry: Option<CursorSample> = None;

        for s in samples {
            if s.t - recording_start_host_time <= 0.0 {
                carry = Some(s);
            } else {
                if let Some(c) = carry.take() {
                    out.push(round_sample(CursorSample::new(0.0, c.x, c.y)));
                }
                out.push(round_sample(CursorSample::new(
                    s.t - recording_start_host_time,
                    s.x,
                    s.y,
                )));
            }
        }
        if out.is_empty() {
            if let Some(c) = carry {
                out.push(round_sample(CursorSample::new(0.0, c.x, c.y)));
            }
        }

        let mut rebased: Vec<ClickEvent> = clicks
            .into_iter()
            .filter(|c| c.t >= recording_start_host_time)
            .map(|c| {
                ClickEvent::new(
                    round_to(c.t - recording_start_host_time, 1e4),
                    round_to(c.x, 1e5),
                    round_to(c.y, 1e5),
                    c.button,
                    c.kind,
                )
            })
            .collect();
        rebased.sort_by(|a, b| a.t.total_cmp(&b.t));

        CursorTrack {
            samples: out,
            clicks: rebased,
            sample_rate,
            shapes: Vec::new(),
        }
    }
}

fn round_to(v: f64, scale: f64) -> f64 {
    (v * scale).round() / scale
}

fn round_sample(s: CursorSample) -> CursorSample {
    CursorSample::new(round_to(s.t, 1e4), round_to(s.x, 1e5), round_to(s.y, 1e5))
}

/// No-op [`InputLogger`] for headless tests (port of upstream `NoopInputLogger`).
#[derive(Debug, Default)]
pub struct NoopInputLogger;

impl NoopInputLogger {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl InputLogger for NoopInputLogger {
    fn start(&mut self, _source: CaptureSource) -> Result<(), NssError> {
        Ok(())
    }

    fn stop(&mut self, _recording_start_host_time: f64) -> CursorTrack {
        CursorTrack::default()
    }
}

/// Configuration builder for [`SystemInputLogger`].
pub struct SystemInputLoggerConfig {
    pub sample_rate: f64,
    pub geometry: Arc<dyn ScreenGeometryProvider>,
    pub position: Option<PositionProvider>,
    pub monitor_clicks: bool,
    pub log_cursor_shapes: bool,
    pub shape_rate: f64,
    pub shape_sampler: Option<CursorShapeSampler>,
}

impl Default for SystemInputLoggerConfig {
    fn default() -> Self {
        Self {
            sample_rate: 120.0,
            geometry: Arc::new(SystemScreenGeometry::new()),
            position: None,
            monitor_clicks: true,
            log_cursor_shapes: true,
            shape_rate: 30.0,
            shape_sampler: None,
        }
    }
}

struct RunningSession {
    stop_flag: Arc<AtomicBool>,
    recorder: Arc<CursorRecorder>,
    shape_recorder: Option<Arc<CursorShapeRecorder>>,
    sample_thread: Option<JoinHandle<()>>,
    refresh_thread: Option<JoinHandle<()>>,
    shape_thread: Option<JoinHandle<()>>,
    hook: Option<MouseHookHandle>,
}

/// Live Windows [`InputLogger`]: 120 Hz `GetCursorPos` sampler + `WH_MOUSE_LL` click hook + 30 Hz
/// cursor-shape sampler, stamped on the shared `nss_core::host_now()` clock.
pub struct SystemInputLogger {
    schedule: SamplingSchedule,
    geometry: Arc<dyn ScreenGeometryProvider>,
    position: Option<PositionProvider>,
    monitor_clicks: bool,
    log_cursor_shapes: bool,
    shape_schedule: SamplingSchedule,
    shape_sampler: Option<CursorShapeSampler>,
    running: Option<RunningSession>,
}

impl Default for SystemInputLogger {
    fn default() -> Self {
        Self::new(SystemInputLoggerConfig::default())
    }
}

impl SystemInputLogger {
    #[must_use]
    pub fn new(config: SystemInputLoggerConfig) -> Self {
        Self {
            schedule: SamplingSchedule::new(config.sample_rate),
            geometry: config.geometry,
            position: config.position,
            monitor_clicks: config.monitor_clicks,
            log_cursor_shapes: config.log_cursor_shapes,
            shape_schedule: SamplingSchedule::new(config.shape_rate),
            shape_sampler: config.shape_sampler,
            running: None,
        }
    }

    fn default_position_provider() -> PositionProvider {
        Arc::new(|| {
            #[cfg(windows)]
            {
                crate::win::cursor_pos()
            }
            #[cfg(not(windows))]
            {
                Some(Point2::ZERO)
            }
        })
    }
}

impl InputLogger for SystemInputLogger {
    fn start(&mut self, source: CaptureSource) -> Result<(), NssError> {
        if self.running.is_some() {
            return Err(NssError::Unsupported("InputLogger already running".into()));
        }
        if self.geometry.global_rect(source).is_none() {
            return Err(NssError::InvalidProject(format!(
                "Capture source {source:?} cannot be located on screen"
            )));
        }

        let position = self
            .position
            .clone()
            .unwrap_or_else(Self::default_position_provider);
        let clock: ClockProvider = Arc::new(host_now);

        let recorder = Arc::new(CursorRecorder::new(
            source,
            Arc::clone(&self.geometry),
            self.schedule,
            position,
            Arc::clone(&clock),
        ));
        // Take an immediate sample at t ~ 0 so even a very short recording has a starting point.
        recorder.tick();

        let stop_flag = Arc::new(AtomicBool::new(false));

        // 1. Optional low-level mouse click hook.
        let hook = if self.monitor_clicks {
            let rec_click = Arc::clone(&recorder);
            Some(MouseHookHandle::install(Arc::new(move |ev| {
                rec_click.click(ev.point, ev.host_time, ev.button, ev.kind);
            }))?)
        } else {
            None
        };

        // 2. 120 Hz position sampling thread with 1 ms Windows timer resolution + precise pacing.
        let sample_rec = Arc::clone(&recorder);
        let sample_stop = Arc::clone(&stop_flag);
        let sample_step = self.schedule.interval_duration();
        let sample_thread = thread::Builder::new()
            .name("nss-input-sampler".into())
            .spawn(move || {
                #[cfg(windows)]
                let _timer_guard = crate::win::TimerPeriodGuard::request_1ms();
                let mut next = Instant::now() + sample_step;
                while !sample_stop.load(Ordering::Relaxed) {
                    let now = Instant::now();
                    if now < next {
                        let rem = next - now;
                        if rem > Duration::from_millis(2) {
                            thread::sleep(rem - Duration::from_millis(1));
                        } else {
                            thread::yield_now();
                        }
                        continue;
                    }
                    sample_rec.tick();
                    next += sample_step;
                    let after = Instant::now();
                    if after > next + sample_step * 4 {
                        next = after + sample_step;
                    }
                }
            })
            .map_err(|e| NssError::Io(format!("Failed to start cursor sampler thread: {e}")))?;

        // 3. Low-rate window-bounds refresh thread (never blocks the 120 Hz loop).
        let refresh_rec = Arc::clone(&recorder);
        let refresh_stop = Arc::clone(&stop_flag);
        let refresh_step = recorder.refresh_interval();
        let refresh_thread = thread::Builder::new()
            .name("nss-input-geometry".into())
            .spawn(move || {
                while !refresh_stop.load(Ordering::Relaxed) {
                    sleep_interruptible(refresh_step, &refresh_stop);
                    if refresh_stop.load(Ordering::Relaxed) {
                        break;
                    }
                    refresh_rec.refresh_rect();
                }
            })
            .map_err(|e| NssError::Io(format!("Failed to start geometry refresh thread: {e}")))?;

        // 4. Optional 30 Hz cursor-shape sampler thread.
        let (shape_recorder, shape_thread) = if self.log_cursor_shapes {
            if let Some(sampler) = self
                .shape_sampler
                .clone()
                .or_else(SystemCursorShapeSampler::make)
            {
                let sr = Arc::new(CursorShapeRecorder::new(sampler, clock));
                sr.tick();
                let sr_bg = Arc::clone(&sr);
                let shape_stop = Arc::clone(&stop_flag);
                let shape_step = self.shape_schedule.interval_duration();
                let handle = thread::Builder::new()
                    .name("nss-input-shapes".into())
                    .spawn(move || {
                        while !shape_stop.load(Ordering::Relaxed) {
                            sleep_interruptible(shape_step, &shape_stop);
                            if shape_stop.load(Ordering::Relaxed) {
                                break;
                            }
                            sr_bg.tick();
                        }
                    })
                    .map_err(|e| {
                        NssError::Io(format!("Failed to start cursor shape thread: {e}"))
                    })?;
                (Some(sr), Some(handle))
            } else {
                (None, None)
            }
        } else {
            (None, None)
        };

        self.running = Some(RunningSession {
            stop_flag,
            recorder,
            shape_recorder,
            sample_thread: Some(sample_thread),
            refresh_thread: Some(refresh_thread),
            shape_thread,
            hook,
        });
        Ok(())
    }

    fn stop(&mut self, recording_start_host_time: f64) -> CursorTrack {
        let Some(mut session) = self.running.take() else {
            return CursorTrack::default();
        };
        session.stop_flag.store(true, Ordering::SeqCst);
        if let Some(mut h) = session.hook.take() {
            h.uninstall();
        }
        if let Some(t) = session.sample_thread.take() {
            let _ = t.join();
        }
        if let Some(t) = session.refresh_thread.take() {
            let _ = t.join();
        }
        if let Some(t) = session.shape_thread.take() {
            let _ = t.join();
        }
        let mut track = session
            .recorder
            .finish(recording_start_host_time, self.schedule.rate);
        if let Some(sr) = session.shape_recorder {
            track.shapes = sr.finish(recording_start_host_time);
        }
        track
    }
}

impl Drop for SystemInputLogger {
    fn drop(&mut self) {
        if self.running.is_some() {
            let _ = self.stop(0.0);
        }
    }
}

fn sleep_interruptible(total: Duration, stop: &AtomicBool) {
    let end = Instant::now() + total;
    let slice = Duration::from_millis(5);
    while !stop.load(Ordering::Relaxed) {
        let now = Instant::now();
        if now >= end {
            break;
        }
        thread::sleep((end - now).min(slice));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schedule::SamplingStats;
    use nss_core::CursorShape;
    use std::sync::atomic::AtomicUsize;

    struct FakeGeometry {
        rect: Mutex<Option<Rect>>,
        dynamic: bool,
    }

    impl FakeGeometry {
        fn new(rect: Option<Rect>, dynamic: bool) -> Arc<Self> {
            Arc::new(Self {
                rect: Mutex::new(rect),
                dynamic,
            })
        }
    }

    impl ScreenGeometryProvider for FakeGeometry {
        fn global_rect(&self, _source: CaptureSource) -> Option<Rect> {
            *self.rect.lock()
        }

        fn is_dynamic(&self, _source: CaptureSource) -> bool {
            self.dynamic
        }
    }

    #[test]
    fn noop_logger_returns_empty_track() {
        let mut l = NoopInputLogger::new();
        l.start(CaptureSource::Display { id: 1 }).unwrap();
        assert_eq!(l.stop(0.0), CursorTrack::default());
    }

    #[test]
    fn recorder_rebases_rounds_and_keeps_position_at_origin() {
        let time = Arc::new(Mutex::new(100.0));
        let pos = Arc::new(Mutex::new(Point2::new(250.0, 125.0)));
        let time_c = Arc::clone(&time);
        let pos_c = Arc::clone(&pos);
        let rec = CursorRecorder::new(
            CaptureSource::Display { id: 1 },
            FakeGeometry::new(Some(Rect::new(0.0, 0.0, 1000.0, 500.0)), false),
            SamplingSchedule::new(120.0),
            Arc::new(move || Some(*pos_c.lock())),
            Arc::new(move || *time_c.lock()),
        );
        for (t, x) in [
            (100.0, 250.0),
            (100.01, 300.0),
            (100.02, 500.0),
            (100.03, 600.0),
            (100.04, 1000.0),
        ] {
            *time.lock() = t;
            *pos.lock() = Point2::new(x, 125.0);
            rec.tick();
        }
        let track = rec.finish(100.015, 120.0);
        assert_eq!(track.sample_rate, 120.0);
        let ts: Vec<f64> = track.samples.iter().map(|s| s.t).collect();
        assert_eq!(ts, [0.0, 0.005, 0.015, 0.025]);
        assert_eq!(track.samples[0], CursorSample::new(0.0, 0.3, 0.25));
        assert_eq!(track.samples.last().map(|s| s.x), Some(1.0));
    }

    #[test]
    fn recorder_drops_clicks_before_origin_and_sorts_rest() {
        let rec = CursorRecorder::new(
            CaptureSource::Display { id: 1 },
            FakeGeometry::new(Some(Rect::new(0.0, 0.0, 1000.0, 500.0)), false),
            SamplingSchedule::new(120.0),
            Arc::new(|| Some(Point2::ZERO)),
            Arc::new(|| 0.0),
        );
        rec.click(
            Point2::new(500.0, 250.0),
            9.9,
            MouseButton::Left,
            ClickKind::Down,
        );
        rec.click(
            Point2::new(500.0, 250.0),
            11.0,
            MouseButton::Right,
            ClickKind::Up,
        );
        rec.click(
            Point2::new(100.0, 50.0),
            10.5,
            MouseButton::Left,
            ClickKind::Down,
        );
        let t = rec.finish(10.0, 120.0);
        assert_eq!(t.clicks.len(), 2);
        assert_eq!(t.clicks.iter().map(|c| c.t).collect::<Vec<_>>(), [0.5, 1.0]);
        assert_eq!(
            t.clicks[0],
            ClickEvent::new(0.5, 0.1, 0.1, MouseButton::Left, ClickKind::Down)
        );
        assert_eq!(t.clicks[1].button, MouseButton::Right);
        assert_eq!(t.clicks[1].kind, ClickKind::Up);
    }

    #[test]
    fn recorder_tracks_moving_window_and_keeps_last_rect_when_disappeared() {
        let geo = FakeGeometry::new(Some(Rect::new(0.0, 0.0, 200.0, 100.0)), true);
        let time = Arc::new(Mutex::new(1.0));
        let time_c = Arc::clone(&time);
        let rec = CursorRecorder::new(
            CaptureSource::Window { id: 7 },
            Arc::clone(&geo) as Arc<dyn ScreenGeometryProvider>,
            SamplingSchedule::new(120.0),
            Arc::new(|| Some(Point2::new(150.0, 50.0))),
            Arc::new(move || *time_c.lock()),
        );
        rec.tick();
        *geo.rect.lock() = Some(Rect::new(100.0, 0.0, 200.0, 100.0));
        *time.lock() = 1.0 + 1.0 / 120.0;
        rec.tick();
        rec.refresh_rect();
        *time.lock() = 1.0 + 2.0 / 120.0;
        rec.tick();
        let track = rec.finish(0.0, 120.0);
        assert_eq!(
            track.samples.iter().map(|s| s.x).collect::<Vec<_>>(),
            [0.75, 0.75, 0.25]
        );

        // Window disappears: last rect is preserved.
        *geo.rect.lock() = None;
        rec.refresh_rect();
        rec.tick();
        assert_eq!(rec.sample_count(), 4);
    }

    #[test]
    fn recorder_skips_ticks_without_position() {
        let rec = CursorRecorder::new(
            CaptureSource::Display { id: 1 },
            FakeGeometry::new(Some(Rect::new(0.0, 0.0, 100.0, 100.0)), false),
            SamplingSchedule::new(120.0),
            Arc::new(|| None),
            Arc::new(|| 1.0),
        );
        rec.tick();
        assert_eq!(rec.sample_count(), 0);
    }

    #[test]
    fn stop_right_after_start_leaves_nothing_running_and_can_restart() {
        let mut logger = SystemInputLogger::new(SystemInputLoggerConfig {
            geometry: FakeGeometry::new(Some(Rect::new(0.0, 0.0, 100.0, 100.0)), false),
            position: Some(Arc::new(|| Some(Point2::new(50.0, 50.0)))),
            monitor_clicks: false,
            log_cursor_shapes: false,
            ..Default::default()
        });
        logger.start(CaptureSource::Display { id: 1 }).unwrap();
        let t = logger.stop(0.0);
        assert!(!t.samples.is_empty());
        assert_eq!(logger.stop(0.0), CursorTrack::default());

        logger.start(CaptureSource::Display { id: 1 }).unwrap();
        let _ = logger.stop(0.0);
    }

    #[test]
    fn logger_stores_shapes_from_injected_sampler() {
        let script = [
            CursorShape::Arrow,
            CursorShape::Arrow,
            CursorShape::IBeam,
            CursorShape::IBeam,
            CursorShape::PointingHand,
        ];
        let idx = Arc::new(AtomicUsize::new(0));
        let idx_c = Arc::clone(&idx);

        let mut logger = SystemInputLogger::new(SystemInputLoggerConfig {
            geometry: FakeGeometry::new(Some(Rect::new(0.0, 0.0, 100.0, 100.0)), false),
            position: Some(Arc::new(|| Some(Point2::new(50.0, 50.0)))),
            monitor_clicks: false,
            log_cursor_shapes: true,
            shape_rate: 200.0,
            shape_sampler: Some(Arc::new(move || {
                let i = idx_c.fetch_add(1, Ordering::Relaxed);
                Some(script[i.min(script.len() - 1)])
            })),
            ..Default::default()
        });
        logger.start(CaptureSource::Display { id: 1 }).unwrap();
        for _ in 0..200 {
            if idx.load(Ordering::Relaxed) >= script.len() {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        let track = logger.stop(0.0);
        assert_eq!(
            track.shapes.iter().map(|s| s.shape).collect::<Vec<_>>(),
            [
                CursorShape::Arrow,
                CursorShape::IBeam,
                CursorShape::PointingHand
            ]
        );
    }

    #[test]
    fn ten_minute_track_stays_compact_and_roundtrips() {
        let samples: Vec<CursorSample> = (0..72_000)
            .map(|i| CursorSample::new(f64::from(i) / 120.0, f64::from(i % 1000) / 1000.0, 0.5))
            .collect();
        let track = CursorTrack {
            samples,
            ..CursorTrack::default()
        };
        let bytes = serde_json::to_vec(&track).unwrap();
        assert!(bytes.len() < 4_500_000, "size={}", bytes.len());
        let back: CursorTrack = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(back.samples.len(), 72_000);
    }

    #[test]
    fn sampler_thread_hits_near_120hz_with_low_jitter() {
        let mut logger = SystemInputLogger::new(SystemInputLoggerConfig {
            geometry: FakeGeometry::new(Some(Rect::new(0.0, 0.0, 1920.0, 1080.0)), false),
            position: Some(Arc::new(|| Some(Point2::new(960.0, 540.0)))),
            monitor_clicks: false,
            log_cursor_shapes: false,
            ..Default::default()
        });
        let origin = host_now();
        logger.start(CaptureSource::Display { id: 1 }).unwrap();
        thread::sleep(Duration::from_millis(300));
        let track = logger.stop(origin);
        let times: Vec<f64> = track.samples.iter().map(|s| s.t).collect();
        let stats = SamplingStats::from_times(&times);
        assert!(
            track.samples.len() >= 30,
            "samples={} mean_rate={:.1}",
            track.samples.len(),
            stats.mean_rate
        );
        assert!(
            stats.mean_rate >= 100.0 && stats.mean_rate <= 135.0,
            "mean_rate={:.1} max_gap={:.4}",
            stats.mean_rate,
            stats.max_gap
        );
    }

    #[test]
    #[ignore = "device: samples live Windows cursor at 120 Hz for 500 ms"]
    fn live_logger_samples_near_120hz() {
        #[cfg(windows)]
        let Some(primary) = crate::win::primary_display_id() else {
            return;
        };
        #[cfg(windows)]
        if crate::win::cursor_pos().is_none() {
            // Locked / secure desktop returns ERROR_ACCESS_DENIED from GetCursorPos (documented
            // Windows limitation in PLAN.md §5 Stage 6). Skip cleanly.
            return;
        }
        #[cfg(not(windows))]
        let primary = 1u32;

        let mut logger = SystemInputLogger::new(SystemInputLoggerConfig {
            monitor_clicks: false,
            ..Default::default()
        });
        let origin = host_now();
        logger
            .start(CaptureSource::Display { id: primary })
            .unwrap();
        thread::sleep(Duration::from_millis(500));
        let track = logger.stop(origin);
        let times: Vec<f64> = track.samples.iter().map(|s| s.t).collect();
        let stats = SamplingStats::from_times(&times);
        assert!(
            track.samples.len() >= 50,
            "samples={} mean_rate={:.1}",
            track.samples.len(),
            stats.mean_rate
        );
        assert!(
            stats.mean_rate >= 100.0 && stats.mean_rate <= 135.0,
            "mean_rate={:.1} max_gap={:.4}",
            stats.mean_rate,
            stats.max_gap
        );
    }
}
