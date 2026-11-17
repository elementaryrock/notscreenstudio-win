# Contributing worker rules

Ported from upstream `AGENTS.md`. These are hard rules, not preferences. Every stage in
`PLAN.md` §5 is written to respect them.

## Language and platform

- Rust 2021, pinned to the MSVC toolchain by `rust-toolchain.toml`. Windows 10 2004+ is the
  floor (ADR 0004).
- UI is React 19 + TypeScript (strict, `noUncheckedIndexedAccess`, `exactOptionalPropertyTypes`).
- No third-party dependency without an ADR in `docs/ADR/`.

## Toolchain

- No build step that needs a GUI session. `--smoke-test` must always work headless.
- Shaders live in `shaders/*.wgsl` and are embedded with `include_str!`. Never generated at
  runtime.
- FFmpeg is the only external binary (ADR 0002).
- **PowerShell scripts stay ASCII-only.** Windows PowerShell 5.1 reads a BOM-less `.ps1` as
  ANSI, so a stray em dash corrupts the parse. Use `-` and `->`, not typographic characters.

## Crate boundaries

- `nss-core` and `nss-motion` are **platform-neutral**. Adding `windows`, `wgpu`, `tauri` or
  any I/O backend to them is a review rejection. Their tests must pass on any host.
- `nss-capture`, `nss-input` are the only crates allowed to call Win32/WinRT input and
  capture APIs.
- `nss-render` owns the GPU device and the shaders. Nothing else creates a `wgpu::Device`.
- `nss-project` owns the filesystem layout of a project bundle. Nothing else writes
  `project.json` or `cursor.json`.
- `src-tauri` owns windows, tray, hotkeys and IPC commands. It must not contain motion or
  rendering maths.

## Contracts

- Times are `f64` seconds from the first screen frame (`MediaTime`).
- Positions are normalised `0...1`, origin top-left.
- Motion is a pure function of time: deterministic, no wall-clock state, no call-order
  dependence. Preview and export share one implementation.
- Every persisted field decodes with a default, so an older `project.json` still loads.
  On-disk JSON key names match upstream exactly.

## Tests

- Rust's built-in test harness. One module per ported Swift test file, named the same, so
  coverage can be diffed against upstream.
- Tests needing a GPU, capture device, camera or microphone must skip cleanly when
  unavailable (`#[ignore]` with a `gpu` / `device` prefix, or an early return logged once).
  CI stays green without hardware.
- Property tests (`proptest`) for the invariants listed in `PLAN.md` Appendix B.

## Performance

- The preview holds 60 fps at 1080p; 4K export under the frame budget in the benchmark.
- No per-frame allocation in the render path. No `Vec` growth inside the frame loop.
- Keep the UI thread free: rendering, capture, encoding and decoding all run off it.

## Privacy

- Nothing leaves the machine. No networking APIs, analytics, crash reporters or update
  pingers. CI has a job that fails the build if a network crate is added to our manifests.

## Pull requests

- Every PR passes `./scripts/test.ps1` locally before pushing; CI runs the same steps.
- Keep PRs focused. State which stage of `PLAN.md` the PR advances and what was tested.
- Changes to `nss-core` must be small and additive, and listed under "Contract changes" in
  the description.
