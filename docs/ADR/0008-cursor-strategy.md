# ADR 0008 — Capture without the system cursor; draw our own from the log

Status: accepted (Stage 0; implemented in Stages 5-6)

## Context

WGC can burn the real cursor into the capture. That is the one thing we must not do: a baked
cursor cannot be smoothed, motion-blurred, squished on press, faded when idle, or replaced
with a shape the atlas knows better.

## Decision

1. Capture with `SetIsCursorCaptureEnabled(false)`.
2. Log position at 120 Hz (`GetCursorPos` on a dedicated thread) and clicks via
   `WH_MOUSE_LL`, stamped on the same clock as the first screen frame.
3. Detect shape at 30 Hz, in this order:
   - handle match against `LoadCursorW(IDC_*)` for the standard cursors;
   - bitmap hash of `GetIconInfoEx` colour + mask compared with a committed fingerprint table;
   - fall back to `arrow` and record the unknown hash in a debug log so the table can grow.
4. Draw the cursor from a generated atlas (`rgba16Float`, full mip chain, CPU-chosen mip
   level), exactly as upstream `CursorAtlas` does.

## Consequences

- Cursor appearance is fully ours, which is what makes the Screen-Studio look possible.
- Some third-party app cursors will render as an arrow until their hash is added. This is a
  cosmetic limitation, never a crash, and matches upstream's `.unknown` behaviour.
- A low-level mouse hook cannot observe input sent to elevated apps or the secure desktop;
  documented in the README.
