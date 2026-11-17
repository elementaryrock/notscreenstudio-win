# ADR 0005 — H.264 by default; HEVC and hardware encoders are opt-in

Status: accepted for v1

## Context

Upstream exports H.264 or HEVC at 60 fps, 1080p or 4K, from AVFoundation.

## Decision

- **Default:** `libx264`, `-preset medium`, CRF-based with a bitrate ceiling, `+faststart`.
- **Optional:** `libx265` when present in the FFmpeg build.
- **Opportunistic:** `h264_nvenc` / `h264_qsv` / `h264_amf` when `ffmpeg -encoders` lists
  them, always with an automatic software retry on failure.
- Audio: AAC 192 kbps, 48 kHz.
- Delivery is SDR 8-bit in v1 (no HDR export); HDR *capture* is tone-mapped for preview.

## Consequences

- Encoder probing happens once per process and is cached.
- An export that fails with a hardware encoder must retry with software before reporting an
  error, or users will blame the app for their driver.
