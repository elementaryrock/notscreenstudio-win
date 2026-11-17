//! The wgpu compositor: one full-screen fragment pass draws the whole frame (background, screen
//! shadow, zoomed screen content with rounded corners, click ripples, the motion-blurred cursor and
//! the camera bubble).
//!
//! Port of upstream `NSSRender/MetalRenderer.swift` onto wgpu + WGSL. What changed, and why, is in
//! `docs/ADR/0011-renderer-parity.md`; the short version:
//!
//! * Frames are tightly packed RGBA8 sRGB (see [`crate::frame`]) instead of IOSurface 32BGRA, so a
//!   frame costs one upload per render instead of a zero-copy texture wrap.
//! * Every sample is an explicit `textureSampleLevel` (see `shader.wgsl`), which is what makes a
//!   preview frame and an export frame of the same instant byte-identical.
//! * `render` is `&mut self` and synchronous. Use one renderer per consumer (preview, export).
//!
//! The compiled shader, the cursor atlas, the placeholder texture and the sampler setup are created
//! once per renderer; per frame only the uniform write, three texture uploads and the pass remain.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use nss_core::{Background, BackgroundBlur, FrameState, NssError, PixelSize, Style};

use crate::cursor_atlas::CursorAtlas;
use crate::frame::{Frame, Image};
use crate::image_blur;
use crate::shaders::{self, Slot};
use crate::uniforms::{self, Scene};

/// What can go wrong setting up or running the compositor.
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("no GPU adapter available: {0}")]
    NoAdapter(String),
    #[error("GPU device request failed: {0}")]
    Device(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error(transparent)]
    Frame(#[from] crate::frame::FrameError),
    #[error("background image error: {0}")]
    Image(String),
}

impl From<RenderError> for NssError {
    fn from(e: RenderError) -> Self {
        match e {
            RenderError::NoAdapter(m) | RenderError::Device(m) => NssError::Unsupported(m),
            RenderError::Unsupported(m) => NssError::Unsupported(m),
            RenderError::Frame(e) => NssError::RenderFailed(e.to_string()),
            RenderError::Image(m) => NssError::RenderFailed(m),
        }
    }
}

/// The contract both compositors implement, so the shell can pick one at startup and the exporter
/// never has to care which.
pub trait FrameRenderer {
    /// Renders one frame. `output_size` is the export/preview size; the source aspect comes from
    /// `screen`.
    fn render(
        &mut self,
        screen: &Image,
        camera: Option<&Image>,
        state: &FrameState,
        style: &Style,
        output_size: PixelSize,
    ) -> Result<Frame, RenderError>;

    /// Short name for diagnostics and the smoke test.
    fn name(&self) -> &'static str;
}

/// Image formats a background image may have.
const MAX_IMAGE_SIDE: u32 = 4096;

/// `wgpu` compositor.
pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    adapter_name: String,
    format: wgpu::TextureFormat,
    pipeline: wgpu::RenderPipeline,
    blur_pipeline: wgpu::RenderPipeline,
    bind_layout: wgpu::BindGroupLayout,
    blur_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    atlas_sampler: wgpu::Sampler,
    uniform_buffer: wgpu::Buffer,
    blur_buffer: wgpu::Buffer,
    atlas: CursorAtlas,
    atlas_tex: GpuTexture,
    blank: GpuTexture,
    /// Reused across frames; recreated only when a size changes.
    screen_tex: Option<GpuTexture>,
    camera_tex: Option<GpuTexture>,
    out_tex: Option<GpuTexture>,
    staging: Option<(
        wgpu::Buffer,
        u32, /*padded row bytes*/
        u32, /*height*/
    )>,
    uniform_bytes: Vec<u8>,
    asset_base_url: Option<PathBuf>,
    image_cache: Option<ImageCacheEntry>,
    blur_cache: Option<BlurCacheEntry>,
}

impl std::fmt::Debug for Renderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Renderer")
            .field("adapter", &self.adapter_name)
            .field("format", &self.format)
            .field(
                "atlas",
                &format!("{}x{}", self.atlas.width, self.atlas.height),
            )
            .finish_non_exhaustive()
    }
}

struct GpuTexture {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    width: u32,
    height: u32,
}

impl GpuTexture {
    fn create(
        device: &wgpu::Device,
        label: &str,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
        mip_levels: u32,
        usage: wgpu::TextureUsages,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: mip_levels.max(1),
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            texture,
            view,
            width: width.max(1),
            height: height.max(1),
        }
    }

    fn upload(&self, queue: &wgpu::Queue, pixels: &[u8], mip_level: u32, width: u32, height: u32) {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
    }
}

struct DecodedImage {
    path: PathBuf,
    stamp: ImageStamp,
    /// Level 0 first, sRGB RGBA8.
    mips: Vec<(u32, u32, Vec<u8>)>,
}

struct ImageCacheEntry {
    decoded: Option<DecodedImage>,
}

struct BlurCacheEntry {
    path: PathBuf,
    stamp: ImageStamp,
    amount: f64,
    texture: Option<GpuTexture>,
}

/// Identity of a file on disk: modification time and size; all zero when missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ImageStamp {
    secs: u64,
    nanos: u32,
    size: u64,
}

impl ImageStamp {
    fn of(path: &Path) -> Self {
        match std::fs::metadata(path) {
            Ok(md) => {
                let mtime = md
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .unwrap_or_default();
                Self {
                    secs: mtime.as_secs(),
                    nanos: mtime.subsec_nanos(),
                    size: md.len(),
                }
            }
            Err(_) => Self {
                secs: 0,
                nanos: 0,
                size: 0,
            },
        }
    }

    fn exists(self) -> bool {
        self.size > 0
    }
}

impl Renderer {
    /// Creates a renderer on the default adapter (falling back to a software adapter).
    pub fn new() -> Result<Self, RenderError> {
        Self::with_backends(wgpu::Backends::from_env().unwrap_or(wgpu::Backends::PRIMARY))
    }

    /// Creates a renderer restricted to `backends`.
    pub fn with_backends(backends: wgpu::Backends) -> Result<Self, RenderError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = [
            // Prefer the discrete/integrated GPU, then anything including a software rasterizer so
            // CI and remote sessions still render.
            wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                ..wgpu::RequestAdapterOptions::default()
            },
            wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                force_fallback_adapter: true,
                ..wgpu::RequestAdapterOptions::default()
            },
        ]
        .into_iter()
        .find_map(|opts| pollster::block_on(instance.request_adapter(&opts)).ok())
        .ok_or_else(|| RenderError::NoAdapter("no wgpu adapter on this machine".to_owned()))?;

        let info = adapter.get_info();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("nss-render"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
            ..wgpu::DeviceDescriptor::default()
        }))
        .map_err(|e| RenderError::Device(format!("{e}")))?;

        Self::from_device(device, queue, info.name)
    }

    /// Builds a renderer on an existing device (used by tests and by the shell when it already has a
    /// device for the preview surface).
    pub fn from_device(
        device: wgpu::Device,
        queue: wgpu::Queue,
        adapter_name: String,
    ) -> Result<Self, RenderError> {
        let render_source = shaders::shader_source();
        let blur_source = shaders::blur_source();
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("nss compositor"),
            source: wgpu::ShaderSource::Wgsl(Cow::Owned(render_source)),
        });
        let blur_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("nss background blur"),
            source: wgpu::ShaderSource::Wgsl(Cow::Owned(blur_source)),
        });

        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("nss bind layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                sampler_entry(1),
                sampler_entry(2),
                texture_entry(3),
                texture_entry(4),
                texture_entry(5),
                texture_entry(6),
            ],
        });
        let blur_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("nss blur bind layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                sampler_entry(1),
                texture_entry(2),
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("nss pipeline layout"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });
        let blur_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("nss blur pipeline layout"),
            bind_group_layouts: &[Some(&blur_layout)],
            immediate_size: 0,
        });

        // The shader writes sRGB-encoded values itself (upstream did the same into `bgra8Unorm`), so
        // the target is the *unorm* format, not the `_srgb` one.
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("nss compositor"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("nss_vertex"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("nss_fragment"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(format.into())],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let blur_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("nss background blur"),
            layout: Some(&blur_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &blur_module,
                entry_point: Some("nss_blur_vertex"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &blur_module,
                entry_point: Some("nss_blur_fragment"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::TextureFormat::Rgba16Float.into())],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("nss linear clamp"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..wgpu::SamplerDescriptor::default()
        });
        // The shader already rejects samples more than 2 pt outside the image, and the atlas padding
        // is transparent, so clamp-to-edge is equivalent to Metal's clamp-to-zero here.
        let atlas_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("nss atlas"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..wgpu::SamplerDescriptor::default()
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("nss uniforms"),
            size: (Slot::COUNT * 16) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let blur_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("nss blur params"),
            size: 32,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let atlas = CursorAtlas::build();
        let mut atlas_tex = GpuTexture::create(
            &device,
            "nss cursor atlas",
            atlas.width,
            atlas.height,
            wgpu::TextureFormat::Rgba16Float,
            atlas.mip_levels(),
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        for (level, mip) in atlas.mips.iter().enumerate() {
            let mut halves = Vec::with_capacity(mip.pixels.len());
            for v in &mip.pixels {
                halves.extend_from_slice(&half::f16::from_f32(*v).to_le_bytes());
            }
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &atlas_tex.texture,
                    mip_level: level as u32,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &halves,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(mip.width * 8),
                    rows_per_image: Some(mip.height),
                },
                wgpu::Extent3d {
                    width: mip.width,
                    height: mip.height,
                    depth_or_array_layers: 1,
                },
            );
        }
        atlas_tex.width = atlas.width;
        atlas_tex.height = atlas.height;

        // A 1x1 opaque black texture for the camera/image slots when the scene has neither.
        let blank = GpuTexture::create(
            &device,
            "nss blank",
            1,
            1,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            1,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        blank.upload(&queue, &[0, 0, 0, 255], 0, 1, 1);

        Ok(Self {
            device,
            queue,
            adapter_name,
            format,
            pipeline,
            blur_pipeline,
            bind_layout,
            blur_layout,
            sampler,
            atlas_sampler,
            uniform_buffer,
            blur_buffer,
            atlas,
            atlas_tex,
            blank,
            screen_tex: None,
            camera_tex: None,
            out_tex: None,
            staging: None,
            uniform_bytes: Vec::with_capacity(Slot::COUNT * 16),
            asset_base_url: None,
            image_cache: None,
            blur_cache: None,
        })
    }

    /// The adapter this renderer is running on.
    #[must_use]
    pub fn adapter_name(&self) -> &str {
        &self.adapter_name
    }

    /// Folder used to resolve relative `Background::Image` names (normally the project bundle).
    pub fn set_asset_base_url(&mut self, url: Option<PathBuf>) {
        if self.asset_base_url != url {
            self.asset_base_url = url;
            self.invalidate_background_image();
        }
    }

    /// The baked cursor atlas (for hit testing in the editor).
    #[must_use]
    pub fn atlas(&self) -> &CursorAtlas {
        &self.atlas
    }

    /// Drops the cached background image so the next frame reloads it from disk. The cache already
    /// reloads when the file's path, stamp or size changes, so this is only needed when a file is
    /// rewritten in place with identical size within the filesystem's timestamp resolution.
    pub fn invalidate_background_image(&mut self) {
        self.image_cache = None;
        self.blur_cache = None;
    }

    /// Loads and caches the background image for `style` now, so the first frame that needs it does
    /// not pay the decode cost.
    pub fn preload(&mut self, style: &Style) {
        let _ = self.background_texture(style);
    }

    // MARK: - FrameRenderer

    fn render_impl(
        &mut self,
        screen: &Image,
        camera: Option<&Image>,
        state: &FrameState,
        style: &Style,
        output_size: PixelSize,
    ) -> Result<Frame, RenderError> {
        let out_w = output_size.width.max(1);
        let out_h = output_size.height.max(1);

        // Screen.
        let screen_tex = self.upload_screen(screen);
        // Camera, only when the bubble will actually be drawn.
        let camera_tex = camera.map(|c| self.upload_camera(c));

        // Background image (decoded + optionally blurred), if the style wants one.
        let (image_size, image_lod, image_view) = match self.background_texture(style) {
            BackgroundTexture::None => (None, 0.0, None),
            BackgroundTexture::Sharp { size, lod, view } => (Some(size), lod, Some(view)),
            BackgroundTexture::Blurred { size, view } => (Some(size), 0.0, Some(view)),
        };

        let layout =
            crate::layout::RenderLayout::new(output_size, screen.size(), style, state.camera);
        let uniforms = uniforms::write(&Scene {
            layout: &layout,
            state,
            style,
            camera: camera.map(|c| c.size()),
            image: image_size,
            image_lod,
            atlas: &self.atlas,
            dither: true,
        });
        uniforms.write_bytes(&mut self.uniform_bytes);
        self.queue
            .write_buffer(&self.uniform_buffer, 0, &self.uniform_bytes);

        let out = self.ensure_output(out_w, out_h);
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("nss bind group"),
            layout: &self.bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.atlas_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&screen_tex.view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(
                        camera_tex.as_ref().map_or(&self.blank.view, |c| &c.view),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(
                        image_view.as_ref().unwrap_or(&self.blank.view),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(&self.atlas_tex.view),
                },
            ],
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("nss frame"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("nss compositor"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &out.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        // Copy the result out through the padded staging buffer (the copy stride must be a multiple
        // of 256 bytes, so the row padding is stripped below).
        let (staging, padded_row, _) = self.ensure_staging(out_w, out_h);
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &out.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_row),
                    rows_per_image: Some(out_h),
                },
            },
            wgpu::Extent3d {
                width: out_w,
                height: out_h,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(encoder.finish()));

        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |res| {
            let _ = tx.send(res);
        });
        self.device.poll(wgpu::PollType::wait_indefinitely()).ok();
        match rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(RenderError::Device(format!("readback failed: {e}"))),
            Err(e) => return Err(RenderError::Device(format!("readback callback lost: {e}"))),
        }

        let map = slice
            .get_mapped_range()
            .map_err(|e| RenderError::Device(format!("readback map failed: {e}")))?;
        let row_bytes = out_w as usize * 4;
        let mut pixels = Vec::with_capacity(row_bytes * out_h as usize);
        for row in 0..out_h as usize {
            let start = row * padded_row as usize;
            pixels.extend_from_slice(&map[start..start + row_bytes]);
        }
        drop(map);
        staging.unmap();
        Ok(Image::new(out_w, out_h, pixels)?)
    }

    // MARK: - Resources

    fn upload_screen(&mut self, screen: &Image) -> GpuTexture {
        let reuse = self
            .screen_tex
            .as_ref()
            .is_some_and(|t| t.width == screen.width && t.height == screen.height);
        if !reuse {
            self.screen_tex = Some(GpuTexture::create(
                &self.device,
                "nss screen",
                screen.width,
                screen.height,
                wgpu::TextureFormat::Rgba8UnormSrgb,
                1,
                wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            ));
        }
        let tex = self.screen_tex.take().expect("just ensured");
        tex.upload(&self.queue, &screen.pixels, 0, screen.width, screen.height);
        self.screen_tex = Some(tex);
        self.screen_tex
            .as_ref()
            .map(|t| GpuTexture {
                texture: t.texture.clone(),
                view: t.view.clone(),
                width: t.width,
                height: t.height,
            })
            .expect("screen texture")
    }

    fn upload_camera(&mut self, camera: &Image) -> GpuTexture {
        let reuse = self
            .camera_tex
            .as_ref()
            .is_some_and(|t| t.width == camera.width && t.height == camera.height);
        if !reuse {
            self.camera_tex = Some(GpuTexture::create(
                &self.device,
                "nss camera",
                camera.width,
                camera.height,
                wgpu::TextureFormat::Rgba8UnormSrgb,
                1,
                wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            ));
        }
        let tex = self.camera_tex.take().expect("just ensured");
        tex.upload(&self.queue, &camera.pixels, 0, camera.width, camera.height);
        self.camera_tex = Some(tex);
        self.camera_tex
            .as_ref()
            .map(|t| GpuTexture {
                texture: t.texture.clone(),
                view: t.view.clone(),
                width: t.width,
                height: t.height,
            })
            .expect("camera texture")
    }

    fn ensure_output(&mut self, width: u32, height: u32) -> GpuTexture {
        let reuse = self
            .out_tex
            .as_ref()
            .is_some_and(|t| t.width == width && t.height == height);
        if !reuse {
            self.out_tex = Some(GpuTexture::create(
                &self.device,
                "nss output",
                width,
                height,
                self.format,
                1,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            ));
        }
        let t = self.out_tex.as_ref().expect("just ensured");
        GpuTexture {
            texture: t.texture.clone(),
            view: t.view.clone(),
            width: t.width,
            height: t.height,
        }
    }

    fn ensure_staging(&mut self, width: u32, height: u32) -> (wgpu::Buffer, u32, u32) {
        let row = width * 4;
        let padded = row.div_ceil(256) * 256;
        let needed = u64::from(padded) * u64::from(height);
        let recreate = self
            .staging
            .as_ref()
            .is_none_or(|(b, p, h)| *p != padded || *h != height || b.size() < needed);
        if recreate {
            self.staging = Some((
                self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("nss readback"),
                    size: needed,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                }),
                padded,
                height,
            ));
        }
        let (b, p, h) = self.staging.as_ref().expect("just ensured");
        (b.clone(), *p, *h)
    }

    /// Resolves a style's background image to a texture, caching the decode and the blur.
    fn background_texture(&mut self, style: &Style) -> BackgroundTexture {
        let file = match &style.background {
            Background::Image { file } => file.clone(),
            Background::SystemWallpaper { path } => path.clone(),
            _ => return BackgroundTexture::None,
        };
        if file.is_empty() {
            return BackgroundTexture::None;
        }
        let path = self.resolve(&file);
        let stamp = ImageStamp::of(&path);

        let decoded = match &self.image_cache {
            Some(entry)
                if entry
                    .decoded
                    .as_ref()
                    .is_some_and(|d| d.path == path && d.stamp == stamp) =>
            {
                entry.decoded.as_ref().map(read_decoded)
            }
            _ => {
                let decoded = if stamp.exists() {
                    decode_image(&path, stamp)
                } else {
                    None
                };
                self.image_cache = Some(ImageCacheEntry { decoded });
                self.image_cache
                    .as_ref()
                    .and_then(|e| e.decoded.as_ref())
                    .map(read_decoded)
            }
        };
        let Some(decoded) = decoded else {
            return BackgroundTexture::None;
        };

        let size = PixelSize::new(decoded.mips[0].0, decoded.mips[0].1);
        let amount = BackgroundBlur::quantized(style.background_blur);
        let short_side = size.width.min(size.height);

        if let Some(sigma) = image_blur::sigma_for(amount, short_side) {
            let (level, working_sigma) = image_blur::plan(sigma, decoded.mips.len() as u32);
            let radius = image_blur::radius_for(working_sigma);
            let reuse = self
                .blur_cache
                .as_ref()
                .is_some_and(|b| b.path == path && b.stamp == stamp && b.amount == amount);
            if !reuse {
                let blurred = self.run_blur(&decoded, level, working_sigma, radius);
                self.blur_cache = Some(BlurCacheEntry {
                    path: path.clone(),
                    stamp,
                    amount,
                    texture: blurred,
                });
            }
            if let Some(tex) = self.blur_cache.as_ref().and_then(|b| b.texture.as_ref()) {
                return BackgroundTexture::Blurred {
                    size,
                    view: tex.view.clone(),
                };
            }
        }

        // Sharp: upload (and cache) the mip chain.
        if self
            .blur_cache
            .as_ref()
            .is_none_or(|b| b.path != path || b.stamp != stamp || b.amount != 0.0)
        {
            self.blur_cache = Some(BlurCacheEntry {
                path: path.clone(),
                stamp,
                amount: 0.0,
                texture: Some(self.upload_image(&decoded)),
            });
        }
        let tex = self
            .blur_cache
            .as_ref()
            .and_then(|b| b.texture.as_ref())
            .expect("just uploaded");
        // Trilinear from level 0 with the footprint's LOD, so a big downscale picks a coarser mip.
        let lod = (f64::from(size.width) / f64::from(short_side.max(1)))
            .log2()
            .max(0.0);
        BackgroundTexture::Sharp {
            size,
            lod: lod as f32,
            view: tex.view.clone(),
        }
    }

    fn upload_image(&self, decoded: &DecodedImage) -> GpuTexture {
        let (w, h, _) = decoded.mips[0];
        let tex = GpuTexture::create(
            &self.device,
            "nss background",
            w,
            h,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            decoded.mips.len() as u32,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        for (level, (mw, mh, pixels)) in decoded.mips.iter().enumerate() {
            tex.upload(&self.queue, pixels, level as u32, *mw, *mh);
        }
        tex
    }

    fn run_blur(
        &self,
        decoded: &DecodedImage,
        level: u32,
        sigma: f64,
        radius: u32,
    ) -> Option<GpuTexture> {
        let (src_w, src_h, _) = decoded.mips[0];
        let w = (src_w >> level).max(1);
        let h = (src_h >> level).max(1);
        let source = self.upload_image(decoded);
        let tmp = GpuTexture::create(
            &self.device,
            "nss blur tmp",
            w,
            h,
            wgpu::TextureFormat::Rgba16Float,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let out = GpuTexture::create(
            &self.device,
            "nss blur out",
            w,
            h,
            wgpu::TextureFormat::Rgba16Float,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        // One encoder and one submit per pass: `queue.write_buffer` is ordered against submissions,
        // not against passes inside a single encoder, so a shared encoder would let the second
        // pass's parameters overwrite the first pass's.
        // Pass one reads the mip level of the decoded image, pass two reads the single-mip
        // intermediate, so only the first needs the source level.
        for (dir, src, dst, level) in [
            ([1.0_f32, 0.0], &source, &tmp, level),
            ([0.0, 1.0], &tmp, &out, 0),
        ] {
            let mut params = [0_u8; 32];
            params[0..4].copy_from_slice(&dir[0].to_le_bytes());
            params[4..8].copy_from_slice(&dir[1].to_le_bytes());
            params[8..12].copy_from_slice(&(sigma as f32).to_le_bytes());
            params[12..16].copy_from_slice(&(radius as f32).to_le_bytes());
            params[16..20].copy_from_slice(&level.to_le_bytes());
            params[20..24].copy_from_slice(&w.to_le_bytes());
            params[24..28].copy_from_slice(&h.to_le_bytes());
            self.queue.write_buffer(&self.blur_buffer, 0, &params);
            let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("nss blur bind group"),
                layout: &self.blur_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.blur_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&src.view),
                    },
                ],
            });
            let mut encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("nss blur"),
                });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("nss blur pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &dst.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.blur_pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
            drop(pass);
            self.queue.submit(Some(encoder.finish()));
        }
        Some(out)
    }

    fn resolve(&self, file: &str) -> PathBuf {
        let p = Path::new(file);
        if p.is_absolute() {
            p.to_path_buf()
        } else if let Some(base) = &self.asset_base_url {
            base.join(p)
        } else {
            p.to_path_buf()
        }
    }
}

impl FrameRenderer for Renderer {
    fn render(
        &mut self,
        screen: &Image,
        camera: Option<&Image>,
        state: &FrameState,
        style: &Style,
        output_size: PixelSize,
    ) -> Result<Frame, RenderError> {
        self.render_impl(screen, camera, state, style, output_size)
    }

    fn name(&self) -> &'static str {
        "wgpu"
    }
}

enum BackgroundTexture {
    None,
    Sharp {
        size: PixelSize,
        lod: f32,
        view: wgpu::TextureView,
    },
    Blurred {
        size: PixelSize,
        view: wgpu::TextureView,
    },
}

fn read_decoded(d: &DecodedImage) -> DecodedImage {
    DecodedImage {
        path: d.path.clone(),
        stamp: d.stamp,
        mips: d.mips.clone(),
    }
}

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

/// Decodes an image file, downscales it to at most 4096 px on the longer side, and builds a mip
/// chain in linear light.
fn decode_image(path: &Path, stamp: ImageStamp) -> Option<DecodedImage> {
    let img = image::open(path).ok()?;
    let (w, h) = (img.width().max(1), img.height().max(1));
    let scale = (MAX_IMAGE_SIDE as f64 / f64::from(w.max(h))).min(1.0);
    let rgba = if scale < 1.0 {
        let nw = ((f64::from(w) * scale).round() as u32).max(1);
        let nh = ((f64::from(h) * scale).round() as u32).max(1);
        img.resize_exact(nw, nh, image::imageops::FilterType::Lanczos3)
            .to_rgba8()
    } else {
        img.to_rgba8()
    };
    let (mut cw, mut ch) = (rgba.width().max(1), rgba.height().max(1));
    let mut mips = vec![(cw, ch, rgba.into_raw())];
    while cw > 1 || ch > 1 {
        let nw = (cw / 2).max(1);
        let nh = (ch / 2).max(1);
        let prev = mips.last().expect("non-empty");
        mips.push((nw, nh, downsample_linear(&prev.2, prev.0, prev.1, nw, nh)));
        cw = nw;
        ch = nh;
    }
    Some(DecodedImage {
        path: path.to_path_buf(),
        stamp,
        mips,
    })
}

/// Box downsample in linear light, back to sRGB bytes (Metal's `generateMipmaps` does the same for
/// sRGB textures; averaging in sRGB space would darken the result).
fn downsample_linear(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    use crate::color::{byte_to_linear, linear_to_byte};
    let mut out = vec![0_u8; (dw * dh * 4) as usize];
    for y in 0..dh {
        for x in 0..dw {
            let mut acc = [0.0_f64; 4];
            let mut n = 0.0;
            for dy in 0..2u32 {
                for dx in 0..2u32 {
                    let sx = (x * 2 + dx).min(sw - 1);
                    let sy = (y * 2 + dy).min(sh - 1);
                    let i = ((sy * sw + sx) * 4) as usize;
                    acc[0] += byte_to_linear(src[i]);
                    acc[1] += byte_to_linear(src[i + 1]);
                    acc[2] += byte_to_linear(src[i + 2]);
                    acc[3] += f64::from(src[i + 3]) / 255.0;
                    n += 1.0;
                }
            }
            let o = ((y * dw + x) * 4) as usize;
            out[o] = linear_to_byte(acc[0] / n);
            out[o + 1] = linear_to_byte(acc[1] / n);
            out[o + 2] = linear_to_byte(acc[2] / n);
            out[o + 3] = ((acc[3] / n) * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
    out
}

/// `true` when this machine can run the GPU compositor (the shell falls back to
/// [`crate::passthrough::PassthroughRenderer`] otherwise, exactly like upstream on a Mac without
/// Metal).
#[must_use]
pub fn is_supported() -> bool {
    Renderer::new().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nss_core::RGBAColor;

    #[test]
    fn stamp_of_a_missing_file_is_zero() {
        let s = ImageStamp::of(Path::new("C:/definitely/not/here.png"));
        assert_eq!(
            s,
            ImageStamp {
                secs: 0,
                nanos: 0,
                size: 0
            }
        );
        assert!(!s.exists());
    }

    #[test]
    fn downsampling_stays_in_linear_light() {
        // Black and white halves: the average of the two in linear light is 50% linear, which is
        // sRGB 188, not 128.
        let src = vec![
            255, 255, 255, 255, 0, 0, 0, 255, // row 0: white, black
            255, 255, 255, 255, 0, 0, 0, 255, // row 1
        ];
        let out = downsample_linear(&src, 2, 2, 1, 1);
        assert_eq!(out[3], 255);
        assert!(
            (186..=189).contains(&out[0]),
            "expected about 188, got {}",
            out[0]
        );
    }

    #[test]
    fn a_renderer_reports_its_name_and_adapter() {
        if let Ok(r) = Renderer::new() {
            assert_eq!(r.name(), "wgpu");
            assert!(!r.adapter_name().is_empty());
            assert!(r.atlas().cell(nss_core::CursorShape::Arrow).width > 0);
        }
    }

    #[test]
    fn a_flat_scene_renders_the_background_where_the_screen_is_not() {
        let Ok(mut r) = Renderer::new() else {
            return; // no adapter on this machine; the GPU tests cover it when there is one
        };
        let style = Style {
            background: Background::Solid(RGBAColor::new(1.0, 0.0, 0.0, 1.0)),
            padding: 0.1,
            corner_radius: 0.0,
            camera: nss_core::CameraBubble {
                visible: false,
                ..Default::default()
            },
            shadow: nss_core::Shadow {
                opacity: 0.0,
                ..Default::default()
            },
            ..Style::default()
        };
        let screen = Image::filled(1920, 1080, RGBAColor::new(0.0, 1.0, 0.0, 1.0));
        let mut state = FrameState::new(0.0);
        // The default frame state has the cursor visible in the middle; this test is about the
        // background and the screen only.
        state.cursor.visible = false;
        let frame = r
            .render(&screen, None, &state, &style, PixelSize::new(640, 360))
            .unwrap();
        assert_eq!((frame.width, frame.height), (640, 360));
        let corner = frame.pixel(1, 1);
        assert!(corner[0] > 250 && corner[1] < 5, "corner {corner:?}");
        let centre = frame.pixel(320, 180);
        assert!(centre[1] > 250 && centre[0] < 5, "centre {centre:?}");
    }
}
