# ADR 0007 — Capture consent, the yellow border, and excluding our own overlays

Status: accepted (Stage 0; implemented in Stage 5)

## Context

macOS ScreenCaptureKit lets an app enumerate windows and capture silently, and exclude its
own windows via a content filter. Windows Graphics Capture does neither by default.

## Decision

1. **Source picker.** Build our own window/display list (`EnumWindows`,
   `DwmGetWindowAttribute(DWMWA_EXTENDED_FRAME_BOUNDS)`, `EnumDisplayMonitors`) so the UX
   matches upstream ("pick a window, one click"), then create the item with
   `GraphicsCaptureItem::TryCreateFromWindowId`. If the system refuses, fall back to the
   system `GraphicsCapturePicker` and remember which path was used.
2. **Yellow border.** Ask for `GraphicsCaptureAccessKind::Borderless` consent once, explain
   it in one line, and degrade silently to the border when refused or unsupported. Never
   attempt `SetWindowDisplayAffinity`-style tricks to hide it.
3. **Own-window exclusion.** Apply `WDA_EXCLUDEFROMCAPTURE` to every HUD, overlay, countdown
   and the WebView2 window. Verify with a self-test that captures a frame while an overlay is
   on screen and asserts the overlay pixels are absent.

## Consequences

- Capture is never silent: Windows shows a border unless the user consents. This is a
  documented product difference from macOS, stated in the README.
- Exclusion is per-HWND, so every new window class must be added to the exclusion list; the
  list lives in one place with a test that fails when a window is created without it.
