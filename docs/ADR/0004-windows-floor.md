# ADR 0004 — Minimum supported OS is Windows 10 2004 (build 19041)

Status: accepted (Stage 0)

## Context

Windows Graphics Capture exists from 1803, but the APIs we rely on land later:
`GraphicsCaptureSession.IsBorderRequired` (2004/20348+), `GraphicsCaptureAccess`
consent, and reliable `Direct3D11CaptureFramePool` resize behaviour. WDA_EXCLUDEFROMCAPTURE
(used for own-window exclusion) requires 2004.

## Decision

- Floor: **Windows 10 version 2004, build 19041**.
- Every newer API is probed at runtime (`ApiInformation`-style check or a
  `GetProcAddress`/interface query), never assumed.
- On Windows 10 the yellow border cannot be disabled; the app says so once, in Settings.

## Consequences

- Windows 10 users get the full feature set minus borderless capture.
- No code may call a 22H2+ API without a feature check. Reviewers should reject otherwise.
