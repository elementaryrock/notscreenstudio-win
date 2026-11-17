# Coding rules for all workers

## Language and platform
- Swift 6 language mode with strict concurrency. SwiftUI for UI. macOS 15 minimum.
- No third-party dependencies without approval.

## Toolchain (no Xcode on the dev machine)
- No `.metal` files, no `xcodebuild`, no asset catalogs that need actool.
- Metal shaders are Swift string sources compiled at runtime with `MTLDevice.makeLibrary(source:options:)`, compiled once and cached.

## Tests
- Swift Testing only (`import Testing`, `@Test`, `#expect`). Never XCTest.
- Tests that need a GPU, screen-recording permission, a camera or a mic must skip cleanly when unavailable (for example `.enabled(if:)`), so CI stays green.

## Ownership
- Each worker edits only its own target folder(s) and its own test target.
- Changes to `NSSCore` must be small and additive only, and listed under "Contract changes" in the PR description.
- Do not edit another worker's folder. Ask the orchestrator instead.

## Conventions
- Times are `Double` seconds from recording start (`MediaTime`).
- Positions are normalized 0...1, origin top-left.
- Motion is a pure function of time (deterministic, no wall-clock state) so preview and export match.

## Performance
- The preview must hold 60 fps.
- No per-frame allocation of textures or buffers in hot paths.
- Keep the main thread free.

## Privacy
- Nothing leaves the machine. No networking APIs, analytics or crash reporters.

## Pull requests
- Every PR must pass `swift build` and `swift test` locally before pushing. CI runs the same on every PR.
- Keep PRs focused. Describe what changed and how it was tested.
