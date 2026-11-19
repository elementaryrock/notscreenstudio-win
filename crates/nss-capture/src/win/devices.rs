//! WASAPI microphone enumeration/capture and Media Foundation camera enumeration/capture.
//!
//! Confined `unsafe` boundary (ADR 0012). All COM initialization happens on the dedicated worker
//! threads owned by each capture handle, never on the caller's thread.

#![allow(unsafe_code)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use nss_core::NssError;
use windows::core::{GUID, PWSTR};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::Media::Audio::{
    eCapture, eConsole, eRender, IAudioCaptureClient, IAudioClient, IMMDevice, IMMDeviceEnumerator,
    MMDeviceEnumerator, AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED,
    AUDCLNT_STREAMFLAGS_LOOPBACK, DEVICE_STATE_ACTIVE, WAVEFORMATEX, WAVEFORMATEXTENSIBLE,
};
use windows::Win32::Media::KernelStreaming::{KSDATAFORMAT_SUBTYPE_PCM, WAVE_FORMAT_EXTENSIBLE};
use windows::Win32::Media::MediaFoundation::{
    IMFActivate, IMFMediaSource, IMFMediaType, IMFSourceReader, MFCreateAttributes,
    MFCreateMediaType, MFCreateSourceReaderFromMediaSource, MFEnumDeviceSources, MFMediaType_Video,
    MFShutdown, MFStartup, MFVideoFormat_RGB32, MFSTARTUP_LITE,
    MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
    MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
    MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK, MF_MT_DEFAULT_STRIDE,
    MF_MT_FRAME_SIZE, MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE, MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING,
    MF_SOURCE_READER_FIRST_VIDEO_STREAM, MF_VERSION,
};
use windows::Win32::Media::Multimedia::{KSDATAFORMAT_SUBTYPE_IEEE_FLOAT, WAVE_FORMAT_IEEE_FLOAT};
use windows::Win32::System::Com::StructuredStorage::PropVariantClear;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
    COINIT_MULTITHREADED, STGM_READ,
};

use crate::capture_errors::{map_code, CaptureContext};
use crate::capture_math::host_now;
use crate::device::CaptureDeviceInfo;

const WAVE_FORMAT_PCM_TAG: u16 = 1;

struct ComGuard {
    should_uninit: bool,
}

impl ComGuard {
    fn init_mta() -> Self {
        // SAFETY: standard COM MTA thread init; `RPC_E_CHANGED_MODE` means COM is already
        // initialized on this thread so we must not uninitialize it.
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        let should_uninit = hr.is_ok() && hr != RPC_E_CHANGED_MODE;
        Self { should_uninit }
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.should_uninit {
            // SAFETY: balanced with our successful `CoInitializeEx`.
            unsafe { CoUninitialize() };
        }
    }
}

/// Enumerates active WASAPI capture endpoints (microphones).
#[must_use]
pub fn enumerate_microphones() -> Vec<CaptureDeviceInfo> {
    let _com = ComGuard::init_mta();
    let mut out = Vec::new();
    // SAFETY: COM interfaces are released automatically via RAII wrappers; `PWSTR` IDs are freed
    // with `CoTaskMemFree`.
    unsafe {
        let Ok(enumerator) =
            CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL)
        else {
            return out;
        };

        let default_id = enumerator
            .GetDefaultAudioEndpoint(eCapture, eConsole)
            .ok()
            .and_then(|d| device_id_string(&d));

        let Ok(collection) = enumerator.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE) else {
            return out;
        };
        let count = collection.GetCount().unwrap_or(0);
        for i in 0..count {
            let Ok(device) = collection.Item(i) else {
                continue;
            };
            let Some(id) = device_id_string(&device) else {
                continue;
            };
            let name = device_friendly_name(&device).unwrap_or_else(|| id.clone());
            let is_default = default_id.as_deref() == Some(id.as_str());
            out.push(CaptureDeviceInfo::new(id, name, is_default));
        }
    }
    out
}

/// Enumerates Media Foundation video capture devices (webcams).
#[must_use]
pub fn enumerate_cameras() -> Vec<CaptureDeviceInfo> {
    let _com = ComGuard::init_mta();
    let mut out = Vec::new();
    // SAFETY: `MFStartup` is paired with `MFShutdown`; all allocated pointers are freed.
    unsafe {
        if MFStartup(MF_VERSION, MFSTARTUP_LITE).is_err() {
            return out;
        }
        let mut attrs = None;
        if MFCreateAttributes(&mut attrs, 1).is_ok() {
            if let Some(attrs) = attrs {
                let _ = attrs.SetGUID(
                    &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
                    &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
                );
                let mut raw_ptr: *mut Option<IMFActivate> = std::ptr::null_mut();
                let mut count: u32 = 0;
                if MFEnumDeviceSources(&attrs, &mut raw_ptr, &mut count).is_ok()
                    && !raw_ptr.is_null()
                {
                    let slice = std::slice::from_raw_parts(raw_ptr, count as usize);
                    for (idx, maybe_dev) in slice.iter().enumerate() {
                        let Some(dev) = maybe_dev else {
                            continue;
                        };
                        let name = mf_attr_string(dev, &MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME)
                            .unwrap_or_else(|| format!("Camera {}", idx + 1));
                        let id = mf_attr_string(
                            dev,
                            &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK,
                        )
                        .unwrap_or_else(|| name.clone());
                        out.push(CaptureDeviceInfo::new(id, name, idx == 0));
                    }
                    // Release COM pointers in the array before freeing the array buffer itself.
                    for item in std::slice::from_raw_parts_mut(raw_ptr, count as usize) {
                        drop(item.take());
                    }
                    CoTaskMemFree(Some(raw_ptr.cast()));
                }
            }
        }
        let _ = MFShutdown();
    }
    out
}

/// Metadata reported once WASAPI initializes the selected microphone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioStreamFormat {
    pub sample_rate: u32,
    pub channels: u16,
}

/// One captured packet of interleaved `f32` PCM audio.
#[derive(Debug, Clone)]
pub struct CapturedAudioPacket {
    pub host_time: f64,
    pub samples: Vec<f32>,
    pub frames: usize,
}

/// Live WASAPI microphone capture handle. Stops cleanly on `stop()` or `Drop`.
pub struct WasapiCaptureHandle {
    stop_flag: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
    pub format: AudioStreamFormat,
}

impl WasapiCaptureHandle {
    /// Starts capturing audio from the WASAPI endpoint `device_id` (or the default endpoint if
    /// `device_id` is empty or `"default"`).
    pub fn start<F, E>(device_id: String, on_packet: F, on_error: E) -> Result<Self, NssError>
    where
        F: FnMut(CapturedAudioPacket) + Send + 'static,
        E: FnOnce(NssError) + Send + 'static,
    {
        Self::start_inner(device_id, false, "nss-wasapi-mic", on_packet, on_error)
    }

    /// Starts capturing desktop/system audio via WASAPI loopback on the default render endpoint.
    pub fn start_loopback<F, E>(on_packet: F, on_error: E) -> Result<Self, NssError>
    where
        F: FnMut(CapturedAudioPacket) + Send + 'static,
        E: FnOnce(NssError) + Send + 'static,
    {
        Self::start_inner(
            String::new(),
            true,
            "nss-wasapi-loopback",
            on_packet,
            on_error,
        )
    }

    fn start_inner<F, E>(
        device_id: String,
        loopback: bool,
        thread_name: &str,
        on_packet: F,
        on_error: E,
    ) -> Result<Self, NssError>
    where
        F: FnMut(CapturedAudioPacket) + Send + 'static,
        E: FnOnce(NssError) + Send + 'static,
    {
        let stop_flag = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop_flag);
        let (tx, rx) = std::sync::mpsc::sync_channel::<Result<AudioStreamFormat, NssError>>(1);

        let join = thread::Builder::new()
            .name(thread_name.into())
            .spawn(move || {
                run_wasapi_loop(device_id, loopback, worker_stop, tx, on_packet, on_error);
            })
            .map_err(|e| NssError::Io(format!("Failed to start audio capture thread: {e}")))?;

        match rx.recv() {
            Ok(Ok(format)) => Ok(Self {
                stop_flag,
                join: Some(join),
                format,
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
                    "Audio capture thread exited unexpectedly.".into(),
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

impl Drop for WasapiCaptureHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// One captured camera frame in tightly-packed BGRA8 (top-down).
#[derive(Debug, Clone)]
pub struct CapturedCameraFrame {
    pub host_time: f64,
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

/// Live Media Foundation webcam capture handle.
pub struct CameraCaptureHandle {
    stop_flag: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
    pub width: u32,
    pub height: u32,
}

impl CameraCaptureHandle {
    pub fn start<F, E>(device_id: String, on_frame: F, on_error: E) -> Result<Self, NssError>
    where
        F: FnMut(CapturedCameraFrame) + Send + 'static,
        E: FnOnce(NssError) + Send + 'static,
    {
        let stop_flag = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop_flag);
        let (tx, rx) = std::sync::mpsc::sync_channel::<Result<(u32, u32), NssError>>(1);

        let join = thread::Builder::new()
            .name("nss-mf-camera".into())
            .spawn(move || {
                run_mf_camera_loop(device_id, worker_stop, tx, on_frame, on_error);
            })
            .map_err(|e| NssError::Io(format!("Failed to start camera capture thread: {e}")))?;

        match rx.recv() {
            Ok(Ok((width, height))) => Ok(Self {
                stop_flag,
                join: Some(join),
                width,
                height,
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
                    "Camera capture thread exited unexpectedly.".into(),
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

impl Drop for CameraCaptureHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

#[derive(Clone, Copy)]
enum SampleKind {
    Float32,
    Pcm16,
    Pcm24,
    Pcm32,
}

fn run_wasapi_loop<F, E>(
    device_id: String,
    loopback: bool,
    stop: Arc<AtomicBool>,
    ready_tx: std::sync::mpsc::SyncSender<Result<AudioStreamFormat, NssError>>,
    mut on_packet: F,
    on_error: E,
) where
    F: FnMut(CapturedAudioPacket),
    E: FnOnce(NssError),
{
    let _com = ComGuard::init_mta();
    let setup = unsafe { setup_wasapi(&device_id, loopback) };
    let (client, capture, raw_format, sample_kind) = match setup {
        Ok(v) => v,
        Err(e) => {
            let _ = ready_tx.send(Err(e));
            return;
        }
    };
    let in_channels = usize::from(raw_format.channels.max(1));
    let out_channels = raw_format.channels.clamp(1, 2);
    let reported_format = AudioStreamFormat {
        sample_rate: raw_format.sample_rate,
        channels: out_channels,
    };
    let _ = ready_tx.send(Ok(reported_format));

    let mut failure: Option<NssError> = None;

    // SAFETY: `capture` is valid on this thread; every `GetBuffer` is paired with `ReleaseBuffer`.
    unsafe {
        while !stop.load(Ordering::Relaxed) {
            let mut packet_len = match capture.GetNextPacketSize() {
                Ok(n) => n,
                Err(e) => {
                    failure = Some(map_code(
                        e.code().0 as u32,
                        &e.to_string(),
                        CaptureContext::Microphone,
                    ));
                    break;
                }
            };
            if packet_len == 0 {
                thread::sleep(Duration::from_millis(5));
                continue;
            }
            while packet_len > 0 && !stop.load(Ordering::Relaxed) {
                let mut data_ptr: *mut u8 = std::ptr::null_mut();
                let mut num_frames: u32 = 0;
                let mut flags: u32 = 0;
                if let Err(e) =
                    capture.GetBuffer(&mut data_ptr, &mut num_frames, &mut flags, None, None)
                {
                    failure = Some(map_code(
                        e.code().0 as u32,
                        &e.to_string(),
                        CaptureContext::Microphone,
                    ));
                    break;
                }
                let t = host_now();
                let frames = num_frames as usize;
                let out_ch = usize::from(out_channels);
                let samples =
                    if (flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0 || data_ptr.is_null() {
                        vec![0.0f32; frames * out_ch]
                    } else {
                        let raw = decode_pcm_to_f32(data_ptr, frames * in_channels, sample_kind);
                        if in_channels == out_ch {
                            raw
                        } else {
                            // Downmix multi-channel (e.g. 5.1/7.1) capture to front L/R stereo so
                            // downstream FFmpeg `-ac 2` stays in lock-step.
                            let mut stereo = Vec::with_capacity(frames * out_ch);
                            for frame in raw.chunks_exact(in_channels) {
                                stereo.push(frame[0]);
                                if out_ch == 2 {
                                    stereo.push(frame[1]);
                                }
                            }
                            stereo
                        }
                    };
                let _ = capture.ReleaseBuffer(num_frames);

                if frames > 0 {
                    on_packet(CapturedAudioPacket {
                        host_time: t,
                        samples,
                        frames,
                    });
                }

                packet_len = capture.GetNextPacketSize().unwrap_or(0);
            }
            if failure.is_some() {
                break;
            }
        }
        let _ = client.Stop();
    }

    if let Some(err) = failure {
        on_error(err);
    }
}

unsafe fn setup_wasapi(
    device_id: &str,
    loopback: bool,
) -> Result<
    (
        IAudioClient,
        IAudioCaptureClient,
        AudioStreamFormat,
        SampleKind,
    ),
    NssError,
> {
    let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
        .map_err(|e| {
        map_code(
            e.code().0 as u32,
            &e.to_string(),
            CaptureContext::Microphone,
        )
    })?;

    let device: IMMDevice = if loopback {
        enumerator
            .GetDefaultAudioEndpoint(eRender, eConsole)
            .map_err(|e| {
                map_code(
                    e.code().0 as u32,
                    &e.to_string(),
                    CaptureContext::Microphone,
                )
            })?
    } else if device_id.is_empty() || device_id.eq_ignore_ascii_case("default") {
        enumerator
            .GetDefaultAudioEndpoint(eCapture, eConsole)
            .map_err(|e| {
                map_code(
                    e.code().0 as u32,
                    &e.to_string(),
                    CaptureContext::Microphone,
                )
            })?
    } else {
        let wide: Vec<u16> = device_id.encode_utf16().chain(std::iter::once(0)).collect();
        enumerator
            .GetDevice(windows::core::PCWSTR(wide.as_ptr()))
            .map_err(|e| {
                map_code(
                    e.code().0 as u32,
                    &e.to_string(),
                    CaptureContext::Microphone,
                )
            })?
    };

    let client: IAudioClient = device.Activate(CLSCTX_ALL, None).map_err(|e| {
        map_code(
            e.code().0 as u32,
            &e.to_string(),
            CaptureContext::Microphone,
        )
    })?;

    let mix_ptr = client.GetMixFormat().map_err(|e| {
        map_code(
            e.code().0 as u32,
            &e.to_string(),
            CaptureContext::Microphone,
        )
    })?;

    let (format, sample_kind) = parse_wave_format(mix_ptr)?;
    // 100 ms buffer in 100-ns units.
    let buffer_duration_hns = 1_000_000i64;
    let stream_flags = if loopback {
        AUDCLNT_STREAMFLAGS_LOOPBACK
    } else {
        0
    };
    let init_res = client.Initialize(
        AUDCLNT_SHAREMODE_SHARED,
        stream_flags,
        buffer_duration_hns,
        0,
        mix_ptr,
        None,
    );
    CoTaskMemFree(Some(mix_ptr.cast()));
    init_res.map_err(|e| {
        map_code(
            e.code().0 as u32,
            &e.to_string(),
            CaptureContext::Microphone,
        )
    })?;

    let capture: IAudioCaptureClient = client.GetService().map_err(|e| {
        map_code(
            e.code().0 as u32,
            &e.to_string(),
            CaptureContext::Microphone,
        )
    })?;
    client.Start().map_err(|e| {
        map_code(
            e.code().0 as u32,
            &e.to_string(),
            CaptureContext::Microphone,
        )
    })?;

    Ok((client, capture, format, sample_kind))
}

unsafe fn parse_wave_format(
    ptr: *const WAVEFORMATEX,
) -> Result<(AudioStreamFormat, SampleKind), NssError> {
    if ptr.is_null() {
        return Err(NssError::Unsupported(
            "Microphone reported no audio format.".into(),
        ));
    }
    let wf = std::ptr::read_unaligned(ptr);
    let sample_rate = wf.nSamplesPerSec;
    let channels = wf.nChannels;
    let bits = wf.wBitsPerSample;
    let tag = wf.wFormatTag;

    let is_float = if tag == WAVE_FORMAT_IEEE_FLOAT as u16 {
        true
    } else if tag == WAVE_FORMAT_EXTENSIBLE as u16 {
        let ext = std::ptr::read_unaligned(ptr.cast::<WAVEFORMATEXTENSIBLE>());
        let sub = ext.SubFormat;
        if sub == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT {
            true
        } else if sub == KSDATAFORMAT_SUBTYPE_PCM {
            false
        } else {
            return Err(NssError::Unsupported(format!(
                "Unsupported microphone sample format ({bits}-bit extensible)."
            )));
        }
    } else if tag == WAVE_FORMAT_PCM_TAG {
        false
    } else {
        return Err(NssError::Unsupported(format!(
            "Unsupported microphone format tag {tag}."
        )));
    };

    let kind = match (is_float, bits) {
        (true, 32) => SampleKind::Float32,
        (false, 16) => SampleKind::Pcm16,
        (false, 24) => SampleKind::Pcm24,
        (false, 32) => SampleKind::Pcm32,
        _ => {
            return Err(NssError::Unsupported(format!(
                "Unsupported microphone bit depth ({bits}-bit)."
            )));
        }
    };

    Ok((
        AudioStreamFormat {
            sample_rate,
            channels,
        },
        kind,
    ))
}

unsafe fn decode_pcm_to_f32(ptr: *const u8, count: usize, kind: SampleKind) -> Vec<f32> {
    let mut out = Vec::with_capacity(count);
    match kind {
        SampleKind::Float32 => {
            let bytes = std::slice::from_raw_parts(ptr, count * 4);
            for chunk in bytes.chunks_exact(4) {
                out.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
            }
        }
        SampleKind::Pcm16 => {
            let bytes = std::slice::from_raw_parts(ptr, count * 2);
            for chunk in bytes.chunks_exact(2) {
                let s = i16::from_le_bytes([chunk[0], chunk[1]]);
                out.push(f32::from(s) / 32768.0);
            }
        }
        SampleKind::Pcm24 => {
            let bytes = std::slice::from_raw_parts(ptr, count * 3);
            for chunk in bytes.chunks_exact(3) {
                let raw = (i32::from(chunk[0])
                    | (i32::from(chunk[1]) << 8)
                    | (i32::from(chunk[2]) << 16))
                    << 8;
                out.push((raw as f32) / (i32::MAX as f32));
            }
        }
        SampleKind::Pcm32 => {
            let bytes = std::slice::from_raw_parts(ptr, count * 4);
            for chunk in bytes.chunks_exact(4) {
                let s = i32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                out.push((s as f32) / (i32::MAX as f32));
            }
        }
    }
    out
}

fn run_mf_camera_loop<F, E>(
    device_id: String,
    stop: Arc<AtomicBool>,
    ready_tx: std::sync::mpsc::SyncSender<Result<(u32, u32), NssError>>,
    mut on_frame: F,
    on_error: E,
) where
    F: FnMut(CapturedCameraFrame),
    E: FnOnce(NssError),
{
    let _com = ComGuard::init_mta();
    let started = unsafe { MFStartup(MF_VERSION, MFSTARTUP_LITE) };
    if let Err(e) = started {
        let _ = ready_tx.send(Err(map_code(
            e.code().0 as u32,
            &e.to_string(),
            CaptureContext::Camera,
        )));
        return;
    }

    let setup = unsafe { setup_mf_camera(&device_id) };
    let (reader, width, height, default_stride) = match setup {
        Ok(v) => v,
        Err(e) => {
            let _ = ready_tx.send(Err(e));
            unsafe {
                let _ = MFShutdown();
            }
            return;
        }
    };
    let _ = ready_tx.send(Ok((width, height)));

    let row_bytes = (width as usize) * 4;
    let expected_bytes = row_bytes * (height as usize);
    let bottom_up = default_stride < 0;
    let abs_stride = default_stride.unsigned_abs() as usize;
    let src_stride = if abs_stride >= row_bytes {
        abs_stride
    } else {
        row_bytes
    };
    let mut failure: Option<NssError> = None;

    // SAFETY: `reader` lives on this thread until loop exit before `MFShutdown`.
    unsafe {
        while !stop.load(Ordering::Relaxed) {
            let mut flags: u32 = 0;
            let mut sample = None;
            let res = reader.ReadSample(
                MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32,
                0,
                None,
                Some(&mut flags),
                None,
                Some(&mut sample),
            );
            if let Err(e) = res {
                failure = Some(map_code(
                    e.code().0 as u32,
                    &e.to_string(),
                    CaptureContext::Camera,
                ));
                break;
            }
            let Some(sample) = sample else {
                thread::sleep(Duration::from_millis(2));
                continue;
            };
            let t = host_now();
            if let Ok(buf) = sample.ConvertToContiguousBuffer() {
                let mut data_ptr: *mut u8 = std::ptr::null_mut();
                let mut cur_len: u32 = 0;
                if buf.Lock(&mut data_ptr, None, Some(&mut cur_len)).is_ok() && !data_ptr.is_null()
                {
                    let cur_len = cur_len as usize;
                    let mut bgra = vec![0u8; expected_bytes];
                    if src_stride * (height as usize) <= cur_len {
                        // Copy row-by-row honoring both row padding and negative (bottom-up) stride.
                        for dst_y in 0..(height as usize) {
                            let src_y = if bottom_up {
                                (height as usize - 1) - dst_y
                            } else {
                                dst_y
                            };
                            let src_row = data_ptr.add(src_y * src_stride);
                            let dst_row = bgra.as_mut_ptr().add(dst_y * row_bytes);
                            std::ptr::copy_nonoverlapping(src_row, dst_row, row_bytes);
                        }
                    } else {
                        let len = cur_len.min(expected_bytes);
                        std::ptr::copy_nonoverlapping(data_ptr, bgra.as_mut_ptr(), len);
                    }
                    // Ensure opaque alpha for RGB32 (Windows often leaves the X byte 0).
                    for px in bgra.chunks_exact_mut(4) {
                        px[3] = 255;
                    }
                    let _ = buf.Unlock();
                    on_frame(CapturedCameraFrame {
                        host_time: t,
                        width,
                        height,
                        bgra,
                    });
                }
            }
        }
        drop(reader);
        let _ = MFShutdown();
    }

    if let Some(err) = failure {
        on_error(err);
    }
}

unsafe fn setup_mf_camera(device_id: &str) -> Result<(IMFSourceReader, u32, u32, i32), NssError> {
    let mut attrs = None;
    MFCreateAttributes(&mut attrs, 1)
        .map_err(|e| map_code(e.code().0 as u32, &e.to_string(), CaptureContext::Camera))?;
    let attrs = attrs.ok_or_else(|| NssError::Io("Failed to create MF attributes.".into()))?;
    attrs
        .SetGUID(
            &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
            &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
        )
        .map_err(|e| map_code(e.code().0 as u32, &e.to_string(), CaptureContext::Camera))?;

    let mut raw_ptr: *mut Option<IMFActivate> = std::ptr::null_mut();
    let mut count: u32 = 0;
    MFEnumDeviceSources(&attrs, &mut raw_ptr, &mut count)
        .map_err(|e| map_code(e.code().0 as u32, &e.to_string(), CaptureContext::Camera))?;
    if raw_ptr.is_null() || count == 0 {
        return Err(NssError::Unsupported("No camera devices found.".into()));
    }

    let mut chosen: Option<IMFActivate> = None;
    let slice = std::slice::from_raw_parts_mut(raw_ptr, count as usize);
    for (idx, slot) in slice.iter_mut().enumerate() {
        let Some(dev) = slot.as_ref() else {
            continue;
        };
        let sym = mf_attr_string(
            dev,
            &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK,
        );
        let name = mf_attr_string(dev, &MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME);
        let matches = device_id.is_empty()
            || device_id.eq_ignore_ascii_case("default")
            || sym.as_deref() == Some(device_id)
            || name.as_deref() == Some(device_id)
            || (idx == 0 && chosen.is_none() && device_id.is_empty());
        if matches {
            chosen = slot.take();
            break;
        }
    }
    for slot in slice.iter_mut() {
        drop(slot.take());
    }
    CoTaskMemFree(Some(raw_ptr.cast()));

    let activate = chosen.ok_or_else(|| {
        NssError::Unsupported(format!(
            "The selected camera ({device_id}) is no longer available."
        ))
    })?;

    let source: IMFMediaSource = activate
        .ActivateObject()
        .map_err(|e| map_code(e.code().0 as u32, &e.to_string(), CaptureContext::Camera))?;

    let mut reader_attrs = None;
    MFCreateAttributes(&mut reader_attrs, 1)
        .map_err(|e| map_code(e.code().0 as u32, &e.to_string(), CaptureContext::Camera))?;
    if let Some(ref ra) = reader_attrs {
        let _ = ra.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1);
    }

    let reader = MFCreateSourceReaderFromMediaSource(&source, reader_attrs.as_ref())
        .map_err(|e| map_code(e.code().0 as u32, &e.to_string(), CaptureContext::Camera))?;

    let desired: IMFMediaType = MFCreateMediaType()
        .map_err(|e| map_code(e.code().0 as u32, &e.to_string(), CaptureContext::Camera))?;
    desired
        .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
        .map_err(|e| map_code(e.code().0 as u32, &e.to_string(), CaptureContext::Camera))?;
    desired
        .SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)
        .map_err(|e| map_code(e.code().0 as u32, &e.to_string(), CaptureContext::Camera))?;
    reader
        .SetCurrentMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32, None, &desired)
        .map_err(|e| map_code(e.code().0 as u32, &e.to_string(), CaptureContext::Camera))?;

    let actual = reader
        .GetCurrentMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32)
        .map_err(|e| map_code(e.code().0 as u32, &e.to_string(), CaptureContext::Camera))?;
    let packed = actual
        .GetUINT64(&MF_MT_FRAME_SIZE)
        .unwrap_or((1280 << 32) | 720);
    let width = ((packed >> 32) as u32).max(2);
    let height = ((packed & 0xFFFF_FFFF) as u32).max(2);
    let default_stride = actual
        .GetUINT32(&MF_MT_DEFAULT_STRIDE)
        .map(|u| u as i32)
        .unwrap_or((width * 4) as i32);

    Ok((reader, width, height, default_stride))
}

unsafe fn device_id_string(device: &IMMDevice) -> Option<String> {
    let raw: PWSTR = device.GetId().ok()?;
    let s = raw.to_string().ok();
    CoTaskMemFree(Some(raw.0.cast()));
    s
}

unsafe fn device_friendly_name(device: &IMMDevice) -> Option<String> {
    let store = device.OpenPropertyStore(STGM_READ).ok()?;
    let mut prop = store.GetValue(&PKEY_Device_FriendlyName).ok()?;
    let result = prop.to_string();
    let _ = PropVariantClear(&mut prop);
    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

unsafe fn mf_attr_string(activate: &IMFActivate, key: &GUID) -> Option<String> {
    let mut raw = PWSTR::null();
    let mut len: u32 = 0;
    activate.GetAllocatedString(key, &mut raw, &mut len).ok()?;
    if raw.is_null() {
        return None;
    }
    let slice = std::slice::from_raw_parts(raw.0, len as usize);
    let s = String::from_utf16_lossy(slice);
    CoTaskMemFree(Some(raw.0.cast()));
    Some(s)
}
