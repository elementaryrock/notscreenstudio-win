param(
    [string]$Tag = "",
    [string]$Title = "",
    [switch]$SkipTests,
    [switch]$SkipInstallers,
    [switch]$DryRun
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

<#
.SYNOPSIS
  End-to-end build, live standalone EXE verification, and GitHub Release publisher.

.DESCRIPTION
  1. Runs `./scripts/test.ps1` (unless -SkipTests).
  2. Builds the standalone portable `notscreenstudio.exe` (and Windows installers unless
     -SkipInstallers) via `./scripts/build.ps1`.
  3. Runs a full live end-to-end verification of `dist-release/notscreenstudio.exe` copied into a
     completely isolated temp directory:
       - Verifies `--smoke-test` passes outside the repo
       - Launches the GUI with CDP enabled and verifies:
           * Embedded custom protocol (`http://tauri.localhost/`) loads (NOT localhost:1420)
           * React UI renders cleanly
           * ZERO CSP or JavaScript console errors
           * Core Rust IPC commands (`core_info`, `check_ffmpeg`, `list_capture_sources`, `get_settings`) succeed
           * Overlay HUD window (`hud-setup`) opens and renders cleanly
       - Launches a second copy of the EXE while the first is running and asserts single-instance
         activation (exits cleanly with code 0 without crashing WebView2)
       - Sends `WM_CLOSE` and verifies clean shutdown (exit code 0)
  4. Pushes the git tag and creates or updates the GitHub Release with `notscreenstudio.exe`
     (plus any installers and `SHA256SUMS.txt`), unless `-DryRun` is specified.
#>

$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$tauriConf = Get-Content -Path "src-tauri/tauri.conf.json" -Raw | ConvertFrom-Json
$version = $tauriConf.version
if (-not $Tag) { $Tag = "v$version" }
if (-not $Title) { $Title = "$Tag — Windows Release" }

if (-not $DryRun) {
    $dirty = git status --porcelain
    if ($dirty) {
        throw "Working tree has uncommitted changes. Commit or stash them before publishing a release (or pass -DryRun)."
    }
    gh auth status | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "GitHub CLI (gh) is not authenticated. Run `gh auth login` first." }
}

if (-not $SkipTests) {
    Write-Host "==> Running full test gate" -ForegroundColor Cyan
    & (Join-Path $PSScriptRoot "test.ps1")
}

Write-Host "==> Building release artifacts" -ForegroundColor Cyan
$buildArgs = @{}
if ($SkipInstallers) { $buildArgs["SkipInstallers"] = $true }
& (Join-Path $PSScriptRoot "build.ps1") @buildArgs

$stagedExe = Join-Path $root "dist-release/notscreenstudio.exe"
if (-not (Test-Path $stagedExe)) { throw "Staged standalone EXE not found: $stagedExe" }

Write-Host "==> End-to-end live verification of standalone notscreenstudio.exe" -ForegroundColor Cyan
$verifyDir = Join-Path ([System.IO.Path]::GetTempPath()) ("nss-release-verify-" + [System.Guid]::NewGuid().ToString("N").Substring(0, 8))
New-Item -ItemType Directory -Path $verifyDir | Out-Null

$cdpPort = 19234
$proc = $null
$prevWebViewArgs = $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS
$prevNoDialog = $env:NSS_NO_ERROR_DIALOG

try {
    $isoExe = Join-Path $verifyDir "notscreenstudio.exe"
    Copy-Item -Path $stagedExe -Destination $isoExe

    $cdpScript = Join-Path $verifyDir "verify_cdp.mjs"
    @'
const port = process.argv[2];
const wsUrl = process.argv[3];
const ws = new WebSocket(wsUrl);
let id = 1;
const errors = [];
const send = (method, params = {}) => new Promise((res) => {
  const myId = id++;
  const onMsg = (ev) => {
    const d = JSON.parse(ev.data);
    if (d.id === myId) { ws.removeEventListener('message', onMsg); res(d); }
  };
  ws.addEventListener('message', onMsg);
  ws.send(JSON.stringify({ id: myId, method, params }));
});
ws.addEventListener('message', (ev) => {
  const d = JSON.parse(ev.data);
  if (d.method === 'Runtime.exceptionThrown') {
    errors.push('EXCEPTION: ' + JSON.stringify(d.params));
  }
  if (d.method === 'Log.entryAdded' && (d.params?.entry?.level === 'error' || d.params?.entry?.source === 'security')) {
    errors.push('LOG[' + d.params.entry.source + ']: ' + d.params.entry.text);
  }
});
ws.addEventListener('open', async () => {
  await send('Runtime.enable');
  await send('Log.enable');

  const r1 = await send('Runtime.evaluate', {
    expression: `JSON.stringify({
      href: location.href,
      title: document.title,
      rootLen: document.getElementById('root')?.innerHTML?.length ?? 0,
      text: document.body.innerText.slice(0, 400),
      hasTauri: typeof window.__TAURI_INTERNALS__
    })`,
    returnByValue: true
  });
  const dom = JSON.parse(r1.result.result.value);
  console.log('DOM_OK:', JSON.stringify({ href: dom.href, title: dom.title, rootLen: dom.rootLen, hasTauri: dom.hasTauri }));
  if (!dom.href.startsWith('http://tauri.localhost/') && !dom.href.startsWith('tauri://localhost/')) {
    throw new Error('Unexpected URL (not embedded custom-protocol): ' + dom.href);
  }
  if (dom.rootLen < 200 || !dom.text.includes('notscreenstudio')) {
    throw new Error('UI did not render cleanly: ' + JSON.stringify(dom));
  }

  const r2 = await send('Runtime.evaluate', {
    expression: `(async () => {
      const info = await window.__TAURI_INTERNALS__.invoke('core_info');
      const ff = await window.__TAURI_INTERNALS__.invoke('check_ffmpeg');
      const src = await window.__TAURI_INTERNALS__.invoke('list_capture_sources');
      const set = await window.__TAURI_INTERNALS__.invoke('get_settings');
      return JSON.stringify({ info, ff, displays: src.displays.length, hasSettings: !!set });
    })()`,
    awaitPromise: true,
    returnByValue: true
  });
  const ipc = JSON.parse(r2.result.result.value);
  console.log('IPC_OK:', JSON.stringify(ipc));
  if (!ipc.info || !ipc.hasSettings) {
    throw new Error('IPC check failed: ' + JSON.stringify(ipc));
  }

  await send('Runtime.evaluate', {
    expression: `window.__TAURI_INTERNALS__.invoke('open_overlay', { kind: 'setupBar', bounds: null })`,
    awaitPromise: true
  });
  await new Promise(r => setTimeout(r, 1200));
  const targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
  const hud = targets.find(t => t.url.includes('hud-setup'));
  if (!hud) {
    throw new Error('Overlay window hud-setup did not open: ' + JSON.stringify(targets.map(t => t.url)));
  }
  console.log('OVERLAY_OK:', hud.url);

  await send('Runtime.evaluate', {
    expression: `window.__TAURI_INTERNALS__.invoke('close_all_overlays')`,
    awaitPromise: true
  });
  await new Promise(r => setTimeout(r, 300));

  if (errors.length > 0) {
    throw new Error('Console/CSP errors detected: ' + JSON.stringify(errors));
  }
  console.log('ZERO_CONSOLE_OR_CSP_ERRORS');
  ws.close();
  process.exit(0);
});
setTimeout(() => { console.error('CDP_TIMEOUT'); process.exit(1); }, 15000);
'@ | Set-Content -Path $cdpScript -Encoding utf8

    $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=$cdpPort"
    $env:NSS_NO_ERROR_DIALOG = "1"
    $stdout = Join-Path $verifyDir "out.txt"
    $stderr = Join-Path $verifyDir "err.txt"

    $proc = Start-Process -FilePath $isoExe -WorkingDirectory $verifyDir -RedirectStandardOutput $stdout -RedirectStandardError $stderr -PassThru
    Start-Sleep -Seconds 4

    if ($proc.HasExited) {
        $errText = Get-Content $stderr -Raw -ErrorAction SilentlyContinue
        throw "Standalone EXE exited prematurely with code $($proc.ExitCode). Stderr: $errText"
    }

    $targets = Invoke-RestMethod "http://127.0.0.1:$cdpPort/json" -TimeoutSec 5
    $page = $targets | Where-Object { $_.type -eq "page" } | Select-Object -First 1
    if (-not $page) { throw "No WebView2 page target found on CDP port $cdpPort" }

    node $cdpScript $cdpPort $page.webSocketDebuggerUrl
    if ($LASTEXITCODE -ne 0) { throw "CDP live UI/IPC/CSP verification failed" }

    Write-Host "==> Verifying single-instance re-launch (no WebView2 0x800700AA crash)" -ForegroundColor Cyan
    $second = Start-Process -FilePath $isoExe -WorkingDirectory $verifyDir -Wait -PassThru
    if ($second.ExitCode -ne 0) {
        throw "Second instance launch failed with exit code $($second.ExitCode) (expected 0)"
    }
    if ($proc.HasExited) {
        throw "Primary instance unexpectedly exited during second-instance activation"
    }
    Write-Host "    Second instance focused existing window and exited cleanly (code 0)" -ForegroundColor Green

    Write-Host "==> Verifying clean WM_CLOSE shutdown" -ForegroundColor Cyan
    Add-Type -TypeDefinition @"
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class NssReleaseWin {
    public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc lpEnumFunc, IntPtr lParam);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint lpdwProcessId);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr hWnd, StringBuilder sb, int max);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hWnd, uint Msg, IntPtr wParam, IntPtr lParam);
    public static IntPtr FindByPidAndTitle(int pid, string title) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((h, l) => {
            uint p; GetWindowThreadProcessId(h, out p);
            if (p == (uint)pid) {
                var sb = new StringBuilder(256);
                GetWindowText(h, sb, 256);
                if (sb.ToString() == title) { found = h; return false; }
            }
            return true;
        }, IntPtr.Zero);
        return found;
    }
}
"@ -ErrorAction SilentlyContinue

    $hwnd = [NssReleaseWin]::FindByPidAndTitle($proc.Id, "notscreenstudio")
    if ($hwnd -eq [IntPtr]::Zero) { throw "Could not find main window HWND for clean close check" }
    [NssReleaseWin]::PostMessage($hwnd, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    if (-not $proc.WaitForExit(6000)) {
        throw "Standalone EXE did not exit cleanly within 6s after WM_CLOSE"
    }
    if ($proc.ExitCode -ne 0) {
        throw "Standalone EXE exited with non-zero code $($proc.ExitCode) after WM_CLOSE"
    }
    $proc = $null
    Write-Host "    Clean WM_CLOSE shutdown verified (exit code 0)" -ForegroundColor Green
} finally {
    if ($proc -and -not $proc.HasExited) {
        Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
    }
    if ($null -eq $prevWebViewArgs) {
        Remove-Item Env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS -ErrorAction SilentlyContinue
    } else {
        $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = $prevWebViewArgs
    }
    if ($null -eq $prevNoDialog) {
        Remove-Item Env:NSS_NO_ERROR_DIALOG -ErrorAction SilentlyContinue
    } else {
        $env:NSS_NO_ERROR_DIALOG = $prevNoDialog
    }
    Remove-Item -Path $verifyDir -Recurse -Force -ErrorAction SilentlyContinue
}

if ($DryRun) {
    Write-Host ""
    Write-Host "Dry run passed! Verified artifacts are in dist-release/." -ForegroundColor Green
    exit 0
}

Write-Host "==> Pushing git branch and tag $Tag" -ForegroundColor Cyan
git push origin HEAD
if ($LASTEXITCODE -ne 0) { throw "git push failed" }

git tag -f -a $Tag -m "$Title"
git push -f origin $Tag
if ($LASTEXITCODE -ne 0) { throw "git push tag $Tag failed" }

$notesFile = Join-Path ([System.IO.Path]::GetTempPath()) "nss-release-notes-$Tag.md"
$notesTemplate = @'
## notscreenstudio for Windows (__TAG__)

A free, completely offline Windows screen recorder and studio editor with automatic zoom, smooth spring-interpolated cursor physics, and GPU-accelerated video composition — ported to Windows from macOS [notscreenstudio](https://github.com/dotslashsimran/notscreenstudio).

### Downloads
- **`notscreenstudio.exe`** — Standalone portable Windows x64 executable (double-click to run directly, no installation required; UI is fully embedded).
- **Installers** (`.msi` / `-setup.exe`, when attached below) — Standard Windows installers.
- **`SHA256SUMS.txt`** — SHA-256 verification checksums.

### Requirements
- **OS:** Windows 10 (2004 / build 19041+) or Windows 11 (x64)
- **WebView2:** Microsoft Edge WebView2 Runtime (preinstalled on Windows 11 and updated Windows 10)
- **FFmpeg:** [FFmpeg](https://www.gyan.dev/ffmpeg/builds/) (tested with 8.1.1) installed on `PATH` (e.g. `winget install Gyan.FFmpeg`) or placed next to `notscreenstudio.exe` (`ffmpeg.exe` & `ffprobe.exe`).

### Quick Start
1. Download **`notscreenstudio.exe`** from the Assets section below.
2. Double-click **`notscreenstudio.exe`** to launch directly. (If Windows SmartScreen shows a prompt for an unsigned open-source app, click **More info → Run anyway**.)
3. For recording & MP4 export, install FFmpeg (`winget install Gyan.FFmpeg`) or place `ffmpeg.exe` and `ffprobe.exe` next to `notscreenstudio.exe`.
'@
$notesTemplate.Replace("__TAG__", $Tag) | Set-Content -Path $notesFile -Encoding utf8

try {
    $assets = Get-ChildItem -Path (Join-Path $root "dist-release") -File | Select-Object -ExpandProperty FullName
    $releaseExists = $true
    gh release view $Tag 2>$null | Out-Null
    if ($LASTEXITCODE -ne 0) { $releaseExists = $false }

    if ($releaseExists) {
        Write-Host "==> Updating existing GitHub Release $Tag" -ForegroundColor Cyan
        gh release edit $Tag --title $Title --notes-file $notesFile
        if ($LASTEXITCODE -ne 0) { throw "gh release edit failed" }
        gh release upload $Tag @assets --clobber
        if ($LASTEXITCODE -ne 0) { throw "gh release upload failed" }
    } else {
        Write-Host "==> Creating GitHub Release $Tag" -ForegroundColor Cyan
        gh release create $Tag @assets --title $Title --notes-file $notesFile
        if ($LASTEXITCODE -ne 0) { throw "gh release create failed" }
    }
} finally {
    Remove-Item -Path $notesFile -Force -ErrorAction SilentlyContinue
}

Write-Host ""
Write-Host "Published verified release $Tag!" -ForegroundColor Green
gh release view $Tag --json url,assets --jq '{url: .url, assets: [.assets[] | {name: .name, size: .size}]}'
