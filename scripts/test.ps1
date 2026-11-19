param([switch]$Fast)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

<#
.SYNOPSIS
  The full local gate: formatting, lints, Rust tests, UI typecheck, smoke test.

.DESCRIPTION
  This is exactly what CI runs (see .github/workflows/ci.yml). If it passes here
  it passes there — keep the two in sync.

  Usage:
    ./scripts/test.ps1            # everything
    ./scripts/test.ps1 -Fast      # skip the release build and the UI build
#>

$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

function Step($name, $block) {
    Write-Host "==> $name" -ForegroundColor Cyan
    & $block
    if ($LASTEXITCODE -ne 0) { throw "$name failed" }
}
Step "cargo fmt --check" { cargo fmt --all -- --check }
Step "cargo clippy" { cargo clippy --workspace --all-targets -- -D warnings }
Step "cargo test" { cargo test --workspace }

if (-not (Test-Path "ui/node_modules")) {
    Write-Host "==> npm install" -ForegroundColor Cyan
    npm --prefix ui install
    if ($LASTEXITCODE -ne 0) { throw "npm install failed" }
}

Step "ui typecheck" { npm --prefix ui run typecheck }
Step "ui test" { npm --prefix ui run test }

if (-not $Fast) {
    Step "ui build" { npm --prefix ui run build }
    Step "app smoke test" { cargo run -p notscreenstudio-win-app --bin notscreenstudio -- --smoke-test }
}

Write-Host ""
Write-Host "All checks passed." -ForegroundColor Green
