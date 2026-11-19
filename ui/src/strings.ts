/**
 * Centralized UI strings (PLAN.md §7.7).
 * Every visible label, button, tooltip, and status message lives here.
 */
export const strings = {
  appName: "notscreenstudio",
  tagline: "Screen recorder with auto-zoom and GPU compositing for Windows.",

  // Home / Library
  recordings: "Recordings",
  newRecording: "New Recording",
  sampleProject: "Sample Project",
  settingsAndPermissions: "Settings",
  openLibraryFolder: "Library Folder",
  emptyLibraryTitle: "No recordings yet",
  emptyLibraryBody:
    "Capture a display, window or dragged area with 120 Hz cursor tracking and automatic click zooms — everything stays offline on your PC.",
  rename: "Rename",
  showInExplorer: "Show in Explorer",
  deleteToRecycleBin: "Move to Recycle Bin",
  confirmDelete: "Move this recording to the Recycle Bin?",

  // Setup / Recording
  setupTitle: "New Recording",
  sourceDisplay: "Display",
  sourceWindow: "Window",
  sourceArea: "Area",
  selectAreaPrompt: "Drag anywhere on screen to select an area (Esc to cancel)",
  chooseArea: "Select Area on Screen",
  microphone: "Microphone",
  camera: "Camera",
  systemAudio: "System audio",
  countdown: "Countdown",
  countdownOff: "Off",
  record: "Record",
  popOutBar: "Floating Bar",
  recording: "Recording",
  paused: "Paused",
  pause: "Pause",
  resume: "Resume",
  stop: "Stop",
  discard: "Discard",
  cancel: "Cancel",
  close: "Close",
  done: "Done",
  save: "Save",

  // Editor Top Bar
  backToRecordings: "Recordings",
  undo: "Undo (Ctrl+Z)",
  redo: "Redo (Ctrl+Shift+Z)",
  export: "Export",

  // Inspector Sections
  sectionBackground: "Background",
  sectionFrame: "Frame",
  sectionCursor: "Cursor",
  sectionZoom: "Zoom",
  sectionCamera: "Camera",
  sectionAudio: "Audio",

  // Background Picker Tabs
  bgWallpapers: "Wallpapers",
  bgGradients: "Gradients",
  bgColors: "Colors",
  bgImage: "Image",
  bgBlur: "Blur",
  bgAngle: "Angle",
  bgCustomColor: "Custom color",
  bgImagePathPlaceholder: "Path to PNG or JPEG image...",
  bgApplyImage: "Use Image",
  bgRecentImages: "Recent Images",

  // Frame Controls
  padding: "Padding",
  cornerRadius: "Corner radius",
  shadowOpacity: "Shadow opacity",
  shadowBlur: "Shadow blur",
  shadowOffsetY: "Shadow offset",

  // Cursor Controls
  showCursor: "Show cursor",
  cursorSize: "Size",
  cursorSmoothing: "Smooth movement",
  motionBlur: "Motion blur",
  clickRipples: "Click ripples",
  hideWhenIdle: "Hide when idle",
  idleDelay: "Idle delay",

  // Zoom Controls
  addZoom: "Add Zoom",
  regenAutoZoom: "Regenerate Auto",
  clearAutoZoom: "Clear Auto",
  zoomLevel: "Zoom level",
  zoomFocus: "Focus",
  focusFollow: "Follow cursor",
  focusFixed: "Fixed point",
  clickPreviewToPick: "Click preview to place focus",
  easeIn: "Ease in",
  easeOut: "Ease out",
  deleteZoom: "Delete Zoom Block",
  noZoomSelected: "Select a zoom block on the timeline or click Add Zoom.",

  // Camera Controls
  showCamera: "Show camera bubble",
  cameraShape: "Shape",
  cameraRoundness: "Roundness",
  cameraSize: "Size",
  cameraBorder: "Border width",
  cameraMirror: "Mirror horizontally",
  cameraShrinkOnZoom: "Shrink while zoomed",
  cameraPositionX: "Position X",
  cameraPositionY: "Position Y",

  // Audio Controls
  audioMicTrack: "Microphone track",
  audioSystemTrack: "System audio",
  audioTrimmedNotice: "Audio is automatically trimmed and resampled to 48 kHz stereo AAC on export.",

  // Timeline Tracks
  trackVideo: "Video",
  trackZoom: "Zoom",
  trackCamera: "Camera",
  trackAudio: "Audio",

  // Export Sheet
  exportTitle: "Export MP4",
  resolution: "Resolution",
  frameRate: "Frame rate",
  codec: "Codec",
  destination: "Output path",
  startExport: "Export Video",
  exporting: "Exporting...",
  exportComplete: "Export complete",
  exportCancelled: "Export cancelled",
  ffmpegMissingTitle: "FFmpeg required for export",

  // Settings / Permissions
  settingsTitle: "Settings & Permissions",
  permissionsHeading: "Windows Permissions",
  hotkeysHeading: "Global Shortcuts",
  defaultsHeading: "Defaults",
  recordHotkeyLabel: "Record / Stop",
  pauseHotkeyLabel: "Pause / Resume",
  requestBorderless: "Request Borderless Capture",
  borderlessHint:
    "Windows shows a yellow border while recording — Windows 11 can turn this off.",
} as const;
