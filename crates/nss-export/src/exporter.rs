//! 60 fps MP4 exporter (`Mp4Exporter`) with atomic sibling-temp replacement and cancellation.
//!
//! Port of upstream `NSSExport/Exporter.swift` and `NSSExport/MP4Exporter.swift`.
//!
//! Pipeline per output frame `i`:
//! 1. `t = trim.start + i / fps`
//! 2. `state = motion.frame_state(t)`
//! 3. Decode at-or-before `screen` and optional `camera` frames from `FrameSource`
//! 4. `frame = renderer.render(screen, camera, state, style, output_size)` (same renderer as preview!)
//! 5. Convert RGBA -> BGRA into a reused scratch buffer (zero per-frame allocation) and pipe to FFmpeg
//! 6. On clean exit, atomically rename sibling temp `.partial.mp4` into `output_path`. On error or
//!    cancel, kill FFmpeg and delete the temp file so `output_path` is never left half-written.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use nss_core::{
    CursorTrack, ExportPreset, ExportProgress, MotionModel, NssError, PixelSize, Project, Style,
    Timeline, VideoCodec,
};
use nss_motion::SpringMotionModel;
use nss_render::{FrameRenderer, FrameSource};
use uuid::Uuid;

use crate::audio_pump::AudioMixPlan;
use crate::ffmpeg::FfmpegPaths;
use crate::frame_source::{AssetFrameSource, DecodeErrorReporting};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// Cancellation handle checked before every frame and before renaming the finished file into place.
#[derive(Debug, Default, Clone)]
pub struct CancelToken {
    cancelled: Arc<AtomicBool>,
}

impl CancelToken {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

/// Export-specific error differentiating user cancellation from I/O / project / render failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExportError {
    #[error("export cancelled")]
    Cancelled,
    #[error(transparent)]
    Core(#[from] NssError),
}

/// Factory creating the motion model for an export (shared with the preview engine).
pub type MotionFactory = Box<
    dyn Fn(&CursorTrack, &Timeline, &nss_core::CursorStyle, f64, f64) -> Box<dyn MotionModel>
        + Send
        + Sync,
>;

/// Factory opening a `FrameSource` for `(screen_path, optional_camera_path)`.
pub type FrameSourceFactory =
    Box<dyn Fn(&Path, Option<&Path>) -> Result<Box<dyn ExportFrameSource>, NssError> + Send + Sync>;

/// Combined trait so custom test doubles can report mid-stream decode failures to `Mp4Exporter`.
pub trait ExportFrameSource: FrameSource + DecodeErrorReporting {}
impl<T: FrameSource + DecodeErrorReporting> ExportFrameSource for T {}

/// Progress callback invoked synchronously from the export worker thread.
pub type ProgressCallback<'a> = &'a mut dyn FnMut(ExportProgress);

/// Bitrate calculation matching upstream `MP4Exporter.bitrate(size:fps:codec:)`.
#[must_use]
pub fn target_bitrate(size: PixelSize, fps: i32, codec: VideoCodec) -> u64 {
    let bits_per_pixel = match codec {
        VideoCodec::Hevc => 0.07,
        VideoCodec::H264 => 0.12,
    };
    let raw =
        f64::from(size.width) * f64::from(size.height) * f64::from(fps.max(1)) * bits_per_pixel;
    raw.clamp(4_000_000.0, 80_000_000.0) as u64
}

/// 60 fps MP4 exporter driven by `nss-motion`, `nss-render` and FFmpeg.
pub struct Mp4Exporter<R: FrameRenderer> {
    pub renderer: R,
    motion_factory: MotionFactory,
    frame_source_factory: FrameSourceFactory,
}

impl<R: FrameRenderer> Mp4Exporter<R> {
    #[must_use]
    pub fn new(renderer: R) -> Self {
        Self {
            renderer,
            motion_factory: Box::new(|track, timeline, cursor, duration, source_aspect| {
                Box::new(SpringMotionModel::new(
                    track,
                    timeline,
                    *cursor,
                    duration,
                    source_aspect,
                    nss_motion::MotionTuning::default(),
                ))
            }),
            frame_source_factory: Box::new(|screen, camera| {
                let src = AssetFrameSource::open(screen, camera)?;
                Ok(Box::new(src))
            }),
        }
    }

    #[must_use]
    pub fn with_motion_factory(mut self, factory: MotionFactory) -> Self {
        self.motion_factory = factory;
        self
    }

    #[must_use]
    pub fn with_frame_source_factory(mut self, factory: FrameSourceFactory) -> Self {
        self.frame_source_factory = factory;
        self
    }

    /// Exports `project`'s trimmed timeline into `output_path`, invoking `progress` along the way
    /// and ending with `Finished`, `Cancelled`, or `Failed`.
    #[allow(clippy::too_many_arguments)]
    pub fn export(
        &mut self,
        project: &Project,
        bundle_dir: &Path,
        track: &CursorTrack,
        preset: ExportPreset,
        output_path: &Path,
        cancel: &CancelToken,
        mut progress: impl FnMut(ExportProgress),
    ) -> Result<(), ExportError> {
        match self.run(
            project,
            bundle_dir,
            track,
            preset,
            output_path,
            cancel,
            &mut progress,
        ) {
            Ok(()) => {
                progress(ExportProgress::Finished(output_path.to_path_buf()));
                Ok(())
            }
            Err(ExportError::Cancelled) => {
                progress(ExportProgress::Cancelled {});
                Err(ExportError::Cancelled)
            }
            Err(ExportError::Core(err)) => {
                progress(ExportProgress::Failed {
                    message: err.user_message().to_owned(),
                });
                Err(ExportError::Core(err))
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn run(
        &mut self,
        project: &Project,
        bundle_dir: &Path,
        track: &CursorTrack,
        preset: ExportPreset,
        output_path: &Path,
        cancel: &CancelToken,
        progress: ProgressCallback<'_>,
    ) -> Result<(), ExportError> {
        if preset.fps <= 0 || preset.fps > 240 {
            return Err(
                NssError::Unsupported(format!("Unsupported frame rate {}", preset.fps)).into(),
            );
        }
        let fps = preset.fps;
        let assets = &project.assets;
        let screen_path = bundle_dir.join(&assets.screen_file);
        if !screen_path.is_file() {
            return Err(NssError::InvalidProject(format!(
                "Missing screen recording {}",
                assets.screen_file
            ))
            .into());
        }
        let camera_path = assets
            .camera_file
            .as_ref()
            .map(|f| bundle_dir.join(f))
            .filter(|p| p.is_file());
        let mic_path = assets
            .mic_file
            .as_ref()
            .map(|f| bundle_dir.join(f))
            .filter(|p| p.is_file());

        let ffmpeg = FfmpegPaths::discover()?;
        let caps = ffmpeg.capabilities();
        let encoder = caps.encoder_for(preset.codec).ok_or_else(|| {
            NssError::Unsupported(format!(
                "FFmpeg does not provide an encoder for {:?}",
                preset.codec
            ))
        })?;

        let probed_duration = ffmpeg
            .probe(&screen_path)
            .ok()
            .map(|p| p.duration)
            .unwrap_or(0.0);
        let duration = if probed_duration.is_finite() && probed_duration > 0.0 {
            probed_duration
        } else {
            assets.duration
        };

        let trim = &project.timeline.trim;
        let start = trim.start.clamp(0.0, duration);
        let end = trim.resolved_end(duration);
        let length = end - start;
        if !length.is_finite() || length <= 0.0 {
            return Err(NssError::InvalidProject("The trim range is empty".into()).into());
        }
        let total_frames = ((length * f64::from(fps)).round() as u64).max(1);

        let size = preset.output_size(project.style.aspect_ratio);
        let render_style: Style = project.style.resolving_assets(bundle_dir);
        let source_aspect = assets.source_pixel_size.aspect();
        let motion = (self.motion_factory)(
            track,
            &project.timeline,
            &project.style.cursor,
            duration,
            source_aspect,
        );
        let mut frames = (self.frame_source_factory)(&screen_path, camera_path.as_deref())?;

        let mut audio_candidates: Vec<&Path> = Vec::new();
        if let Some(ref mic) = mic_path {
            audio_candidates.push(mic.as_path());
        }
        audio_candidates.push(screen_path.as_path());
        let audio_plan = AudioMixPlan::discover(&ffmpeg, &audio_candidates, start, length);

        let output_dir = output_path.parent().unwrap_or_else(|| Path::new("."));
        if !output_dir.as_os_str().is_empty() {
            std::fs::create_dir_all(output_dir).map_err(|e| {
                NssError::Io(format!(
                    "Cannot create export folder {}: {e}",
                    output_dir.display()
                ))
            })?;
        }
        let file_name = output_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "export.mp4".into());
        let temp_path = output_dir.join(format!(".{file_name}.{}.partial.mp4", Uuid::new_v4()));

        let mut child = spawn_export_ffmpeg(
            &ffmpeg.ffmpeg,
            size,
            fps,
            preset.codec,
            encoder,
            audio_plan.as_ref(),
            &temp_path,
        )?;
        let mut guard = TempFileGuard {
            path: temp_path.clone(),
            armed: true,
        };

        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| NssError::Io("FFmpeg export stdin was not piped.".into()))?;

        progress(ExportProgress::Running {
            fraction: 0.0,
            frames_written: 0,
            total_frames,
        });
        let report_every = (total_frames / 100).max(1);

        // Single reusable BGRA scratch buffer: zero per-frame allocation in the frame loop.
        let mut bgra_scratch =
            Vec::with_capacity((size.width as usize) * (size.height as usize) * 4);

        for i in 0..total_frames {
            if cancel.is_cancelled() {
                drop(stdin);
                let _ = child.kill();
                let _ = child.wait();
                return Err(ExportError::Cancelled);
            }
            let t = start + (i as f64) / f64::from(fps);
            let state = motion.frame_state(t);
            let screen = frames
                .screen_frame(t)
                .map_err(|e| NssError::Io(e.to_string()))?;
            let camera = frames
                .camera_frame(t)
                .map_err(|e| NssError::Io(e.to_string()))?;

            if let Some(err) = frames.screen_error().or_else(|| frames.camera_error()) {
                drop(stdin);
                let _ = child.kill();
                let _ = child.wait();
                return Err(err.into());
            }
            let Some(screen) = screen else {
                drop(stdin);
                let _ = child.kill();
                let _ = child.wait();
                return Err(NssError::Io(format!("No screen frame at {t:.3} s")).into());
            };

            let rendered = self
                .renderer
                .render(&screen, camera.as_ref(), &state, &render_style, size)
                .map_err(|e| NssError::RenderFailed(e.to_string()))?;

            rendered.write_bgra_rows(&mut bgra_scratch);
            if let Err(e) = stdin.write_all(&bgra_scratch) {
                drop(stdin);
                let out = child.wait_with_output().ok();
                let detail = out
                    .as_ref()
                    .map(|o| String::from_utf8_lossy(&o.stderr).trim().to_owned())
                    .unwrap_or_default();
                return Err(
                    NssError::Io(format!("Video encoder write failed: {e} {detail}")).into(),
                );
            }

            let written = i + 1;
            if written % report_every == 0 || written == total_frames {
                progress(ExportProgress::Running {
                    fraction: (written as f64) / (total_frames as f64),
                    frames_written: written,
                    total_frames,
                });
            }
        }

        let _ = stdin.flush();
        drop(stdin);

        let output = child
            .wait_with_output()
            .map_err(|e| NssError::Io(format!("Failed waiting for FFmpeg exporter: {e}")))?;

        if cancel.is_cancelled() {
            return Err(ExportError::Cancelled);
        }
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(NssError::Io(format!(
                "Export encoder failed ({}): {}",
                output.status,
                stderr.trim()
            ))
            .into());
        }

        // Atomic move into destination (on Windows `fs::rename` replaces an existing file via
        // MoveFileExW(MOVEFILE_REPLACE_EXISTING)).
        std::fs::rename(&temp_path, output_path).map_err(|e| {
            NssError::Io(format!(
                "Could not move finished export into {}: {e}",
                output_path.display()
            ))
        })?;
        guard.armed = false;
        Ok(())
    }
}

struct TempFileGuard {
    path: PathBuf,
    armed: bool,
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

fn spawn_export_ffmpeg(
    ffmpeg: &Path,
    size: PixelSize,
    fps: i32,
    codec: VideoCodec,
    encoder: &str,
    audio_plan: Option<&AudioMixPlan>,
    temp_path: &Path,
) -> Result<Child, NssError> {
    let bitrate = target_bitrate(size, fps, codec);
    let mut args: Vec<String> = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-y".into(),
        "-f".into(),
        "rawvideo".into(),
        "-pixel_format".into(),
        "bgra".into(),
        "-video_size".into(),
        format!("{}x{}", size.width, size.height),
        "-framerate".into(),
        fps.to_string(),
        "-i".into(),
        "pipe:0".into(),
    ];

    let mut audio_out_args = Vec::new();
    if let Some(plan) = audio_plan {
        let (a_in, a_out) = plan.ffmpeg_args();
        args.extend(a_in);
        audio_out_args = a_out;
    }

    args.extend([
        "-map".into(),
        "0:v:0".into(),
        "-c:v".into(),
        encoder.into(),
        "-b:v".into(),
        bitrate.to_string(),
        "-g".into(),
        (fps * 2).to_string(),
        "-bf".into(),
        "0".into(),
        "-pix_fmt".into(),
        "yuv420p".into(),
        "-color_primaries".into(),
        "bt709".into(),
        "-color_trc".into(),
        "bt709".into(),
        "-colorspace".into(),
        "bt709".into(),
    ]);

    if encoder == "libx264" {
        args.extend([
            "-preset".into(),
            "medium".into(),
            "-profile:v".into(),
            "high".into(),
        ]);
    } else if encoder == "libx265" {
        args.extend([
            "-preset".into(),
            "medium".into(),
            "-tag:v".into(),
            "hvc1".into(),
        ]);
    } else if codec == VideoCodec::Hevc {
        args.extend(["-tag:v".into(), "hvc1".into()]);
    }

    if !audio_out_args.is_empty() {
        args.extend(audio_out_args);
        args.push("-shortest".into());
    }

    args.extend([
        "-movflags".into(),
        "+faststart".into(),
        temp_path.to_string_lossy().into_owned(),
    ]);

    let mut cmd = Command::new(ffmpeg);
    cmd.args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    cmd.creation_flags(crate::ffmpeg::CREATE_NO_WINDOW);

    cmd.spawn()
        .map_err(|e| NssError::Io(format!("Could not spawn FFmpeg exporter: {e}")))
}
