//! Microphone and camera discovery and recording.
//!
//! Port of upstream `NSSCapture/CaptureDevices.swift` and `NSSCapture/DeviceRecorder.swift`.
//! On Windows, microphones use WASAPI (`IMMDeviceEnumerator` / `IAudioCaptureClient`) and cameras
//! use Media Foundation (`MFEnumDeviceSources` / `IMFSourceReader`). Both drop samples until the
//! screen recorder establishes [`SharedOrigin`], and skip samples inside paused intervals on
//! [`PauseClock`], so every track in a bundle shares one timeline.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use nss_core::NssError;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::capture_math::{even_pixels, video_bitrate, SharedOrigin};
use crate::pause_clock::PauseClock;
use crate::writer::{AudioWriterConfig, PcmAudioWriter, RawVideoWriter, VideoWriterConfig};

/// One microphone or webcam that `RecordingOptions` can address by `id`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureDeviceInfo {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

impl CaptureDeviceInfo {
    #[must_use]
    pub fn new(id: impl Into<String>, name: impl Into<String>, is_default: bool) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            is_default,
        }
    }
}

/// Which secondary capture device is being recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    Microphone,
    Camera,
}

/// Lists microphones and cameras. Selection is by `id` in `RecordingOptions`.
pub struct CaptureDevices;

impl CaptureDevices {
    #[must_use]
    pub fn microphones() -> Vec<CaptureDeviceInfo> {
        #[cfg(windows)]
        {
            crate::win::devices::enumerate_microphones()
        }
        #[cfg(not(windows))]
        {
            Vec::new()
        }
    }

    #[must_use]
    pub fn cameras() -> Vec<CaptureDeviceInfo> {
        #[cfg(windows)]
        {
            crate::win::devices::enumerate_cameras()
        }
        #[cfg(not(windows))]
        {
            Vec::new()
        }
    }
}

/// AAC bitrate chosen for a microphone stream, clamped to the sample rate so low-rate Bluetooth
/// headsets (8 kHz / 16 kHz HFP) do not ask the encoder for an unsupported bitrate.
///
/// Exact port of upstream `DeviceRecorder.aacBitrate(channels:sampleRate:)`.
#[must_use]
pub fn aac_bitrate(channels: u32, sample_rate: f64) -> u32 {
    let ch = channels.clamp(1, 2);
    let desired = if ch == 1 { 96_000 } else { 160_000 };
    let cap = if sample_rate <= 12_000.0 {
        32_000 * ch
    } else if sample_rate <= 24_000.0 {
        64_000 * ch
    } else {
        desired
    };
    desired.min(cap)
}

/// Records a microphone via WASAPI into `mic.m4a` (AAC) via FFmpeg, synced to `origin` and `pause`.
pub struct MicRecorder {
    #[cfg(windows)]
    handle: Option<crate::win::devices::WasapiCaptureHandle>,
    writer: Arc<Mutex<Option<PcmAudioWriter>>>,
    packets_written: Arc<AtomicUsize>,
    stopped: AtomicBool,
}

impl MicRecorder {
    pub fn start<F>(
        device_id: String,
        output_path: PathBuf,
        origin: Arc<SharedOrigin>,
        pause: Arc<PauseClock>,
        on_failure: F,
    ) -> Result<Self, NssError>
    where
        F: Fn(NssError) + Send + Sync + 'static,
    {
        #[cfg(windows)]
        {
            let writer_slot: Arc<Mutex<Option<PcmAudioWriter>>> = Arc::new(Mutex::new(None));
            let packets_written = Arc::new(AtomicUsize::new(0));
            let on_failure = Arc::new(on_failure);

            // Create the writer lazily once we know the device's mix format.
            let writer_cb = Arc::clone(&writer_slot);
            let packets_cb = Arc::clone(&packets_written);
            let origin_cb = Arc::clone(&origin);
            let pause_cb = Arc::clone(&pause);
            let fail_cb = Arc::clone(&on_failure);
            let fail_err = Arc::clone(&on_failure);

            let handle = crate::win::devices::WasapiCaptureHandle::start(
                device_id,
                move |pkt| {
                    let Some(orig) = origin_cb.current() else {
                        return;
                    };
                    if orig.is_entirely_before(pkt.host_time, 0.0) {
                        return;
                    }
                    if pause_cb.retime(pkt.host_time).is_none() {
                        return;
                    }
                    let guard = writer_cb.lock();
                    if let Some(ref w) = *guard {
                        if let Err(e) = w.write_samples(&pkt.samples) {
                            fail_cb(e);
                        } else {
                            packets_cb.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                },
                move |err| {
                    fail_err(err);
                },
            )?;

            let cfg = AudioWriterConfig {
                sample_rate: handle.format.sample_rate,
                channels: u32::from(handle.format.channels.max(1)),
                bitrate: aac_bitrate(
                    u32::from(handle.format.channels.max(1)),
                    f64::from(handle.format.sample_rate),
                ),
                output_path,
            };
            let writer = PcmAudioWriter::start(cfg)?;
            *writer_slot.lock() = Some(writer);

            Ok(Self {
                handle: Some(handle),
                writer: writer_slot,
                packets_written,
                stopped: AtomicBool::new(false),
            })
        }

        #[cfg(not(windows))]
        {
            let _ = (device_id, output_path, origin, pause, on_failure);
            Err(NssError::Unsupported(
                "Microphone capture is only supported on Windows.".into(),
            ))
        }
    }

    /// Stops capture and finalizes the audio file. Returns `true` if at least one packet was written.
    pub fn stop(&mut self) -> Result<bool, NssError> {
        if self.stopped.swap(true, Ordering::SeqCst) {
            return Ok(self.packets_written.load(Ordering::Relaxed) > 0);
        }
        #[cfg(windows)]
        if let Some(mut h) = self.handle.take() {
            h.stop();
        }
        let writer = self.writer.lock().take();
        if let Some(w) = writer {
            w.finish()?;
        }
        Ok(self.packets_written.load(Ordering::Relaxed) > 0)
    }
}

/// Records a webcam via Media Foundation into `camera.mov` via FFmpeg, synced to `origin` and `pause`.
pub struct CameraRecorder {
    #[cfg(windows)]
    handle: Option<crate::win::devices::CameraCaptureHandle>,
    writer: Arc<Mutex<Option<RawVideoWriter>>>,
    frames_written: Arc<AtomicUsize>,
    stopped: AtomicBool,
}

impl CameraRecorder {
    pub fn start<F>(
        device_id: String,
        output_path: PathBuf,
        origin: Arc<SharedOrigin>,
        pause: Arc<PauseClock>,
        on_failure: F,
    ) -> Result<Self, NssError>
    where
        F: Fn(NssError) + Send + Sync + 'static,
    {
        #[cfg(windows)]
        {
            let writer_slot: Arc<Mutex<Option<RawVideoWriter>>> = Arc::new(Mutex::new(None));
            let frames_written = Arc::new(AtomicUsize::new(0));
            let on_failure = Arc::new(on_failure);

            let writer_cb = Arc::clone(&writer_slot);
            let frames_cb = Arc::clone(&frames_written);
            let origin_cb = Arc::clone(&origin);
            let pause_cb = Arc::clone(&pause);
            let fail_cb = Arc::clone(&on_failure);
            let fail_err = Arc::clone(&on_failure);

            let handle = crate::win::devices::CameraCaptureHandle::start(
                device_id,
                move |frame| {
                    let Some(orig) = origin_cb.current() else {
                        return;
                    };
                    if orig.is_entirely_before(frame.host_time, 0.0) {
                        return;
                    }
                    if pause_cb.retime(frame.host_time).is_none() {
                        return;
                    }
                    let guard = writer_cb.lock();
                    if let Some(ref w) = *guard {
                        if let Err(e) = w.write_frame(frame.bgra) {
                            fail_cb(e);
                        } else {
                            frames_cb.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                },
                move |err| {
                    fail_err(err);
                },
            )?;

            let width = even_pixels(f64::from(handle.width));
            let height = even_pixels(f64::from(handle.height));
            let fps = 30;
            let cfg = VideoWriterConfig {
                width,
                height,
                fps,
                bitrate: u64::from(video_bitrate(width, height, f64::from(fps), 0.12)),
                output_path,
            };
            let writer = RawVideoWriter::start(cfg)?;
            *writer_slot.lock() = Some(writer);

            Ok(Self {
                handle: Some(handle),
                writer: writer_slot,
                frames_written,
                stopped: AtomicBool::new(false),
            })
        }

        #[cfg(not(windows))]
        {
            let _ = (device_id, output_path, origin, pause, on_failure);
            Err(NssError::Unsupported(
                "Camera capture is only supported on Windows.".into(),
            ))
        }
    }

    pub fn stop(&mut self) -> Result<bool, NssError> {
        if self.stopped.swap(true, Ordering::SeqCst) {
            return Ok(self.frames_written.load(Ordering::Relaxed) > 0);
        }
        #[cfg(windows)]
        if let Some(mut h) = self.handle.take() {
            h.stop();
        }
        let writer = self.writer.lock().take();
        if let Some(w) = writer {
            let _ = w.finish()?;
        }
        Ok(self.frames_written.load(Ordering::Relaxed) > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aac_bitrate_clamps_to_sample_rate() {
        // Mirrors upstream StreamPlanTests.aacBitrateClampsToSampleRate.
        assert_eq!(aac_bitrate(1, 48_000.0), 96_000);
        assert_eq!(aac_bitrate(2, 44_100.0), 160_000);
        assert_eq!(aac_bitrate(1, 16_000.0), 64_000);
        assert_eq!(aac_bitrate(1, 8_000.0), 32_000);
        assert_eq!(aac_bitrate(2, 8_000.0), 64_000);
    }
}
