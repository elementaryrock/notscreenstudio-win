//! Central application state (`AppState`) and Tauri IPC command surface.
//!
//! Orchestrates recording (`nss-capture` + `nss-input`), project bundles (`nss-project`),
//! motion planning (`nss-motion`), live GPU preview (`nss-render`), MP4 export (`nss-export`),
//! settings, tray state, global hotkeys, and HUD overlays.
//!
//! Every command that creates windows, touches WinRT/COM/FFmpeg/GPU, or performs blocking disk
//! I/O is `async fn` and runs heavy work via `tauri::async_runtime::spawn_blocking`. On Windows,
//! running `WebviewWindowBuilder::build`, WinRT `.join()`, or COM MTA init synchronously on the
//! main WebView2 STA UI thread deadlocks the application (`STATUS_APPLICATION_HANG` / `0xCFFFFFFF`).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use nss_capture::{
    CaptureDeviceInfo, CaptureDevices, CapturePermissions, CaptureSourceCatalog, CaptureSourceList,
    PermissionStatus, ScreenCaptureRecordingSession, WarmableRecordingSession,
};
use nss_core::background_presets;
use nss_core::{
    host_now, Background, CaptureSource, ClickEvent, ClickKind, CursorSample, CursorShape,
    CursorShapeChange, CursorTrack, ExportPreset, ExportProgress, InputLogger, MediaTime,
    MouseButton, PixelSize, Project, ProjectReference, ProjectSummary, RecordingAssets,
    RecordingOptions, RecordingSession, Rect, ZoomBlock,
};
use nss_export::{
    CancelToken, FfmpegCapabilities, FfmpegPaths, Mp4Exporter, FFMPEG_DOWNLOAD_URL,
    TESTED_FFMPEG_VERSION,
};
use nss_input::{SystemInputLogger, SystemInputLoggerConfig};
use nss_motion::{MotionTuning, ZoomPlanner};
use nss_project::{
    default_library_dir, FfmpegThumbnailer, FileProjectLibrary, FileProjectStore, LibraryOptions,
    ProjectError, RecycleBinTrash, Thumbnailer,
};
use nss_render::{PassthroughRenderer, Renderer};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::ipc::Response as IpcResponse;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::overlay::{self, OverlayKind, WindowRegistry};
use crate::preview::{
    discover_system_wallpapers, render_background_swatch, PreviewEngine, WallpaperEntry,
};
use crate::settings::{ShellSettings, ShellSettingsStore};
use crate::shell_model::{BubbleGeometry, ElapsedClock, ShellEvent, ShellPhase, ShellStateMachine};
use crate::tray;
use crate::win_shell::{self, GlobalHotKeyManager, HotKeyAction, HotKeyRegistrationStatus};

/// Current implementation stage reported to the UI.
pub const PORT_STAGE: u32 = 10;

/// Lazy FFmpeg thumbnailer so installing FFmpeg after launch starts producing thumbnails without
/// requiring an app restart.
#[derive(Debug, Default)]
struct DynamicFfmpegThumbnailer {
    cached: Mutex<Option<FfmpegThumbnailer>>,
}

impl Thumbnailer for DynamicFfmpegThumbnailer {
    fn generate(&self, video: &Path, at: MediaTime, to: &Path) -> Result<bool, ProjectError> {
        let thumb = {
            let mut guard = self.cached.lock();
            if guard.is_none() {
                if let Ok(paths) = FfmpegPaths::discover() {
                    *guard = Some(FfmpegThumbnailer::new(paths.ffmpeg));
                }
            }
            guard.clone()
        };
        match thumb {
            Some(t) => t.generate(video, at, to),
            None => Ok(false),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreInfo {
    pub version: String,
    pub platform: String,
    pub stage: u32,
    pub library_dir: String,
    pub gpu_available: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellSnapshot {
    pub phase: ShellPhase,
    pub paused: bool,
    pub elapsed_seconds: f64,
    pub countdown_remaining: Option<i32>,
    pub hotkeys: HotKeyRegistrationStatus,
    pub borderless_note: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DevicesList {
    pub microphones: Vec<CaptureDeviceInfo>,
    pub cameras: Vec<CaptureDeviceInfo>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionsDto {
    pub screen_recording: String,
    pub microphone: String,
    pub camera: String,
    pub borderless_capture: String,
    pub borderless_note: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FfmpegStatusDto {
    pub available: bool,
    pub path: Option<String>,
    pub tested_version: String,
    pub download_url: String,
    pub h264: bool,
    pub hevc: bool,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorOpenDto {
    pub reference: ProjectReference,
    pub project: Project,
    pub track: CursorTrack,
    pub duration: f64,
    pub renderer_name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewLayoutDto {
    pub width: u32,
    pub height: u32,
    pub content_x: f64,
    pub content_y: f64,
    pub content_width: f64,
    pub content_height: f64,
    pub camera_scale: f64,
    pub camera_center_x: f64,
    pub camera_center_y: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundPresetsDto {
    pub solids: Vec<PresetEntry>,
    pub gradients: Vec<PresetEntry>,
    pub meshes: Vec<PresetEntry>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetEntry {
    pub id: String,
    pub name: String,
    pub background: Background,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineThumbTile {
    pub time: f64,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

struct ActiveRecording {
    session: ScreenCaptureRecordingSession,
    input: SystemInputLogger,
    bundle_ref: ProjectReference,
    clock: Option<ElapsedClock>,
    source: CaptureSource,
    has_camera: bool,
}

struct ShellRuntime {
    phase: ShellPhase,
    countdown_remaining: Option<i32>,
    countdown_cancel: Arc<AtomicBool>,
    recording_gen: u64,
    active: Option<ActiveRecording>,
    hotkeys: Option<GlobalHotKeyManager>,
    hotkey_status: HotKeyRegistrationStatus,
}

/// Shared state managed by Tauri across all windows and commands.
pub struct AppState {
    pub store: FileProjectStore,
    pub library: FileProjectLibrary,
    pub settings_store: ShellSettingsStore,
    pub settings: Mutex<ShellSettings>,
    pub windows: Arc<WindowRegistry>,
    runtime: Mutex<ShellRuntime>,
    preview: Mutex<Option<PreviewEngine>>,
    export_cancel: Mutex<Option<CancelToken>>,
    export_gen: AtomicU64,
}

impl AppState {
    #[must_use]
    pub fn new() -> Self {
        let library_dir = resolve_library_dir();
        let store = FileProjectStore::at(&library_dir);
        let thumbnailer: Arc<dyn Thumbnailer> = Arc::new(DynamicFfmpegThumbnailer::default());
        let library = FileProjectLibrary::new(LibraryOptions {
            library: library_dir,
            thumbnailer,
            trash: Arc::new(RecycleBinTrash),
            ..LibraryOptions::default()
        });
        let settings_store = ShellSettingsStore::default_location();
        let settings = settings_store.load();

        Self {
            store,
            library,
            settings_store,
            settings: Mutex::new(settings),
            windows: Arc::new(WindowRegistry::new()),
            runtime: Mutex::new(ShellRuntime {
                phase: ShellPhase::Idle,
                countdown_remaining: None,
                countdown_cancel: Arc::new(AtomicBool::new(false)),
                recording_gen: 0,
                active: None,
                hotkeys: None,
                hotkey_status: HotKeyRegistrationStatus::default(),
            }),
            preview: Mutex::new(None),
            export_cancel: Mutex::new(None),
            export_gen: AtomicU64::new(0),
        }
    }

    pub fn install_hotkeys(&self, app: AppHandle) {
        // Stop and unregister the previous hotkey manager BEFORE registering the new chords so
        // unchanged shortcuts never self-conflict with our own previous registration.
        let prev = self.runtime.lock().hotkeys.take();
        drop(prev);

        let (rec, pause) = {
            let s = self.settings.lock();
            (s.record_hotkey.clone(), s.pause_hotkey.clone())
        };
        let app_cb = app.clone();
        let mgr = GlobalHotKeyManager::start(&rec, &pause, move |action| match action {
            HotKeyAction::ToggleRecord => {
                let _ = app_cb.emit("nss://hotkey", "toggle-record");
            }
            HotKeyAction::TogglePause => {
                let _ = app_cb.emit("nss://hotkey", "toggle-pause");
            }
        });
        let mut rt = self.runtime.lock();
        rt.hotkey_status = mgr.status.clone();
        rt.hotkeys = Some(mgr);
    }

    pub fn is_busy_recording(&self) -> bool {
        self.runtime.lock().phase.is_busy()
    }

    pub fn snapshot(&self) -> ShellSnapshot {
        let rt = self.runtime.lock();
        let (paused, elapsed) = match rt.active.as_ref().and_then(|a| a.clock) {
            Some(clock) => (clock.is_paused(), clock.elapsed(host_now())),
            None => (false, 0.0),
        };
        ShellSnapshot {
            phase: rt.phase.clone(),
            paused,
            elapsed_seconds: elapsed,
            countdown_remaining: rt.countdown_remaining,
            hotkeys: rt.hotkey_status.clone(),
            borderless_note: CapturePermissions::BORDERLESS_NOTE.to_owned(),
        }
    }

    pub fn emit_state(&self, app: &AppHandle) {
        let snap = self.snapshot();
        tray::update_tray(app, &snap.phase, snap.paused, snap.elapsed_seconds);
        let _ = app.emit("nss://shell-state", &snap);
    }

    /// Cancels any active recording/countdown/export, flushes debounced autosaves, and unregisters
    /// global hotkeys on application exit.
    pub fn shutdown(&self) {
        if let Some(token) = self.export_cancel.lock().take() {
            token.cancel();
        }
        let active_to_cancel = {
            let mut rt = self.runtime.lock();
            rt.countdown_cancel.store(true, Ordering::SeqCst);
            rt.recording_gen = rt.recording_gen.wrapping_add(1);
            rt.countdown_remaining = None;
            rt.phase = ShellPhase::Idle;
            rt.active.take()
        };
        if let Some(mut active) = active_to_cancel {
            active.session.cancel();
            let _ = active.input.stop(0.0);
            let _ = std::fs::remove_dir_all(&active.bundle_ref.url);
        }
        self.store.flush_pending_autosaves();
        let hk = self.runtime.lock().hotkeys.take();
        drop(hk);
    }
}

fn resolve_library_dir() -> PathBuf {
    win_shell::known_videos_dir()
        .map(|v| v.join("notscreenstudio"))
        .unwrap_or_else(default_library_dir)
}

fn status_label(s: PermissionStatus) -> String {
    match s {
        PermissionStatus::Granted => "granted".into(),
        PermissionStatus::Denied => "denied".into(),
        PermissionStatus::NotDetermined => "notDetermined".into(),
    }
}

fn pack_rgba_response(width: u32, height: u32, rgba: Vec<u8>) -> IpcResponse {
    let mut payload = Vec::with_capacity(8 + rgba.len());
    payload.extend_from_slice(&width.to_le_bytes());
    payload.extend_from_slice(&height.to_le_bytes());
    payload.extend_from_slice(&rgba);
    IpcResponse::new(payload)
}

async fn run_blocking<T, F>(f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("Background task failed: {e}"))
}

// ============================================================================
// IPC Commands
// ============================================================================

#[tauri::command]
pub fn ping(message: String) -> String {
    message
}

#[tauri::command]
pub async fn core_info(app: AppHandle) -> Result<CoreInfo, String> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        CoreInfo {
            version: env!("CARGO_PKG_VERSION").to_owned(),
            platform: std::env::consts::OS.to_owned(),
            stage: PORT_STAGE,
            library_dir: state.store.library().to_string_lossy().into_owned(),
            gpu_available: nss_render::is_supported(),
        }
    })
    .await
}

#[tauri::command]
pub fn get_shell_snapshot(state: State<'_, AppState>) -> ShellSnapshot {
    state.snapshot()
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> ShellSettings {
    state.settings.lock().clone()
}

#[tauri::command]
pub async fn save_settings(
    app: AppHandle,
    settings: ShellSettings,
) -> Result<ShellSettings, String> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        let clean = settings.sanitized();
        let hotkeys_changed = {
            let cur = state.settings.lock();
            cur.record_hotkey != clean.record_hotkey || cur.pause_hotkey != clean.pause_hotkey
        };
        state.settings_store.save(&clean)?;
        *state.settings.lock() = clean.clone();
        if hotkeys_changed {
            state.install_hotkeys(app.clone());
        }
        let _ = app.emit("nss://settings-changed", &clean);
        state.emit_state(&app);
        Ok(clean)
    })
    .await?
}

#[tauri::command]
pub async fn list_capture_sources() -> Result<CaptureSourceList, String> {
    run_blocking(CaptureSourceCatalog::load).await
}

#[tauri::command]
pub async fn list_devices() -> Result<DevicesList, String> {
    run_blocking(|| DevicesList {
        microphones: CaptureDevices::microphones(),
        cameras: CaptureDevices::cameras(),
    })
    .await
}

#[tauri::command]
pub async fn get_permissions() -> Result<PermissionsDto, String> {
    run_blocking(read_permissions_dto).await
}

fn read_permissions_dto() -> PermissionsDto {
    let snap = CapturePermissions::snapshot();
    PermissionsDto {
        screen_recording: status_label(snap.screen_recording),
        microphone: status_label(snap.microphone),
        camera: status_label(snap.camera),
        borderless_capture: status_label(snap.borderless_capture),
        borderless_note: CapturePermissions::BORDERLESS_NOTE.to_owned(),
    }
}

#[tauri::command]
pub async fn request_borderless_access() -> Result<PermissionsDto, String> {
    run_blocking(|| {
        let _ = CapturePermissions::request_borderless_once();
        read_permissions_dto()
    })
    .await
}

#[tauri::command]
pub async fn check_ffmpeg() -> Result<FfmpegStatusDto, String> {
    run_blocking(|| match FfmpegPaths::discover() {
        Ok(paths) => {
            let caps: FfmpegCapabilities = paths.capabilities();
            FfmpegStatusDto {
                available: true,
                path: Some(paths.ffmpeg.to_string_lossy().into_owned()),
                tested_version: TESTED_FFMPEG_VERSION.to_owned(),
                download_url: FFMPEG_DOWNLOAD_URL.to_owned(),
                h264: caps.encoder_for(nss_core::VideoCodec::H264).is_some(),
                hevc: caps.encoder_for(nss_core::VideoCodec::Hevc).is_some(),
                message: None,
            }
        }
        Err(e) => FfmpegStatusDto {
            available: false,
            path: None,
            tested_version: TESTED_FFMPEG_VERSION.to_owned(),
            download_url: FFMPEG_DOWNLOAD_URL.to_owned(),
            h264: false,
            hevc: false,
            message: Some(e.user_message().to_owned()),
        },
    })
    .await
}

// ----------------------------------------------------------------------------
// Home / Library commands
// ----------------------------------------------------------------------------

#[tauri::command]
pub async fn list_projects(app: AppHandle) -> Result<Vec<ProjectSummary>, String> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        let mut list = state.library.summaries();
        for summary in &mut list {
            if summary.thumbnail_url.is_none() {
                summary.thumbnail_url = state.library.thumbnail_url(summary);
            }
        }
        list
    })
    .await
}

#[tauri::command]
pub async fn get_project_thumbnail(path: String) -> Result<IpcResponse, String> {
    run_blocking(move || {
        let bytes = std::fs::read(&path).unwrap_or_default();
        IpcResponse::new(bytes)
    })
    .await
}

#[tauri::command]
pub async fn rename_project(
    app: AppHandle,
    summary: ProjectSummary,
    new_name: String,
) -> Result<(), String> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        // Flush any pending editor autosave first so a queued write cannot revert the rename.
        state.store.flush_pending_autosaves();
        state
            .library
            .rename(&summary, &new_name)
            .map_err(|e| e.to_string())
    })
    .await?
}

#[tauri::command]
pub async fn delete_project(app: AppHandle, summary: ProjectSummary) -> Result<(), String> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        state.store.flush_pending_autosaves();
        state.library.delete(&summary).map_err(|e| e.to_string())?;
        state.store.remove_from_recents(&summary.bundle);
        Ok(())
    })
    .await?
}

#[tauri::command]
pub async fn reveal_path(app: AppHandle, path: String) -> Result<(), String> {
    run_blocking(move || {
        let target = PathBuf::from(&path);
        let lib_dir = app.state::<AppState>().store.library().to_path_buf();
        if target == lib_dir && !target.exists() {
            let _ = std::fs::create_dir_all(&target);
        }
        win_shell::reveal_in_explorer(&target)
    })
    .await?
}

/// Creates a synthetic interactive demo project (with synthetic cursor path, clicks, and a
/// generated 6-second 1080p `screen.mov` when FFmpeg is installed) so users can explore the full
/// editor, auto-zoom timeline, GPU preview and MP4 export even before their first live capture.
#[tauri::command]
pub async fn create_demo_project(app: AppHandle) -> Result<ProjectReference, String> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        let reference = state
            .store
            .create_project("Sample Recording")
            .map_err(|e| e.to_string())?;
        let duration = 6.0;
        let assets = RecordingAssets::new(PixelSize::new(1920, 1080), 60.0, duration);

        // Best-effort synthetic video so the sample project renders a real timeline strip and
        // exports cleanly to MP4 when FFmpeg is installed.
        let screen_path = reference.url.join(&assets.screen_file);
        let _ = generate_demo_video(&screen_path, duration);

        let mut samples = Vec::with_capacity(721);
        for i in 0..=720 {
            let t = f64::from(i) / 120.0;
            let phase = t / duration;
            let x = 0.25 + 0.5 * (phase * std::f64::consts::TAU).sin() * 0.5 + 0.25;
            let y = 0.30 + 0.4 * (phase * std::f64::consts::PI).sin();
            samples.push(CursorSample::new(
                t,
                x.clamp(0.05, 0.95),
                y.clamp(0.05, 0.95),
            ));
        }
        let clicks = vec![
            ClickEvent::new(1.2, 0.62, 0.52, MouseButton::Left, ClickKind::Down),
            ClickEvent::new(1.3, 0.62, 0.52, MouseButton::Left, ClickKind::Up),
            ClickEvent::new(3.8, 0.32, 0.66, MouseButton::Left, ClickKind::Down),
            ClickEvent::new(3.9, 0.32, 0.66, MouseButton::Left, ClickKind::Up),
        ];
        let shapes = vec![
            CursorShapeChange::new(0.0, CursorShape::Arrow),
            CursorShapeChange::new(1.0, CursorShape::PointingHand),
            CursorShapeChange::new(2.2, CursorShape::IBeam),
            CursorShapeChange::new(3.5, CursorShape::Arrow),
        ];
        let track = CursorTrack {
            samples,
            clicks,
            sample_rate: 120.0,
            shapes,
        };

        let mut project = Project::new("Sample Recording", assets.clone());
        project.style.camera.visible = false;
        project.timeline.zoom_blocks = ZoomPlanner::auto_zoom_blocks(
            &track,
            duration,
            assets.source_pixel_size.aspect(),
            &[],
            &MotionTuning::default(),
        );
        // Blocks are materialized on the timeline; disable dynamic auto-zoom so editing/deleting
        // those blocks stays authoritative.
        project.timeline.auto_zoom_enabled = false;
        project.touch();

        state
            .store
            .save_cursor_track(&track, &reference, &assets)
            .map_err(|e| e.to_string())?;
        state
            .store
            .save(&project, &reference)
            .map_err(|e| e.to_string())?;
        Ok(reference)
    })
    .await?
}

fn generate_demo_video(output: &Path, duration_secs: f64) -> Result<(), String> {
    let paths = FfmpegPaths::discover().map_err(|e| e.to_string())?;
    let mut cmd = Command::new(&paths.ffmpeg);
    cmd.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-f",
        "lavfi",
        "-i",
        &format!("testsrc2=size=1280x720:rate=30:duration={duration_secs:.1}"),
        "-vf",
        "scale=1920:1080",
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-pix_fmt",
        "yuv420p",
        "-movflags",
        "+faststart",
    ])
    .arg(output)
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null());
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    let status = cmd.status().map_err(|e| e.to_string())?;
    if status.success() && output.is_file() {
        Ok(())
    } else {
        let _ = std::fs::remove_file(output);
        Err("Failed to generate sample video".into())
    }
}

// ----------------------------------------------------------------------------
// Recording commands (Setup -> Countdown -> Record -> Pause/Resume -> Stop)
// ----------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartRecordingRequest {
    pub source: CaptureSource,
    pub mic_device_id: Option<String>,
    pub camera_device_id: Option<String>,
    pub capture_system_audio: Option<bool>,
    pub countdown_seconds: Option<i32>,
}

#[tauri::command]
pub async fn start_recording(
    app: AppHandle,
    request: StartRecordingRequest,
) -> Result<ShellSnapshot, String> {
    run_blocking(move || start_recording_sync(app, request)).await?
}

fn start_recording_sync(
    app: AppHandle,
    request: StartRecordingRequest,
) -> Result<ShellSnapshot, String> {
    let state = app.state::<AppState>();
    let countdown = request
        .countdown_seconds
        .unwrap_or_else(|| state.settings.lock().countdown_seconds)
        .clamp(0, 5);

    let (cancel_flag, rec_gen) = {
        let mut rt = state.runtime.lock();
        let next = ShellStateMachine::reduce(&rt.phase, ShellEvent::StartRequested)
            .ok_or_else(|| "A recording is already in progress.".to_owned())?;
        rt.phase = next;
        rt.recording_gen = rt.recording_gen.wrapping_add(1);
        let cancel = Arc::new(AtomicBool::new(false));
        rt.countdown_cancel = Arc::clone(&cancel);
        rt.countdown_remaining = if countdown > 0 { Some(countdown) } else { None };
        (cancel, rt.recording_gen)
    };
    state.emit_state(&app);

    let mut options = RecordingOptions::new(request.source);
    options.mic_device_id = request.mic_device_id;
    options.camera_device_id = request.camera_device_id;
    options.capture_system_audio = request.capture_system_audio.unwrap_or(false);
    options.countdown_seconds = countdown;
    let has_camera = options.camera_device_id.is_some();

    let name = format!("Recording {}", timestamp_slug());
    let bundle_ref = match state.store.create_project(&name) {
        Ok(r) => r,
        Err(e) => {
            fail_start(&app, &state, e.to_string());
            return Err(e.to_string());
        }
    };

    let own_windows = Arc::clone(&state.windows);
    let mut session = ScreenCaptureRecordingSession::new(Arc::new(move || own_windows.all_hwnds()));

    // Subscribe to mid-recording fatal capture events (e.g., captured window closed or disk full)
    // so the UI and overlays never stay stuck in Recording state.
    let app_err = app.clone();
    session.subscribe_events(Box::new(move |ev| {
        if ev.is_fatal {
            let msg = ev.error.user_message().to_owned();
            let app_cleanup = app_err.clone();
            let _ = thread::Builder::new()
                .name("nss-capture-fatal".into())
                .spawn(move || {
                    handle_fatal_capture_error(&app_cleanup, rec_gen, msg);
                });
        }
    }));

    if let Err(e) = session.preflight(&options) {
        let _ = std::fs::remove_dir_all(&bundle_ref.url);
        fail_start(&app, &state, e.user_message().to_owned());
        return Err(e.user_message().to_owned());
    }

    let mut input = SystemInputLogger::new(SystemInputLoggerConfig::default());
    if let Err(e) = input.start(options.source) {
        let _ = std::fs::remove_dir_all(&bundle_ref.url);
        fail_start(&app, &state, e.user_message().to_owned());
        return Err(e.user_message().to_owned());
    }

    if countdown == 0 {
        if let Err(e) = session.start(options.clone(), &bundle_ref.url) {
            let _ = input.stop(0.0);
            let _ = std::fs::remove_dir_all(&bundle_ref.url);
            fail_start(&app, &state, e.user_message().to_owned());
            return Err(e.user_message().to_owned());
        }
        let now = host_now();
        {
            let mut rt = state.runtime.lock();
            if cancel_flag.load(Ordering::SeqCst) || rt.recording_gen != rec_gen {
                drop(rt);
                session.cancel();
                let _ = input.stop(0.0);
                let _ = std::fs::remove_dir_all(&bundle_ref.url);
                return Ok(state.snapshot());
            }
            rt.phase = ShellPhase::Recording { started_at: now };
            rt.countdown_remaining = None;
            rt.active = Some(ActiveRecording {
                session,
                input,
                bundle_ref,
                clock: Some(ElapsedClock::new(now)),
                source: request.source,
                has_camera,
            });
        }
        overlay::close_overlay(&app, &state.windows, OverlayKind::SetupBar);
        let _ = overlay::open_overlay_for_source(
            &app,
            &state.windows,
            OverlayKind::RecordingBar,
            None,
            Some(request.source),
        );
        if has_camera {
            let _ = overlay::open_overlay_for_source(
                &app,
                &state.windows,
                OverlayKind::CameraBubble,
                None,
                Some(request.source),
            );
        }
        state.windows.refresh_all();
        state.emit_state(&app);
        spawn_elapsed_ticker(app.clone(), rec_gen);
        return Ok(state.snapshot());
    }

    // Warm up capture during the countdown (ADR 0007 / FrameGate).
    if let Err(e) = session.prepare(options.clone(), &bundle_ref.url) {
        let _ = input.stop(0.0);
        let _ = std::fs::remove_dir_all(&bundle_ref.url);
        fail_start(&app, &state, e.user_message().to_owned());
        return Err(e.user_message().to_owned());
    }

    {
        let mut rt = state.runtime.lock();
        if cancel_flag.load(Ordering::SeqCst) || rt.recording_gen != rec_gen {
            drop(rt);
            session.cancel_prepared();
            let _ = input.stop(0.0);
            let _ = std::fs::remove_dir_all(&bundle_ref.url);
            return Ok(state.snapshot());
        }
        rt.active = Some(ActiveRecording {
            session,
            input,
            bundle_ref,
            clock: None,
            source: request.source,
            has_camera,
        });
    }

    overlay::close_overlay(&app, &state.windows, OverlayKind::SetupBar);
    let _ = overlay::open_overlay_for_source(
        &app,
        &state.windows,
        OverlayKind::Countdown,
        None,
        Some(request.source),
    );
    state.windows.refresh_all();
    state.emit_state(&app);

    let app_bg = app.clone();
    let _ = thread::Builder::new()
        .name("nss-countdown".into())
        .spawn(move || {
            for remaining in (1..=countdown).rev() {
                if cancel_flag.load(Ordering::SeqCst) {
                    return;
                }
                {
                    let st = app_bg.state::<AppState>();
                    let mut rt = st.runtime.lock();
                    if rt.recording_gen != rec_gen || !matches!(rt.phase, ShellPhase::Preparing) {
                        return;
                    }
                    rt.countdown_remaining = Some(remaining);
                    drop(rt);
                    st.emit_state(&app_bg);
                }
                for _ in 0..20 {
                    if cancel_flag.load(Ordering::SeqCst) {
                        return;
                    }
                    thread::sleep(Duration::from_millis(50));
                }
            }
            if cancel_flag.load(Ordering::SeqCst) {
                return;
            }

            let st = app_bg.state::<AppState>();
            // Take `active` out BEFORE calling `begin_recording()` so we never hold `runtime.lock()`
            // across the up-to-3-second wait for the first screen frame.
            let mut active = {
                let mut rt = st.runtime.lock();
                if cancel_flag.load(Ordering::SeqCst)
                    || rt.recording_gen != rec_gen
                    || !matches!(rt.phase, ShellPhase::Preparing)
                {
                    return;
                }
                match rt.active.take() {
                    Some(a) => a,
                    None => return,
                }
            };

            let begin_res = active.session.begin_recording();
            overlay::close_overlay(&app_bg, &st.windows, OverlayKind::Countdown);

            // Re-check cancellation after `begin_recording()` returns.
            if cancel_flag.load(Ordering::SeqCst) {
                active.session.cancel();
                let _ = active.input.stop(0.0);
                let _ = std::fs::remove_dir_all(&active.bundle_ref.url);
                return;
            }

            match begin_res {
                Ok(()) => {
                    let now = host_now();
                    let src = active.source;
                    let show_cam = active.has_camera;
                    active.clock = Some(ElapsedClock::new(now));
                    active.session.refresh_exclusions();
                    {
                        let mut rt = st.runtime.lock();
                        if rt.recording_gen != rec_gen || !matches!(rt.phase, ShellPhase::Preparing)
                        {
                            drop(rt);
                            active.session.cancel();
                            let _ = active.input.stop(0.0);
                            let _ = std::fs::remove_dir_all(&active.bundle_ref.url);
                            return;
                        }
                        rt.phase = ShellPhase::Recording { started_at: now };
                        rt.countdown_remaining = None;
                        rt.active = Some(active);
                    }
                    let _ = overlay::open_overlay_for_source(
                        &app_bg,
                        &st.windows,
                        OverlayKind::RecordingBar,
                        None,
                        Some(src),
                    );
                    if show_cam {
                        let _ = overlay::open_overlay_for_source(
                            &app_bg,
                            &st.windows,
                            OverlayKind::CameraBubble,
                            None,
                            Some(src),
                        );
                    }
                    st.windows.refresh_all();
                    st.emit_state(&app_bg);
                    spawn_elapsed_ticker(app_bg, rec_gen);
                }
                Err(e) => {
                    active.session.cancel_prepared();
                    let _ = active.input.stop(0.0);
                    let _ = std::fs::remove_dir_all(&active.bundle_ref.url);
                    let mut rt = st.runtime.lock();
                    if rt.recording_gen == rec_gen {
                        rt.countdown_remaining = None;
                        rt.phase = ShellPhase::Failed {
                            message: e.user_message().to_owned(),
                        };
                    }
                    drop(rt);
                    st.emit_state(&app_bg);
                }
            }
        });

    Ok(state.snapshot())
}

fn handle_fatal_capture_error(app: &AppHandle, rec_gen: u64, message: String) {
    let st = app.state::<AppState>();
    let failed_active = {
        let mut rt = st.runtime.lock();
        if rt.recording_gen != rec_gen || !rt.phase.is_recording() {
            return;
        }
        rt.countdown_cancel.store(true, Ordering::SeqCst);
        rt.countdown_remaining = None;
        rt.phase = ShellPhase::Failed { message };
        rt.active.take()
    };
    overlay::close_all_overlays(app, &st.windows);
    if let Some(mut active) = failed_active {
        active.session.cancel();
        let _ = active.input.stop(0.0);
        let _ = std::fs::remove_dir_all(&active.bundle_ref.url);
    }
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.set_focus();
    }
    st.emit_state(app);
}

#[tauri::command]
pub async fn cancel_countdown(app: AppHandle) -> Result<ShellSnapshot, String> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        let cancelled = {
            let mut rt = state.runtime.lock();
            if !matches!(rt.phase, ShellPhase::Preparing) {
                return state.snapshot();
            }
            rt.countdown_cancel.store(true, Ordering::SeqCst);
            rt.recording_gen = rt.recording_gen.wrapping_add(1);
            rt.countdown_remaining = None;
            if let Some(next) = ShellStateMachine::reduce(&rt.phase, ShellEvent::StartCancelled) {
                rt.phase = next;
            }
            rt.active.take()
        };
        if let Some(mut active) = cancelled {
            active.session.cancel_prepared();
            let _ = active.input.stop(0.0);
            let _ = std::fs::remove_dir_all(&active.bundle_ref.url);
        }
        overlay::close_overlay(&app, &state.windows, OverlayKind::Countdown);
        state.emit_state(&app);
        state.snapshot()
    })
    .await
}

#[tauri::command]
pub async fn pause_recording(app: AppHandle) -> Result<ShellSnapshot, String> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        {
            let mut rt = state.runtime.lock();
            let active = rt
                .active
                .as_mut()
                .ok_or_else(|| "No active recording.".to_owned())?;
            active
                .session
                .pause()
                .map_err(|e| e.user_message().to_owned())?;
            if let Some(ref mut clock) = active.clock {
                clock.pause(host_now());
            }
        }
        state.emit_state(&app);
        Ok(state.snapshot())
    })
    .await?
}

#[tauri::command]
pub async fn resume_recording(app: AppHandle) -> Result<ShellSnapshot, String> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        {
            let mut rt = state.runtime.lock();
            let active = rt
                .active
                .as_mut()
                .ok_or_else(|| "No active recording.".to_owned())?;
            active
                .session
                .resume()
                .map_err(|e| e.user_message().to_owned())?;
            if let Some(ref mut clock) = active.clock {
                clock.resume(host_now());
            }
        }
        state.emit_state(&app);
        Ok(state.snapshot())
    })
    .await?
}

#[tauri::command]
pub async fn stop_recording(app: AppHandle) -> Result<ProjectReference, String> {
    run_blocking(move || stop_recording_sync(app)).await?
}

fn stop_recording_sync(app: AppHandle) -> Result<ProjectReference, String> {
    let state = app.state::<AppState>();
    let mut active = {
        let mut rt = state.runtime.lock();
        let next = ShellStateMachine::reduce(&rt.phase, ShellEvent::StopRequested)
            .ok_or_else(|| "No recording is in progress.".to_owned())?;
        let active = rt
            .active
            .take()
            .ok_or_else(|| "No active recording session.".to_owned())?;
        rt.phase = next;
        rt.recording_gen = rt.recording_gen.wrapping_add(1);
        active
    };
    overlay::close_overlay(&app, &state.windows, OverlayKind::RecordingBar);
    overlay::close_overlay(&app, &state.windows, OverlayKind::CameraBubble);
    state.emit_state(&app);

    let pause_timeline = active.session.pause_timeline();
    let assets = match active.session.stop() {
        Ok(a) => a,
        Err(e) => {
            let _ = active.input.stop(0.0);
            let _ = std::fs::remove_dir_all(&active.bundle_ref.url);
            fail_save(&app, &state, e.user_message().to_owned());
            return Err(e.user_message().to_owned());
        }
    };

    let raw_track = active.input.stop(assets.start_host_time);
    let track = pause_timeline.removing_pauses(&raw_track, assets.start_host_time);

    let name = active
        .bundle_ref
        .url
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Recording".into());
    let mut project = Project::new(name, assets.clone());
    {
        let settings = state.settings.lock();
        project.style.aspect_ratio = settings.default_aspect;
        project.style.camera.shape = settings.camera_shape;
        project.style.camera.roundness = settings.bubble_roundness;
        project.style.camera.visible = assets.camera_file.is_some();
        let out_aspect = settings
            .default_aspect
            .output_size(nss_core::ExportResolution::P1080)
            .aspect();
        let (out_pos, out_size) = BubbleGeometry::to_output_space(
            settings.bubble_center,
            settings.bubble_size,
            out_aspect,
            assets.source_pixel_size.aspect(),
            project.style.padding,
        );
        project.style.camera.position = out_pos;
        project.style.camera.size = out_size;
    }
    project.timeline.zoom_blocks = ZoomPlanner::auto_zoom_blocks(
        &track,
        assets.duration,
        assets.source_pixel_size.aspect(),
        &[],
        &MotionTuning::default(),
    );
    project.timeline.auto_zoom_enabled = false;
    project.touch();

    if let Err(e) = state
        .store
        .save_cursor_track(&track, &active.bundle_ref, &assets)
    {
        fail_save(&app, &state, e.to_string());
        return Err(e.to_string());
    }
    if let Err(e) = state.store.save(&project, &active.bundle_ref) {
        fail_save(&app, &state, e.to_string());
        return Err(e.to_string());
    }

    {
        let mut rt = state.runtime.lock();
        if let Some(next) =
            ShellStateMachine::reduce(&rt.phase, ShellEvent::Saved(active.bundle_ref.clone()))
        {
            rt.phase = next;
        }
    }
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.set_focus();
    }
    state.emit_state(&app);
    let _ = app.emit("nss://recording-saved", &active.bundle_ref);
    Ok(active.bundle_ref)
}

#[tauri::command]
pub async fn discard_recording(app: AppHandle) -> Result<ShellSnapshot, String> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        let discarded = {
            let mut rt = state.runtime.lock();
            rt.countdown_cancel.store(true, Ordering::SeqCst);
            rt.recording_gen = rt.recording_gen.wrapping_add(1);
            rt.phase = ShellPhase::Idle;
            rt.countdown_remaining = None;
            rt.active.take()
        };
        if let Some(mut active) = discarded {
            active.session.cancel();
            let _ = active.input.stop(0.0);
            let _ = std::fs::remove_dir_all(&active.bundle_ref.url);
        }
        overlay::close_all_overlays(&app, &state.windows);
        state.emit_state(&app);
        state.snapshot()
    })
    .await
}

#[tauri::command]
pub fn dismiss_error(app: AppHandle, state: State<'_, AppState>) -> ShellSnapshot {
    {
        let mut rt = state.runtime.lock();
        if let Some(next) = ShellStateMachine::reduce(&rt.phase, ShellEvent::DismissError) {
            rt.phase = next;
        }
    }
    state.emit_state(&app);
    state.snapshot()
}

// ----------------------------------------------------------------------------
// Overlay HUD commands
// ----------------------------------------------------------------------------

#[tauri::command]
pub async fn open_overlay(
    app: AppHandle,
    kind: OverlayKind,
    bounds: Option<(f64, f64, f64, f64)>,
) -> Result<(), String> {
    let source = app.state::<AppState>().settings.lock().source;
    run_blocking(move || {
        let state = app.state::<AppState>();
        overlay::open_overlay_for_source(&app, &state.windows, kind, bounds, source)
    })
    .await?
}

#[tauri::command]
pub fn close_overlay(app: AppHandle, state: State<'_, AppState>, kind: OverlayKind) {
    overlay::close_overlay(&app, &state.windows, kind);
}

#[tauri::command]
pub fn close_all_overlays(app: AppHandle, state: State<'_, AppState>) {
    overlay::close_all_overlays(&app, &state.windows);
}

#[tauri::command]
pub async fn select_area_rect(
    app: AppHandle,
    display_id: u32,
    rect: Rect,
) -> Result<ShellSettings, String> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        let mut settings = state.settings.lock().clone();
        settings.source = Some(CaptureSource::Area { display_id, rect });
        state.settings_store.save(&settings)?;
        *state.settings.lock() = settings.clone();
        overlay::close_overlay(&app, &state.windows, OverlayKind::AreaSelection);
        let _ = app.emit("nss://area-selected", &settings.source);
        let _ = app.emit("nss://settings-changed", &settings);
        Ok(settings)
    })
    .await?
}

// ----------------------------------------------------------------------------
// Editor & Preview commands
// ----------------------------------------------------------------------------

#[tauri::command]
pub async fn open_editor_project(
    app: AppHandle,
    reference: ProjectReference,
) -> Result<EditorOpenDto, String> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        let mut project = state.store.load(&reference).map_err(|e| e.to_string())?;
        let track = state
            .store
            .load_cursor_track(&reference, &project.assets)
            .unwrap_or_default();
        // If an older bundle still has dynamic auto-zoom enabled with no materialized blocks,
        // materialize them once so the timeline editor stays the single source of truth.
        if project.timeline.auto_zoom_enabled && project.timeline.zoom_blocks.is_empty() {
            project.timeline.zoom_blocks = ZoomPlanner::auto_zoom_blocks(
                &track,
                project.assets.duration,
                project.assets.source_pixel_size.aspect(),
                &[],
                &MotionTuning::default(),
            );
            project.timeline.auto_zoom_enabled = false;
        }
        let engine = PreviewEngine::open(reference.url.clone(), project.clone(), track.clone());
        let duration = engine.duration();
        let renderer_name = engine.renderer_name().to_owned();
        *state.preview.lock() = Some(engine);

        Ok(EditorOpenDto {
            reference,
            project,
            track,
            duration,
            renderer_name,
        })
    })
    .await?
}

#[tauri::command]
pub async fn update_editor_project(
    app: AppHandle,
    reference: ProjectReference,
    mut project: Project,
    immediate_save: Option<bool>,
) -> Result<(), String> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        project.touch();
        if let Some(ref mut engine) = *state.preview.lock() {
            engine.update_project(project.clone());
        }
        if immediate_save.unwrap_or(false) {
            state
                .store
                .save(&project, &reference)
                .map_err(|e| e.to_string())
        } else {
            state.store.schedule_autosave(project, &reference);
            Ok(())
        }
    })
    .await?
}

#[tauri::command]
pub async fn plan_auto_zooms(
    app: AppHandle,
    existing: Vec<ZoomBlock>,
) -> Result<Vec<ZoomBlock>, String> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        let guard = state.preview.lock();
        let engine = guard
            .as_ref()
            .ok_or_else(|| "No project open in editor.".to_owned())?;
        Ok(ZoomPlanner::auto_zoom_blocks(
            &engine.track,
            engine.duration(),
            engine.project.assets.source_pixel_size.aspect(),
            &existing,
            &MotionTuning::default(),
        ))
    })
    .await?
}

/// Renders one composited preview frame on the GPU using the exact same `nss-motion` + `nss-render`
/// path as `Mp4Exporter`. Returns raw `[u32_le width][u32_le height][rgba8...]` bytes directly via
/// `tauri::ipc::Response` (zero JSON, zero base64).
#[tauri::command]
pub async fn render_preview_frame(
    app: AppHandle,
    time: f64,
    max_width: u32,
    max_height: u32,
) -> Result<IpcResponse, String> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        let mut guard = state.preview.lock();
        let engine = guard
            .as_mut()
            .ok_or_else(|| "No project open in editor.".to_owned())?;
        let (w, h, rgba) = engine
            .render_frame(time, max_width, max_height)
            .map_err(|e| e.to_string())?;
        Ok(pack_rgba_response(w, h, rgba))
    })
    .await?
}

/// Returns the exact `RenderLayout` geometry at `time` so the editor canvas can project manual
/// zoom focus clicks and reticles between canvas space and normalized source space.
#[tauri::command]
pub async fn get_preview_layout(
    app: AppHandle,
    time: f64,
    max_width: u32,
    max_height: u32,
) -> Result<PreviewLayoutDto, String> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        let guard = state.preview.lock();
        let engine = guard
            .as_ref()
            .ok_or_else(|| "No project open in editor.".to_owned())?;
        let l = engine.layout_at(time, max_width, max_height);
        Ok(PreviewLayoutDto {
            width: l.output_size.width,
            height: l.output_size.height,
            content_x: l.content_rect.origin.x,
            content_y: l.content_rect.origin.y,
            content_width: l.content_rect.size.width,
            content_height: l.content_rect.size.height,
            camera_scale: l.camera.scale,
            camera_center_x: l.camera.center.x,
            camera_center_y: l.camera.center.y,
        })
    })
    .await?
}

#[tauri::command]
pub async fn get_timeline_thumbnails(
    app: AppHandle,
    count: usize,
    thumb_width: u32,
    thumb_height: u32,
) -> Result<Vec<TimelineThumbTile>, String> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        let mut guard = state.preview.lock();
        let engine = guard
            .as_mut()
            .ok_or_else(|| "No project open in editor.".to_owned())?;
        Ok(engine
            .timeline_thumbnails(count, thumb_width, thumb_height)
            .into_iter()
            .map(|(time, width, height, rgba)| TimelineThumbTile {
                time,
                width,
                height,
                rgba,
            })
            .collect())
    })
    .await?
}

#[tauri::command]
pub fn list_background_presets() -> BackgroundPresetsDto {
    let solids = background_presets::colors()
        .into_iter()
        .map(|p| PresetEntry {
            id: p.id,
            name: p.name,
            background: p.background,
        })
        .collect();
    let mut gradients = Vec::new();
    let mut meshes = Vec::new();
    for p in background_presets::gradients() {
        let entry = PresetEntry {
            id: p.id,
            name: p.name,
            background: p.background,
        };
        if matches!(entry.background, Background::Mesh { .. }) {
            meshes.push(entry);
        } else {
            gradients.push(entry);
        }
    }
    BackgroundPresetsDto {
        solids,
        gradients,
        meshes,
    }
}

#[tauri::command]
pub async fn list_system_wallpapers() -> Result<Vec<WallpaperEntry>, String> {
    run_blocking(discover_system_wallpapers).await
}

#[tauri::command]
pub async fn render_background_preview(
    background: Background,
    width: u32,
    height: u32,
) -> Result<IpcResponse, String> {
    run_blocking(move || {
        let w = width.clamp(8, 256);
        let h = height.clamp(8, 256);
        let rgba = render_background_swatch(&background, w, h);
        pack_rgba_response(w, h, rgba)
    })
    .await
}

// ----------------------------------------------------------------------------
// Export commands
// ----------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportRequest {
    pub reference: ProjectReference,
    pub project: Project,
    pub preset: ExportPreset,
    pub output_path: Option<String>,
}

#[tauri::command]
pub async fn start_export(app: AppHandle, request: ExportRequest) -> Result<String, String> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        // Flush latest project changes before exporting.
        state
            .store
            .save(&request.project, &request.reference)
            .map_err(|e| e.to_string())?;
        let track = state
            .store
            .load_cursor_track(&request.reference, &request.project.assets)
            .unwrap_or_default();

        let output_path = match request.output_path {
            Some(p) if !p.trim().is_empty() => PathBuf::from(p),
            _ => {
                let dir = state
                    .settings
                    .lock()
                    .default_export_dir
                    .clone()
                    .unwrap_or_else(resolve_library_dir);
                let stem = nss_project::sanitize_name(&request.project.name);
                dir.join(format!("{stem}.mp4"))
            }
        };
        let out_str = output_path.to_string_lossy().into_owned();

        let token = CancelToken::new();
        let my_gen = state.export_gen.fetch_add(1, Ordering::AcqRel) + 1;
        {
            let mut slot = state.export_cancel.lock();
            if let Some(prev) = slot.replace(token.clone()) {
                prev.cancel();
            }
        }

        let bundle_dir = request.reference.url.clone();
        let project = request.project;
        let preset = request.preset;
        let app_bg = app.clone();

        thread::Builder::new()
            .name("nss-mp4-exporter".into())
            .spawn(move || {
                let emit = |prog: ExportProgress| {
                    let st = app_bg.state::<AppState>();
                    // Ignore late Cancelled/Failed events from a superseded export run.
                    if st.export_gen.load(Ordering::Acquire) == my_gen {
                        let _ = app_bg.emit("nss://export-progress", &prog);
                    }
                };
                match Renderer::new() {
                    Ok(gpu) => {
                        let _ = Mp4Exporter::new(gpu).export(
                            &project,
                            &bundle_dir,
                            &track,
                            preset,
                            &output_path,
                            &token,
                            emit,
                        );
                    }
                    Err(_) => {
                        let _ = Mp4Exporter::new(PassthroughRenderer::new()).export(
                            &project,
                            &bundle_dir,
                            &track,
                            preset,
                            &output_path,
                            &token,
                            emit,
                        );
                    }
                }
            })
            .map_err(|e| format!("Failed to start export worker: {e}"))?;

        Ok(out_str)
    })
    .await?
}

#[tauri::command]
pub fn cancel_export(state: State<'_, AppState>) {
    if let Some(token) = state.export_cancel.lock().as_ref() {
        token.cancel();
    }
}

// ----------------------------------------------------------------------------
// Internal helpers
// ----------------------------------------------------------------------------

fn fail_start(app: &AppHandle, state: &AppState, message: String) {
    {
        let mut rt = state.runtime.lock();
        rt.countdown_cancel.store(true, Ordering::SeqCst);
        rt.countdown_remaining = None;
        rt.phase = ShellPhase::Failed { message };
    }
    overlay::close_all_overlays(app, &state.windows);
    state.emit_state(app);
}

fn fail_save(app: &AppHandle, state: &AppState, message: String) {
    {
        let mut rt = state.runtime.lock();
        rt.phase = ShellPhase::Failed { message };
    }
    state.emit_state(app);
}

fn spawn_elapsed_ticker(app: AppHandle, generation: u64) {
    let _ = thread::Builder::new()
        .name("nss-tray-ticker".into())
        .spawn(move || loop {
            thread::sleep(Duration::from_secs(1));
            let st = app.state::<AppState>();
            let active_gen = st.runtime.lock().recording_gen;
            if active_gen != generation {
                break;
            }
            let snap = st.snapshot();
            if !snap.phase.is_recording() {
                break;
            }
            tray::update_tray(&app, &snap.phase, snap.paused, snap.elapsed_seconds);
            let _ = app.emit("nss://elapsed", snap.elapsed_seconds);
        });
}

fn timestamp_slug() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}
