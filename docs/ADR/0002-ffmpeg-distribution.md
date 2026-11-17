# ADR 0002 — FFmpeg is a bundled external binary, with PATH detection as fallback

Status: accepted for v1 (revisit at Stage 10 packaging)

## Context

Windows has no AVAssetWriter equivalent that is both simple and licence-clean for
H.264/HEVC + AAC muxing. The existing machine already has FFmpeg 8.1.1.

## Decision

1. Look for FFmpeg next to the executable (`ffmpeg.exe` in the app directory).
2. Fall back to `ffmpeg` on `PATH`.
3. If neither exists, show a blocking dialog that explains the dependency, shows the exact
   version we tested against, and links to a download — never a silent failure.

Ship a pinned LGPL-compatible build in the installer at Stage 10 if redistribution terms
are confirmed; until then the installer documents the external dependency.

## Consequences

- Installer size stays small in v1.
- First-run can be confusing without a clear dialog, so the dialog is part of the
  acceptance criteria for Stage 7, not a nice-to-have.
- Hardware encoders (`h264_nvenc`, `h264_qsv`, `h264_amf`) are used only when the FFmpeg
  build reports them, and always with a software fallback.
