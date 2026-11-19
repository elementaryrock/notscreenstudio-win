//! Cross-crate integration & parity tests porting upstream `NSSIntegrationTests`:
//! - `AssetNameSafetyTests` (bundle path-traversal guard)
//! - `CursorMappingTests` (capture stream plan <-> input logger coordinate alignment)
//! - `BackgroundParityTests` (linear, mesh, blurred bundle image, blurred system wallpaper export vs GPU render)
//! - `EdgeGapTests` (corner & edge auto-zoom across 16:9 / 1:1 / 9:16 with zero background bleed or smear)
//! - `PreviewPerformanceTests` / `RendererHeadroomTests` (busy-scene 1080p GPU timing; budget gated on `NSS_BENCH=1`)

use std::collections::HashSet;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Instant;

use image::{ImageBuffer, Rgba};
use nss_capture::ScreenStreamPlan;
use nss_core::{
    background_presets, AspectRatio, Background, CaptureSource, ClickEvent, ClickKind,
    CursorSample, CursorShape, CursorShapeChange, CursorStyle, CursorTrack, ExportPreset,
    ExportResolution, MotionModel, MouseButton, PixelSize, Point2, Project, RGBAColor,
    RecordingAssets, Rect, Size2, Style, Timeline, TrimRange, VideoCodec,
};
use nss_export::{AssetFrameSource, CancelToken, FfmpegPaths, Mp4Exporter};
use nss_input::{CoordinateMath, CursorRecorder, SamplingSchedule, ScreenGeometryProvider};
use nss_motion::{CameraViewport, MotionTuning, SpringMotionModel};
use nss_project::{FileProjectLibrary, FileProjectStore};
use nss_render::{Frame, FrameRenderer, FrameSource, Image, RenderLayout, Renderer};
use parking_lot::Mutex;
use tempfile::TempDir;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn ffmpeg_or_skip() -> Option<FfmpegPaths> {
    FfmpegPaths::discover().ok()
}

fn gpu_or_skip() -> Option<Renderer> {
    Renderer::new().ok()
}

fn run_bench() -> bool {
    std::env::var("NSS_BENCH").ok().as_deref() == Some("1")
}

// ----------------------------------------------------------------------------
// 1. AssetNameSafetyTests
// ----------------------------------------------------------------------------

#[test]
fn asset_name_safety_rejects_traversal_and_accepts_plain_names() {
    for bad in [
        "../x.jpg",
        "a/b.jpg",
        "/etc/passwd",
        "..",
        ".",
        "",
        "a\\b",
        "x\0y",
    ] {
        assert!(
            !RecordingAssets::is_safe_file_name(bad),
            "should reject disallowed asset name: {bad:?}"
        );
    }

    for good in [
        "thumbnail.jpg",
        "screen.mov",
        "mic.m4a",
        "my file (1).mov",
        "..hidden",
    ] {
        assert!(
            RecordingAssets::is_safe_file_name(good),
            "should accept plain name: {good:?}"
        );
    }

    let base = RecordingAssets::new(PixelSize::new(10, 10), 60.0, 1.0);
    assert!(base.unsafe_file_name().is_none());

    let edits: [fn(&mut RecordingAssets); 5] = [
        |a| a.screen_file = "../s.mov".into(),
        |a| a.cursor_file = "a/c.json".into(),
        |a| a.camera_file = Some("../c.mov".into()),
        |a| a.mic_file = Some("/m.m4a".into()),
        |a| a.thumbnail_file = Some("../../thumb.jpg".into()),
    ];
    for edit in edits {
        let mut a = base.clone();
        edit(&mut a);
        assert!(a.unsafe_file_name().is_some());
    }
}

#[test]
fn store_and_library_refuse_bundle_naming_file_outside_it() {
    let dir = TempDir::new().unwrap();
    let store = FileProjectStore::at(dir.path());
    let reference = store.create_project("evil").unwrap();

    let mut assets = RecordingAssets::new(PixelSize::new(10, 10), 60.0, 1.0);
    assets.thumbnail_file = Some("../../outside.jpg".into());
    store
        .save(&Project::new("evil", assets), &reference)
        .unwrap();

    assert!(
        store.load(&reference).is_err(),
        "store.load must refuse path-traversal asset filename"
    );
    let summaries = FileProjectLibrary::at(dir.path()).summaries();
    assert!(
        summaries.is_empty(),
        "library listing must skip hostile bundles"
    );
}

// ----------------------------------------------------------------------------
// 2. CursorMappingTests (nss-capture ScreenStreamPlan <-> nss-input CursorRecorder)
// ----------------------------------------------------------------------------

struct MovingGeometry {
    rect: Mutex<Rect>,
}

impl MovingGeometry {
    fn new(r: Rect) -> Self {
        Self {
            rect: Mutex::new(r),
        }
    }
    fn move_to(&self, r: Rect) {
        *self.rect.lock() = r;
    }
}

impl ScreenGeometryProvider for MovingGeometry {
    fn global_rect(&self, _source: CaptureSource) -> Option<Rect> {
        Some(*self.rect.lock())
    }
    fn is_dynamic(&self, source: CaptureSource) -> bool {
        matches!(source, CaptureSource::Window { .. })
    }
}

fn logged_video_px(
    global: Point2,
    source: CaptureSource,
    geometry: Arc<MovingGeometry>,
    video_pixels: PixelSize,
) -> (Point2, Point2) {
    let recorder = CursorRecorder::new(
        source,
        geometry,
        SamplingSchedule::new(120.0),
        Arc::new(move || Some(global)),
        Arc::new(|| 10.0),
    );
    recorder.tick();
    recorder.click(global, 10.001, MouseButton::Left, ClickKind::Down);
    let track = recorder.finish(9.0, 120.0);
    let s = track.samples.last().expect("sample");
    let c = track.clicks.first().expect("click");
    assert!((s.t - 1.0).abs() < 1e-3);
    assert!((c.t - 1.001).abs() < 1e-3);
    let to_px = |x: f64, y: f64| {
        Point2::new(
            x * f64::from(video_pixels.width),
            y * f64::from(video_pixels.height),
        )
    };
    (to_px(s.x, s.y), to_px(c.x, c.y))
}

fn assert_point_near(a: Point2, b: Point2, tol: f64) {
    assert!(
        (a.x - b.x).abs() <= tol && (a.y - b.y).abs() <= tol,
        "{a:?} vs {b:?}"
    );
}

#[test]
fn cursor_mapping_matches_capture_plan_across_hidpi_secondary_area_and_moving_window() {
    // 1. HiDPI primary display
    let display = Rect::new(0.0, 0.0, 1440.0, 900.0);
    let plan = ScreenStreamPlan::make(display.size, 2.0, None, false, false).unwrap();
    assert_eq!(plan.pixel_size, PixelSize::new(2880, 1800));
    let (sample, click) = logged_video_px(
        Point2::new(360.0, 675.0),
        CaptureSource::Display { id: 1 },
        Arc::new(MovingGeometry::new(display)),
        plan.pixel_size,
    );
    assert_point_near(sample, Point2::new(720.0, 1350.0), 0.1);
    assert_point_near(click, sample, 0.1);

    // 2. Secondary display above and right of primary (negative Y)
    let secondary = Rect::new(1440.0, -200.0, 1920.0, 1080.0);
    let plan = ScreenStreamPlan::make(secondary.size, 1.0, None, false, false).unwrap();
    let (sample, _) = logged_video_px(
        Point2::new(1440.0 + 100.0, -200.0 + 540.0),
        CaptureSource::Display { id: 2 },
        Arc::new(MovingGeometry::new(secondary)),
        plan.pixel_size,
    );
    assert_point_near(sample, Point2::new(100.0, 540.0), 0.1);

    // 3. Area crop on HiDPI display
    let area = Rect::new(200.0, 100.0, 400.0, 300.0);
    let plan =
        ScreenStreamPlan::make(Size2::new(1920.0, 1080.0), 2.0, Some(area), false, false).unwrap();
    assert_eq!(plan.pixel_size, PixelSize::new(800, 600));
    assert_eq!(plan.source_rect, Some(area));
    let global = CoordinateMath::resolve(
        CaptureSource::Area {
            display_id: 2,
            rect: area,
        },
        |_| Some((Rect::new(1440.0, 0.0, 3840.0, 2160.0), 2.0)),
        |_| None,
    )
    .unwrap();
    let (sample, _) = logged_video_px(
        Point2::new(global.min_x() + 100.0, global.min_y() + 120.0),
        CaptureSource::Area {
            display_id: 2,
            rect: area,
        },
        Arc::new(MovingGeometry::new(global)),
        plan.pixel_size,
    );
    assert_point_near(sample, Point2::new(100.0, 120.0), 0.1);

    // 4. Moving window
    let start = Rect::new(100.0, 80.0, 800.0, 600.0);
    let geom = Arc::new(MovingGeometry::new(start));
    let plan = ScreenStreamPlan::make(start.size, 2.0, None, true, false).unwrap();
    assert_eq!(plan.pixel_size, PixelSize::new(1600, 1200));
    let now = Arc::new(Mutex::new(10.0f64));
    let now_clone = Arc::clone(&now);
    let still = Point2::new(500.0, 380.0);
    let recorder = CursorRecorder::new(
        CaptureSource::Window { id: 9 },
        Arc::clone(&geom) as Arc<dyn ScreenGeometryProvider>,
        SamplingSchedule::new(120.0),
        Arc::new(move || Some(still)),
        Arc::new(move || *now_clone.lock()),
    );
    recorder.tick();
    geom.move_to(Rect::new(250.0, 120.0, 800.0, 600.0));
    recorder.refresh_rect();
    *now.lock() = 11.0;
    recorder.tick();
    let track = recorder.finish(9.5, 120.0);
    let px = |s: CursorSample| Point2::new(s.x * 1600.0, s.y * 1200.0);
    assert_point_near(
        px(track.samples[0]),
        Point2::new((500.0 - 100.0) * 2.0, (380.0 - 80.0) * 2.0),
        0.1,
    );
    assert_point_near(
        px(track.samples[1]),
        Point2::new((500.0 - 250.0) * 2.0, (380.0 - 120.0) * 2.0),
        0.1,
    );
}

// ----------------------------------------------------------------------------
// Helpers for synthetic ramp video & test PNGs
// ----------------------------------------------------------------------------

/// Writes an H.264 test clip whose pixel `(x, y)` encodes a smooth RGB coordinate ramp
/// (`R = x/W * 255`, `G = y/H * 255`, `B = 90`) — matches upstream `Synthetic` ramp so
/// `edge_gap_tests` can verify exact source UV sampling at every pixel.
fn write_ramp_video(
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

    let mut child = cmd.spawn().expect("spawn ffmpeg ramp writer");
    let mut frame_buf = vec![0u8; (width as usize) * (height as usize) * 4];
    for y in 0..height {
        let g = ((f64::from(y) + 0.5) / f64::from(height) * 255.0).round() as u8;
        for x in 0..width {
            let r = ((f64::from(x) + 0.5) / f64::from(width) * 255.0).round() as u8;
            let off = ((y * width + x) * 4) as usize;
            frame_buf[off] = r;
            frame_buf[off + 1] = g;
            frame_buf[off + 2] = 90;
            frame_buf[off + 3] = 255;
        }
    }
    {
        let mut stdin = child.stdin.take().unwrap();
        for _ in 0..frames {
            stdin.write_all(&frame_buf).unwrap();
        }
    }
    let out_status = child.wait_with_output().unwrap();
    assert!(
        out_status.status.success(),
        "ramp ffmpeg failed: {}",
        String::from_utf8_lossy(&out_status.stderr)
    );
}

/// 480x270 PNG: warm top-left (`R=240, B=40`), cool elsewhere (`R=30, B=220`), with stripes so
/// blur has contrast to smooth (mirrors upstream `BackgroundParityTests.writePicture`).
fn write_background_picture(path: &Path) {
    let (w, h) = (480u32, 270u32);
    let mut img: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::new(w, h);
    for (x, y, px) in img.enumerate_pixels_mut() {
        let warm = x < w / 2 && y < h / 2;
        let stripe: u8 = if (x / 6) % 2 == 0 { 0 } else { 40 };
        *px = Rgba([
            if warm { 240 } else { 30 },
            60 + stripe,
            if warm { 40 } else { 220 },
            255,
        ]);
    }
    img.save(path).expect("save background test png");
}

fn psnr_rgba(a: &Image, b: &Image, step: usize) -> f64 {
    assert_eq!((a.width, a.height), (b.width, b.height));
    let mut mse = 0.0f64;
    let mut n = 0usize;
    for y in (0..a.height as usize).step_by(step) {
        for x in (0..a.width as usize).step_by(step) {
            let off = (y * a.width as usize + x) * 4;
            for c in 0..3 {
                let d = f64::from(a.pixels[off + c]) - f64::from(b.pixels[off + c]);
                mse += d * d;
                n += 1;
            }
        }
    }
    if n == 0 || mse == 0.0 {
        return 99.0;
    }
    mse /= n as f64;
    10.0 * (255.0 * 255.0 / mse).log10()
}

// ----------------------------------------------------------------------------
// 3. BackgroundParityTests
// ----------------------------------------------------------------------------

#[test]
fn background_parity_export_matches_direct_gpu_render_across_all_kinds() {
    let Some(paths) = ffmpeg_or_skip() else {
        return;
    };
    let Some(renderer) = gpu_or_skip() else {
        return;
    };
    let Some(mut reference_renderer) = gpu_or_skip() else {
        return;
    };

    let tmp = TempDir::new().unwrap();
    let bundle = tmp.path().join("bg-parity.nssproj");
    std::fs::create_dir_all(&bundle).unwrap();
    let screen_path = bundle.join("screen.mov");
    write_ramp_video(&paths, &screen_path, 320, 180, 30, 30);

    let bundle_pic_name = "background-test-picture.png";
    write_background_picture(&bundle.join(bundle_pic_name));
    let ext_wall_path = tmp.path().join("external-wallpaper.png");
    write_background_picture(&ext_wall_path);

    let presets = background_presets::gradients();
    let linear_bg = presets
        .iter()
        .find(|p| matches!(p.background, Background::Gradient { .. }))
        .unwrap()
        .background
        .clone();
    let mesh_bg = presets
        .iter()
        .find(|p| matches!(p.background, Background::Mesh { .. }))
        .unwrap()
        .background
        .clone();

    let cases: [(&str, Background, f64, bool); 4] = [
        ("linearPreset", linear_bg, 0.0, false),
        ("meshPreset", mesh_bg, 0.0, false),
        (
            "bundleImageBlurred",
            Background::Image {
                file: bundle_pic_name.into(),
            },
            0.5,
            true,
        ),
        (
            "systemWallpaperBlurred",
            Background::SystemWallpaper {
                path: ext_wall_path.to_string_lossy().into_owned(),
            },
            0.3,
            true,
        ),
    ];

    let preset = ExportPreset {
        resolution: ExportResolution::P1080,
        fps: 30,
        codec: VideoCodec::H264,
    };

    for (label, bg, blur, check_warm_corner) in cases {
        let mut style = Style {
            background: bg,
            background_blur: blur,
            padding: 0.12,
            ..Style::default()
        };
        style.cursor.click_ripples = false;
        let assets = RecordingAssets::new(PixelSize::new(320, 180), 30.0, 1.0);
        let mut project = Project::new(label, assets);
        project.style = style;
        project.timeline = Timeline {
            trim: TrimRange::new(0.1, Some(0.5)),
            zoom_blocks: Vec::new(),
            auto_zoom_enabled: false,
        };

        let out_mp4 = bundle.join(format!("{label}.mp4"));
        let mut exporter = Mp4Exporter::new(Renderer::new().unwrap());
        let token = CancelToken::new();
        exporter
            .export(
                &project,
                &bundle,
                &CursorTrack::default(),
                preset,
                &out_mp4,
                &token,
                |_| {},
            )
            .unwrap_or_else(|e| panic!("{label} export failed: {e}"));

        let mut exported = AssetFrameSource::open(&out_mp4, None).unwrap();
        let mut src = AssetFrameSource::open(&screen_path, None).unwrap();
        let size = preset.output_size(project.style.aspect_ratio);
        let motion = SpringMotionModel::new(
            &CursorTrack::default(),
            &project.timeline,
            project.style.cursor,
            1.0,
            PixelSize::new(320, 180).aspect(),
            MotionTuning::default(),
        );
        let resolved_style = project.style.resolving_assets(&bundle);

        let mut min_psnr = f64::INFINITY;
        for i in [0u32, 4, 8] {
            let t_out = f64::from(i) / 30.0;
            let t_src = project.timeline.trim.start + t_out;
            let got = exported.screen_frame(t_out).unwrap().unwrap();
            let raw = src.screen_frame(t_src).unwrap().unwrap();
            let state = motion.frame_state(t_src);
            let want = reference_renderer
                .render(&raw, None, &state, &resolved_style, size)
                .unwrap();

            let p = psnr_rgba(&got, &want, 8);
            min_psnr = min_psnr.min(p);

            if i == 0 && check_warm_corner {
                let off = (8 * want.width as usize + 8) * 4;
                let (r, g, b) = (want.pixels[off], want.pixels[off + 1], want.pixels[off + 2]);
                assert!(
                    !(r.abs_diff(128) < 4 && g.abs_diff(128) < 4 && b.abs_diff(128) < 4),
                    "{label}: background fell back to missing-image grey ({r},{g},{b})"
                );
                assert!(
                    i32::from(r) > i32::from(b) + 40,
                    "{label}: expected warm top-left picture corner, got ({r},{g},{b})"
                );
            }
        }
        assert!(
            min_psnr > 28.0,
            "{label}: worst PSNR {min_psnr:.1} dB below threshold"
        );
    }
    drop(renderer);
}

// ----------------------------------------------------------------------------
// 4. EdgeGapTests (corner & edge clicks across 16:9, 1:1, 9:16)
// ----------------------------------------------------------------------------

#[test]
fn edge_gap_export_has_zero_bleed_across_all_three_aspect_ratios() {
    let Some(paths) = ffmpeg_or_skip() else {
        return;
    };
    if gpu_or_skip().is_none() {
        return;
    }

    let tmp = TempDir::new().unwrap();
    let bundle = tmp.path().join("edge-gaps.nssproj");
    std::fs::create_dir_all(&bundle).unwrap();
    let src_size = PixelSize::new(320, 180);
    let screen_path = bundle.join("screen.mov");

    let positions: [(f64, f64); 8] = [
        (0.0, 0.0),
        (1.0, 0.0),
        (1.0, 1.0),
        (0.0, 1.0),
        (0.5, 0.0),
        (1.0, 0.5),
        (0.5, 1.0),
        (0.0, 0.5),
    ];
    let spacing = 1.4;
    let duration = 0.6 + positions.len() as f64 * spacing + 0.6;
    let fps = 30u32;
    let total_src_frames = (duration * f64::from(fps)).round() as usize;
    write_ramp_video(
        &paths,
        &screen_path,
        src_size.width,
        src_size.height,
        total_src_frames,
        fps,
    );

    let mut samples = Vec::new();
    let mut clicks = Vec::new();
    for (idx, &(x, y)) in positions.iter().enumerate() {
        let t = 0.6 + idx as f64 * spacing;
        // Hold the pointer on the corner/edge across its dwell window so the pan spring settles there.
        for step in 0..12 {
            samples.push(CursorSample::new(t - 0.1 + f64::from(step) * 0.1, x, y));
        }
        clicks.push(ClickEvent::new(t, x, y, MouseButton::Left, ClickKind::Down));
        clicks.push(ClickEvent::new(
            t + 0.05,
            x,
            y,
            MouseButton::Left,
            ClickKind::Up,
        ));
    }
    let track = CursorTrack {
        samples,
        clicks,
        sample_rate: 60.0,
        shapes: Vec::new(),
    };

    for aspect in AspectRatio::ALL {
        let mut style = Style {
            background: Background::Solid(RGBAColor::new(0.10, 0.50, 0.90, 1.0)),
            aspect_ratio: aspect,
            ..Style::default()
        };
        style.cursor.click_ripples = false;
        let assets = RecordingAssets::new(src_size, f64::from(fps), duration);
        let mut project = Project::new("edges", assets);
        project.style = style;
        project.timeline = Timeline {
            trim: TrimRange::default(),
            zoom_blocks: Vec::new(),
            auto_zoom_enabled: true,
        };

        let out_mp4 = bundle.join(format!("edges-{aspect:?}.mp4"));
        let preset = ExportPreset {
            resolution: ExportResolution::P1080,
            fps: fps as i32,
            codec: VideoCodec::H264,
        };
        let mut exporter = Mp4Exporter::new(Renderer::new().unwrap());
        exporter
            .export(
                &project,
                &bundle,
                &track,
                preset,
                &out_mp4,
                &CancelToken::new(),
                |_| {},
            )
            .unwrap();

        let size = preset.output_size(aspect);
        let motion = SpringMotionModel::new(
            &track,
            &project.timeline,
            project.style.cursor,
            duration,
            src_size.aspect(),
            MotionTuning::default(),
        );
        let mut exported = AssetFrameSource::open(&out_mp4, None).unwrap();

        let mut peak_zoom = 1.0f64;
        let mut touched: HashSet<&'static str> = HashSet::new();

        // Sample across the exported video (every 6th frame keeps test snappy while hitting every click).
        for frame_i in (0..total_src_frames).step_by(6) {
            let t = frame_i as f64 / f64::from(fps);
            let state = motion.frame_state(t);
            let vp = state.camera.source_viewport();
            assert!(
                vp.is_inside_source(),
                "{aspect:?} frame {frame_i}: viewport {vp:?} escaped source"
            );
            peak_zoom = peak_zoom.max(state.camera.scale);
            if vp.min_x <= 1e-6 {
                touched.insert("left");
            }
            if vp.max_x >= 1.0 - 1e-6 {
                touched.insert("right");
            }
            if vp.min_y <= 1e-6 {
                touched.insert("top");
            }
            if vp.max_y >= 1.0 - 1e-6 {
                touched.insert("bottom");
            }

            let decoded = exported.screen_frame(t).unwrap().unwrap();
            let layout = RenderLayout::new(size, src_size, &project.style, state.camera);
            let rect = layout.content_rect;
            let cursor_out = layout.output_point(state.cursor.position);

            // Check points 5 px inside every edge of the screen content rect.
            let probes = [
                (rect.origin.x + 6.0, rect.origin.y + rect.size.height * 0.5),
                (
                    rect.origin.x + rect.size.width - 7.0,
                    rect.origin.y + rect.size.height * 0.5,
                ),
                (rect.origin.x + rect.size.width * 0.5, rect.origin.y + 6.0),
                (
                    rect.origin.x + rect.size.width * 0.5,
                    rect.origin.y + rect.size.height - 7.0,
                ),
            ];
            for (ox, oy) in probes {
                if (ox - cursor_out.x).hypot(oy - cursor_out.y) < 0.1 * f64::from(size.height) {
                    continue;
                }
                let u = (ox + 0.5 - rect.origin.x) / rect.size.width;
                let v = (oy + 0.5 - rect.origin.y) / rect.size.height;
                let want_r = (vp.min_x + u * vp.width()) * 255.0;
                let want_g = (vp.min_y + v * vp.height()) * 255.0;
                let px_x = (ox as u32).min(decoded.width - 1) as usize;
                let px_y = (oy as u32).min(decoded.height - 1) as usize;
                let off = (px_y * decoded.width as usize + px_x) * 4;
                let got_r = f64::from(decoded.pixels[off]);
                let got_g = f64::from(decoded.pixels[off + 1]);
                let err = (got_r - want_r).abs().max((got_g - want_g).abs());
                assert!(
                    err <= 18.0,
                    "{aspect:?} frame {frame_i} at ({px_x},{px_y}): got ({got_r:.0},{got_g:.0}) vs expected ({want_r:.0},{want_g:.0})"
                );
            }
        }
        assert!(peak_zoom > 1.3, "{aspect:?}: auto-zoom never engaged");
        assert_eq!(
            touched.len(),
            4,
            "{aspect:?}: viewport did not touch all 4 edges: {touched:?}"
        );
    }
}

// ----------------------------------------------------------------------------
// 5. PreviewPerformanceTests / RendererHeadroomTests
// ----------------------------------------------------------------------------

#[test]
fn renderer_headroom_1080p_busy_scene() {
    let Some(mut renderer) = gpu_or_skip() else {
        return;
    };

    let duration = 4.0;
    let mut clicks = Vec::new();
    let mut samples = Vec::new();
    for k in 0..5 {
        let t = 0.4 + f64::from(k) * 0.7;
        let x = if k % 2 == 0 { 0.15 } else { 0.85 };
        let y = 0.2 + 0.12 * f64::from(k);
        samples.push(CursorSample::new(t - 0.15, 1.0 - x, y));
        samples.push(CursorSample::new(t, x, y));
        clicks.push(ClickEvent::new(t, x, y, MouseButton::Left, ClickKind::Down));
        clicks.push(ClickEvent::new(
            t + 0.08,
            x,
            y,
            MouseButton::Left,
            ClickKind::Up,
        ));
    }
    let shapes_cycle = [
        CursorShape::Arrow,
        CursorShape::IBeam,
        CursorShape::PointingHand,
        CursorShape::ResizeLeftRight,
        CursorShape::Crosshair,
        CursorShape::ClosedHand,
        CursorShape::OpenHand,
    ];
    let shapes: Vec<CursorShapeChange> = (0..7)
        .map(|i| CursorShapeChange::new(i as f64 * 0.55, shapes_cycle[i]))
        .collect();
    let track = CursorTrack {
        samples,
        clicks,
        sample_rate: 120.0,
        shapes,
    };

    let style = Style {
        cursor: CursorStyle {
            size: 1.5,
            smoothing: 0.5,
            motion_blur: 1.0,
            click_ripples: true,
            hide_when_idle: false,
        },
        ..Style::default()
    };
    let timeline = Timeline {
        trim: TrimRange::default(),
        zoom_blocks: Vec::new(),
        auto_zoom_enabled: true,
    };
    let motion = SpringMotionModel::new(
        &track,
        &timeline,
        style.cursor,
        duration,
        16.0 / 9.0,
        MotionTuning::default(),
    );

    let screen = Frame::filled(640, 360, RGBAColor::new(0.19, 0.33, 0.56, 1.0));
    let camera = Frame::filled(320, 180, RGBAColor::new(0.77, 0.51, 0.36, 1.0));
    let out_size = PixelSize::new(1920, 1080);

    let mut render_ms = Vec::new();
    let frames = if run_bench() { 180 } else { 30 };
    for i in 0..frames {
        let t = f64::from(i) / 60.0;
        let state = motion.frame_state(t);
        let t0 = Instant::now();
        let _out = renderer
            .render(&screen, Some(&camera), &state, &style, out_size)
            .unwrap();
        if i >= 5 {
            render_ms.push(t0.elapsed().as_secs_f64() * 1000.0);
        }
    }
    render_ms.sort_by(|a, b| a.total_cmp(b));
    let p50 = render_ms[render_ms.len() / 2];
    let p95 = render_ms[(render_ms.len() - 1) * 95 / 100];
    eprintln!(
        "renderer 1920x1080 busy scene ({} frames): p50 {p50:.2} ms, p95 {p95:.2} ms",
        render_ms.len()
    );
    if run_bench() {
        assert!(
            p95 < 16.6,
            "1080p GPU render p95 ({p95:.2} ms) exceeded 60 fps 16.6 ms budget"
        );
    }
}
