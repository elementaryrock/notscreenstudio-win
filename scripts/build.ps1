param(
    [switch]$SkipSmoke,
    [switch]$SkipInstallers,
    [string]$Bundles = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

<#
.SYNOPSIS
  Builds the standalone portable EXE (with embedded UI via `custom-protocol`), verifies it in an
  isolated temporary directory, and optionally packages Windows MSI / NSIS installers.

.DESCRIPTION
  Builds the React/Vite UI and compiles `notscreenstudio.exe` via `tauri build --no-bundle` so
  Tauri's `custom-protocol` feature is always active and `ui/dist` is embedded inside the EXE.
  Verifies that the built EXE embeds the frontend bundle and passes `--smoke-test` when copied
  out of the repository into an empty folder, then stages release artifacts in `dist-release/`
  with a `SHA256SUMS.txt` manifest.

  Flags / Environment:
    -SkipSmoke         or NSS_SKIP_SMOKE=1       skip isolated `--smoke-test`
    -SkipInstallers    or NSS_SKIP_INSTALLERS=1  build only the standalone portable EXE
    -Bundles           or NSS_BUNDLES            comma list of bundle targets (default "msi,nsis")
#>

$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

if (-not (Test-Path "ui/node_modules")) {
    Write-Host "==> Installing UI dependencies" -ForegroundColor Cyan
    npm --prefix ui install
    if ($LASTEXITCODE -ne 0) { throw "npm install failed" }
}

$localTauri = Join-Path $root "ui/node_modules/.bin/tauri.cmd"
function Invoke-Tauri {
    param([string[]]$TauriArgs)
    if (Test-Path $localTauri) {
        & $localTauri @TauriArgs
    } else {
        cargo tauri @TauriArgs
    }
    if ($LASTEXITCODE -ne 0) { throw "tauri $($TauriArgs -join ' ') failed" }
}

$skipInstallersEff = $SkipInstallers.IsPresent -or ($env:NSS_SKIP_INSTALLERS -eq "1")
if ($skipInstallersEff) {
    Write-Host "==> Building standalone release EXE (with embedded UI)" -ForegroundColor Cyan
    Invoke-Tauri @("build", "--no-bundle")
} else {
    $bundleList = if ($Bundles) { $Bundles } elseif ($env:NSS_BUNDLES) { $env:NSS_BUNDLES } else { "msi,nsis" }
    Write-Host "==> Building standalone release EXE (with embedded UI) + installers ($bundleList)" -ForegroundColor Cyan
    # `--no-binary-patching` keeps `target/release/notscreenstudio.exe` clean and portable in a single
    # release compile instead of compiling twice.
    Invoke-Tauri @("build", "--bundles", $bundleList, "--no-binary-patching")
}

$exe = Join-Path $root "target/release/notscreenstudio.exe"
if (-not (Test-Path $exe)) { throw "expected release binary not found: $exe" }

Write-Host "==> Verifying embedded frontend assets in standalone EXE" -ForegroundColor Cyan
$jsAssets = Get-ChildItem -Path (Join-Path $root "ui/dist/assets") -Filter "index-*.js" -ErrorAction Stop
if (-not $jsAssets) { throw "ui/dist/assets/index-*.js not found after build" }
$jsName = $jsAssets[0].Name
$exeAscii = [System.Text.Encoding]::ASCII.GetString([System.IO.File]::ReadAllBytes($exe))
if (-not $exeAscii.Contains($jsName)) {
    throw "Standalone EXE verification FAILED: '$jsName' is not embedded in $exe (custom-protocol missing?)"
}
Write-Host "    Confirmed embedded UI bundle ($jsName) inside notscreenstudio.exe" -ForegroundColor Green

$skipSmokeEff = $SkipSmoke.IsPresent -or ($env:NSS_SKIP_SMOKE -eq "1")
if (-not $skipSmokeEff) {
    Write-Host "==> Isolated standalone smoke test" -ForegroundColor Cyan
    $isolatedDir = Join-Path ([System.IO.Path]::GetTempPath()) ("nss-build-smoke-" + [System.Guid]::NewGuid().ToString("N").Substring(0, 8))
    New-Item -ItemType Directory -Path $isolatedDir | Out-Null
    try {
        $isolatedExe = Join-Path $isolatedDir "notscreenstudio.exe"
        Copy-Item -Path $exe -Destination $isolatedExe
        $smoke = Start-Process -FilePath $isolatedExe -ArgumentList "--smoke-test" -WorkingDirectory $isolatedDir -Wait -PassThru -NoNewWindow
        if ($smoke.ExitCode -ne 0) { throw "isolated smoke test failed (exit code $($smoke.ExitCode))" }
    } finally {
        Remove-Item -Path $isolatedDir -Recurse -Force -ErrorAction SilentlyContinue
    }
} else {
    Write-Host "==> Smoke test skipped" -ForegroundColor Yellow
}

$stagingDir = Join-Path $root "dist-release"
if (Test-Path $stagingDir) { Remove-Item -Path $stagingDir -Recurse -Force }
New-Item -ItemType Directory -Path $stagingDir | Out-Null
Copy-Item -Path $exe -Destination (Join-Path $stagingDir "notscreenstudio.exe")

if (-not $skipInstallersEff) {
    $bundleRoot = Join-Path $root "target/release/bundle"
    if (Test-Path $bundleRoot) {
        $installers = Get-ChildItem -Recurse -Path $bundleRoot -Include *.msi, *-setup.exe
        foreach ($item in $installers) {
            Copy-Item -Path $item.FullName -Destination (Join-Path $stagingDir $item.Name)
        }
    }
} else {
    Write-Host "==> Installer bundling skipped (-SkipInstallers)" -ForegroundColor Yellow
}

Write-Host "==> Writing SHA256SUMS.txt" -ForegroundColor Cyan
$checksumLines = Get-ChildItem -Path $stagingDir -File |
    Where-Object { $_.Name -ne "SHA256SUMS.txt" } |
    Sort-Object Name |
    ForEach-Object {
        $hash = (Get-FileHash -Path $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        "$hash  $($_.Name)"
    }
$checksumLines | Set-Content -Path (Join-Path $stagingDir "SHA256SUMS.txt") -Encoding ascii

Write-Host ""
Write-Host "Release artifacts staged in dist-release/:" -ForegroundColor Green
Get-ChildItem -Path $stagingDir -File | ForEach-Object {
    $mb = [math]::Round($_.Length / 1MB, 2)
    Write-Host ("  {0,-42} {1,6} MB" -f $_.Name, $mb)
}
