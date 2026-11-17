# ADR 0003 — Preview frames never cross the JavaScript boundary

Status: accepted; transport mechanism to be confirmed by a Stage 4 spike

## Context

PLAN.md §6 risk 5. The editor preview must hold 60 fps while the UI also renders React
chrome. Serialising frames through the Tauri IPC bridge would either cap the frame rate or
burn a core on base64.

## Decision

- Rust renders the preview. The UI only sends **state** (playhead position, style edits,
  aspect ratio) and receives **events** (rendered frame number, duration, errors).
- Two candidate transports, decided by a Stage 4 spike:
  1. **Child window with its own wgpu surface** — best performance, needs manual layout
     synchronisation with the React chrome (and a hole in the WebView, or a sibling HWND).
  2. **Shared texture into WebView2** — simpler layout, depends on WebView2 texture
     interop stability on the target Windows versions.
- Whichever wins, the preview and the exporter call the **same** `FrameRenderer` with the
  same `FrameState`, which is the property that actually has to hold (PLAN.md §7.2).

## Consequences

- No `requestAnimationFrame`-driven frame pulling from JS anywhere in the design.
- The spike must measure both paths at 1080p and 4K before Stage 9 starts.
