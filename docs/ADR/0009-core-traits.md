# ADR 0009 — Synchronous core traits, and the one place async is allowed

Status: accepted (Stage 1; kept for the rest of the port)

## Context

Upstream `NSSCore/Protocols.swift` declares most contracts `async`: `MotionModel.frameState`,
`ProjectStore.save`, `RecordingSession.start`, `InputLogger.stop`, `ProjectLibrary.summaries`.
That is idiomatic for Swift actors and `await`-friendly call sites.

A mechanical translation would make the same Rust traits `async`. That has two costs:

1. `nss-core` must pick an async runtime (`tokio`/`async-std`), and every crate that touches
   it inherits that choice. `nss-core` is meant to stay a dependency-free model crate, and the
   hard rule in PLAN.md §3 forbids I/O backends there.
2. The motion model is a **pure function of time**. Framing it as a future hides that and
   invites `block_on` at the call site (the exporter writes 60-120 frames per second of
   output), where a wrong runtime or a nested `block_on` is a panic.

On Windows the async work is not in the *contract* anyway, it is in the *implementation*:

- Screen capture is a WinRT/D3D11 object that must be created and driven on an STA thread
  with a message loop.
- Microphone/camera capture is Media Foundation, also message-loop bound.
- Encoding is an FFmpeg child process fed by a pipe.

All three want their own thread that owns the object, with a channel in front of it. That is a
synchronous API from the caller's point of view.

## Decision

1. **`nss-core` traits are synchronous.** `MotionModel`, `ProjectStore`, `RecordingSession`,
   `InputLogger`, `ProjectLibrary` take `&self`/`&mut self` and return `Result`.
2. **Implementations own their concurrency.** A capture session spawns its own thread and
   talks to it over a channel; `ProjectStore` methods may block on file I/O; a library may
   hold a `Mutex`. Where upstream needed an actor for isolation, the Rust impl uses interior
   mutability + a thread of its own.
3. **The UI boundary is the only async edge.** `src-tauri` commands are `async` because Tauri
   requires it; they call the synchronous core and, for long jobs (export), run it on a worker
   thread and stream progress over events.
4. `FrameSource`/`FrameRenderer` are **not** defined in `nss-core`. They exist for
   `CVPixelBuffer` upstream; in this port they are byte-plane based and live in `nss-render`
   (Stage 4). `nss-core` stays buildable on any host, which keeps its tests host-agnostic.

## Consequences

- No runtime in the dependency tree of `nss-core`/`nss-motion`; their tests are plain `cargo test`.
- Cancellation is explicit (a flag or a dropped channel), not a dropped future. Upstream's
  `cancel()` becomes `RecordingSession::cancel` / an export cancel token.
- Callers that already are async may block the calling thread briefly. That is acceptable
  because every blocking call is either a fast model query or already running on a worker:
  the render loop never calls a blocking store method per frame.
- Two related format decisions, made in the same stage, for the same "no surprise drift"
  reason:
  - `serde_json` is built with `float_roundtrip`. A property test showed a stored `f64`
    could otherwise return 1 ULP off, so `save` → `load` did not guarantee equality.
  - `UUID`s serialize **uppercase** and `Date` as seconds since the CoreFoundation reference
    date (2001-01-01Z), because that is what Swift's `Codable` emits. Both are pinned by
    `crates/nss-core/tests/compat.rs`.
