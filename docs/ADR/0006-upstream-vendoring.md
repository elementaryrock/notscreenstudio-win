# ADR 0006 — Upstream is vendored read-only at `_upstream/`

Status: accepted (Stage 0)

## Context

The port needs to reference the original Swift at each stage: exact JSON key names, motion
maths, shader expressions and 499 test expectations.

## Decision

- Commit `_upstream/` as a read-only snapshot (pinned to the revision recorded in
  `_upstream/UPSTREAM_REVISION`), excluded from the Cargo workspace and from CI builds.
  The nested `.git` directory is removed so the snapshot is a plain file tree and cannot be
  added as a stray gitlink. Fetch the history separately if you need `git log` on upstream.
- Never edit files inside `_upstream/`. If the port needs a change that looks like it
  belongs upstream, write it in our crates and note the divergence in the stage's ADR.

## Consequences

- Offline reference, and diffs against upstream are always meaningful.
- Test fixtures generated from a Mac (golden frames, `project.json` samples) are copied
  into `fixtures/`, not read from `_upstream/`, so removing the vendor tree cannot break
  the test suite.
