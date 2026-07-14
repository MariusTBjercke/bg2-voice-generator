<#
.SYNOPSIS
  Build the portable Windows ZIP of BG2 Voice Generator: the exe plus everything the
  local OmniVoice engine + native export path need, all next to it, so a user unzips
  one folder and runs it with no dev toolchain.

.DESCRIPTION
  Produces dist/BG2VoiceGenerator-<version>.zip; the archive wraps one top-level
  BG2VoiceGenerator-<version>/ folder:

      <root>/bg2-voice-generator.exe    the app
      <root>/engine/                    omnivoice_server.py, requirements-*.txt, README.md (shipped)
      <root>/tools/                     python/, ffmpeg.exe, ffprobe.exe, weidu.exe, THIRD-PARTY-LICENSES/
      <root>/engine-runtime/            (created on first run: venv + models)
      <root>/README.txt                 first-run guide + vendored-tool licensing

  paths.rs flips to "portable" purely by probing for <exe_dir>/engine/omnivoice_server.py,
  so the engine/ + tools/ siblings ARE the switch - no build flag involved. The writable
  engine-runtime/ is created on first launch.

  Steps: build the exe (gates + cargo tauri build --no-bundle) -> ensure tools/ via
  fetch-tools.ps1 -> stage -> zip -> (optionally) DEPLOY into a stable run folder whose
  engine-runtime/ survives rebuilds. Pass -SkipBuild to re-stage an already-built exe.

.PARAMETER Version    Version string baked into the zip name. Defaults to package.json.
.PARAMETER SkipBuild  Don't run gates / cargo build; stage the exe already at target/release.
.PARAMETER Force      Re-fetch the vendored tools (passes -Force to fetch-tools.ps1).
.PARAMETER SkipWeidu  Pass -SkipWeidu to fetch-tools.ps1 (offline/CI; app ships without the installer).
.PARAMETER InstallDir The persistent run folder the build deploys into (default: dist/portable/).
.PARAMETER NoDeploy   Build + zip only; don't deploy into the run folder.
.PARAMETER CleanRuntime  Delete $InstallDir/engine-runtime/ at deploy time (forces a fresh engine install).
#>
[CmdletBinding()]
param(
    [string]$Version,
    [switch]$SkipBuild,
    [switch]$Force,
    [switch]$SkipWeidu,
    [string]$InstallDir,
    [switch]$NoDeploy,
    [switch]$CleanRuntime
)

$ErrorActionPreference = 'Stop'
$root = $PSScriptRoot
Set-Location $root

if (-not $Version) {
    $Version = (Get-Content (Join-Path $root 'package.json') -Raw | ConvertFrom-Json).version
}

$binName   = 'bg2-voice-generator.exe'
$stageName = 'BG2VoiceGenerator'
$distDir   = Join-Path $root 'dist'
# Internal zip-staging scratch (wiped + rebuilt every run). Kept SEPARATE from the
# persistent run folder so wiping it can never touch a deployed engine-runtime/.
$stageRoot = Join-Path $distDir '_stage'
$stageDir  = Join-Path $stageRoot "$stageName-$Version"
$zipPath   = Join-Path $distDir "$stageName-$Version.zip"
if (-not $InstallDir) { $InstallDir = Join-Path $distDir 'portable' }

function Write-Step($m) { Write-Host "==> $m" -ForegroundColor Green }

# Resolve the built app exe. cargo (--no-bundle) emits the cargo bin name; fall back to
# the productName-based name in case a future config renames it.
function Resolve-AppExe {
    $relDir = Join-Path $root 'src-tauri\target\release'
    foreach ($cand in @($binName, 'BG2 Voice Generator.exe')) {
        $p = Join-Path $relDir $cand
        if (Test-Path $p) { return $p }
    }
    return $null
}

# --- 1) Build the exe (gates + no-bundle) ---------------------------------------
if (-not $SkipBuild) {
    Write-Step "Frontend checks (npm run check)"
    npm run check; if ($LASTEXITCODE -ne 0) { throw "npm run check failed" }

    Write-Step "Frontend tests (npm run test)"
    npm run test; if ($LASTEXITCODE -ne 0) { throw "npm run test failed" }

    Write-Step "Rust checks (cargo check + cargo test)"
    Push-Location (Join-Path $root 'src-tauri')
    try {
        cargo check; if ($LASTEXITCODE -ne 0) { throw "cargo check failed" }
        cargo test;  if ($LASTEXITCODE -ne 0) { throw "cargo test failed" }
    } finally { Pop-Location }

    Write-Step "Building app exe v$Version (cargo tauri build --no-bundle)"
    npm run tauri build -- --no-bundle
    if ($LASTEXITCODE -ne 0) { throw "tauri build failed" }

    Write-Step "Building synthesis agent CLI"
    cargo build --release --manifest-path (Join-Path $root 'src-tauri\Cargo.toml') -p bg2-synthesis-cli
    if ($LASTEXITCODE -ne 0) { throw "bg2-synthesis CLI build failed" }
}

$exePath = Resolve-AppExe
if (-not $exePath) {
    throw "app exe not found under src-tauri/target/release - run without -SkipBuild, or build first."
}
$synthesisCli = Join-Path $root 'src-tauri\target\release\bg2-synthesis.exe'
if (-not (Test-Path $synthesisCli)) {
    throw "bg2-synthesis.exe not found under src-tauri/target/release - run without -SkipBuild, or build the bg2-synthesis-cli package first."
}

# --- 2) Ensure vendored tools (python + ffmpeg + WeiDU) -------------------------
# fetch-tools.ps1 uses ErrorActionPreference=Stop, so a real failure throws and
# propagates here; don't gate on $LASTEXITCODE (stale when fetch skips work).
Write-Step "Ensuring tools/ (python + ffmpeg + WeiDU)"
$fetchArgs = @()
if ($Force)     { $fetchArgs += '-Force' }
if ($SkipWeidu) { $fetchArgs += '-SkipWeidu' }
& (Join-Path $root 'fetch-tools.ps1') @fetchArgs

# --- 3) Stage the portable tree -------------------------------------------------
Write-Step "Staging $stageDir"
if (Test-Path $stageRoot) { Remove-Item $stageRoot -Recurse -Force }
New-Item -ItemType Directory -Force -Path $stageDir | Out-Null

Copy-Item $exePath (Join-Path $stageDir $binName) -Force
Copy-Item $synthesisCli (Join-Path $stageDir 'bg2-synthesis.exe') -Force

# engine/ - ship only the scripts + requirements + README, NEVER a dev venv or cache.
$engineDst = Join-Path $stageDir 'engine'
New-Item -ItemType Directory -Force -Path $engineDst | Out-Null
Get-ChildItem (Join-Path $root 'engine') -File | Where-Object {
    $_.Extension -in '.py','.txt','.md'
} | ForEach-Object { Copy-Item $_.FullName $engineDst -Force }

# tools/ (python + ffmpeg + weidu + THIRD-PARTY-LICENSES) verbatim. tools/python MUST
# ship: the in-app installer bootstraps engine-runtime/venv from it (python -m venv). The
# venv, its deps, and the HF model cache live under the writable engine-runtime/ (created
# on first run, never staged here), so no dev venv or model cache is bundled.
Copy-Item (Join-Path $root 'tools') (Join-Path $stageDir 'tools') -Recurse -Force

if (-not (Test-Path (Join-Path $stageDir 'tools\weidu.exe'))) {
    Write-Warning "tools/weidu.exe absent - the portable app can generate but not run a native install/uninstall until WeiDU is fetched (drop -SkipWeidu)."
}

# README.txt: the portable build's first-run guide + vendored-tool licensing summary.
# Generated at stage time (not a committed repo file); THIRD-PARTY-LICENSES/ carries the
# full texts. Item-10 requires licensing for the vendored tools be documented in the app.
@"
BG2 Voice Generator - portable build v$Version

FIRST RUN
  1. Unzip this whole folder anywhere (keep bg2-voice-generator.exe, bg2-synthesis.exe,
     engine/, and tools/ together - that sibling layout is what switches the app into
     portable mode).
  2. Run bg2-voice-generator.exe. On first launch it creates engine-runtime/ next to the
     exe and installs the local OmniVoice engine (a Python venv + model download) into it.
     Generation needs a GPU; the exported voice packs do NOT (see below).
  3. Point the app at your BG2EE install, scan, harvest references, generate, export.

WHAT THE EXPORTED PACKS NEED
  Nothing from this app. A generated pack is a native WeiDU mod (audio copied to
  override/ + STRING_SET); it plays with no EEex, sidecar, runtime TTS, or background
  process. Package a project's pack into a self-contained ZIP (with a bundled
  setup-<pack>.exe) from the app's export screen.

BUNDLED THIRD-PARTY TOOLS (full license texts in tools/THIRD-PARTY-LICENSES/)
  - WeiDU 251.00 - GPLv2. Its author expressly permits redistributing an unmodified
    WeiDU.exe with a mod (the setup-<mod>.exe pattern). See weidu-REDISTRIBUTION.txt.
  - ffmpeg 8.1.2 (gyan.dev static) - GPLv3. Written source offer in ffmpeg-SOURCE-OFFER.txt.
  - CPython (python-build-standalone) - PSF license. See python-LICENSE.txt.
  - OmniVoice + its Python dependencies are downloaded into engine-runtime/ on first run
    under their own upstream licenses; they are not redistributed inside this ZIP.

This build redistributes only the tools above and the app itself. No original
game-derived audio and no third-party reference mods are included.
"@ | Set-Content (Join-Path $stageDir 'README.txt') -Encoding UTF8

# --- 4) Zip --------------------------------------------------------------------
# Build the archive entry-by-entry so every entry name is a forward-slash path under one
# top-level BG2VoiceGenerator-<version>/ folder. Compress-Archive writes backslash
# separators for the wrapper prefix on Windows PowerShell, which is invalid per the ZIP
# spec - setting the entry name explicitly is the only reliable way.
Write-Step "Zipping $zipPath"
if (Test-Path $zipPath) { Remove-Item $zipPath -Force }
New-Item -ItemType Directory -Force -Path $distDir | Out-Null
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
$prefix  = Split-Path $stageDir -Leaf
$baseLen = ((Resolve-Path $stageDir).Path.TrimEnd('\')).Length + 1
$zip = [System.IO.Compression.ZipFile]::Open($zipPath, [System.IO.Compression.ZipArchiveMode]::Create)
try {
    Get-ChildItem -LiteralPath $stageDir -Recurse -File | ForEach-Object {
        $rel = $_.FullName.Substring($baseLen) -replace '\\', '/'
        [void][System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile(
            $zip, $_.FullName, "$prefix/$rel",
            [System.IO.Compression.CompressionLevel]::Optimal)
    }
} finally { $zip.Dispose() }

# --- 5) Deploy into the persistent run folder ----------------------------------
# Mirror every staged top-level entry into $InstallDir but LEAVE engine-runtime/ (the
# venv + downloaded models) in place, so rebuilds never force an engine reinstall. The
# stage never contains engine-runtime/, so the mirror only ever touches shipped files.
if (-not $NoDeploy) {
    Write-Step "Deploying into run folder $InstallDir"
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null

    # Stop a running instance from the deploy target so a locked exe can't abort the
    # mirror partway through and leave a version-skewed run folder.
    $target = Join-Path $InstallDir $binName
    if (Test-Path $target) {
        $targetFull = (Resolve-Path $target).Path
        $procName = [System.IO.Path]::GetFileNameWithoutExtension($binName)
        $blocking = Get-Process -Name $procName -ErrorAction SilentlyContinue |
            Where-Object { $_.Path -and $_.Path -eq $targetFull }
        if ($blocking) {
            Write-Step "Stopping running $binName so the deploy can overwrite it"
            $blocking | Stop-Process -Force
            Start-Sleep -Milliseconds 500
        }
    }

    if ($CleanRuntime) {
        $rt = Join-Path $InstallDir 'engine-runtime'
        if (Test-Path $rt) {
            Write-Step "Discarding engine-runtime/ (-CleanRuntime) - engines reinstall on next launch"
            Remove-Item $rt -Recurse -Force
        }
    }

    Get-ChildItem -LiteralPath $stageDir -Force | ForEach-Object {
        if ($_.Name -eq 'engine-runtime') { return }   # never staged; belt-and-suspenders
        $dst = Join-Path $InstallDir $_.Name
        if (Test-Path $dst) { Remove-Item -LiteralPath $dst -Recurse -Force }
        Copy-Item -LiteralPath $_.FullName -Destination $dst -Recurse -Force
    }

    if (Test-Path (Join-Path $InstallDir 'engine-runtime')) {
        Write-Host "    engine-runtime/ preserved - engine stays installed, no reinstall." -ForegroundColor DarkGray
    } else {
        Write-Host "    First deploy: launch the app once to install the engine into engine-runtime/." -ForegroundColor DarkGray
    }
    Write-Host "    Run: $(Join-Path $InstallDir $binName)" -ForegroundColor DarkGray
}

$sizeMB = [math]::Round((Get-Item $zipPath).Length / 1MB, 1)
Write-Step "Done: $zipPath ($sizeMB MB)"
Write-Host "    Unzipped layout staged at: $stageDir" -ForegroundColor DarkGray
