# ADR 0001 — Tauri 2 + Rust core + React UI, wgpu (WGSL) compositor

Status: accepted (owner decision, Stage 0)

## Context

Upstream is Swift 6 + SwiftUI + Metal, macOS 15 only. Windows has no Swift toolchain
installed, and even with one there is no AppKit, SwiftUI, ScreenCaptureKit, Metal or
AVFoundation — so every platform-facing module is a rewrite regardless of language.

## Decision

- **Shell:** Tauri 2 (WebView2), not Electron.
- **Core:** Rust crates `nss-core`, `nss-motion`, `nss-project` (platform-neutral) plus
  `nss-capture`, `nss-input`, `nss-render`, `nss-export` (Windows-heavy).
- **Compositor:** wgpu on D3D12, shaders in WGSL.
- **UI:** React 19 + TypeScript + Vite.

## Consequences

- The 274-line MSL shader translates to WGSL nearly mechanically, and one D3D11 capture
  texture feeds one wgpu device end to end.
- Win32/WinRT APIs are reachable without an N-API addon, which matters at 120 Hz.
- We own more Rust plumbing than an Electron port would need, and UI iteration is slower.
- Rejected: Electron (heavier, addon boundary on the hot path), .NET (furthest from the
  original, riskiest 60 fps preview), Swift-on-Windows (no UI/render/capture frameworks).
