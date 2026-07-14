<#
.SYNOPSIS
  Fetch the vendored Windows binaries the portable build ships in tools/:
  a relocatable CPython (python-build-standalone), a static ffmpeg/ffprobe, and WeiDU.

.DESCRIPTION
  Downloads each archive (cached under vendor/), verifies its SHA-256 against the
  pinned value, and extracts the pieces we ship into tools/:

      tools/python/python.exe        base interpreter (resolver: paths.rs base_python_at)
      tools/ffmpeg.exe               PCM-WAV encode / probing (resolver: paths.rs tool_at)
      tools/ffprobe.exe              audio probing
      tools/weidu.exe                native pack installer (resolver: paths.rs tool_at)
      tools/THIRD-PARTY-LICENSES/    ffmpeg + python + WeiDU license texts

  None of these binaries are committed to the repo; this script is the reproducible,
  checksum-pinned way to materialize them at packaging time. It is idempotent: a cached
  archive whose hash matches is not re-downloaded, and an already-extracted tool is left
  alone unless -Force is passed.

  WeiDU redistribution (open question from items 02/03) is now CONFIRMED: WeiDU is
  GPLv2 AND its author explicitly grants distributing an unmodified binary copy of
  WeiDU.exe (without source) alongside a mod - the canonical setup-<mod>.exe pattern
  every Infinity Engine mod uses (see the WeiDU README). The pin
  below is a real, immutable GitHub release; the license text ships in
  tools/THIRD-PARTY-LICENSES/. -SkipWeidu remains only as an offline/CI escape hatch.

.PARAMETER Force        Re-download and re-extract even if the cached tool exists.
.PARAMETER SkipChecksum Skip SHA-256 verification (offline-mirror escape hatch).
.PARAMETER SkipWeidu    Do not fetch WeiDU (offline/CI escape hatch only).
#>
[CmdletBinding()]
param(
    [switch]$Force,
    [switch]$SkipChecksum,
    [switch]$SkipWeidu
)

$ErrorActionPreference = 'Stop'
$root      = $PSScriptRoot
$vendorDir = Join-Path $root 'vendor'
$toolsDir  = Join-Path $root 'tools'
$licDir    = Join-Path $toolsDir 'THIRD-PARTY-LICENSES'

# --- Pinned assets (immutable, dated URLs so a build is byte-reproducible) ---
$python = @{
    Name   = 'CPython 3.12.13 (python-build-standalone 20260602, install_only_stripped)'
    Url    = 'https://github.com/astral-sh/python-build-standalone/releases/download/20260602/cpython-3.12.13+20260602-x86_64-pc-windows-msvc-install_only_stripped.tar.gz'
    Sha256 = 'dbada31f91a4fff934dae85e7998d91f1e926135bd88ffed4921a337d5680f48'
    File   = 'cpython-3.12.13-20260602-windows-install_only_stripped.tar.gz'
}
$ffmpeg = @{
    Name   = 'ffmpeg 8.1.2 (gyan.dev essentials, static, GPLv3)'
    Url    = 'https://www.gyan.dev/ffmpeg/builds/packages/ffmpeg-8.1.2-essentials_build.zip'
    Sha256 = 'db580001caa24ac104c8cb856cd113a87b0a443f7bdf47d8c12b1d740584a2ec'
    File   = 'ffmpeg-8.1.2-essentials_build.zip'
}
# WeiDU 251.00, official 64-bit Unicode Windows build. GPLv2 + explicit grant to
# redistribute an unmodified WeiDU.exe with a mod (see .DESCRIPTION). The 64-bit
# Unicode package (not the +legacy 32-bit one) is what the target install runs.
$weidu = @{
    Name   = 'WeiDU 251.00 (Windows, 64-bit Unicode; GPLv2, binary redistribution granted)'
    Url    = 'https://github.com/WeiDUorg/weidu/releases/download/v251.00/WeiDU-Windows-251.zip'
    Sha256 = 'a54c6198d6ebed8139793fcacb225500d563657c23fc775b840383f9750493d8'
    File   = 'WeiDU-Windows-251.zip'
}

function Write-Step($msg) { Write-Host "==> $msg" -ForegroundColor Cyan }

function Get-Pinned($asset) {
    New-Item -ItemType Directory -Force -Path $vendorDir | Out-Null
    $dest = Join-Path $vendorDir $asset.File
    $haveValid = $false
    if ((Test-Path $dest) -and -not $Force) {
        if ($SkipChecksum) { $haveValid = $true }
        else {
            $h = (Get-FileHash -Algorithm SHA256 -Path $dest).Hash.ToLower()
            if ($h -eq $asset.Sha256.ToLower()) { $haveValid = $true }
            else { Write-Step "Cached $($asset.File) hash mismatch - re-downloading"; Remove-Item $dest -Force }
        }
    }
    if (-not $haveValid) {
        Write-Step "Downloading $($asset.Name)"
        try { Start-BitsTransfer -Source $asset.Url -Destination $dest -ErrorAction Stop }
        catch {
            $old = $ProgressPreference; $ProgressPreference = 'SilentlyContinue'
            Invoke-WebRequest -Uri $asset.Url -OutFile $dest -UseBasicParsing
            $ProgressPreference = $old
        }
    }
    if (-not $SkipChecksum) {
        $h = (Get-FileHash -Algorithm SHA256 -Path $dest).Hash.ToLower()
        if ($h -ne $asset.Sha256.ToLower()) {
            throw "SHA-256 mismatch for $($asset.File): expected $($asset.Sha256), got $h"
        }
        Write-Step "Verified $($asset.File) (sha256 ok)"
    }
    return $dest
}

# --- Python ---
$pyExe = Join-Path $toolsDir 'python\python.exe'
if ((Test-Path $pyExe) -and -not $Force) {
    Write-Step "tools/python already present - skipping (use -Force to refresh)"
} else {
    $archive = Get-Pinned $python
    Write-Step "Extracting CPython"
    if (Test-Path (Join-Path $toolsDir 'python')) { Remove-Item (Join-Path $toolsDir 'python') -Recurse -Force }
    New-Item -ItemType Directory -Force -Path $toolsDir | Out-Null
    $tmp = Join-Path $toolsDir '_py_tmp'
    if (Test-Path $tmp) { Remove-Item $tmp -Recurse -Force }
    New-Item -ItemType Directory -Force -Path $tmp | Out-Null
    tar -xzf $archive -C $tmp
    $found = Get-ChildItem -Path $tmp -Recurse -Filter 'python.exe' | Select-Object -First 1
    if (-not $found) { throw "python.exe not found inside $($python.File)" }
    Move-Item $found.Directory.FullName (Join-Path $toolsDir 'python')
    Remove-Item $tmp -Recurse -Force
    $sys32 = Join-Path $env:SystemRoot 'System32'
    foreach ($dll in 'vcruntime140.dll','vcruntime140_1.dll','msvcp140.dll') {
        $src = Join-Path $sys32 $dll
        if (Test-Path $src) { Copy-Item $src (Join-Path $toolsDir 'python') -Force }
        else { Write-Warning "VC++ runtime $dll not found - end users may need the MSVC redistributable" }
    }
    # Carry CPython's PSF license text for redistribution.
    New-Item -ItemType Directory -Force -Path $licDir | Out-Null
    $pyLic = Get-ChildItem -Path (Join-Path $toolsDir 'python') -Recurse -Include 'LICENSE*','COPYING*' -File | Select-Object -First 1
    if ($pyLic) { Copy-Item $pyLic.FullName (Join-Path $licDir 'python-LICENSE.txt') -Force }
    else { Write-Warning "python LICENSE not found in the extracted tree" }
}

# --- ffmpeg / ffprobe ---
$ffExe = Join-Path $toolsDir 'ffmpeg.exe'
if ((Test-Path $ffExe) -and -not $Force) {
    Write-Step "tools/ffmpeg.exe already present - skipping (use -Force to refresh)"
} else {
    $archive = Get-Pinned $ffmpeg
    Write-Step "Extracting ffmpeg + ffprobe"
    $tmp = Join-Path $toolsDir '_ff_tmp'
    if (Test-Path $tmp) { Remove-Item $tmp -Recurse -Force }
    Expand-Archive -Path $archive -DestinationPath $tmp -Force
    foreach ($exe in 'ffmpeg.exe','ffprobe.exe') {
        $hit = Get-ChildItem -Path $tmp -Recurse -Filter $exe | Select-Object -First 1
        if (-not $hit) { throw "$exe not found inside $($ffmpeg.File)" }
        Copy-Item $hit.FullName (Join-Path $toolsDir $exe) -Force
    }
    New-Item -ItemType Directory -Force -Path $licDir | Out-Null
    $lic = Get-ChildItem -Path $tmp -Recurse -Include 'LICENSE','LICENSE.txt','COPYING*' -File | Select-Object -First 1
    if ($lic) { Copy-Item $lic.FullName (Join-Path $licDir 'ffmpeg-LICENSE.txt') -Force }
    Remove-Item $tmp -Recurse -Force
}

# The ffmpeg GPLv3 section-6 written source offer is STATIC text with no dependency on
# the extracted archive, so (re)write it on EVERY run - even when the cached binary
# skipped the fetch above - so the shipped license text never goes stale.
New-Item -ItemType Directory -Force -Path $licDir | Out-Null
@"
ffmpeg 8.1.2 - static build from gyan.dev (https://www.gyan.dev/ffmpeg/builds/),
licensed under the GNU General Public License, version 3 (GPLv3). The full license
text is in ffmpeg-LICENSE.txt in this folder.

This portable build redistributes the unmodified ffmpeg 8.1.2 object code. Under
GPLv3 section 6, the complete Corresponding Source for this exact version is the
official FFmpeg 8.1.2 release tarball, permanently archived at:

    https://ffmpeg.org/releases/ffmpeg-8.1.2.tar.xz

together with the build configuration used to produce this static Windows build,
published by the gyan.dev FFmpeg builds project:

    https://github.com/GyanD/codexffmpeg

WRITTEN OFFER: For at least three (3) years from the date you received this build,
the distributor will, on request, provide a complete machine-readable copy of the
Corresponding Source for the ffmpeg 8.1.2 binary included here, for no more than the
cost of physically performing the distribution. The same source is permanently and
freely available at the FFmpeg release URL above.

ffmpeg is invoked only as a separate external process (for PCM-WAV decode/probe);
this application is not linked against it.
"@ | Set-Content (Join-Path $licDir 'ffmpeg-SOURCE-OFFER.txt') -Encoding UTF8

# --- WeiDU (license confirmed: GPLv2 + binary-redistribution grant; -SkipWeidu is an
#     offline/CI escape hatch only) ---
if ($SkipWeidu) {
    Write-Warning "WeiDU fetch skipped (-SkipWeidu) - the portable app and pack ZIPs will lack the bundled installer."
} else {
    $wdExe = Join-Path $toolsDir 'weidu.exe'
    if ((Test-Path $wdExe) -and -not $Force) {
        Write-Step "tools/weidu.exe already present - skipping (use -Force to refresh)"
    } else {
        $archive = Get-Pinned $weidu
        Write-Step "Extracting WeiDU"
        $tmp = Join-Path $toolsDir '_wd_tmp'
        if (Test-Path $tmp) { Remove-Item $tmp -Recurse -Force }
        Expand-Archive -Path $archive -DestinationPath $tmp -Force
        $hit = Get-ChildItem -Path $tmp -Recurse -Filter 'weidu.exe' | Select-Object -First 1
        if (-not $hit) { throw "weidu.exe not found inside $($weidu.File)" }
        Copy-Item $hit.FullName $wdExe -Force
        # Carry WeiDU's GPLv2 license text (COPYING) for redistribution.
        New-Item -ItemType Directory -Force -Path $licDir | Out-Null
        $wdLic = Get-ChildItem -Path $tmp -Recurse -Include 'COPYING*','LICENSE*' -File | Select-Object -First 1
        if ($wdLic) { Copy-Item $wdLic.FullName (Join-Path $licDir 'weidu-COPYING.txt') -Force }
        Remove-Item $tmp -Recurse -Force
    }
    # The binary-redistribution grant is static text; write it every run so it never
    # goes stale even when the cached weidu.exe skipped the fetch above.
    New-Item -ItemType Directory -Force -Path $licDir | Out-Null
    @"
WeiDU 251.00 - the Infinity Engine mod installer (https://weidu.org,
https://github.com/WeiDUorg/weidu), licensed under the GNU General Public License
version 2 (GPLv2). The full license text is in weidu-COPYING.txt in this folder.

REDISTRIBUTION: WeiDU's author expressly permits distributing an unmodified binary
copy of WeiDU.exe (without the source code) together with a mod - the canonical
setup-<mod>.exe pattern used by Infinity Engine mods. This portable build and the
generated per-project pack ZIPs redistribute the unmodified official WeiDU.exe
under that grant. Corresponding Source for this exact version is the WeiDU v251.00
release, permanently available at:

    https://github.com/WeiDUorg/weidu/releases/tag/v251.00
"@ | Set-Content (Join-Path $licDir 'weidu-REDISTRIBUTION.txt') -Encoding UTF8
}

Write-Step "tools/ is ready:"
Get-ChildItem $toolsDir | ForEach-Object { "    $($_.Name)" } | Write-Host
