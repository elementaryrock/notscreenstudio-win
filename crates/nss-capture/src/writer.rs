//! FFmpeg-backed video and audio sample writers (ADR 0002 & ADR 0012).
//!
//! Writes `screen.mov`, `camera.mov` (H.264 in MOV/MP4 with `-movflags +faststart`) and `mic.m4a`
//! (AAC in M4A) through a bounded background channel so the real-time WGC / WASAPI / MF threads
//! never block on disk or encoder I/O.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use nss_core::NssError;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Locates `ffmpeg` per ADR 0002: next to the running executable first, then `PATH`.
pub fn find_ffmpeg() -> Result<PathBuf, NssError> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let bundled = dir.join(if cfg!(windows) {
                "ffmpeg.exe"
            } else {
                "ffmpeg"
            });
            if bundled.is_file() {
                return Ok(bundled);
            }
        }
    }
    // Probe `ffmpeg` on PATH.
    let probe = Command::new("ffmpeg")
        .arg("-version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    match probe {
        Ok(st) if st.success() => Ok(PathBuf::from("ffmpeg")),
        _ => Err(NssError::Unsupported(
            "FFmpeg was not found. Place ffmpeg.exe next to the app or install FFmpeg 8+ on PATH."
                .into(),
        )),
    }
}

/// Configuration for [`RawVideoWriter`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoWriterConfig {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate: u64,
    pub output_path: PathBuf,
}

/// Configuration for [`PcmAudioWriter`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioWriterConfig {
    pub sample_rate: u32,
    pub channels: u32,
    pub bitrate: u32,
    pub output_path: PathBuf,
}

/// Builds the FFmpeg CLI arguments for piping tightly-packed BGRA8 frames into a MOV/MP4 file.
#[must_use]
pub fn video_ffmpeg_args(cfg: &VideoWriterConfig) -> Vec<String> {
    let size = format!("{}x{}", cfg.width, cfg.height);
    let fps = cfg.fps.max(1).to_string();
    let bitrate = cfg.bitrate.to_string();
    let out = cfg.output_path.to_string_lossy().into_owned();
    vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-y".into(),
        "-f".into(),
        "rawvideo".into(),
        "-pixel_format".into(),
        "bgra".into(),
        "-video_size".into(),
        size,
        "-framerate".into(),
        fps,
        "-i".into(),
        "pipe:0".into(),
        "-c:v".into(),
        "libx264".into(),
        "-preset".into(),
        "ultrafast".into(),
        "-tune".into(),
        "zerolatency".into(),
        "-pix_fmt".into(),
        "yuv420p".into(),
        "-b:v".into(),
        bitrate,
        "-movflags".into(),
        "+faststart".into(),
        out,
    ]
}

/// Builds the FFmpeg CLI arguments for piping interleaved `f32le` PCM into an M4A (AAC) file.
#[must_use]
pub fn audio_ffmpeg_args(cfg: &AudioWriterConfig) -> Vec<String> {
    let rate = cfg.sample_rate.max(8_000).to_string();
    let ch = cfg.channels.clamp(1, 2).to_string();
    let bitrate = cfg.bitrate.to_string();
    let out = cfg.output_path.to_string_lossy().into_owned();
    vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-y".into(),
        "-f".into(),
        "f32le".into(),
        "-ar".into(),
        rate,
        "-ac".into(),
        ch,
        "-i".into(),
        "pipe:0".into(),
        "-c:a".into(),
        "aac".into(),
        "-b:a".into(),
        bitrate,
        "-movflags".into(),
        "+faststart".into(),
        out,
    ]
}

/// Bounded-queue FFmpeg writer for BGRA8 video frames.
///
/// Once the first frame arrives, paces output at a steady `fps` cadence by repeating the most
/// recent frame whenever the source is static (WGC only delivers changed frames), and stops
/// emitting while `paused` is true so paused intervals leave no gap in the encoded video.
pub struct RawVideoWriter {
    tx: Option<mpsc::SyncSender<Vec<u8>>>,
    join: Option<JoinHandle<Result<usize, NssError>>>,
    expected_bytes: usize,
    written_frames: Arc<AtomicUsize>,
    failed: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
}

impl RawVideoWriter {
    pub fn start(cfg: VideoWriterConfig) -> Result<Self, NssError> {
        ensure_parent_dir(&cfg.output_path)?;
        let ffmpeg = find_ffmpeg()?;
        let args = video_ffmpeg_args(&cfg);
        let child = spawn_ffmpeg(&ffmpeg, &args)?;
        let expected_bytes = (cfg.width as usize) * (cfg.height as usize) * 4;
        let fps = cfg.fps.max(1);
        let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(8);
        let written_frames = Arc::new(AtomicUsize::new(0));
        let failed = Arc::new(AtomicBool::new(false));
        let paused = Arc::new(AtomicBool::new(false));

        let wf = Arc::clone(&written_frames);
        let fl = Arc::clone(&failed);
        let pz = Arc::clone(&paused);
        let join = thread::Builder::new()
            .name("nss-video-writer".into())
            .spawn(move || pump_video_child(child, rx, expected_bytes, fps, wf, fl, pz))
            .map_err(|e| NssError::Io(format!("Failed to start video writer thread: {e}")))?;

        Ok(Self {
            tx: Some(tx),
            join: Some(join),
            expected_bytes,
            written_frames,
            failed,
            paused,
        })
    }

    /// Suspends or resumes constant-frame-rate pacing while the recording is paused.
    pub fn set_paused(&self, is_paused: bool) {
        self.paused.store(is_paused, Ordering::SeqCst);
    }

    /// Enqueues a single tightly-packed BGRA8 frame. Drops the frame rather than deadlocking if
    /// the encoder falls far behind; returns an error only if the writer thread has already died.
    pub fn write_frame(&self, bgra: Vec<u8>) -> Result<(), NssError> {
        if self.failed.load(Ordering::Relaxed) {
            return Err(NssError::Io(
                "The video encoder stopped unexpectedly.".into(),
            ));
        }
        if bgra.len() != self.expected_bytes {
            return Err(NssError::Io(format!(
                "Video frame size mismatch: got {} bytes, expected {}.",
                bgra.len(),
                self.expected_bytes
            )));
        }
        if let Some(ref tx) = self.tx {
            match tx.try_send(bgra) {
                Ok(()) | Err(mpsc::TrySendError::Full(_)) => Ok(()),
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    Err(NssError::Io("Video encoder pipe closed.".into()))
                }
            }
        } else {
            Ok(())
        }
    }

    #[must_use]
    pub fn frames_written(&self) -> usize {
        self.written_frames.load(Ordering::Relaxed)
    }

    /// Closes the pipe, waits for FFmpeg to finalize the container (`+faststart`), and returns the
    /// number of written frames.
    pub fn finish(mut self) -> Result<usize, NssError> {
        drop(self.tx.take());
        if let Some(j) = self.join.take() {
            j.join()
                .map_err(|_| NssError::Io("Video writer thread panicked.".into()))?
        } else {
            Ok(self.frames_written())
        }
    }
}

impl Drop for RawVideoWriter {
    fn drop(&mut self) {
        drop(self.tx.take());
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

/// Bounded-queue FFmpeg writer for interleaved `f32le` PCM audio -> M4A (AAC).
pub struct PcmAudioWriter {
    tx: Option<mpsc::SyncSender<Vec<u8>>>,
    join: Option<JoinHandle<Result<(), NssError>>>,
    failed: Arc<AtomicBool>,
}

impl PcmAudioWriter {
    pub fn start(cfg: AudioWriterConfig) -> Result<Self, NssError> {
        ensure_parent_dir(&cfg.output_path)?;
        let ffmpeg = find_ffmpeg()?;
        let args = audio_ffmpeg_args(&cfg);
        let child = spawn_ffmpeg(&ffmpeg, &args)?;
        let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(64);
        let failed = Arc::new(AtomicBool::new(false));
        let fl = Arc::clone(&failed);

        let join = thread::Builder::new()
            .name("nss-audio-writer".into())
            .spawn(move || pump_audio_child(child, rx, fl))
            .map_err(|e| NssError::Io(format!("Failed to start audio writer thread: {e}")))?;

        Ok(Self {
            tx: Some(tx),
            join: Some(join),
            failed,
        })
    }

    pub fn write_samples(&self, samples: &[f32]) -> Result<(), NssError> {
        if self.failed.load(Ordering::Relaxed) {
            return Err(NssError::Io(
                "The audio encoder stopped unexpectedly.".into(),
            ));
        }
        let mut bytes = Vec::with_capacity(samples.len() * 4);
        for &s in samples {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        if let Some(ref tx) = self.tx {
            match tx.try_send(bytes) {
                Ok(()) | Err(mpsc::TrySendError::Full(_)) => Ok(()),
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    Err(NssError::Io("Audio encoder pipe closed.".into()))
                }
            }
        } else {
            Ok(())
        }
    }

    pub fn finish(mut self) -> Result<(), NssError> {
        drop(self.tx.take());
        if let Some(j) = self.join.take() {
            j.join()
                .map_err(|_| NssError::Io("Audio writer thread panicked.".into()))?
        } else {
            Ok(())
        }
    }
}

impl Drop for PcmAudioWriter {
    fn drop(&mut self) {
        drop(self.tx.take());
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

fn ensure_parent_dir(path: &Path) -> Result<(), NssError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| {
                NssError::Io(format!(
                    "Could not create capture directory {}: {e}",
                    parent.display()
                ))
            })?;
        }
    }
    Ok(())
}

fn spawn_ffmpeg(ffmpeg: &Path, args: &[String]) -> Result<Child, NssError> {
    let mut cmd = Command::new(ffmpeg);
    cmd.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd.spawn().map_err(|e| {
        NssError::Io(format!(
            "Could not start FFmpeg ({}): {e}",
            ffmpeg.display()
        ))
    })
}

fn pump_video_child(
    mut child: Child,
    rx: mpsc::Receiver<Vec<u8>>,
    expected_bytes: usize,
    fps: u32,
    written_frames: Arc<AtomicUsize>,
    failed: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
) -> Result<usize, NssError> {
    let mut write_err: Option<std::io::Error> = None;
    let frame_step = Duration::from_secs_f64(1.0 / f64::from(fps.max(1)));
    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| NssError::Io("FFmpeg stdin was not piped.".into()))?;

        // Wait for the very first valid frame before starting the CFR pacer.
        let mut latest: Option<Vec<u8>> = None;
        while latest.is_none() {
            match rx.recv() {
                Ok(f) if f.len() == expected_bytes => latest = Some(f),
                Ok(_) => {}
                Err(_) => break,
            }
        }

        if let Some(mut current) = latest {
            // Emit frame 0 immediately so t = 0 matches the first captured frame.
            if let Err(e) = stdin.write_all(&current) {
                failed.store(true, Ordering::SeqCst);
                write_err = Some(e);
            } else {
                written_frames.fetch_add(1, Ordering::Relaxed);
                let mut next_tick = Instant::now() + frame_step;
                let mut disconnected = false;
                while !disconnected {
                    // Wait until the next frame boundary while updating `current` with any newer
                    // frame delivered by WGC.
                    loop {
                        let now = Instant::now();
                        if now >= next_tick {
                            break;
                        }
                        match rx.recv_timeout(next_tick - now) {
                            Ok(f) if f.len() == expected_bytes => current = f,
                            Ok(_) => {}
                            Err(mpsc::RecvTimeoutError::Timeout) => break,
                            Err(mpsc::RecvTimeoutError::Disconnected) => {
                                disconnected = true;
                                break;
                            }
                        }
                    }
                    if disconnected {
                        break;
                    }
                    // Drain any extra queued frames so we always encode the freshest one.
                    loop {
                        match rx.try_recv() {
                            Ok(f) if f.len() == expected_bytes => current = f,
                            Ok(_) => {}
                            Err(mpsc::TryRecvError::Empty) => break,
                            Err(mpsc::TryRecvError::Disconnected) => {
                                disconnected = true;
                                break;
                            }
                        }
                    }
                    if disconnected {
                        break;
                    }
                    if !paused.load(Ordering::Relaxed) {
                        if let Err(e) = stdin.write_all(&current) {
                            failed.store(true, Ordering::SeqCst);
                            write_err = Some(e);
                            break;
                        }
                        written_frames.fetch_add(1, Ordering::Relaxed);
                    }
                    next_tick += frame_step;
                    let now = Instant::now();
                    if now > next_tick + frame_step * 4 {
                        // Avoid runaway catch-up bursts after a pause or system stall.
                        next_tick = now + frame_step;
                    }
                }
            }
        }
        let _ = stdin.flush();
    }

    let output = child
        .wait_with_output()
        .map_err(|e| NssError::Io(format!("Failed waiting for FFmpeg: {e}")))?;
    let count = written_frames.load(Ordering::Relaxed);

    if let Some(err) = write_err {
        return Err(map_writer_io_error(&err, &output.stderr));
    }
    if count > 0 && !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.to_lowercase().contains("no space left") {
            return Err(NssError::Io(
                "Recording stopped because the disk is full.".into(),
            ));
        }
        return Err(NssError::Io(format!(
            "FFmpeg failed while writing video ({}): {}",
            output.status,
            stderr.trim()
        )));
    }
    Ok(count)
}

fn pump_audio_child(
    mut child: Child,
    rx: mpsc::Receiver<Vec<u8>>,
    failed: Arc<AtomicBool>,
) -> Result<(), NssError> {
    let mut write_err: Option<std::io::Error> = None;
    let mut wrote_any = false;
    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| NssError::Io("FFmpeg audio stdin was not piped.".into()))?;
        for chunk in rx {
            if let Err(e) = stdin.write_all(&chunk) {
                failed.store(true, Ordering::SeqCst);
                write_err = Some(e);
                break;
            }
            wrote_any = true;
        }
        let _ = stdin.flush();
    }

    let output = child
        .wait_with_output()
        .map_err(|e| NssError::Io(format!("Failed waiting for FFmpeg audio: {e}")))?;
    if let Some(err) = write_err {
        return Err(map_writer_io_error(&err, &output.stderr));
    }
    if wrote_any && !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(NssError::Io(format!(
            "FFmpeg failed while writing audio ({}): {}",
            output.status,
            stderr.trim()
        )));
    }
    Ok(())
}

fn map_writer_io_error(err: &std::io::Error, stderr: &[u8]) -> NssError {
    if matches!(err.raw_os_error(), Some(112 | 39 | 28)) {
        return NssError::Io("Recording stopped because the disk is full.".into());
    }
    let detail = String::from_utf8_lossy(stderr);
    if detail.to_lowercase().contains("no space left") {
        return NssError::Io("Recording stopped because the disk is full.".into());
    }
    NssError::Io(format!("Capture writer failed: {err} {}", detail.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_args_carry_dimensions_fps_bitrate_and_faststart() {
        let cfg = VideoWriterConfig {
            width: 1600,
            height: 1200,
            fps: 60,
            bitrate: 25_000_000,
            output_path: PathBuf::from("screen.mov"),
        };
        let args = video_ffmpeg_args(&cfg);
        assert!(args.contains(&"1600x1200".to_string()));
        assert!(args.contains(&"60".to_string()));
        assert!(args.contains(&"25000000".to_string()));
        assert!(args.contains(&"+faststart".to_string()));
        assert_eq!(args.last().map(String::as_str), Some("screen.mov"));
    }

    #[test]
    fn audio_args_carry_sample_rate_channels_and_aac() {
        let cfg = AudioWriterConfig {
            sample_rate: 48_000,
            channels: 2,
            bitrate: 160_000,
            output_path: PathBuf::from("mic.m4a"),
        };
        let args = audio_ffmpeg_args(&cfg);
        assert!(args.contains(&"48000".to_string()));
        assert!(args.contains(&"2".to_string()));
        assert!(args.contains(&"aac".to_string()));
        assert!(args.contains(&"160000".to_string()));
    }
}
