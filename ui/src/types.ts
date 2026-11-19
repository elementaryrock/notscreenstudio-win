/**
 * Strict TypeScript mirror of `nss-core` and `src-tauri` serde wire formats.
 * Persisted project JSON uses Swift-compatible camelCase keys and external enum tagging.
 */

export interface CoreInfo {
  version: string;
  platform: string;
  stage: number;
  libraryDir: string;
  gpuAvailable: boolean;
}

export interface NormalizedPoint {
  x: number;
  y: number;
}

export interface PixelSize {
  width: number;
  height: number;
}

export interface Rect {
  origin: { x: number; y: number };
  size: { width: number; height: number };
}

export interface RGBAColor {
  r: number;
  g: number;
  b: number;
  a: number;
}

export interface GradientStop {
  location: number;
  color: RGBAColor;
}

export interface MeshPoint {
  x: number;
  y: number;
  color: RGBAColor;
}

export type Background =
  | { solid: RGBAColor }
  | { gradient: { stops: GradientStop[]; angleDegrees: number } }
  | { image: { file: string } }
  | { mesh: { points: MeshPoint[]; warp: number } }
  | { wallpaper: { name: string } }
  | { systemWallpaper: { path: string } };

export interface Shadow {
  radius: number;
  opacity: number;
  offsetX: number;
  offsetY: number;
}

export type AspectRatio = "r16x9" | "r1x1" | "r9x16";
export type ExportResolution = "p1080" | "p2160";
export type VideoCodec = "h264" | "hevc";
export type CameraShape = "circle" | "square" | "tall" | "wide";

export interface CursorStyle {
  size: number;
  smoothing: number;
  motionBlur: number;
  clickRipples: boolean;
  hideWhenIdle: boolean;
}

export interface CameraBorder {
  width: number;
  color: RGBAColor;
}

export interface CameraBubble {
  visible: boolean;
  shape: CameraShape;
  position: NormalizedPoint;
  size: number;
  border: CameraBorder | null;
  mirror: boolean;
  roundness: number;
  zoomedScale: number;
}

export interface Style {
  background: Background;
  backgroundBlur: number;
  padding: number;
  cornerRadius: number;
  shadow: Shadow;
  aspectRatio: AspectRatio;
  cursor: CursorStyle;
  camera: CameraBubble;
}

export type ZoomFocus = { auto: Record<string, never> } | { manual: NormalizedPoint };

export interface ZoomBlock {
  id: string;
  start: number;
  end: number;
  scale: number;
  focus: ZoomFocus;
  easeIn: number;
  easeOut: number;
  isAuto: boolean;
}

export interface TrimRange {
  start: number;
  end: number | null;
}

export interface Timeline {
  trim: TrimRange;
  zoomBlocks: ZoomBlock[];
  autoZoomEnabled: boolean;
}

export interface RecordingAssets {
  screenFile: string;
  cameraFile: string | null;
  micFile: string | null;
  cursorFile: string;
  sourcePixelSize: PixelSize;
  frameRate: number;
  duration: number;
  startHostTime: number;
  thumbnailFile: string | null;
}

export interface Project {
  schemaVersion: number;
  id: string;
  name: string;
  createdAt: number;
  modifiedAt: number;
  assets: RecordingAssets;
  timeline: Timeline;
  style: Style;
}

export interface ProjectReference {
  url: string;
}

export interface ProjectSummary {
  id: string;
  name: string;
  createdAt: number;
  duration: number;
  bundle: ProjectReference;
  thumbnailURL: string | null;
}

export interface CursorSample {
  t: number;
  x: number;
  y: number;
}

export interface ClickEvent {
  t: number;
  x: number;
  y: number;
  button: "left" | "right" | "other";
  kind: "down" | "up";
}

export interface CursorShapeChange {
  t: number;
  shape: string;
}

export interface CursorTrack {
  samples: CursorSample[];
  clicks: ClickEvent[];
  sampleRate: number;
  shapes: CursorShapeChange[];
}

export type CaptureSource =
  | { display: { id: number } }
  | { window: { id: number } }
  | { area: { displayID: number; rect: Rect } };

export interface CaptureDisplayInfo {
  id: number;
  name: string;
  frame: Rect;
  pixelSize: PixelSize;
  scale: number;
  isMain: boolean;
}

export interface CaptureWindowInfo {
  id: number;
  hwnd: number;
  title: string;
  appName: string;
  processId: number;
  frame: Rect;
  displayId: number | null;
}

export interface CaptureSourceList {
  displays: CaptureDisplayInfo[];
  windows: CaptureWindowInfo[];
}

export interface CaptureDeviceInfo {
  id: string;
  name: string;
  isDefault: boolean;
}

export interface DevicesList {
  microphones: CaptureDeviceInfo[];
  cameras: CaptureDeviceInfo[];
}

export type ShellPhase =
  | { kind: "idle" }
  | { kind: "preparing" }
  | { kind: "recording"; startedAt: number }
  | { kind: "saving" }
  | { kind: "editing"; reference: ProjectReference }
  | { kind: "failed"; message: string };

export interface HotKeyRegistrationStatus {
  recordRegistered: boolean;
  recordConflict: string | null;
  pauseRegistered: boolean;
  pauseConflict: string | null;
}

export interface ShellSnapshot {
  phase: ShellPhase;
  paused: boolean;
  elapsedSeconds: number;
  countdownRemaining: number | null;
  hotkeys: HotKeyRegistrationStatus;
  borderlessNote: string;
}

export interface ShellSettings {
  source: CaptureSource | null;
  micEnabled: boolean;
  micDeviceId: string | null;
  captureSystemAudio: boolean;
  cameraEnabled: boolean;
  cameraDeviceId: string | null;
  cameraShape: CameraShape;
  bubbleSize: number;
  bubbleCenter: NormalizedPoint;
  bubbleRoundness: number;
  countdownSeconds: number;
  recordHotkey: string;
  pauseHotkey: string;
  defaultAspect: AspectRatio;
  defaultExportDir: string | null;
  recentBackgroundImages: string[];
  inspectorSections: Record<string, boolean>;
}

export interface PermissionsDto {
  screenRecording: "granted" | "denied" | "notDetermined";
  microphone: "granted" | "denied" | "notDetermined";
  camera: "granted" | "denied" | "notDetermined";
  borderlessCapture: "granted" | "denied" | "notDetermined";
  borderlessNote: string;
}

export interface FfmpegStatusDto {
  available: boolean;
  path: string | null;
  testedVersion: string;
  downloadUrl: string;
  h264: boolean;
  hevc: boolean;
  message: string | null;
}

export interface EditorOpenDto {
  reference: ProjectReference;
  project: Project;
  track: CursorTrack;
  duration: number;
  rendererName: string;
}

export interface PreviewLayoutDto {
  width: number;
  height: number;
  contentX: number;
  contentY: number;
  contentWidth: number;
  contentHeight: number;
  cameraScale: number;
  cameraCenterX: number;
  cameraCenterY: number;
}

export interface PresetEntry {
  id: string;
  name: string;
  background: Background;
}

export interface BackgroundPresetsDto {
  solids: PresetEntry[];
  gradients: PresetEntry[];
  meshes: PresetEntry[];
}

export interface WallpaperEntry {
  name: string;
  path: string;
}

export interface TimelineThumbTile {
  time: number;
  width: number;
  height: number;
  rgba: number[];
}

export interface ExportPreset {
  resolution: ExportResolution;
  fps: number;
  codec: VideoCodec;
}

export type ExportProgress =
  | { idle: Record<string, never> }
  | { running: { fraction: number; framesWritten: number; totalFrames: number } }
  | { finished: string }
  | { failed: { message: string } }
  | { cancelled: Record<string, never> };

export type OverlayKind =
  | "setupBar"
  | "recordingBar"
  | "countdown"
  | "areaSelection"
  | "cameraBubble";

export function aspectLabel(aspect: AspectRatio): string {
  if (aspect === "r16x9") return "16:9";
  if (aspect === "r9x16") return "9:16";
  return "1:1";
}

export function resolutionLabel(resolution: ExportResolution): string {
  return resolution === "p2160" ? "4K" : "1080p";
}

export function outputPixelSize(resolution: ExportResolution, aspect: AspectRatio): PixelSize {
  const shortSide = resolution === "p2160" ? 2160 : 1080;
  if (aspect === "r16x9") return { width: Math.round((shortSide * 16) / 9), height: shortSide };
  if (aspect === "r9x16") return { width: shortSide, height: Math.round((shortSide * 16) / 9) };
  return { width: shortSide, height: shortSide };
}

export function rgbaToHex(c: RGBAColor): string {
  const ch = (v: number): string =>
    Math.max(0, Math.min(255, Math.round(v * 255)))
      .toString(16)
      .padStart(2, "0");
  return `#${ch(c.r)}${ch(c.g)}${ch(c.b)}`;
}

export function hexToRgba(hex: string, alpha = 1): RGBAColor {
  const clean = hex.replace(/^#/, "");
  if (clean.length !== 6) return { r: 0.1, g: 0.1, b: 0.12, a: alpha };
  const r = parseInt(clean.slice(0, 2), 16) / 255;
  const g = parseInt(clean.slice(2, 4), 16) / 255;
  const b = parseInt(clean.slice(4, 6), 16) / 255;
  return { r, g, b, a: alpha };
}
