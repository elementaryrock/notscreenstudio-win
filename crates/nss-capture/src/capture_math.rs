//! Pure, hardware-free helpers for configuring and timing a capture.
//!
//! Mirrors upstream `NSSCapture/CaptureMath.swift`. Everything here is unit-tested and touches no
//! Win32 API, so the rules the recorders obey can be checked without a display, a camera or an
//! encoder.

use std::path::Path;
use std::sync::Mutex;

use nss_core::{MediaTime, PixelSize, RecordingAssets, RecordingOptions, Rect, Size2};

/// Target capture rate. WGC delivers on the compositor's cadence and we sample `GetCursorPos` at
/// 120 Hz, but the recording file is written at this rate.
pub const FRAME_RATE: u32 = 60;

/// Round to the nearest even pixel count (hardware encoders want even dimensions), minimum 2.
#[must_use]
pub fn even_pixels(value: f64) -> u32 {
    let v = if value.is_finite() { value } else { 2.0 };
    ((v / 2.0).round() * 2.0).max(2.0) as u32
}

/// Native pixel size for a size in points at `scale` pixels per point.
#[must_use]
pub fn pixel_size(points: Size2, scale: f64) -> PixelSize {
    let s = if scale > 0.0 { scale } else { 1.0 };
    PixelSize::new(
        even_pixels(points.width * s),
        even_pixels(points.height * s),
    )
}

/// Clamp an area (display points, origin top-left) to the display bounds. `None` if nothing is left.
#[must_use]
pub fn clamped_area(rect: Rect, display_size: Size2) -> Option<Rect> {
    // A non-finite rect has no overlap with anything: `f64::max` would quietly swallow the NaN and
    // return a plausible-looking rectangle instead.
    if !([rect.min_x(), rect.min_y(), rect.max_x(), rect.max_y()]
        .iter()
        .all(|v| v.is_finite())
        && display_size.width.is_finite()
        && display_size.height.is_finite())
    {
        return None;
    }
    let bounds = Rect::new(0.0, 0.0, display_size.width, display_size.height);
    let x0 = rect.min_x().max(bounds.min_x());
    let y0 = rect.min_y().max(bounds.min_y());
    let x1 = rect.max_x().min(bounds.max_x());
    let y1 = rect.max_y().min(bounds.max_y());
    if !(x1 - x0 >= 2.0 && y1 - y0 >= 2.0) {
        return None;
    }
    Some(Rect::new(x0, y0, x1 - x0, y1 - y0))
}

/// Target video bitrate: high quality for screen content (lots of sharp text), clamped.
#[must_use]
pub fn video_bitrate(width: u32, height: u32, frame_rate: f64, bits_per_pixel: f64) -> u32 {
    let raw = f64::from(width) * f64::from(height) * frame_rate.max(1.0) * bits_per_pixel;
    raw.clamp(8_000_000.0, 200_000_000.0) as u32
}

/// Whether a running app belongs to this process (so it is excluded from the capture).
///
/// Upstream matched on PID and bundle ID. Windows has no bundle ID, so the second key is the
/// executable's file name, which plays the same role: one app process can spawn helper windows.
#[must_use]
pub fn is_own_app(
    process_id: u32,
    app_name: Option<&str>,
    own_pid: u32,
    own_app_name: Option<&str>,
) -> bool {
    if process_id == own_pid {
        return true;
    }
    match (app_name, own_app_name) {
        (Some(b), Some(o)) => !b.is_empty() && b.eq_ignore_ascii_case(o),
        _ => false,
    }
}

/// Whether a window should be offered in the source picker.
#[must_use]
pub fn is_pickable_window(
    is_on_screen: bool,
    is_cloaked: bool,
    is_tool: bool,
    has_app: bool,
    width: f64,
    height: f64,
) -> bool {
    is_on_screen && !is_cloaked && !is_tool && has_app && width >= 50.0 && height >= 50.0
}

/// Monotonic host clock in seconds, shared by every recorder and input logger of one session.
#[must_use]
pub fn host_now() -> f64 {
    nss_core::host_now()
}

/// Maps host-clock times onto the shared recording timeline (t = 0 is the first screen frame).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeOrigin {
    pub host_time: f64,
}

impl TimeOrigin {
    #[must_use]
    pub fn new(host_time: f64) -> Self {
        Self { host_time }
    }

    /// Seconds from the origin, never negative.
    #[must_use]
    pub fn relative(&self, host_seconds: f64) -> MediaTime {
        (host_seconds - self.host_time).max(0.0)
    }

    /// True when a sample ends at or before the origin and so must be dropped.
    #[must_use]
    pub fn is_entirely_before(&self, start: f64, duration: f64) -> bool {
        if duration > 0.0 {
            start + duration <= self.host_time
        } else {
            start < self.host_time
        }
    }

    /// Duration of a sample that ends at `end_host`, never negative.
    #[must_use]
    pub fn duration(&self, end_host: f64) -> MediaTime {
        (end_host - self.host_time).max(0.0)
    }
}

/// Thread-safe, write-once origin shared by all writers. Set by the first complete screen frame.
#[derive(Debug, Default)]
pub struct SharedOrigin {
    value: Mutex<Option<f64>>,
}

impl SharedOrigin {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn current(&self) -> Option<TimeOrigin> {
        self.lock().map(TimeOrigin::new)
    }

    /// Sets the origin if it is still unset. Returns true if this call set it.
    pub fn set_if_unset(&self, host_seconds: f64) -> bool {
        let mut guard = self.lock_mut();
        if guard.is_some() {
            return false;
        }
        *guard = Some(host_seconds);
        true
    }

    fn lock(&self) -> Option<f64> {
        *self.lock_mut()
    }

    fn lock_mut(&self) -> std::sync::MutexGuard<'_, Option<f64>> {
        self.value.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// File names inside the bundle for a given set of options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureFilePlan {
    pub screen: String,
    pub camera: Option<String>,
    pub mic: Option<String>,
}

impl CaptureFilePlan {
    #[must_use]
    pub fn new(options: &RecordingOptions) -> Self {
        Self {
            screen: RecordingAssets::DEFAULT_SCREEN_FILE.to_owned(),
            camera: options
                .camera_device_id
                .as_ref()
                .map(|_| RecordingAssets::DEFAULT_CAMERA_FILE.to_owned()),
            mic: options
                .mic_device_id
                .as_ref()
                .map(|_| RecordingAssets::DEFAULT_MIC_FILE.to_owned()),
        }
    }

    /// Path of `name` inside `bundle`.
    #[must_use]
    pub fn path(&self, name: &str, bundle: &Path) -> std::path::PathBuf {
        bundle.join(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn even_pixels_rounds_to_an_even_minimum_of_two() {
        assert_eq!(even_pixels(1920.0), 1920);
        assert_eq!(even_pixels(1919.6), 1920);
        assert_eq!(even_pixels(1918.4), 1918);
        assert_eq!(even_pixels(0.4), 2);
        assert_eq!(even_pixels(-30.0), 2);
        assert_eq!(even_pixels(f64::NAN), 2);
        assert_eq!(even_pixels(f64::INFINITY), 2);
    }

    #[test]
    fn pixel_size_scales_and_never_returns_zero() {
        let p = pixel_size(Size2::new(1280.0, 720.0), 1.5);
        assert_eq!((p.width, p.height), (1920, 1080));
        let q = pixel_size(Size2::new(1.0, 1.0), 0.0);
        assert_eq!((q.width, q.height), (2, 2));
    }

    #[test]
    fn clamped_area_keeps_the_overlap_only() {
        let display = Size2::new(1920.0, 1080.0);
        let r = clamped_area(Rect::new(1800.0, 1000.0, 400.0, 400.0), display).expect("overlap");
        assert_eq!((r.min_x(), r.min_y()), (1800.0, 1000.0));
        assert_eq!((r.width(), r.height()), (120.0, 80.0));
        // Negative origin (a monitor to the left) clamps to the display, not to zero width.
        let l = clamped_area(Rect::new(-100.0, -100.0, 300.0, 300.0), display).expect("overlap");
        assert_eq!((l.min_x(), l.min_y()), (0.0, 0.0));
        assert_eq!((l.width(), l.height()), (200.0, 200.0));
        // A sliver that still has two pixels on each axis is kept, not thrown away.
        let sliver = clamped_area(Rect::new(1900.0, 1070.0, 10.0, 10.0), display).expect("sliver");
        assert_eq!((sliver.min_x(), sliver.min_y()), (1900.0, 1070.0));
        assert_eq!((sliver.width(), sliver.height()), (10.0, 10.0));
        assert!(clamped_area(Rect::new(2000.0, 0.0, 100.0, 100.0), display).is_none());
        assert!(clamped_area(Rect::new(-500.0, 0.0, 400.0, 100.0), display).is_none());
        assert!(clamped_area(Rect::new(f64::NAN, 0.0, 400.0, 100.0), display).is_none());
    }

    #[test]
    fn bitrate_is_clamped_to_the_documented_range() {
        assert_eq!(video_bitrate(16, 16, 1.0, 0.12), 8_000_000);
        assert_eq!(video_bitrate(3840, 2160, 60.0, 0.12), 59_719_680);
        assert_eq!(video_bitrate(7680, 4320, 120.0, 0.12), 200_000_000);
    }

    #[test]
    fn own_app_matches_by_process_and_by_app_name() {
        assert!(is_own_app(500, None, 500, Some("notscreenstudio.exe")));
        assert!(is_own_app(
            700,
            Some("NotScreenStudio.EXE"),
            500,
            Some("notscreenstudio.exe")
        ));
        assert!(!is_own_app(700, Some("explorer.exe"), 500, None));
        assert!(!is_own_app(700, Some(""), 500, Some("")));
        assert!(!is_own_app(700, None, 500, Some("notscreenstudio.exe")));
    }

    #[test]
    fn only_real_windows_are_pickable() {
        assert!(is_pickable_window(true, false, false, true, 800.0, 600.0));
        assert!(!is_pickable_window(false, false, false, true, 800.0, 600.0));
        assert!(!is_pickable_window(true, true, false, true, 800.0, 600.0));
        assert!(!is_pickable_window(true, false, true, true, 800.0, 600.0));
        assert!(!is_pickable_window(true, false, false, false, 800.0, 600.0));
        assert!(!is_pickable_window(true, false, false, true, 40.0, 600.0));
        assert!(!is_pickable_window(true, false, false, true, 800.0, 49.0));
    }

    #[test]
    fn relative_times_never_go_negative() {
        let o = TimeOrigin::new(100.0);
        assert_eq!(o.relative(100.5), 0.5);
        assert_eq!(o.relative(99.0), 0.0);
        assert!(o.is_entirely_before(99.0, 0.5));
        assert!(!o.is_entirely_before(99.0, 2.0));
        assert!(o.is_entirely_before(99.5, 0.0));
        assert!(!o.is_entirely_before(100.0, 0.0));
        assert_eq!(o.duration(100.25), 0.25);
        assert_eq!(o.duration(50.0), 0.0);
    }

    #[test]
    fn the_origin_is_written_once() {
        let origin = SharedOrigin::new();
        assert!(origin.current().is_none());
        assert!(origin.set_if_unset(42.0));
        assert!(!origin.set_if_unset(43.0));
        assert_eq!(origin.current().map(|o| o.host_time), Some(42.0));
    }

    #[test]
    fn the_file_plan_follows_the_options() {
        let mut options = RecordingOptions::new(nss_core::CaptureSource::Display { id: 1 });
        let plan = CaptureFilePlan::new(&options);
        assert_eq!(plan.screen, RecordingAssets::DEFAULT_SCREEN_FILE);
        assert!(plan.camera.is_none() && plan.mic.is_none());

        options.camera_device_id = Some("cam-1".into());
        options.mic_device_id = Some("mic-1".into());
        let plan = CaptureFilePlan::new(&options);
        assert_eq!(
            plan.camera.as_deref(),
            Some(RecordingAssets::DEFAULT_CAMERA_FILE)
        );
        assert_eq!(plan.mic.as_deref(), Some(RecordingAssets::DEFAULT_MIC_FILE));
        assert_eq!(
            plan.path(&plan.screen, Path::new("C:/rec")),
            Path::new("C:/rec").join(RecordingAssets::DEFAULT_SCREEN_FILE)
        );
    }

    #[test]
    fn the_host_clock_advances() {
        let a = host_now();
        let b = host_now();
        assert!(b >= a && a.is_finite());
    }
}
