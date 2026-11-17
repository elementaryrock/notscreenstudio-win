Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

<#
.SYNOPSIS
  Builds the release MSI + NSIS installers and runs the non-interactive smoke test.

.DESCRIPTION
  Port of upstream's scripts/bundle.sh. Order matters: the smoke test runs the
  freshly built binary before we spend time producing installers, so a broken
  dependency graph fails in seconds instead of minutes.

  Environment:
    NSS_SKIP_SMOKE=1   skip the smoke test (e.g. a machine with no GUI session)
    NSS_BUNDLES        comma list of bundle targets, default "msi,nsis"
#>

$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

Write-Host "==> Building release binaries" -ForegroundColor Cyan
cargo build --release -p notscreenstudio-win-app
if ($LASTEXITCODE -ne 0) { throw "release build failed" }

$exe = Join-Path $root "target/release/notscreenstudio.exe"
if (-not (Test-Path $exe)) { throw "expected binary not found: $exe" }

if ($env:NSS_SKIP_SMOKE -ne "1") {
    Write-Host "==> Smoke test" -ForegroundColor Cyan
    & $exe --smoke-test
    if ($LASTEXITCODE -ne 0) { throw "smoke test failed" }
} else {
    Write-Host "==> Smoke test skipped (NSS_SKIP_SMOKE=1)" -ForegroundColor Yellow
}

Write-Host "==> Bundling installers" -ForegroundColor Cyan
$bundles = if ($env:NSS_BUNDLES) { $env:NSS_BUNDLES } else { "msi,nsis" }
cargo tauri build --bundles $bundles
if ($LASTEXITCODE -ne 0) { throw "bundle failed" }

Write-Host ""
Write-Host "Built:" -ForegroundColor Green
Get-ChildItem -Recurse -Path "target/release/bundle" -Include *.msi, *.exe |
    Where-Object { $_.FullName -notmatch "nsis\\" -or $_.Extension -eq ".exe" } |
    Select-Object -ExpandProperty FullName
