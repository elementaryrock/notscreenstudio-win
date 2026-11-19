//! Editor preview engine: shares the exact `nss-motion` + `nss-render` + `AssetFrameSource`
//! pipeline with `Mp4Exporter` so preview and export agree frame-for-frame (ADR 0003).

use std::path::{Path, PathBuf};

use nss_core::{
    Background, CursorTrack, MediaTime, MotionModel, NssError, PixelSize, Project, RGBAColor, Style,
};
use nss_export::AssetFrameSource;
use nss_motion::{MotionTuning, SpringMotionModel};
use nss_render::{
    BackgroundRasterizer, FrameRenderer, FrameSource, Image, PassthroughRenderer, Renderer,
    SolidColorFrameSource,
};

enum BackendRenderer {
    Gpu(Box<Renderer>),
    Cpu(PassthroughRenderer),
}

impl BackendRenderer {
    fn new() -> Self {
        match Renderer::new() {
            Ok(r) => Self::Gpu(Box::new(r)),
            Err(_) => Self::Cpu(PassthroughRenderer::new()),
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Gpu(r) => r.name(),
            Self::Cpu(r) => r.name(),
        }
    }

    fn render(
        &mut self,
        screen: &Image,
        camera: Option<&Image>,
        state: &nss_core::FrameState,
        style: &Style,
        output_size: PixelSize,
    ) -> Result<Image, NssError> {
        match self {
            Self::Gpu(r) => r
                .render(screen, camera, state, style, output_size)
                .map_err(|e| NssError::RenderFailed(e.to_string())),
            Self::Cpu(r) => r
                .render(screen, camera, state, style, output_size)
                .map_err(|e| NssError::RenderFailed(e.to_string())),
        }
    }
}

/// Active editor preview session for one open project bundle.
pub struct PreviewEngine {
    pub bundle_dir: PathBuf,
    pub project: Project,
    pub track: CursorTrack,
    duration: f64,
    motion: SpringMotionModel,
    frames: Box<dyn FrameSource>,
    renderer: BackendRenderer,
}

impl PreviewEngine {
    /// Opens a project bundle for live preview, falling back to a synthetic frame source only when
    /// the bundle has no video file yet (e.g. an empty demo bundle).
    pub fn open(bundle_dir: PathBuf, project: Project, track: CursorTrack) -> Self {
        let screen_path = bundle_dir.join(&project.assets.screen_file);
        let camera_path = project
            .assets
            .camera_file
            .as_ref()
            .map(|f| bundle_dir.join(f))
            .filter(|p| p.is_file());

        let (frames, duration): (Box<dyn FrameSource>, f64) =
            match AssetFrameSource::open(&screen_path, camera_path.as_deref()) {
                Ok(src) => {
                    let dur = if src.screen_duration() > 0.0 {
                        src.screen_duration()
                    } else {
                        project.assets.duration.max(0.1)
                    };
                    (Box::new(src), dur)
                }
                Err(_) => {
                    let w = project.assets.source_pixel_size.width.max(320);
                    let h = project.assets.source_pixel_size.height.max(180);
                    let solid = SolidColorFrameSource::new(
                        w,
                        h,
                        RGBAColor::new(0.12, 0.14, 0.19, 1.0),
                        project.assets.camera_file.is_some(),
                    );
                    (Box::new(solid), project.assets.duration.max(1.0))
                }
            };

        let motion = build_motion(&track, &project, duration);
        let renderer = BackendRenderer::new();

        Self {
            bundle_dir,
            project,
            track,
            duration,
            motion,
            frames,
            renderer,
        }
    }

    #[must_use]
    pub fn renderer_name(&self) -> &'static str {
        self.renderer.name()
    }

    #[must_use]
    pub fn duration(&self) -> f64 {
        self.duration
    }

    /// Updates project style/timeline and rebuilds the motion table only if timeline or cursor
    /// style changed (fast interactive slider updates for background/padding/shadow cost 0 rebuilds).
    pub fn update_project(&mut self, next: Project) {
        let motion_changed = next.timeline != self.project.timeline
            || next.style.cursor != self.project.style.cursor
            || next.assets.source_pixel_size != self.project.assets.source_pixel_size;
        self.project = next;
        if motion_changed {
            self.motion = build_motion(&self.track, &self.project, self.duration);
        }
    }

    /// Renders a single composited preview frame at `t` fitted within `(max_width, max_height)`
    /// while preserving the project's active `AspectRatio`.
    ///
    /// Returns `(width, height, tightly_packed_rgba8)` with zero PNG/base64 encoding so Tauri's raw
    /// `ipc::Response` transfers it directly into `<canvas>`.
    pub fn render_frame(
        &mut self,
        t: MediaTime,
        max_width: u32,
        max_height: u32,
    ) -> Result<(u32, u32, Vec<u8>), NssError> {
        let clamped_t = t.clamp(0.0, self.duration);
        let state = self.motion.frame_state(clamped_t);
        let screen = self
            .frames
            .screen_frame(clamped_t)
            .map_err(|e| NssError::Io(e.to_string()))?
            .unwrap_or_else(|| {
                Image::filled(
                    self.project.assets.source_pixel_size.width.max(16),
                    self.project.assets.source_pixel_size.height.max(16),
                    RGBAColor::new(0.1, 0.12, 0.16, 1.0),
                )
            });
        let camera = self
            .frames
            .camera_frame(clamped_t)
            .map_err(|e| NssError::Io(e.to_string()))?;

        let out_size = fitted_preview_size(
            self.project
                .style
                .aspect_ratio
                .output_size(nss_core::ExportResolution::P1080)
                .aspect(),
            max_width.clamp(64, 1920),
            max_height.clamp(64, 1080),
        );
        let resolved_style = self.project.style.resolving_assets(&self.bundle_dir);
        let rendered =
            self.renderer
                .render(&screen, camera.as_ref(), &state, &resolved_style, out_size)?;
        Ok((rendered.width, rendered.height, rendered.pixels))
    }

    /// Extracts `count` evenly spaced downscaled RGBA thumbnails from the screen recording for the
    /// timeline video strip.
    #[must_use]
    pub fn timeline_thumbnails(
        &mut self,
        count: usize,
        thumb_w: u32,
        thumb_h: u32,
    ) -> Vec<(f64, u32, u32, Vec<u8>)> {
        let n = count.clamp(1, 24);
        let tw = thumb_w.clamp(32, 240);
        let th = thumb_h.clamp(18, 135);
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let frac = if n == 1 {
                0.0
            } else {
                (i as f64) / ((n - 1) as f64)
            };
            let t = frac * self.duration;
            if let Ok(Some(img)) = self.frames.screen_frame(t) {
                let small = downscale_nearest(&img, tw, th);
                out.push((t, tw, th, small));
            }
        }
        out
    }
}

/// Renders a procedural background swatch (solid, gradient, mesh, wallpaper) on the CPU with the
/// exact shader maths (`BackgroundRasterizer`) for the inspector background picker.
#[must_use]
pub fn render_background_swatch(background: &Background, width: u32, height: u32) -> Vec<u8> {
    let w = width.clamp(8, 256) as usize;
    let h = height.clamp(8, 256) as usize;
    BackgroundRasterizer::shading(background, w as f64, h as f64).to_rgba(w, h)
}

/// Discovers installed Windows desktop wallpapers from `%WINDIR%\Web\Wallpaper` and
/// `%APPDATA%\Microsoft\Windows\Themes` (PLAN.md §5 Stage 9).
#[must_use]
pub fn discover_system_wallpapers() -> Vec<WallpaperEntry> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(windir) = std::env::var_os("WINDIR") {
        dirs.push(PathBuf::from(windir).join("Web").join("Wallpaper"));
    }
    if let Some(appdata) = std::env::var_os("APPDATA") {
        dirs.push(
            PathBuf::from(appdata)
                .join("Microsoft")
                .join("Windows")
                .join("Themes"),
        );
    }

    let mut found = Vec::new();
    for dir in dirs {
        collect_images(&dir, &mut found, 3);
    }
    found.sort_by_key(|a| a.name.to_lowercase());
    found.dedup_by(|a, b| a.path.eq_ignore_ascii_case(&b.path));
    found.truncate(40);
    found
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WallpaperEntry {
    pub name: String,
    pub path: String,
}

fn collect_images(dir: &Path, out: &mut Vec<WallpaperEntry>, depth: usize) {
    if depth == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_images(&path, out, depth - 1);
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if matches!(
                ext.to_ascii_lowercase().as_str(),
                "jpg" | "jpeg" | "png" | "webp" | "bmp"
            ) {
                let name = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "Wallpaper".into());
                out.push(WallpaperEntry {
                    name,
                    path: path.to_string_lossy().into_owned(),
                });
            }
        }
    }
}

fn build_motion(track: &CursorTrack, project: &Project, duration: f64) -> SpringMotionModel {
    SpringMotionModel::new(
        track,
        &project.timeline,
        project.style.cursor,
        duration,
        project.assets.source_pixel_size.aspect(),
        MotionTuning::default(),
    )
}

fn fitted_preview_size(aspect: f64, max_w: u32, max_h: u32) -> PixelSize {
    let a = if aspect.is_finite() && aspect > 0.0 {
        aspect
    } else {
        16.0 / 9.0
    };
    let mw = f64::from(max_w);
    let mh = f64::from(max_h);
    let (w, h) = if mw / mh > a {
        (mh * a, mh)
    } else {
        (mw, mw / a)
    };
    let even = |v: f64| (((v / 2.0).round() * 2.0).max(2.0)) as u32;
    PixelSize::new(even(w), even(h))
}

fn downscale_nearest(src: &Image, dst_w: u32, dst_h: u32) -> Vec<u8> {
    let mut out = vec![0u8; (dst_w as usize) * (dst_h as usize) * 4];
    for y in 0..dst_h {
        let sy = ((y as u64) * u64::from(src.height) / u64::from(dst_h)) as u32;
        for x in 0..dst_w {
            let sx = ((x as u64) * u64::from(src.width) / u64::from(dst_w)) as u32;
            let px = src.pixel(sx, sy);
            let off = ((y * dst_w + x) as usize) * 4;
            out[off..off + 4].copy_from_slice(&px);
        }
    }
    out
}
