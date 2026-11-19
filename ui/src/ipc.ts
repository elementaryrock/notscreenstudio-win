/**
 * Typed IPC bridge to `src-tauri` commands and events.
 * Preview frames and swatches arrive as raw binary `ArrayBuffer` RGBA packets (zero base64).
 */
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  Background,
  BackgroundPresetsDto,
  CaptureSource,
  CaptureSourceList,
  CoreInfo,
  DevicesList,
  EditorOpenDto,
  ExportPreset,
  ExportProgress,
  FfmpegStatusDto,
  OverlayKind,
  PermissionsDto,
  Project,
  ProjectReference,
  ProjectSummary,
  Rect,
  ShellSettings,
  ShellSnapshot,
  TimelineThumbTile,
  WallpaperEntry,
  ZoomBlock,
} from "./types";

export interface DecodedRgbaPacket {
  width: number;
  height: number;
  pixels: Uint8ClampedArray<ArrayBuffer>;
}

/** Unpacks `[u32_le width][u32_le height][rgba8...]` returned by raw `tauri::ipc::Response`. */
export function decodeRgbaPacket(buf: ArrayBuffer): DecodedRgbaPacket | null {
  if (buf.byteLength < 8) return null;
  const view = new DataView(buf);
  const width = view.getUint32(0, true);
  const height = view.getUint32(4, true);
  const expected = width * height * 4;
  if (width === 0 || height === 0 || buf.byteLength < 8 + expected) return null;
  const pixels = new Uint8ClampedArray(buf, 8, expected);
  return { width, height, pixels };
}

export async function ping(message: string): Promise<string> {
  return invoke<string>("ping", { message });
}

export async function coreInfo(): Promise<CoreInfo> {
  return invoke<CoreInfo>("core_info");
}

export async function getShellSnapshot(): Promise<ShellSnapshot> {
  return invoke<ShellSnapshot>("get_shell_snapshot");
}

export async function getSettings(): Promise<ShellSettings> {
  return invoke<ShellSettings>("get_settings");
}

export async function saveSettings(settings: ShellSettings): Promise<ShellSettings> {
  return invoke<ShellSettings>("save_settings", { settings });
}

export async function listCaptureSources(): Promise<CaptureSourceList> {
  return invoke<CaptureSourceList>("list_capture_sources");
}

export async function listDevices(): Promise<DevicesList> {
  return invoke<DevicesList>("list_devices");
}

export async function getPermissions(): Promise<PermissionsDto> {
  return invoke<PermissionsDto>("get_permissions");
}

export async function requestBorderlessAccess(): Promise<PermissionsDto> {
  return invoke<PermissionsDto>("request_borderless_access");
}

export async function checkFfmpeg(): Promise<FfmpegStatusDto> {
  return invoke<FfmpegStatusDto>("check_ffmpeg");
}

// Library / Home
export async function listProjects(): Promise<ProjectSummary[]> {
  return invoke<ProjectSummary[]>("list_projects");
}

export async function getProjectThumbnail(path: string): Promise<ArrayBuffer> {
  return invoke<ArrayBuffer>("get_project_thumbnail", { path });
}

export async function renameProject(summary: ProjectSummary, newName: string): Promise<void> {
  return invoke<void>("rename_project", { summary, newName });
}

export async function deleteProject(summary: ProjectSummary): Promise<void> {
  return invoke<void>("delete_project", { summary });
}

export async function revealPath(path: string): Promise<void> {
  return invoke<void>("reveal_path", { path });
}

export async function createDemoProject(): Promise<ProjectReference> {
  return invoke<ProjectReference>("create_demo_project");
}

// Recording
export interface StartRecordingRequest {
  source: CaptureSource;
  micDeviceId?: string | null;
  cameraDeviceId?: string | null;
  captureSystemAudio?: boolean;
  countdownSeconds?: number;
}

export async function startRecording(request: StartRecordingRequest): Promise<ShellSnapshot> {
  return invoke<ShellSnapshot>("start_recording", { request });
}

export async function cancelCountdown(): Promise<ShellSnapshot> {
  return invoke<ShellSnapshot>("cancel_countdown");
}

export async function pauseRecording(): Promise<ShellSnapshot> {
  return invoke<ShellSnapshot>("pause_recording");
}

export async function resumeRecording(): Promise<ShellSnapshot> {
  return invoke<ShellSnapshot>("resume_recording");
}

export async function stopRecording(): Promise<ProjectReference> {
  return invoke<ProjectReference>("stop_recording");
}

export async function discardRecording(): Promise<ShellSnapshot> {
  return invoke<ShellSnapshot>("discard_recording");
}

export async function dismissError(): Promise<ShellSnapshot> {
  return invoke<ShellSnapshot>("dismiss_error");
}

// Overlays
export async function openOverlay(
  kind: OverlayKind,
  bounds?: [number, number, number, number],
): Promise<void> {
  return invoke<void>("open_overlay", { kind, bounds: bounds ?? null });
}

export async function closeOverlay(kind: OverlayKind): Promise<void> {
  return invoke<void>("close_overlay", { kind });
}

export async function closeAllOverlays(): Promise<void> {
  return invoke<void>("close_all_overlays");
}

export async function selectAreaRect(displayId: number, rect: Rect): Promise<ShellSettings> {
  return invoke<ShellSettings>("select_area_rect", { displayId, rect });
}

// Editor & Preview
export async function openEditorProject(reference: ProjectReference): Promise<EditorOpenDto> {
  return invoke<EditorOpenDto>("open_editor_project", { reference });
}

export async function updateEditorProject(
  reference: ProjectReference,
  project: Project,
  immediateSave = false,
): Promise<void> {
  return invoke<void>("update_editor_project", { reference, project, immediateSave });
}

export async function planAutoZooms(existing: ZoomBlock[]): Promise<ZoomBlock[]> {
  return invoke<ZoomBlock[]>("plan_auto_zooms", { existing });
}

export async function renderPreviewFrame(
  time: number,
  maxWidth: number,
  maxHeight: number,
): Promise<DecodedRgbaPacket | null> {
  const buf = await invoke<ArrayBuffer>("render_preview_frame", { time, maxWidth, maxHeight });
  return decodeRgbaPacket(buf);
}

export async function getTimelineThumbnails(
  count: number,
  thumbWidth: number,
  thumbHeight: number,
): Promise<TimelineThumbTile[]> {
  return invoke<TimelineThumbTile[]>("get_timeline_thumbnails", {
    count,
    thumbWidth,
    thumbHeight,
  });
}

export async function listBackgroundPresets(): Promise<BackgroundPresetsDto> {
  return invoke<BackgroundPresetsDto>("list_background_presets");
}

export async function listSystemWallpapers(): Promise<WallpaperEntry[]> {
  return invoke<WallpaperEntry[]>("list_system_wallpapers");
}

export async function renderBackgroundPreview(
  background: Background,
  width: number,
  height: number,
): Promise<DecodedRgbaPacket | null> {
  const buf = await invoke<ArrayBuffer>("render_background_preview", {
    background,
    width,
    height,
  });
  return decodeRgbaPacket(buf);
}

// Export
export async function startExport(
  reference: ProjectReference,
  project: Project,
  preset: ExportPreset,
  outputPath?: string,
): Promise<string> {
  return invoke<string>("start_export", {
    request: { reference, project, preset, outputPath: outputPath ?? null },
  });
}

export async function cancelExport(): Promise<void> {
  return invoke<void>("cancel_export");
}

// Events
export function onShellState(cb: (s: ShellSnapshot) => void): Promise<UnlistenFn> {
  return listen<ShellSnapshot>("nss://shell-state", (e) => cb(e.payload));
}

export function onExportProgress(cb: (p: ExportProgress) => void): Promise<UnlistenFn> {
  return listen<ExportProgress>("nss://export-progress", (e) => cb(e.payload));
}

export function onHotkey(cb: (action: string) => void): Promise<UnlistenFn> {
  return listen<string>("nss://hotkey", (e) => cb(e.payload));
}

export function onTrayAction(cb: (action: string) => void): Promise<UnlistenFn> {
  return listen<string>("nss://tray-action", (e) => cb(e.payload));
}

export function onRecordingSaved(cb: (ref: ProjectReference) => void): Promise<UnlistenFn> {
  return listen<ProjectReference>("nss://recording-saved", (e) => cb(e.payload));
}

export function onElapsed(cb: (secs: number) => void): Promise<UnlistenFn> {
  return listen<number>("nss://elapsed", (e) => cb(e.payload));
}

export function onAreaSelected(cb: (src: CaptureSource | null) => void): Promise<UnlistenFn> {
  return listen<CaptureSource | null>("nss://area-selected", (e) => cb(e.payload));
}
