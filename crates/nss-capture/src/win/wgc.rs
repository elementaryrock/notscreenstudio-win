//! `Windows.Graphics.Capture` (WGC) screen/window/area capture engine.
//!
//! Confined `unsafe` boundary (ADR 0012):
//! - Creates a D3D11 device with `BGRA_SUPPORT` (hardware first, WARP fallback) and wraps it for
//!   WinRT via `CreateDirect3D11DeviceFromDXGIDevice`.
//! - Creates a `GraphicsCaptureItem` via `IGraphicsCaptureItemInterop` (`CreateForMonitor` for
//!   display/area capture, `CreateForWindow` for window capture).
//! - Sets `SetIsCursorCaptureEnabled(false)` (ADR 0008) and best-effort `SetIsBorderRequired(false)`
//!   (ADR 0007).
//! - Copies each captured GPU texture to a reusable CPU-readable D3D11 staging texture, crops to
//!   the requested physical-pixel sub-rect when recording an area, and delivers tightly-packed
//!   BGRA8 bytes stamped with `capture_math::host_now()`.

#![allow(unsafe_code)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use nss_core::{NssError, PixelSize};
use parking_lot::Mutex;
use windows::core::{IInspectable, Interface};
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::{HMODULE, HWND, RPC_E_CHANGED_MODE};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_CPU_ACCESS_READ,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_SDK_VERSION,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED};

use crate::capture_errors::{map_code, CaptureContext};
use crate::capture_math::host_now;

/// Which desktop object WGC should attach to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WgcTarget {
    /// Whole display or area (monitor handle + optional physical-pixel crop rectangle).
    Monitor {
        hmonitor: isize,
        crop: Option<CropPixels>,
    },
    /// Single top-level window handle.
    Window { hwnd: u64 },
}

/// Physical-pixel crop rectangle inside a monitor capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CropPixels {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// One captured screen frame delivered from WGC.
#[derive(Debug, Clone)]
pub struct CapturedScreenFrame {
    pub host_time: f64,
    pub width: u32,
    pub height: u32,
    /// Tightly-packed top-down BGRA8 (`width * height * 4` bytes).
    pub bgra: Vec<u8>,
}

/// Live WGC capture handle. Dropping or calling `stop()` shuts down the session and joins the
/// worker thread.
pub struct WgcCaptureHandle {
    stop_flag: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
    pub initial_size: PixelSize,
    pub borderless_applied: bool,
}

struct RoGuard {
    should_uninit: bool,
}

impl RoGuard {
    fn init_mta() -> Self {
        // SAFETY: standard WinRT MTA init; skip uninit if thread was already initialized differently.
        let hr = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
        let should_uninit = match hr {
            Ok(()) => true,
            Err(e) if e.code() == RPC_E_CHANGED_MODE => false,
            Err(_) => false,
        };
        Self { should_uninit }
    }
}

impl Drop for RoGuard {
    fn drop(&mut self) {
        if self.should_uninit {
            // SAFETY: balanced with our successful `RoInitialize`.
            unsafe { RoUninitialize() };
        }
    }
}

struct StagingState {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    winrt_device: IDirect3DDevice,
    staging: Option<(ID3D11Texture2D, u32, u32)>,
    pool_size: SizeInt32,
}

// SAFETY: `ID3D11Device`, `ID3D11DeviceContext` (serialized exclusively behind `Mutex<StagingState>`)
// and WinRT `IDirect3DDevice` (created from an MTA DXGI device) are agile/free-threaded COM
// interfaces safe to move across MTA threads when access is mutually excluded.
unsafe impl Send for StagingState {}

impl StagingState {
    fn ensure_staging(
        &mut self,
        desc: &D3D11_TEXTURE2D_DESC,
    ) -> Result<&ID3D11Texture2D, NssError> {
        let needs_new = match &self.staging {
            Some((_, w, h)) => *w != desc.Width || *h != desc.Height,
            None => true,
        };
        if needs_new {
            let mut staging_desc = *desc;
            staging_desc.Usage = D3D11_USAGE_STAGING;
            staging_desc.BindFlags = 0;
            staging_desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
            staging_desc.MiscFlags = 0;
            staging_desc.MipLevels = 1;
            staging_desc.ArraySize = 1;
            let mut tex = None;
            // SAFETY: valid D3D11 device + initialized descriptor.
            unsafe {
                self.device
                    .CreateTexture2D(&staging_desc, None, Some(&mut tex))
                    .map_err(|e| NssError::Io(format!("Failed to create staging texture: {e}")))?;
            }
            let tex = tex.ok_or_else(|| NssError::Io("Staging texture was null.".into()))?;
            self.staging = Some((tex, desc.Width, desc.Height));
        }
        Ok(&self.staging.as_ref().expect("just populated").0)
    }
}

impl WgcCaptureHandle {
    /// Starts a live `Windows.Graphics.Capture` session on a dedicated MTA thread.
    pub fn start<F, C>(target: WgcTarget, on_frame: F, on_closed: C) -> Result<Self, NssError>
    where
        F: FnMut(CapturedScreenFrame) + Send + 'static,
        C: FnMut(NssError) + Send + 'static,
    {
        let stop_flag = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop_flag);
        let (tx, rx) = mpsc::sync_channel::<Result<(PixelSize, bool), NssError>>(1);

        let join = thread::Builder::new()
            .name("nss-wgc-capture".into())
            .spawn(move || {
                run_wgc_worker(target, worker_stop, tx, on_frame, on_closed);
            })
            .map_err(|e| NssError::Io(format!("Failed to spawn WGC thread: {e}")))?;

        match rx.recv() {
            Ok(Ok((initial_size, borderless_applied))) => Ok(Self {
                stop_flag,
                join: Some(join),
                initial_size,
                borderless_applied,
            }),
            Ok(Err(err)) => {
                stop_flag.store(true, Ordering::SeqCst);
                let _ = join.join();
                Err(err)
            }
            Err(_) => {
                stop_flag.store(true, Ordering::SeqCst);
                let _ = join.join();
                Err(NssError::Io(
                    "WGC capture worker exited before starting.".into(),
                ))
            }
        }
    }

    pub fn stop(&mut self) {
        self.stop_flag.store(true, Ordering::SeqCst);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

impl Drop for WgcCaptureHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

fn run_wgc_worker<F, C>(
    target: WgcTarget,
    stop: Arc<AtomicBool>,
    ready_tx: mpsc::SyncSender<Result<(PixelSize, bool), NssError>>,
    on_frame: F,
    on_closed: C,
) where
    F: FnMut(CapturedScreenFrame) + Send + 'static,
    C: FnMut(NssError) + Send + 'static,
{
    let _ro = RoGuard::init_mta();
    let ctx_kind = match target {
        WgcTarget::Window { .. } => CaptureContext::Window,
        WgcTarget::Monitor { .. } => CaptureContext::Screen,
    };

    let (device, context, winrt_device) = match create_d3d11_and_winrt_device() {
        Ok(v) => v,
        Err(e) => {
            let _ = ready_tx.send(Err(e));
            return;
        }
    };

    let item = match create_capture_item(target, ctx_kind) {
        Ok(i) => i,
        Err(e) => {
            let _ = ready_tx.send(Err(e));
            return;
        }
    };

    let item_size = match item.Size() {
        Ok(sz) if sz.Width > 0 && sz.Height > 0 => sz,
        Ok(_) => {
            let _ = ready_tx.send(Err(NssError::Unsupported(
                "Capture target has zero dimensions.".into(),
            )));
            return;
        }
        Err(e) => {
            let _ = ready_tx.send(Err(map_code(e.code().0 as u32, &e.to_string(), ctx_kind)));
            return;
        }
    };

    let frame_pool = match Direct3D11CaptureFramePool::CreateFreeThreaded(
        &winrt_device,
        DirectXPixelFormat::B8G8R8A8UIntNormalized,
        2,
        item_size,
    ) {
        Ok(p) => p,
        Err(e) => {
            let _ = ready_tx.send(Err(map_code(e.code().0 as u32, &e.to_string(), ctx_kind)));
            return;
        }
    };

    let session = match frame_pool.CreateCaptureSession(&item) {
        Ok(s) => s,
        Err(e) => {
            let _ = frame_pool.Close();
            let _ = ready_tx.send(Err(map_code(e.code().0 as u32, &e.to_string(), ctx_kind)));
            return;
        }
    };

    // ADR 0008: exclude the system cursor so we draw our own smooth cursor from the 120 Hz log.
    let _ = session.SetIsCursorCaptureEnabled(false);
    // ADR 0007: best-effort borderless; silently degrades to the yellow border when refused.
    let borderless_applied = session.SetIsBorderRequired(false).is_ok();

    let crop = match target {
        WgcTarget::Monitor { crop, .. } => crop,
        WgcTarget::Window { .. } => None,
    };

    let initial_size = match crop {
        Some(c) => PixelSize::new(c.width.max(2), c.height.max(2)),
        None => PixelSize::new(item_size.Width as u32, item_size.Height as u32),
    };

    let staging = Arc::new(Mutex::new(StagingState {
        device,
        context,
        winrt_device,
        staging: None,
        pool_size: item_size,
    }));
    let on_frame = Arc::new(Mutex::new(on_frame));
    let on_closed = Arc::new(Mutex::new(on_closed));
    let closed_flag = Arc::new(AtomicBool::new(false));

    let closed_flag_cb = Arc::clone(&closed_flag);
    let on_closed_cb = Arc::clone(&on_closed);
    let closed_token = item.Closed(
        &TypedEventHandler::<GraphicsCaptureItem, IInspectable>::new(move |_, _| {
            closed_flag_cb.store(true, Ordering::SeqCst);
            let msg = match ctx_kind {
                CaptureContext::Window => NssError::Io("The captured window was closed.".into()),
                _ => NssError::Io("The capture source is no longer available.".into()),
            };
            (on_closed_cb.lock())(msg);
            Ok(())
        }),
    );

    let stop_cb = Arc::clone(&stop);
    let staging_cb = Arc::clone(&staging);
    let on_frame_cb = Arc::clone(&on_frame);

    let arrived_token = frame_pool.FrameArrived(&TypedEventHandler::<
        Direct3D11CaptureFramePool,
        IInspectable,
    >::new(move |pool_opt, _| {
        if stop_cb.load(Ordering::Relaxed) {
            return Ok(());
        }
        let Some(pool) = pool_opt.as_ref() else {
            return Ok(());
        };
        let Ok(frame) = pool.TryGetNextFrame() else {
            return Ok(());
        };
        let t = host_now();
        let content_size = frame.ContentSize().unwrap_or(SizeInt32 {
            Width: 0,
            Height: 0,
        });

        let extracted = extract_bgra_frame(&frame, &staging_cb, crop, t);
        let _ = frame.Close();

        if let Ok(Some(captured)) = extracted {
            (on_frame_cb.lock())(captured);
        }

        // Resize the frame pool if a captured window resized dynamically.
        if content_size.Width > 0 && content_size.Height > 0 {
            let mut st = staging_cb.lock();
            let resized = content_size.Width != st.pool_size.Width
                || content_size.Height != st.pool_size.Height;
            if resized
                && pool
                    .Recreate(
                        &st.winrt_device,
                        DirectXPixelFormat::B8G8R8A8UIntNormalized,
                        2,
                        content_size,
                    )
                    .is_ok()
            {
                st.pool_size = content_size;
            }
        }

        Ok(())
    }));

    if let Err(e) = session.StartCapture() {
        if let Ok(tok) = arrived_token {
            let _ = frame_pool.RemoveFrameArrived(tok);
        }
        if let Ok(tok) = closed_token {
            let _ = item.RemoveClosed(tok);
        }
        let _ = session.Close();
        let _ = frame_pool.Close();
        let _ = ready_tx.send(Err(map_code(e.code().0 as u32, &e.to_string(), ctx_kind)));
        return;
    }

    let _ = ready_tx.send(Ok((initial_size, borderless_applied)));

    // Keep the session, frame pool and item alive until stop() or target close.
    while !stop.load(Ordering::Relaxed) && !closed_flag.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_millis(10));
    }

    if let Ok(tok) = arrived_token {
        let _ = frame_pool.RemoveFrameArrived(tok);
    }
    if let Ok(tok) = closed_token {
        let _ = item.RemoveClosed(tok);
    }
    let _ = session.Close();
    let _ = frame_pool.Close();
}

fn create_d3d11_and_winrt_device(
) -> Result<(ID3D11Device, ID3D11DeviceContext, IDirect3DDevice), NssError> {
    let mut device = None;
    let mut context = None;
    // SAFETY: valid out-pointers; try hardware first and fall back to WARP software adapter.
    let res = unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
        .or_else(|_| {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_WARP,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
        })
    };
    res.map_err(|e| {
        NssError::Unsupported(format!("Failed to initialize Direct3D 11 for capture: {e}"))
    })?;

    let device = device.ok_or_else(|| NssError::Unsupported("D3D11 device was null.".into()))?;
    let context = context.ok_or_else(|| NssError::Unsupported("D3D11 context was null.".into()))?;
    let dxgi_device: IDXGIDevice = device
        .cast()
        .map_err(|e| NssError::Unsupported(format!("D3D11 device missing IDXGIDevice: {e}")))?;

    // SAFETY: `dxgi_device` is a live COM interface on this process's D3D11 device.
    let inspectable =
        unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi_device) }.map_err(|e| {
            NssError::Unsupported(format!("Failed to wrap D3D11 device for WinRT: {e}"))
        })?;
    let winrt_device: IDirect3DDevice = inspectable
        .cast()
        .map_err(|e| NssError::Unsupported(format!("WinRT device cast failed: {e}")))?;

    Ok((device, context, winrt_device))
}

fn create_capture_item(
    target: WgcTarget,
    ctx_kind: CaptureContext,
) -> Result<GraphicsCaptureItem, NssError> {
    let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
        .map_err(|e| {
            NssError::Unsupported(format!(
                "Windows Graphics Capture is not available on this OS version: {e}"
            ))
        })?;
    // SAFETY: `CreateForMonitor`/`CreateForWindow` validate the handle and return an HRESULT on error.
    let res = unsafe {
        match target {
            WgcTarget::Monitor { hmonitor, .. } => {
                interop.CreateForMonitor::<GraphicsCaptureItem>(HMONITOR(hmonitor as *mut _))
            }
            WgcTarget::Window { hwnd } => {
                interop.CreateForWindow::<GraphicsCaptureItem>(HWND(hwnd as usize as *mut _))
            }
        }
    };
    res.map_err(|e| map_code(e.code().0 as u32, &e.to_string(), ctx_kind))
}

fn extract_bgra_frame(
    frame: &windows::Graphics::Capture::Direct3D11CaptureFrame,
    staging_lock: &Arc<Mutex<StagingState>>,
    crop: Option<CropPixels>,
    host_time: f64,
) -> Result<Option<CapturedScreenFrame>, NssError> {
    let surface = frame
        .Surface()
        .map_err(|e| NssError::Io(format!("Failed to read WGC surface: {e}")))?;
    let access: IDirect3DDxgiInterfaceAccess = surface
        .cast()
        .map_err(|e| NssError::Io(format!("Surface missing DXGI interop: {e}")))?;
    // SAFETY: `IDirect3DDxgiInterfaceAccess::GetInterface` returns an AddRef'd `ID3D11Texture2D`.
    let src_tex: ID3D11Texture2D = unsafe { access.GetInterface() }
        .map_err(|e| NssError::Io(format!("Failed to obtain ID3D11Texture2D: {e}")))?;

    let mut desc = D3D11_TEXTURE2D_DESC::default();
    // SAFETY: valid texture pointer and desc out-pointer.
    unsafe { src_tex.GetDesc(&mut desc) };
    if desc.Width == 0 || desc.Height == 0 {
        return Ok(None);
    }

    let mut st = staging_lock.lock();
    let staging_tex = st.ensure_staging(&desc)?.clone();
    let context = st.context.clone();

    let (crop_x, crop_y, out_w, out_h) = match crop {
        Some(c) => {
            let x = c.x.min(desc.Width.saturating_sub(1));
            let y = c.y.min(desc.Height.saturating_sub(1));
            let w = c.width.min(desc.Width - x).max(2);
            let h = c.height.min(desc.Height - y).max(2);
            (x, y, w, h)
        }
        None => (0, 0, desc.Width, desc.Height),
    };

    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    // SAFETY: staging texture matches `src_tex` dimensions and format; unmapped before return.
    unsafe {
        context.CopyResource(&staging_tex, &src_tex);
        context
            .Map(&staging_tex, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
            .map_err(|e| NssError::Io(format!("Failed to map staging texture: {e}")))?;
    }

    let row_pitch = mapped.RowPitch as usize;
    let out_row_bytes = (out_w as usize) * 4;
    let mut bgra = vec![0u8; out_row_bytes * (out_h as usize)];

    if !mapped.pData.is_null() {
        let base = mapped.pData.cast::<u8>();
        // SAFETY: `crop_y + out_h <= desc.Height` and `(crop_x + out_w) * 4 <= row_pitch`.
        unsafe {
            for row in 0..(out_h as usize) {
                let src_row =
                    base.add(((crop_y as usize) + row) * row_pitch + (crop_x as usize) * 4);
                let dst_row = bgra.as_mut_ptr().add(row * out_row_bytes);
                std::ptr::copy_nonoverlapping(src_row, dst_row, out_row_bytes);
            }
        }
    }

    // SAFETY: paired with `Map` above.
    unsafe {
        context.Unmap(&staging_tex, 0);
    }

    Ok(Some(CapturedScreenFrame {
        host_time,
        width: out_w,
        height: out_h,
        bgra,
    }))
}
