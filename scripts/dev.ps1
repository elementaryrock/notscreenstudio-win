Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

<#
.SYNOPSIS
  Runs the app with hot reload (Vite for the UI, cargo for the Rust core).

.DESCRIPTION
  Equivalent of `scripts/bundle.sh` + `open` from upstream, but as a dev loop.
  The UI dev server is started by Tauri itself via beforeDevCommand.
#>

$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

if (-not (Test-Path "ui/node_modules")) {
    Write-Host "Installing UI dependencies..." -ForegroundColor Cyan
    npm --prefix ui install
    if ($LASTEXITCODE -ne 0) { throw "npm install failed" }
}

Write-Host "Starting notscreenstudio (dev)..." -ForegroundColor Cyan
cargo tauri dev
