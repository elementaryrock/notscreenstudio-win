//! Port of upstream `NSSExportTests.swift` and `EndToEndExportTests.swift`.
//!
//! Tests that invoke FFmpeg skip cleanly when `ffmpeg`/`ffprobe` is not installed (so headless CI
//! without FFmpeg stays green) and run full real-MP4 verification whenever FFmpeg is present.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use nss_core::{
    AspectRatio, CursorTrack, ExportPreset, ExportProgress, ExportResolution, FrameState,
    MediaTime, MotionModel, NssError, PixelSize, Project, RecordingAssets, Style, Timeline,
    TrimRange, VideoCodec,
};
use nss_export::{
    AssetFrameSource, CancelToken, DecodeErrorReporting, ExportError, FfmpegPaths, Mp4Exporter,
};
use nss_motion::{MotionTuning, SpringMotionModel};
use nss_render::{
    Frame, FrameError, FrameRenderer, FrameSource, Image, PassthroughRenderer, RenderError,
};
use parking_lot::Mutex;
use tempfile::TempDir;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn ffmpeg_or_skip() -> Option<FfmpegPaths> {
    FfmpegPaths::discover().ok()
}

fn near(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

/// Writes a lossless-ish H.264 video with `frames` frames at `fps`, where frame `i` is solid gray
/// `(i * 16 + 8)` so tests can read back which exact source frame was selected.
fn write_indexed_video(
    paths: &FfmpegPaths,
    out: &Path,
    width: u32,
    height: u32,
    frames: usize,
    fps: u32,
) {
    let mut cmd = Command::new(&paths.ffmpeg);
    cmd.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-f",
        "rawvideo",
        "-pixel_format",
        "rgba",
        "-video_size",
        &format!("{width}x{height}"),
        "-framerate",
        &fps.to_string(),
        "-i",
        "pipe:0",
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-crf",
        "10",
        "-bf",
        "0",
        "-pix_fmt",
        "yuv420p",
    ])
    .arg(out)
    .stdin(Stdio::piped())
    .stdout(Stdio::null())
    .stderr(Stdio::piped());
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let mut child = cmd.spawn().expect("spawn ffmpeg fixture writer");
    {
        let mut stdin = child.stdin.take().unwrap();
        let pixel_count = (width as usize) * (height as usize);
        let mut buf = vec![0u8; pixel_count * 4];
        for i in 0..frames {
            let v = ((i * 16 + 8).min(255)) as u8;
            for px in buf.chunks_exact_mut(4) {
                px[0] = v;
                px[1] = v;
                px[2] = v;
                px[3] = 255;
            }
            stdin.write_all(&buf).unwrap();
        }
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "fixture ffmpeg failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn write_sine_audio(paths: &FfmpegPaths, out: &Path, duration_secs: f64) {
    let mut cmd = Command::new(&paths.ffmpeg);
    cmd.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-f",
        "lavfi",
        "-i",
        &format!("sine=frequency=440:sample_rate=44100:duration={duration_secs:.3}"),
        "-c:a",
        "aac",
        "-b:a",
        "128k",
    ])
    .arg(out)
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::piped());
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let output = cmd.output().expect("spawn ffmpeg audio fixture");
    assert!(
        output.status.success(),
        "audio fixture failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

struct TestBundle {
    dir: TempDir,
    project: Project,
}

fn make_bundle(
    paths: &FfmpegPaths,
    frames: usize,
    fps: u32,
    trim: TrimRange,
    aspect: AspectRatio,
    with_mic: bool,
) -> TestBundle {
    let dir = tempfile::tempdir().unwrap();
    let width = 160;
    let height = 90;
    write_indexed_video(
        paths,
        &dir.path().join("screen.mov"),
        width,
        height,
        frames,
        fps,
    );

    let duration = (frames as f64) / f64::from(fps);
    let mut assets = RecordingAssets::new(PixelSize::new(width, height), f64::from(fps), duration);
    if with_mic {
        write_sine_audio(paths, &dir.path().join("mic.m4a"), duration);
        assets.mic_file = Some("mic.m4a".into());
    }

    let mut project = Project::new("test", assets);
    project.timeline = Timeline {
        trim,
        ..Timeline::default()
    };
    project.style.aspect_ratio = aspect;

    TestBundle { dir, project }
}

#[derive(Debug, Clone)]
struct RenderCall {
    time: f64,
    width: u32,
    height: u32,
    gray: u8,
}

struct RecordingRenderer {
    calls: Arc<Mutex<Vec<RenderCall>>>,
    on_render: Option<Box<dyn Fn(usize) + Send + Sync>>,
}

impl RecordingRenderer {
    fn new() -> (Self, Arc<Mutex<Vec<RenderCall>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                calls: Arc::clone(&calls),
                on_render: None,
            },
            calls,
        )
    }

    fn with_hook(mut self, hook: impl Fn(usize) + Send + Sync + 'static) -> Self {
        self.on_render = Some(Box::new(hook));
        self
    }
}

impl FrameRenderer for RecordingRenderer {
    fn render(
        &mut self,
        screen: &Image,
        _camera: Option<&Image>,
        state: &FrameState,
        _style: &Style,
        output_size: PixelSize,
    ) -> Result<Frame, RenderError> {
        let mid = screen.pixel(screen.width / 2, screen.height / 2);
        let n = {
            let mut c = self.calls.lock();
            c.push(RenderCall {
                time: state.time,
                width: output_size.width,
                height: output_size.height,
                gray: mid[0],
            });
            c.len()
        };
        if let Some(ref hook) = self.on_render {
            hook(n);
        }
        Ok(Image::filled(
            output_size.width,
            output_size.height,
            nss_core::RGBAColor::new(
                f64::from(mid[0]) / 255.0,
                f64::from(mid[1]) / 255.0,
                f64::from(mid[2]) / 255.0,
                1.0,
            ),
        ))
    }

    fn name(&self) -> &'static str {
        "recording-test"
    }
}

#[test]
fn exports_1080p_for_every_aspect() {
    let Some(paths) = ffmpeg_or_skip() else {
        return;
    };
    for aspect in AspectRatio::ALL {
        let b = make_bundle(
            &paths,
            15,
            30,
            TrimRange {
                start: 0.0,
                end: Some(0.1),
            },
            aspect,
            false,
        );
        let out = b.dir.path().join("out.mp4");
        let (renderer, calls) = RecordingRenderer::new();
        let mut events = Vec::new();
        let preset = ExportPreset {
            resolution: ExportResolution::P1080,
            fps: 60,
            codec: VideoCodec::H264,
        };
        Mp4Exporter::new(renderer)
            .export(
                &b.project,
                b.dir.path(),
                &CursorTrack::default(),
                preset,
                &out,
                &CancelToken::new(),
                |e| events.push(e),
            )
            .unwrap();

        let expected = preset.output_size(aspect);
        let p = paths.probe(&out).unwrap();
        assert_eq!((p.width, p.height), (expected.width, expected.height));
        assert!(near(p.duration, 0.1, 0.04), "duration={}", p.duration);
        assert_eq!(p.video_frames, 6);
        assert!(near(p.nominal_fps, 60.0, 0.5));
        assert_eq!(p.audio_tracks, 0);
        assert_eq!(calls.lock().len(), 6);
        assert!(calls
            .lock()
            .iter()
            .all(|c| c.width == expected.width && c.height == expected.height));
        assert_eq!(events.last(), Some(&ExportProgress::Finished(out)));
    }
}

#[test]
fn exports_4k_across_16x9_1x1_and_9x16() {
    let Some(paths) = ffmpeg_or_skip() else {
        return;
    };
    for (aspect, w, h) in [
        (AspectRatio::R16x9, 3840, 2160),
        (AspectRatio::R1x1, 2160, 2160),
        (AspectRatio::R9x16, 2160, 3840),
    ] {
        let b = make_bundle(
            &paths,
            6,
            30,
            TrimRange {
                start: 0.0,
                end: Some(0.05),
            },
            aspect,
            false,
        );
        let out = b.dir.path().join("out4k.mp4");
        let preset = ExportPreset {
            resolution: ExportResolution::P2160,
            fps: 60,
            codec: VideoCodec::H264,
        };
        Mp4Exporter::new(PassthroughRenderer::new())
            .export(
                &b.project,
                b.dir.path(),
                &CursorTrack::default(),
                preset,
                &out,
                &CancelToken::new(),
                |_| {},
            )
            .unwrap();
        let p = paths.probe(&out).unwrap();
        assert_eq!((p.width, p.height), (w, h));
        assert_eq!(p.video_frames, 3);
    }
}

#[test]
fn exports_hevc() {
    let Some(paths) = ffmpeg_or_skip() else {
        return;
    };
    if paths.capabilities().encoder_for(VideoCodec::Hevc).is_none() {
        return;
    }
    let b = make_bundle(
        &paths,
        15,
        30,
        TrimRange {
            start: 0.0,
            end: Some(0.1),
        },
        AspectRatio::R16x9,
        false,
    );
    let out = b.dir.path().join("hevc.mp4");
    let preset = ExportPreset {
        resolution: ExportResolution::P1080,
        fps: 60,
        codec: VideoCodec::Hevc,
    };
    Mp4Exporter::new(PassthroughRenderer::new())
        .export(
            &b.project,
            b.dir.path(),
            &CursorTrack::default(),
            preset,
            &out,
            &CancelToken::new(),
            |_| {},
        )
        .unwrap();
    let p = paths.probe(&out).unwrap();
    assert_eq!(p.video_codec.as_deref(), Some("hevc"));
    assert_eq!(p.video_frames, 6);
}

#[test]
fn trim_is_respected_and_frames_map_at_or_before() {
    let Some(paths) = ffmpeg_or_skip() else {
        return;
    };
    // 30 fps source: frame k covers [k/30, (k+1)/30). Trim 0.2..0.4 -> source frames 6..=11.
    let b = make_bundle(
        &paths,
        15,
        30,
        TrimRange {
            start: 0.2,
            end: Some(0.4),
        },
        AspectRatio::R16x9,
        false,
    );
    let out = b.dir.path().join("trim.mp4");
    let (renderer, calls) = RecordingRenderer::new();
    Mp4Exporter::new(renderer)
        .export(
            &b.project,
            b.dir.path(),
            &CursorTrack::default(),
            ExportPreset::default(),
            &out,
            &CancelToken::new(),
            |_| {},
        )
        .unwrap();

    let p = paths.probe(&out).unwrap();
    assert!(near(p.duration, 0.2, 0.04));
    assert_eq!(p.video_frames, 12);

    let recorded = calls.lock();
    assert_eq!(recorded.len(), 12);
    assert!(near(recorded[0].time, 0.2, 1e-9));
    assert!(near(recorded[11].time, 0.2 + 11.0 / 60.0, 1e-9));
    let indexes: Vec<usize> = recorded.iter().map(|c| usize::from(c.gray) / 16).collect();
    assert_eq!(indexes, [6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11]);
}

#[test]
fn slow_source_holds_frames() {
    let Some(paths) = ffmpeg_or_skip() else {
        return;
    };
    let b = make_bundle(
        &paths,
        2,
        10,
        TrimRange {
            start: 0.0,
            end: None,
        },
        AspectRatio::R16x9,
        false,
    );
    let out = b.dir.path().join("slow.mp4");
    let (renderer, calls) = RecordingRenderer::new();
    Mp4Exporter::new(renderer)
        .export(
            &b.project,
            b.dir.path(),
            &CursorTrack::default(),
            ExportPreset::default(),
            &out,
            &CancelToken::new(),
            |_| {},
        )
        .unwrap();
    let indexes: Vec<usize> = calls
        .lock()
        .iter()
        .map(|c| usize::from(c.gray) / 16)
        .collect();
    assert_eq!(indexes.len(), 12);
    assert_eq!(indexes, [0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1]);
}

#[test]
fn mixes_audio_as_aac_48k_stereo() {
    let Some(paths) = ffmpeg_or_skip() else {
        return;
    };
    let b = make_bundle(
        &paths,
        15,
        30,
        TrimRange {
            start: 0.1,
            end: Some(0.4),
        },
        AspectRatio::R16x9,
        true,
    );
    let out = b.dir.path().join("audio.mp4");
    Mp4Exporter::new(PassthroughRenderer::new())
        .export(
            &b.project,
            b.dir.path(),
            &CursorTrack::default(),
            ExportPreset::default(),
            &out,
            &CancelToken::new(),
            |_| {},
        )
        .unwrap();
    let p = paths.probe(&out).unwrap();
    assert_eq!(p.audio_tracks, 1);
    assert_eq!(p.audio_sample_rate, 48_000);
    assert_eq!(p.audio_channels, 2);
    assert!(
        near(p.audio_duration, 0.3, 0.08),
        "audio={}",
        p.audio_duration
    );
    assert!(near(p.duration, 0.3, 0.08), "dur={}", p.duration);
}

#[test]
fn cancellation_leaves_no_files() {
    let Some(paths) = ffmpeg_or_skip() else {
        return;
    };
    let b = make_bundle(
        &paths,
        15,
        30,
        TrimRange {
            start: 0.0,
            end: None,
        },
        AspectRatio::R16x9,
        false,
    );
    let out_dir = b.dir.path().join("exports");
    let out = out_dir.join("cancel.mp4");
    let cancel = CancelToken::new();
    let cancel_cb = cancel.clone();
    let (renderer, calls) = RecordingRenderer::new();
    let renderer = renderer.with_hook(move |n| {
        if n == 3 {
            cancel_cb.cancel();
        }
    });
    let mut events = Vec::new();
    let res = Mp4Exporter::new(renderer).export(
        &b.project,
        b.dir.path(),
        &CursorTrack::default(),
        ExportPreset::default(),
        &out,
        &cancel,
        |e| events.push(e),
    );
    assert_eq!(res, Err(ExportError::Cancelled));
    assert!(!out.exists());
    let leftovers: Vec<PathBuf> = std::fs::read_dir(&out_dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .collect();
    assert!(leftovers.is_empty(), "leftovers={leftovers:?}");
    assert_eq!(events.last(), Some(&ExportProgress::Cancelled {}));
    assert!(calls.lock().len() < 30);
}

#[test]
fn missing_screen_file_fails_clearly_and_keeps_existing_output() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("keep.mp4");
    std::fs::write(&out, b"old").unwrap();
    let project = Project::new("t", RecordingAssets::new(PixelSize::new(10, 10), 60.0, 1.0));
    let mut events = Vec::new();
    let res = Mp4Exporter::new(PassthroughRenderer::new()).export(
        &project,
        dir.path(),
        &CursorTrack::default(),
        ExportPreset::default(),
        &out,
        &CancelToken::new(),
        |e| events.push(e),
    );
    assert!(res.is_err());
    assert_eq!(std::fs::read(&out).unwrap(), b"old");
    assert!(matches!(
        events.last(),
        Some(ExportProgress::Failed { message }) if message.contains("screen.mov")
    ));
}

#[test]
fn empty_trim_fails() {
    let Some(paths) = ffmpeg_or_skip() else {
        return;
    };
    let b = make_bundle(
        &paths,
        15,
        30,
        TrimRange {
            start: 0.3,
            end: Some(0.3),
        },
        AspectRatio::R16x9,
        false,
    );
    let res = Mp4Exporter::new(PassthroughRenderer::new()).export(
        &b.project,
        b.dir.path(),
        &CursorTrack::default(),
        ExportPreset::default(),
        &b.dir.path().join("o.mp4"),
        &CancelToken::new(),
        |_| {},
    );
    assert!(res.is_err());
}

#[test]
fn overwrites_existing_output() {
    let Some(paths) = ffmpeg_or_skip() else {
        return;
    };
    let b = make_bundle(
        &paths,
        15,
        30,
        TrimRange {
            start: 0.0,
            end: Some(0.1),
        },
        AspectRatio::R16x9,
        false,
    );
    let out = b.dir.path().join("o.mp4");
    std::fs::write(&out, b"old").unwrap();
    Mp4Exporter::new(PassthroughRenderer::new())
        .export(
            &b.project,
            b.dir.path(),
            &CursorTrack::default(),
            ExportPreset::default(),
            &out,
            &CancelToken::new(),
            |_| {},
        )
        .unwrap();
    assert_eq!(paths.probe(&out).unwrap().video_frames, 6);
}

#[test]
fn asset_frame_source_selects_at_or_before_and_seeks_backward() {
    let Some(paths) = ffmpeg_or_skip() else {
        return;
    };
    let b = make_bundle(
        &paths,
        15,
        30,
        TrimRange::default(),
        AspectRatio::R16x9,
        false,
    );
    let mut source = AssetFrameSource::open(&b.dir.path().join("screen.mov"), None).unwrap();
    let mut index = |t: MediaTime| -> usize {
        let img = source.screen_frame(t).unwrap().unwrap();
        usize::from(img.pixel(img.width / 2, img.height / 2)[0]) / 16
    };
    assert_eq!(index(0.0), 0);
    assert_eq!(index(0.05), 1); // 1/30 = 0.0333 <= 0.05 < 0.0667
    assert_eq!(index(0.2), 6);
    assert_eq!(index(0.1), 3); // backward seek
    assert_eq!(index(99.0), 14); // past end holds last frame
    assert!(source.camera_frame(0.0).unwrap().is_none());
    assert!(source.screen_error().is_none());
}

struct FailingSource {
    calls: usize,
    fail_after: usize,
    err: Option<NssError>,
}

impl FrameSource for FailingSource {
    fn screen_frame(&mut self, _t: MediaTime) -> Result<Option<Image>, FrameError> {
        self.calls += 1;
        if self.calls > self.fail_after {
            self.err = Some(NssError::Io("simulated decode failure".into()));
        }
        Ok(Some(Image::filled(16, 16, nss_core::RGBAColor::WHITE)))
    }
}

impl DecodeErrorReporting for FailingSource {
    fn screen_error(&self) -> Option<NssError> {
        self.err.clone()
    }
    fn camera_error(&self) -> Option<NssError> {
        None
    }
}

#[test]
fn decode_failure_mid_export_fails_and_leaves_no_file() {
    let Some(paths) = ffmpeg_or_skip() else {
        return;
    };
    let b = make_bundle(
        &paths,
        15,
        30,
        TrimRange {
            start: 0.0,
            end: Some(0.2),
        },
        AspectRatio::R16x9,
        false,
    );
    let out = b.dir.path().join("fail.mp4");
    let mut events = Vec::new();
    let res = Mp4Exporter::new(PassthroughRenderer::new())
        .with_frame_source_factory(Box::new(|_, _| {
            Ok(Box::new(FailingSource {
                calls: 0,
                fail_after: 3,
                err: None,
            }))
        }))
        .export(
            &b.project,
            b.dir.path(),
            &CursorTrack::default(),
            ExportPreset::default(),
            &out,
            &CancelToken::new(),
            |e| events.push(e),
        );
    assert!(res.is_err());
    assert!(!out.exists());
    assert!(matches!(
        events.last(),
        Some(ExportProgress::Failed { message }) if message.contains("simulated decode failure")
    ));
}

#[test]
fn export_matches_preview_frame_for_frame() {
    let Some(paths) = ffmpeg_or_skip() else {
        return;
    };
    let b = make_bundle(
        &paths,
        10,
        30,
        TrimRange {
            start: 0.0,
            end: Some(0.1),
        },
        AspectRatio::R16x9,
        false,
    );
    let out = b.dir.path().join("parity.mp4");
    let preset = ExportPreset {
        resolution: ExportResolution::P1080,
        fps: 60,
        codec: VideoCodec::H264,
    };
    let output_size = preset.output_size(b.project.style.aspect_ratio);
    let track = CursorTrack::default();

    // Render using real wgpu Renderer if an adapter is present, else PassthroughRenderer.
    let mut preview_frames: Vec<Frame> = Vec::new();
    let screen_path = b.dir.path().join("screen.mov");
    let motion = SpringMotionModel::new(
        &track,
        &b.project.timeline,
        b.project.style.cursor,
        b.project.assets.duration,
        b.project.assets.source_pixel_size.aspect(),
        MotionTuning::default(),
    );
    let style = b.project.style.resolving_assets(b.dir.path());

    if let Ok(mut gpu) = nss_render::Renderer::new() {
        let mut src = AssetFrameSource::open(&screen_path, None).unwrap();
        for i in 0..6 {
            let t = f64::from(i) / 60.0;
            let state = motion.frame_state(t);
            let screen = src.screen_frame(t).unwrap().unwrap();
            preview_frames.push(
                gpu.render(&screen, None, &state, &style, output_size)
                    .unwrap(),
            );
        }
        Mp4Exporter::new(gpu)
            .export(
                &b.project,
                b.dir.path(),
                &track,
                preset,
                &out,
                &CancelToken::new(),
                |_| {},
            )
            .unwrap();
    } else {
        let mut cpu = PassthroughRenderer::new();
        let mut src = AssetFrameSource::open(&screen_path, None).unwrap();
        for i in 0..6 {
            let t = f64::from(i) / 60.0;
            let state = motion.frame_state(t);
            let screen = src.screen_frame(t).unwrap().unwrap();
            preview_frames.push(
                cpu.render(&screen, None, &state, &style, output_size)
                    .unwrap(),
            );
        }
        Mp4Exporter::new(cpu)
            .export(
                &b.project,
                b.dir.path(),
                &track,
                preset,
                &out,
                &CancelToken::new(),
                |_| {},
            )
            .unwrap();
    }

    // Decode every frame back from the exported MP4 and compare against preview frames.
    let mut exported = AssetFrameSource::open(&out, None).unwrap();
    for (i, expected) in preview_frames.iter().enumerate() {
        let t = (i as f64) / 60.0;
        let actual = exported.screen_frame(t).unwrap().unwrap();
        assert_eq!(
            (actual.width, actual.height),
            (expected.width, expected.height)
        );
        let mut diff_sum: u64 = 0;
        let sample_step = 17usize;
        let mut count: u64 = 0;
        for px_idx in (0..(expected.pixels.len() / 4)).step_by(sample_step) {
            let off = px_idx * 4;
            for c in 0..3 {
                diff_sum += u64::from(expected.pixels[off + c].abs_diff(actual.pixels[off + c]));
                count += 1;
            }
        }
        let mae = (diff_sum as f64) / (count as f64);
        assert!(mae < 4.0, "frame {i} diverged from preview: MAE={mae:.2}");
    }
}
