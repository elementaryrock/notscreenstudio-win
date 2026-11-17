/**
 * All user-facing copy lives here (PLAN.md §7.7) so i18n is a translation pass,
 * not an archaeology exercise. Nothing user-visible should be hardcoded in a component.
 */
export const strings = {
  appName: "notscreenstudio",
  tagline: "Record your screen. Zoom in on what matters.",
  bootstrapTitle: "Port bootstrap",
  bootstrapBody:
    "The Rust core, the IPC bridge and the UI shell are wired. Capture, the compositor and the editor arrive in stages 4 through 9 (PLAN.md).",
  stageLabel: "Stage",
  platformLabel: "Platform",
  versionLabel: "Core version",
  ipcOk: "IPC round-trip OK",
  ipcPending: "checking IPC…",
  ipcFailed: "IPC failed",
} as const;
