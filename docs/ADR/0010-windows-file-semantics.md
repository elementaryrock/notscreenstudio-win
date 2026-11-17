# ADR 0010 — Windows file semantics in the project layer

Status: accepted (Stage 3)

## Context

`NSSProject` is the only module that touches the filesystem, and every line of it was written
against POSIX assumptions that do not hold on Windows. Four of them change the code, and one of
them changes the crate's `unsafe` policy:

1. **Atomic replace.** Upstream writes a temp file, `fsync`s it, then `rename(2)`s it over the
   destination. On POSIX that is the canonical crash-safe replace.
2. **Debounce.** Upstream keeps an ordered dictionary of pending saves and one cancellable `Task`
   per schedule, with `flush()` at quit.
3. **Delete.** Upstream uses `FileManager.trashItem`, the macOS Trash.
4. **Thumbnails.** Upstream uses `AVAssetImageGenerator` to pull a frame out of the recording.

## Decision

### 1. Atomic replace: `fs::rename` plus a retry window, no directory fsync

`std::fs::rename` on Windows already maps to `MoveFileExW(.., MOVEFILE_REPLACE_EXISTING)`, so it
replaces an existing file and no "destination exists?" branch is needed. Two things are still
different from POSIX:

- **Sharing violations are normal, not exceptional.** Explorer, Defender, the search indexer and
  our own thumbnail generator all open files briefly. A rename can fail with
  `ERROR_SHARING_VIOLATION` (32) / `ERROR_LOCK_VIOLATION` (33) and succeed 5 ms later. The swap is
  therefore retried with exponential backoff (2 ms → 100 ms) inside a 1.5 s budget. Beyond that
  the caller sees `ProjectError::Io` with `is_transient() == true`, so the UI can offer a retry
  rather than reporting data loss.
- **A directory cannot be `fsync`ed without an unsafe handle**, and Windows does not offer a
  portable equivalent (`FlushFileBuffers` on a directory handle requires `FILE_FLAG_BACKUP_SEMANTICS`
  and raw Win32). We `sync_all()` the temp file and then rename. The window a POSIX build would
  close with a directory fsync is covered instead by the `.bak` copy and the fallback in
  `FileProjectStore::load`, which is where a user notices it anyway.

`ReplaceFileW` was considered for the "atomic replace with backup" behaviour and rejected: it needs
`unsafe` for no gain here. We write the backup ourselves (from the previous, still-valid
`project.json`) before the swap, which also lets us skip a backup that no longer decodes.

### 2. Debounce: one worker thread with an interruptible clock

Upstream's per-schedule `Task` exists because Swift concurrency was already there. Rust has no
runtime in this workspace (ADR 0009), and the store's API is deliberately synchronous, so:

- `AutosaveDebouncer` owns **one** worker thread and a `HashMap<bundle, Pending>`.
- A `schedule` replaces the entry for that bundle, which is exactly what "restart the timer"
  means; the superseded save cannot write because its entry no longer exists.
- The worker parks in `DebounceClock::wait_until(deadline, epoch)` **outside** the state lock.
  The clock is a trait so tests advance virtual time instead of sleeping; the `epoch` counter is
  what makes the wait interruptible, so a new schedule or a shutdown returns immediately instead
  of waiting out the old deadline.
- `flush()` and `exclusive()` take the same `write_lock` the worker takes around a write, so an
  explicit save cannot interleave with an in-flight autosave. That is the property upstream's
  test `explicitSaveDuringInFlightAutosaveWriteLandsLast` pins down.

Cost of the change: nothing measurable. A 1 s debounce with one parked thread is cheaper than a
timer wheel, and the lock order (write_lock → state) is fixed to keep it deadlock-free.

### 3. Delete: `SHFileOperationW(FO_DELETE)` — the one `unsafe` block in this layer

`FileManager.trashItem` has no std equivalent. The options were:

- `IFileOperation` (COM): correct and long-path aware, but ~150 lines of interface plumbing plus
  COM initialisation, and we would still be writing `unsafe`.
- `SHFileOperationW` with `FOF_ALLOWUNDO`: two struct fields and one string, same Recycle Bin
  behaviour, no COM.
- The `trash` crate: another dependency in the offline build for the same FFI.

We take `SHFileOperationW`. Consequences, all deliberate:

- `nss-project` moves from `#![forbid(unsafe_code)]` to `#![deny(unsafe_code)]` with
  `#[allow(unsafe_code)]` on **one** function, plus a `#![deny]`-level audit rule in CI. Every
  other crate keeps `forbid`.
- `SHFileOperationW` is an ANSI-era API with `MAX_PATH` limits. Library bundles live under
  `%USERPROFILE%\Videos\notscreenstudio\<name>.nssproj`, far below that, and a failure returns an
  error instead of falling back to a permanent delete — we never silently destroy a recording.
- Tests inject a [`Trash`] implementation, exactly like upstream, so the normal suite never
  touches the real Recycle Bin. One Windows-only test does hit it, on a temp bundle, because an
  untested FFI call is the thing most likely to be wrong.

### 4. Thumbnails: packaged FFmpeg behind a trait

`AVAssetImageGenerator` becomes `ffmpeg -ss <t> -i <video> -frames:v 1 -q:v 4` (ADR 0002 already
ships FFmpeg for export, so this adds no dependency). Differences that matter:

- Upstream asks for JPEG quality 0.8; `-q:v 4` is the mjpeg equivalent.
- Upstream allows ±0.5 s from the requested time; `-ss` before `-i` seeks to the nearest
  keyframe, which is the same tolerance in practice.
- FFmpeg writes to a temp file which we then move in atomically, so a killed thumbnail never
  leaves a half JPEG that the home screen would render.

The `Thumbnailer` trait keeps the suite FFmpeg-free (no encoder needed on CI) and lets the
renderer's own rasteriser be substituted later.

### 5. Names: Windows-only sanitisation

`sanitize` gains rules that macOS does not need, because otherwise `create_project` can fail for a
name the user typed:

- reserved device names (`CON`, `PRN`, `AUX`, `NUL`, `COM1..9`, `LPT1..9`) are prefixed with `_`,
  with or without an extension;
- trailing dots and spaces are stripped (Windows silently drops them, which would make two
  different names collide);
- `< > : " | ? *` and control characters become `-` (macOS only forbids `/` and `:`);
- the result is capped at 100 characters to leave room for the library path and `.nssproj` under
  the 260-character `MAX_PATH` that most Windows tools still assume.

Extensions are compared case-insensitively throughout, because Windows does: a bundle named
`Demo.NSSPROJ` is listed, and `path_key` folds case so two spellings share one autosave timer.

### 6. Library root and JSON formatting

The root is `%USERPROFILE%\Videos\notscreenstudio` (PLAN.md §5 Stage 3), read from the environment
rather than `SHGetKnownFolderPath`. That is a documented gap: a Videos folder redirected into
OneDrive keeps its real location, and Stage 8 replaces this with `SHGetKnownFolderPath(FOLDERID_Videos)`.

`project.json` is written pretty-printed with sorted keys (upstream's
`.prettyPrinted, .sortedKeys`), `cursor.json` compact. **Byte-identical output is not the
promise:** Apple's `JSONSerialization` writes `"key" : value` with a space before the colon.
The contract we do keep, and test, is that a file written by either build loads in the other —
that is what PLAN.md §7.3 needs from the Mac fixtures.

## Consequences

- A save interrupted at any point leaves a loadable bundle; `tests/crash_recovery.rs` walks the
  window rather than asserting it.
- The project layer is sync, testable without a runtime, and its only background thread is the
  autosave worker.
- One `unsafe` block exists, is 12 lines, and is the only place in the workspace where the Win32
  ABI is touched outside the capture/input crates.
- Two Windows quirks are now visible in the API: `ProjectError::is_transient()`, and
  `Thumbnailer`/`Trash` as injectable traits rather than direct calls.
