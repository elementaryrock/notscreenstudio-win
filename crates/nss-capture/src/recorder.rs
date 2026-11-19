//! Recording sessions, stream planning and WGC screen capture orchestration.
//!
//! Port of upstream `NSSCapture/ScreenRecorder.swift`,
//! `NSSCapture/ScreenCaptureRecordingSession.swift`, and
//! `NSSCapture/NoopRecordingSession.swift`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use nss_core::{
    NssError, PixelSize, RecordingAssets, RecordingOptions, RecordingSession, RecordingState, Rect,
    Size2, Timestamp,
};
use parking_lot::Mutex;

use crate::capture_filter::{plan as plan_for, FilterPlan, Target};
use crate::capture_math::{
    clamped_area, host_now, pixel_size, video_bitrate, CaptureFilePlan, SharedOrigin, FRAME_RATE,
};
use crate::device::{CameraRecorder, MicRecorder, SystemAudioRecorder};
use crate::frame_gate::FrameGate;
use crate::pause_clock::{PauseClock, PauseTimeline, PausedFrameHold};
use crate::permissions::CapturePermissions;
use crate::state_hub::{CaptureEvent, Component, EventHub, Listener, StateHub};
use crate::writer::{RawVideoWriter, VideoWriterConfig};

/// Everything needed to configure the screen stream, computed without touching hardware.
///
/// Exact port of upstream `ScreenStreamPlan`.
#[derive(Debug, Clone, PartialEq)]
pub struct ScreenStreamPlan {
    pub pixel_size: PixelSize,
    /// Optional crop rect in display points (origin top-left).
    pub source_rect: Option<Rect>,
    pub frame_rate: u32,
    pub captures_system_audio: bool,
    pub is_window: bool,
}

impl ScreenStreamPlan {
    #[must_use]
    pub fn bitrate(&self) -> u64 {
        u64::from(video_bitrate(
            self.pixel_size.width,
            self.pixel_size.height,
            f64::from(self.frame_rate),
            0.12,
        ))
    }

    pub fn make(
        content_points: Size2,
        scale: f64,
        area: Option<Rect>,
        is_window: bool,
        system_audio: bool,
    ) -> Result<Self, NssError> {
        let mut rect = None;
        let mut points = content_points;
        if let Some(area) = area {
            let clamped = clamped_area(area, content_points).ok_or_else(|| {
                NssError::Unsupported("The selected area is outside the display.".into())
            })?;
            rect = Some(clamped);
            points = clamped.size;
        }
        Ok(Self {
            pixel_size: pixel_size(points, scale),
            source_rect: rect,
            frame_rate: FRAME_RATE,
            captures_system_audio: system_audio,
            is_window,
        })
    }
}

/// Extension of [`RecordingSession`] that supports warming up before a countdown, pausing without
/// leaving gaps, refreshing window exclusions, and subscribing to state / capture-event hubs.
///
/// Mirrors upstream `WarmableRecordingSession` + `PausableRecordingSession`.
pub trait WarmableRecordingSession: RecordingSession {
    /// Opens capture sources and holds frames back until [`Self::begin_recording`].
    fn prepare(&mut self, options: RecordingOptions, bundle: &Path) -> Result<(), NssError>;

    /// Opens the countdown gate and returns once the first screen frame establishes `t = 0`.
    fn begin_recording(&mut self) -> Result<(), NssError>;

    /// Abandons a prepared session and deletes any files it created.
    fn cancel_prepared(&mut self);

    fn pause(&mut self) -> Result<(), NssError>;
    fn resume(&mut self) -> Result<(), NssError>;
    fn pause_timeline(&self) -> PauseTimeline;

    /// Re-applies `WDA_EXCLUDEFROMCAPTURE` to our own windows (e.g. after HUDs appear).
    fn refresh_exclusions(&self);

    fn subscribe_state(&self, listener: Listener<RecordingState>) -> u64;
    fn unsubscribe_state(&self, id: u64);
    fn subscribe_events(&self, listener: Listener<CaptureEvent>) -> u64;
    fn unsubscribe_events(&self, id: u64);
}

/// Callback returning the live HWNDs owned by this application so they are excluded from capture.
pub type OwnWindowsProvider = Arc<dyn Fn() -> Vec<u64> + Send + Sync>;

/// No-op session for tests and headless flows (port of upstream `NoopRecordingSession`).
pub struct NoopRecordingSession {
    hub: StateHub,
    events: EventHub,
    pause_clock: PauseClock,
    pending_size: PixelSize,
    start_host_time: f64,
    awaiting_begin: bool,
}

impl Default for NoopRecordingSession {
    fn default() -> Self {
        Self::new()
    }
}

impl NoopRecordingSession {
    #[must_use]
    pub fn new() -> Self {
        Self {
            hub: StateHub::new(),
            events: EventHub::new(),
            pause_clock: PauseClock::new(),
            pending_size: PixelSize::new(1920, 1080),
            start_host_time: 0.0,
            awaiting_begin: false,
        }
    }

    pub fn with_pixel_size(mut self, size: PixelSize) -> Self {
        self.pending_size = size;
        self
    }
}

impl RecordingSession for NoopRecordingSession {
    fn preflight(&self, _options: &RecordingOptions) -> Result<(), NssError> {
        Ok(())
    }

    fn start(&mut self, _options: RecordingOptions, _bundle: &Path) -> Result<(), NssError> {
        self.start_host_time = host_now();
        self.pause_clock = PauseClock::new();
        self.awaiting_begin = false;
        self.hub.set(RecordingState::Recording {
            started_at: Timestamp::now(),
        });
        Ok(())
    }

    fn stop(&mut self) -> Result<RecordingAssets, NssError> {
        if !matches!(self.hub.current(), RecordingState::Recording { .. }) {
            return Err(NssError::Unsupported("No recording is in progress.".into()));
        }
        let end = self.pause_clock.shift(host_now());
        let duration = (end - self.start_host_time).max(0.0);
        self.hub.set(RecordingState::Finished {});
        let mut assets = RecordingAssets::new(self.pending_size, f64::from(FRAME_RATE), duration);
        assets.start_host_time = self.start_host_time;
        Ok(assets)
    }

    fn cancel(&mut self) {
        self.awaiting_begin = false;
        self.hub.set(RecordingState::Idle {});
    }

    fn state(&self) -> RecordingState {
        self.hub.current()
    }
}

impl WarmableRecordingSession for NoopRecordingSession {
    fn prepare(&mut self, _options: RecordingOptions, _bundle: &Path) -> Result<(), NssError> {
        self.awaiting_begin = true;
        self.pause_clock = PauseClock::new();
        self.hub.set(RecordingState::Preparing {});
        Ok(())
    }

    fn begin_recording(&mut self) -> Result<(), NssError> {
        if !self.awaiting_begin {
            return Err(NssError::Unsupported(
                "No prepared recording to start.".into(),
            ));
        }
        self.awaiting_begin = false;
        self.start_host_time = host_now();
        self.hub.set(RecordingState::Recording {
            started_at: Timestamp::now(),
        });
        Ok(())
    }

    fn cancel_prepared(&mut self) {
        if self.awaiting_begin {
            self.awaiting_begin = false;
            self.hub.set(RecordingState::Idle {});
        }
    }

    fn pause(&mut self) -> Result<(), NssError> {
        if !matches!(self.hub.current(), RecordingState::Recording { .. }) {
            return Err(NssError::Unsupported("No recording is in progress.".into()));
        }
        self.pause_clock.pause(host_now());
        Ok(())
    }

    fn resume(&mut self) -> Result<(), NssError> {
        if !matches!(self.hub.current(), RecordingState::Recording { .. }) {
            return Err(NssError::Unsupported("No recording is in progress.".into()));
        }
        self.pause_clock.resume(host_now());
        Ok(())
    }

    fn pause_timeline(&self) -> PauseTimeline {
        self.pause_clock.snapshot()
    }

    fn refresh_exclusions(&self) {}

    fn subscribe_state(&self, listener: Listener<RecordingState>) -> u64 {
        self.hub.subscribe(listener)
    }

    fn unsubscribe_state(&self, id: u64) {
        self.hub.unsubscribe(id);
    }

    fn subscribe_events(&self, listener: Listener<CaptureEvent>) -> u64 {
        self.events.subscribe(listener)
    }

    fn unsubscribe_events(&self, id: u64) {
        self.events.unsubscribe(id);
    }
}

type RawFrame = (u32, u32, Vec<u8>);

/// Active screen stream state driven by WGC callbacks.
struct ActiveScreen {
    #[cfg(windows)]
    wgc: Option<crate::win::wgc::WgcCaptureHandle>,
    writer: Arc<Mutex<Option<RawVideoWriter>>>,
    gate: Arc<Mutex<FrameGate<RawFrame>>>,
    paused_hold: Arc<Mutex<PausedFrameHold<Vec<u8>>>>,
    origin: Arc<SharedOrigin>,
    pause: Arc<PauseClock>,
    frames_written: Arc<AtomicUsize>,
    has_first_frame: Arc<AtomicBool>,
    ended_host: Arc<Mutex<Option<f64>>>,
    target_width: u32,
    target_height: u32,
}

impl ActiveScreen {
    fn arm(&self) {
        let now = host_now();
        let held = {
            let mut g = self.gate.lock();
            let h = g.arm(now);
            if h.is_some() {
                let _ = g.accept(now);
            }
            h
        };
        if let Some((fw, fh, raw_bgra)) = held {
            let bgra = if fw == self.target_width && fh == self.target_height {
                raw_bgra
            } else {
                fit_bgra_into(&raw_bgra, fw, fh, self.target_width, self.target_height)
            };
            self.emit_frame(bgra, now);
        }
    }

    fn resume(&self) {
        let now = host_now();
        self.pause.resume(now);
        if let Some(frame_bytes) = self.paused_hold.lock().take() {
            self.emit_frame(frame_bytes, now);
        }
    }

    fn emit_frame(&self, bgra: Vec<u8>, host_time: f64) {
        let expected = (self.target_width as usize) * (self.target_height as usize) * 4;
        if bgra.len() != expected {
            return;
        }
        let Some(_shifted) = self.pause.retime(host_time) else {
            self.paused_hold.lock().hold(bgra);
            return;
        };
        let _ = self.origin.set_if_unset(host_time);
        if let Some(ref w) = *self.writer.lock() {
            if w.write_frame(bgra).is_ok() {
                self.frames_written.fetch_add(1, Ordering::Relaxed);
                self.has_first_frame.store(true, Ordering::SeqCst);
            }
        }
    }

    fn stop(mut self, stop_host: f64) -> Result<(usize, f64), NssError> {
        #[cfg(windows)]
        if let Some(mut h) = self.wgc.take() {
            h.stop();
        }
        let writer = self.writer.lock().take();
        let written = if let Some(w) = writer {
            w.finish()?
        } else {
            self.frames_written.load(Ordering::Relaxed)
        };
        let raw_end = self.ended_host.lock().unwrap_or(stop_host);
        let end_host = self.pause.shift(raw_end);
        Ok((written, end_host))
    }
}

/// Real Windows `RecordingSession`: WGC screen/window/area + optional WASAPI mic + Media Foundation
/// camera, sharing a single `SharedOrigin` (t = 0 at first delivered screen frame) and `PauseClock`.
pub struct ScreenCaptureRecordingSession {
    hub: Arc<StateHub>,
    events: Arc<EventHub>,
    own_windows: OwnWindowsProvider,

    screen: Option<ActiveScreen>,
    mic: Option<MicRecorder>,
    system_audio: Option<SystemAudioRecorder>,
    camera: Option<CameraRecorder>,
    origin: Arc<SharedOrigin>,
    pause: Arc<PauseClock>,
    plan: Option<ScreenStreamPlan>,
    files: Option<CaptureFilePlan>,
    bundle_dir: Option<PathBuf>,
    created_files: Vec<PathBuf>,
    awaiting_begin: bool,
    last_plan: Option<FilterPlan>,
    fatal_error: Arc<Mutex<Option<NssError>>>,
}

impl Default for ScreenCaptureRecordingSession {
    fn default() -> Self {
        Self::new(Arc::new(Vec::new))
    }
}

impl ScreenCaptureRecordingSession {
    #[must_use]
    pub fn new(own_windows: OwnWindowsProvider) -> Self {
        Self {
            hub: Arc::new(StateHub::new()),
            events: Arc::new(EventHub::new()),
            own_windows,
            screen: None,
            mic: None,
            system_audio: None,
            camera: None,
            origin: Arc::new(SharedOrigin::new()),
            pause: Arc::new(PauseClock::new()),
            plan: None,
            files: None,
            bundle_dir: None,
            created_files: Vec::new(),
            awaiting_begin: false,
            last_plan: None,
            fatal_error: Arc::new(Mutex::new(None)),
        }
    }

    fn prepare_inner(
        &mut self,
        options: RecordingOptions,
        bundle: &Path,
        armed: bool,
    ) -> Result<(), NssError> {
        if matches!(
            self.hub.current(),
            RecordingState::Preparing {}
                | RecordingState::Recording { .. }
                | RecordingState::Stopping {}
        ) {
            return Err(NssError::Unsupported(
                "A recording is already in progress.".into(),
            ));
        }

        self.hub.set(RecordingState::Preparing {});
        self.discard_recorders();
        self.files = None;
        self.plan = None;
        self.bundle_dir = None;
        self.created_files.clear();
        *self.fatal_error.lock() = None;
        self.awaiting_begin = false;

        match self.begin_inner(options, bundle, armed) {
            Ok(()) => Ok(()),
            Err(err) => {
                self.teardown_after_failed_start();
                self.hub.set(RecordingState::Failed {
                    message: err.user_message().to_owned(),
                });
                Err(err)
            }
        }
    }

    fn begin_inner(
        &mut self,
        options: RecordingOptions,
        bundle: &Path,
        armed: bool,
    ) -> Result<(), NssError> {
        CapturePermissions::ensure(&options)?;
        std::fs::create_dir_all(bundle).map_err(|e| {
            NssError::Io(format!(
                "Could not create recording bundle {}: {e}",
                bundle.display()
            ))
        })?;

        self.bundle_dir = Some(bundle.to_path_buf());
        self.origin = Arc::new(SharedOrigin::new());
        self.pause = Arc::new(PauseClock::new());

        let files = CaptureFilePlan::new(&options);
        self.files = Some(files.clone());

        #[cfg(windows)]
        {
            self.begin_windows(options, bundle, files, armed)
        }
        #[cfg(not(windows))]
        {
            let _ = (options, files, armed);
            Err(NssError::Unsupported(
                "Screen capture is only available on Windows.".into(),
            ))
        }
    }

    #[cfg(windows)]
    fn begin_windows(
        &mut self,
        options: RecordingOptions,
        bundle: &Path,
        files: CaptureFilePlan,
        armed: bool,
    ) -> Result<(), NssError> {
        let enum_monitors = crate::win::enumerate::enumerate_monitors();
        let monitors: Vec<crate::win::Monitor> =
            enum_monitors.iter().map(|m| m.monitor.clone()).collect();
        let windows = crate::win::enumerate::enumerate_windows();
        let snapshot = crate::win::snapshot(&monitors, &windows);
        let own_pid = std::process::id();
        let own_app = crate::win::enumerate::current_app_name();
        let own_hwnds = (self.own_windows)();

        let filter_plan = plan_for(
            options.source,
            &snapshot,
            own_pid,
            own_app.as_deref(),
            &own_hwnds,
        )?;

        // Apply WDA_EXCLUDEFROMCAPTURE to all of our own windows (ADR 0007).
        if !filter_plan.is_window {
            let mut to_exclude = own_hwnds;
            to_exclude.extend_from_slice(&filter_plan.exclusion.window_handles);
            to_exclude.sort_unstable();
            to_exclude.dedup();
            let _ = crate::win::exclusion::exclude_all(&to_exclude);
        }

        let (wgc_target, content_points, scale) = match filter_plan.target {
            Target::Display(display_id) => {
                let em = enum_monitors
                    .iter()
                    .find(|m| m.monitor.id == display_id)
                    .ok_or_else(|| {
                        NssError::Unsupported("The selected display is no longer connected.".into())
                    })?;
                let scale = em.monitor.scale();
                // Virtual-desktop monitor rect is in physical pixels when per-monitor DPI aware;
                // convert to display points so `ScreenStreamPlan::make` computes identical sizes.
                let content_points = Size2::new(
                    em.monitor.rect.width() / scale,
                    em.monitor.rect.height() / scale,
                );
                let crop = filter_plan.area.and_then(|a| {
                    let clamped = clamped_area(a, content_points)?;
                    Some(crate::win::wgc::CropPixels {
                        x: (clamped.min_x() * scale).round().max(0.0) as u32,
                        y: (clamped.min_y() * scale).round().max(0.0) as u32,
                        width: crate::capture_math::even_pixels(clamped.width() * scale),
                        height: crate::capture_math::even_pixels(clamped.height() * scale),
                    })
                });
                (
                    crate::win::wgc::WgcTarget::Monitor {
                        hmonitor: em.hmonitor,
                        crop,
                    },
                    content_points,
                    scale,
                )
            }
            Target::Window(hwnd) => {
                let bounds = crate::win::enumerate::window_bounds(hwnd)
                    .ok_or_else(|| NssError::Io("The captured window was closed.".into()))?;
                (
                    crate::win::wgc::WgcTarget::Window { hwnd },
                    Size2::new(bounds.width().max(2.0), bounds.height().max(2.0)),
                    1.0,
                )
            }
        };

        let mut stream_plan = ScreenStreamPlan::make(
            content_points,
            scale,
            filter_plan.area,
            filter_plan.is_window,
            options.capture_system_audio,
        )?;
        self.last_plan = Some(filter_plan);

        // Start optional devices first (they warm up slowly and hold until SharedOrigin is set).
        if let (Some(mic_id), Some(mic_name)) = (options.mic_device_id.clone(), files.mic.clone()) {
            let path = files.path(&mic_name, bundle);
            self.created_files.push(path.clone());
            let events = Arc::clone(&self.events);
            let recorder = MicRecorder::start(
                mic_id,
                path,
                Arc::clone(&self.origin),
                Arc::clone(&self.pause),
                move |err| events.send(CaptureEvent::device(Component::Microphone, err)),
            )?;
            self.mic = Some(recorder);
        }

        if let Some(sys_name) = files.system_audio.clone() {
            let path = files.path(&sys_name, bundle);
            let events = Arc::clone(&self.events);
            match SystemAudioRecorder::start(
                path.clone(),
                Arc::clone(&self.origin),
                Arc::clone(&self.pause),
                move |err| events.send(CaptureEvent::device(Component::SystemAudio, err)),
            ) {
                Ok(recorder) => {
                    self.created_files.push(path);
                    self.system_audio = Some(recorder);
                }
                Err(err) => {
                    // Best-effort: headless/no-speaker machines can still record screen cleanly.
                    self.events
                        .send(CaptureEvent::device(Component::SystemAudio, err));
                }
            }
        }

        if let (Some(cam_id), Some(cam_name)) =
            (options.camera_device_id.clone(), files.camera.clone())
        {
            let path = files.path(&cam_name, bundle);
            self.created_files.push(path.clone());
            let events = Arc::clone(&self.events);
            let recorder = CameraRecorder::start(
                cam_id,
                path,
                Arc::clone(&self.origin),
                Arc::clone(&self.pause),
                move |err| events.send(CaptureEvent::device(Component::Camera, err)),
            )?;
            self.camera = Some(recorder);
        }

        // Start WGC + video writer.
        let screen_path = files.path(&files.screen, bundle);
        self.created_files.push(screen_path.clone());

        let writer_slot: Arc<Mutex<Option<RawVideoWriter>>> = Arc::new(Mutex::new(None));
        // Always start the gate unarmed so frames arriving while FFmpeg spawns are held (preserving
        // their raw dimensions) and flushed cleanly once the writer is ready and `arm()` runs.
        let gate = Arc::new(Mutex::new(FrameGate::<(u32, u32, Vec<u8>)>::new(false)));
        let paused_hold = Arc::new(Mutex::new(PausedFrameHold::<Vec<u8>>::new()));
        let frames_written = Arc::new(AtomicUsize::new(0));
        let has_first_frame = Arc::new(AtomicBool::new(false));
        let ended_host = Arc::new(Mutex::new(None));

        let origin_cb = Arc::clone(&self.origin);
        let pause_cb = Arc::clone(&self.pause);
        let gate_cb = Arc::clone(&gate);
        let hold_cb = Arc::clone(&paused_hold);
        let writer_cb = Arc::clone(&writer_slot);
        let written_cb = Arc::clone(&frames_written);
        let first_cb = Arc::clone(&has_first_frame);
        let events_frame = Arc::clone(&self.events);
        let fatal_frame = Arc::clone(&self.fatal_error);
        let hub_frame = Arc::clone(&self.hub);

        let events_closed = Arc::clone(&self.events);
        let fatal_closed = Arc::clone(&self.fatal_error);
        let hub_closed = Arc::clone(&self.hub);
        let ended_closed = Arc::clone(&ended_host);

        let expected_size = Arc::new(Mutex::new(stream_plan.pixel_size));
        let expected_cb = Arc::clone(&expected_size);

        let wgc = crate::win::wgc::WgcCaptureHandle::start(
            wgc_target,
            move |frame| {
                let accepted = {
                    let mut g = gate_cb.lock();
                    if !g.is_armed() {
                        g.hold((frame.width, frame.height, frame.bgra));
                        None
                    } else if g.accept(frame.host_time) {
                        Some((frame.width, frame.height, frame.bgra))
                    } else {
                        None
                    }
                };
                let Some((fw, fh, raw_bgra)) = accepted else {
                    return;
                };
                let target_sz = *expected_cb.lock();
                let payload = if fw == target_sz.width && fh == target_sz.height {
                    raw_bgra
                } else {
                    fit_bgra_into(&raw_bgra, fw, fh, target_sz.width, target_sz.height)
                };
                if pause_cb.retime(frame.host_time).is_none() {
                    hold_cb.lock().hold(payload);
                    return;
                }
                let _ = origin_cb.set_if_unset(frame.host_time);
                let guard = writer_cb.lock();
                if let Some(ref w) = *guard {
                    match w.write_frame(payload) {
                        Ok(()) => {
                            written_cb.fetch_add(1, Ordering::Relaxed);
                            first_cb.store(true, Ordering::SeqCst);
                        }
                        Err(err) => {
                            *fatal_frame.lock() = Some(err.clone());
                            events_frame.send(CaptureEvent::screen(err.clone()));
                            if matches!(hub_frame.current(), RecordingState::Recording { .. }) {
                                hub_frame.set(RecordingState::Failed {
                                    message: err.user_message().to_owned(),
                                });
                            }
                        }
                    }
                }
            },
            move |err| {
                *ended_closed.lock() = Some(host_now());
                *fatal_closed.lock() = Some(err.clone());
                events_closed.send(CaptureEvent::screen(err.clone()));
                if matches!(hub_closed.current(), RecordingState::Recording { .. }) {
                    hub_closed.set(RecordingState::Failed {
                        message: err.user_message().to_owned(),
                    });
                }
            },
        )?;

        // Align the stream plan to WGC's actual even pixel dimensions (handles DPI rounding).
        let actual_w = crate::capture_math::even_pixels(f64::from(wgc.initial_size.width));
        let actual_h = crate::capture_math::even_pixels(f64::from(wgc.initial_size.height));
        stream_plan.pixel_size = PixelSize::new(actual_w, actual_h);
        *expected_size.lock() = stream_plan.pixel_size;

        let video_cfg = VideoWriterConfig {
            width: actual_w,
            height: actual_h,
            fps: stream_plan.frame_rate,
            bitrate: stream_plan.bitrate(),
            output_path: screen_path,
        };
        let writer = RawVideoWriter::start(video_cfg)?;
        *writer_slot.lock() = Some(writer);

        self.plan = Some(stream_plan);
        let active = ActiveScreen {
            wgc: Some(wgc),
            writer: writer_slot,
            gate,
            paused_hold,
            origin: Arc::clone(&self.origin),
            pause: Arc::clone(&self.pause),
            frames_written,
            has_first_frame: Arc::clone(&has_first_frame),
            ended_host,
            target_width: actual_w,
            target_height: actual_h,
        };
        if armed {
            active.arm();
        }
        self.screen = Some(active);

        if armed {
            self.wait_for_first_frame(&has_first_frame)?;
        }
        Ok(())
    }

    fn wait_for_first_frame(&self, flag: &AtomicBool) -> Result<(), NssError> {
        let deadline = Instant::now() + Duration::from_secs(3);
        while !flag.load(Ordering::SeqCst) && Instant::now() < deadline {
            if let Some(err) = self.fatal_error.lock().clone() {
                return Err(err);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        if let Some(err) = self.fatal_error.lock().clone() {
            return Err(err);
        }
        if !flag.load(Ordering::SeqCst) {
            return Err(NssError::Io(
                "No screen frames arrived. Check that the selected source is visible and try again."
                    .into(),
            ));
        }
        Ok(())
    }

    fn discard_recorders(&mut self) {
        let now = host_now();
        if let Some(s) = self.screen.take() {
            let _ = s.stop(now);
        }
        if let Some(mut m) = self.mic.take() {
            let _ = m.stop();
        }
        if let Some(mut s) = self.system_audio.take() {
            let _ = s.stop();
        }
        if let Some(mut c) = self.camera.take() {
            let _ = c.stop();
        }
    }

    fn teardown_after_failed_start(&mut self) {
        self.discard_recorders();
        for p in self.created_files.drain(..) {
            let _ = std::fs::remove_file(p);
        }
    }
}

impl RecordingSession for ScreenCaptureRecordingSession {
    fn preflight(&self, options: &RecordingOptions) -> Result<(), NssError> {
        CapturePermissions::ensure(options)
    }

    fn start(&mut self, options: RecordingOptions, bundle: &Path) -> Result<(), NssError> {
        self.prepare_inner(options, bundle, true)?;
        self.hub.set(RecordingState::Recording {
            started_at: Timestamp::now(),
        });
        Ok(())
    }

    fn stop(&mut self) -> Result<RecordingAssets, NssError> {
        if self.awaiting_begin {
            return Err(NssError::Unsupported(
                "The recording has not started yet.".into(),
            ));
        }
        let (Some(screen), Some(plan), Some(files)) =
            (self.screen.take(), self.plan.take(), self.files.take())
        else {
            return Err(NssError::Unsupported("No recording is in progress.".into()));
        };
        let mic = self.mic.take();
        let system_audio = self.system_audio.take();
        let camera = self.camera.take();
        let created = std::mem::take(&mut self.created_files);
        let bundle_dir = self.bundle_dir.take();

        self.hub.set(RecordingState::Stopping {});
        let stop_host = host_now();

        let screen_res = screen.stop(stop_host);
        let mic_has = mic.and_then(|mut m| m.stop().ok()).unwrap_or(false);
        let sys_has = system_audio
            .and_then(|mut s| s.stop().ok())
            .unwrap_or(false);
        let cam_has = camera.and_then(|mut c| c.stop().ok()).unwrap_or(false);

        let (frames, end_host) = match screen_res {
            Ok((n, end)) if n > 0 => (n, end),
            Ok(_) => {
                let err = self
                    .fatal_error
                    .lock()
                    .clone()
                    .unwrap_or_else(|| NssError::Io("No screen frames were captured.".into()));
                for p in created {
                    let _ = std::fs::remove_file(p);
                }
                self.hub.set(RecordingState::Failed {
                    message: err.user_message().to_owned(),
                });
                return Err(err);
            }
            Err(e) => {
                let err = self.fatal_error.lock().clone().unwrap_or(e);
                for p in created {
                    let _ = std::fs::remove_file(p);
                }
                self.hub.set(RecordingState::Failed {
                    message: err.user_message().to_owned(),
                });
                return Err(err);
            }
        };

        let _ = frames;
        let Some(origin) = self.origin.current() else {
            let err = NssError::Io("No screen frames were captured.".into());
            for p in created {
                let _ = std::fs::remove_file(p);
            }
            self.hub.set(RecordingState::Failed {
                message: err.user_message().to_owned(),
            });
            return Err(err);
        };

        if let Some(ref dir) = bundle_dir {
            if !mic_has {
                if let Some(ref name) = files.mic {
                    let _ = std::fs::remove_file(files.path(name, dir));
                }
            }
            if !sys_has {
                if let Some(ref name) = files.system_audio {
                    let _ = std::fs::remove_file(files.path(name, dir));
                }
            }
            if !cam_has {
                if let Some(ref name) = files.camera {
                    let _ = std::fs::remove_file(files.path(name, dir));
                }
            }
        }

        let mut assets = RecordingAssets::new(
            plan.pixel_size,
            f64::from(plan.frame_rate),
            origin.duration(end_host),
        );
        assets.start_host_time = origin.host_time;
        assets.screen_file = files.screen;
        assets.camera_file = if cam_has { files.camera } else { None };
        assets.mic_file = if mic_has { files.mic } else { None };

        self.hub.set(RecordingState::Finished {});
        Ok(assets)
    }

    fn cancel(&mut self) {
        self.awaiting_begin = false;
        self.teardown_after_failed_start();
        self.hub.set(RecordingState::Idle {});
    }

    fn state(&self) -> RecordingState {
        self.hub.current()
    }
}

impl WarmableRecordingSession for ScreenCaptureRecordingSession {
    fn prepare(&mut self, options: RecordingOptions, bundle: &Path) -> Result<(), NssError> {
        self.prepare_inner(options, bundle, false)?;
        self.awaiting_begin = true;
        Ok(())
    }

    fn begin_recording(&mut self) -> Result<(), NssError> {
        if !self.awaiting_begin {
            return Err(NssError::Unsupported(
                "No prepared recording to start.".into(),
            ));
        }
        let Some(ref screen) = self.screen else {
            return Err(NssError::Unsupported(
                "No prepared recording to start.".into(),
            ));
        };
        self.awaiting_begin = false;
        let fatal = self.fatal_error.lock().clone();
        if let Some(err) = fatal {
            self.teardown_after_failed_start();
            self.hub.set(RecordingState::Failed {
                message: err.user_message().to_owned(),
            });
            return Err(err);
        }
        let first_flag = Arc::clone(&screen.has_first_frame);
        screen.arm();
        if let Err(err) = self.wait_for_first_frame(&first_flag) {
            self.teardown_after_failed_start();
            self.hub.set(RecordingState::Failed {
                message: err.user_message().to_owned(),
            });
            return Err(err);
        }
        self.hub.set(RecordingState::Recording {
            started_at: Timestamp::now(),
        });
        Ok(())
    }

    fn cancel_prepared(&mut self) {
        if !self.awaiting_begin {
            return;
        }
        self.awaiting_begin = false;
        self.teardown_after_failed_start();
        self.hub.set(RecordingState::Idle {});
    }

    fn pause(&mut self) -> Result<(), NssError> {
        if !matches!(self.hub.current(), RecordingState::Recording { .. }) {
            return Err(NssError::Unsupported("No recording is in progress.".into()));
        }
        self.pause.pause(host_now());
        if let Some(ref screen) = self.screen {
            if let Some(ref w) = *screen.writer.lock() {
                w.set_paused(true);
            }
        }
        if let Some(ref cam) = self.camera {
            cam.set_paused(true);
        }
        Ok(())
    }

    fn resume(&mut self) -> Result<(), NssError> {
        if !matches!(self.hub.current(), RecordingState::Recording { .. }) {
            return Err(NssError::Unsupported("No recording is in progress.".into()));
        }
        if let Some(ref cam) = self.camera {
            cam.set_paused(false);
        }
        if let Some(ref screen) = self.screen {
            if let Some(ref w) = *screen.writer.lock() {
                w.set_paused(false);
            }
            screen.resume();
        } else {
            self.pause.resume(host_now());
        }
        Ok(())
    }

    fn pause_timeline(&self) -> PauseTimeline {
        self.pause.snapshot()
    }

    fn refresh_exclusions(&self) {
        if self.last_plan.as_ref().is_some_and(|p| p.is_window) {
            return;
        }
        #[cfg(windows)]
        {
            let mut hwnds = (self.own_windows)();
            let own_pid = std::process::id();
            for w in crate::win::enumerate::enumerate_windows() {
                if w.process_id == own_pid {
                    hwnds.push(w.handle);
                }
            }
            hwnds.sort_unstable();
            hwnds.dedup();
            let _ = crate::win::exclusion::exclude_all(&hwnds);
        }
    }

    fn subscribe_state(&self, listener: Listener<RecordingState>) -> u64 {
        self.hub.subscribe(listener)
    }

    fn unsubscribe_state(&self, id: u64) {
        self.hub.unsubscribe(id);
    }

    fn subscribe_events(&self, listener: Listener<CaptureEvent>) -> u64 {
        self.events.subscribe(listener)
    }

    fn unsubscribe_events(&self, id: u64) {
        self.events.unsubscribe(id);
    }
}

/// Copies a resized WGC frame into the fixed encoder canvas (top-left anchored, padded with
/// opaque black if smaller, clipped if larger) so a window resize mid-recording never stalls FFmpeg.
fn fit_bgra_into(src: &[u8], src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> Vec<u8> {
    let mut dst = vec![0u8; (dst_w as usize) * (dst_h as usize) * 4];
    for px in dst.chunks_exact_mut(4) {
        px[3] = 255;
    }
    let copy_w = src_w.min(dst_w) as usize;
    let copy_h = src_h.min(dst_h) as usize;
    let src_stride = (src_w as usize) * 4;
    let dst_stride = (dst_w as usize) * 4;
    let row_bytes = copy_w * 4;
    for y in 0..copy_h {
        let s_off = y * src_stride;
        let d_off = y * dst_stride;
        if s_off + row_bytes <= src.len() && d_off + row_bytes <= dst.len() {
            dst[d_off..d_off + row_bytes].copy_from_slice(&src[s_off..s_off + row_bytes]);
        }
    }
    dst
}

#[cfg(test)]
mod tests {
    use super::*;
    use nss_core::CaptureSource;

    #[test]
    fn display_plan_uses_native_pixels() {
        let p = ScreenStreamPlan::make(Size2::new(1512.0, 982.0), 2.0, None, false, false).unwrap();
        assert_eq!(p.pixel_size, PixelSize::new(3024, 1964));
        assert_eq!(p.source_rect, None);
        assert_eq!(p.frame_rate, 60);
    }

    #[test]
    fn area_plan_sets_source_rect() {
        let area = Rect::new(100.0, 50.0, 400.0, 300.0);
        let p = ScreenStreamPlan::make(Size2::new(1512.0, 982.0), 2.0, Some(area), false, false)
            .unwrap();
        assert_eq!(p.source_rect, Some(area));
        assert_eq!(p.pixel_size, PixelSize::new(800, 600));
    }

    #[test]
    fn area_outside_display_errors() {
        let err = ScreenStreamPlan::make(
            Size2::new(100.0, 100.0),
            1.0,
            Some(Rect::new(500.0, 500.0, 10.0, 10.0)),
            false,
            false,
        )
        .unwrap_err();
        assert!(matches!(err, NssError::Unsupported(_)));
    }

    #[test]
    fn noop_session_transitions() {
        let mut s = NoopRecordingSession::new();
        assert_eq!(s.state(), RecordingState::Idle {});
        let dir = tempfile::tempdir().unwrap();
        s.start(
            RecordingOptions::new(CaptureSource::Display { id: 1 }),
            dir.path(),
        )
        .unwrap();
        assert!(matches!(s.state(), RecordingState::Recording { .. }));
        let assets = s.stop().unwrap();
        assert_eq!(s.state(), RecordingState::Finished {});
        assert_eq!(assets.source_pixel_size, PixelSize::new(1920, 1080));
        assert!(assets.start_host_time > 0.0);
    }

    #[test]
    fn noop_session_warm_prepare_begin_and_cancel() {
        let mut s = NoopRecordingSession::new();
        let dir = tempfile::tempdir().unwrap();
        let opts = RecordingOptions::new(CaptureSource::Display { id: 1 });
        s.prepare(opts.clone(), dir.path()).unwrap();
        assert_eq!(s.state(), RecordingState::Preparing {});
        s.cancel_prepared();
        assert_eq!(s.state(), RecordingState::Idle {});

        s.prepare(opts, dir.path()).unwrap();
        s.begin_recording().unwrap();
        assert!(matches!(s.state(), RecordingState::Recording { .. }));
        s.pause().unwrap();
        s.resume().unwrap();
        assert_eq!(s.pause_timeline().intervals().len(), 1);
        let _ = s.stop().unwrap();
        assert_eq!(s.state(), RecordingState::Finished {});
    }

    #[test]
    fn real_session_stop_without_start_errors_without_changing_state() {
        let mut s = ScreenCaptureRecordingSession::default();
        assert_eq!(s.state(), RecordingState::Idle {});
        assert!(s.stop().is_err());
        assert!(s.stop().is_err());
        assert_eq!(s.state(), RecordingState::Idle {});
    }

    #[test]
    #[ignore = "device: captures live primary monitor for 1s using WGC + FFmpeg"]
    fn live_records_one_second_of_primary_display() {
        let list = crate::catalog::CaptureSourceCatalog::load();
        let primary = list
            .displays
            .first()
            .expect("at least one display connected");
        let dir = tempfile::tempdir().unwrap();
        let mut session = ScreenCaptureRecordingSession::default();
        session
            .start(RecordingOptions::new(primary.source()), dir.path())
            .unwrap();
        std::thread::sleep(Duration::from_millis(1200));
        let assets = session.stop().unwrap();
        assert!(assets.duration > 0.4, "duration={}", assets.duration);
        assert!(assets.start_host_time > 0.0);
        assert!(dir.path().join(&assets.screen_file).is_file());
    }

    #[test]
    #[ignore = "device: captures every connected monitor (including secondary / multi-GPU) via warm prepare + begin_recording"]
    fn live_records_all_connected_displays() {
        let list = crate::catalog::CaptureSourceCatalog::load();
        assert!(!list.displays.is_empty(), "at least one display connected");
        for disp in &list.displays {
            println!(
                "Display id={} name={:?} main={} frame={:?} pixel_size={}x{} scale={}",
                disp.id,
                disp.name,
                disp.is_main,
                disp.frame,
                disp.pixel_size.width,
                disp.pixel_size.height,
                disp.scale
            );
            let dir = tempfile::tempdir().unwrap();
            let mut session = ScreenCaptureRecordingSession::default();
            session
                .prepare(RecordingOptions::new(disp.source()), dir.path())
                .unwrap_or_else(|e| panic!("prepare failed for display {}: {e}", disp.name));
            std::thread::sleep(Duration::from_millis(800));
            session.begin_recording().unwrap_or_else(|e| {
                panic!("begin_recording failed for display {}: {e}", disp.name)
            });
            std::thread::sleep(Duration::from_millis(1200));
            let assets = session
                .stop()
                .unwrap_or_else(|e| panic!("stop failed for display {}: {e}", disp.name));
            let mov = dir.path().join(&assets.screen_file);
            let bytes = std::fs::metadata(&mov).map(|m| m.len()).unwrap_or(0);
            println!(
                "  -> recorded {}x{} duration={:.2}s file={} ({} bytes)",
                assets.source_pixel_size.width,
                assets.source_pixel_size.height,
                assets.duration,
                assets.screen_file,
                bytes
            );
            assert!(
                assets.duration > 0.4,
                "display {} short duration={}",
                disp.name,
                assets.duration
            );
            assert!(
                bytes > 4096,
                "display {} produced tiny screen.mov ({} bytes)",
                disp.name,
                bytes
            );
        }
    }
}
