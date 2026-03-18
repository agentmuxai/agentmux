#!/usr/bin/env pwsh
<#
.SYNOPSIS
  Package AgentMux as an MSIX for Windows Store submission.

.DESCRIPTION
  Creates an MSIX package from the Tauri release build artifacts.
  Run `task package` first to build the application, then run this script.

  The resulting .msix can be submitted directly to the Windows Store via
  Microsoft Partner Center. Microsoft re-signs the package during ingestion,
  so self-signed test certificates are fine for submission.

.PARAMETER Publisher
  Publisher CN from Microsoft Partner Center.
  Format: "CN=XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX"
  Find it: Partner Center → Account settings → Legal info → Publisher ID
  Default: "CN=AgentMux Corp" (placeholder — update before Store submission)

.PARAMETER OutputDir
  Directory for the output .msix file. Default: dist/msix

.PARAMETER SkipBuild
  Skip the `task package` step (use if you already ran it).

.EXAMPLE
  # First build:
  task package

  # Then package for Store:
  pwsh -File scripts/package-msix.ps1 -Publisher "CN=XXXXXXXX..."

.EXAMPLE
  # Build + package in one go:
  pwsh -File scripts/package-msix.ps1
#>
param(
  [string]$Publisher  = "CN=AgentMux Corp",
  [string]$OutputDir  = "dist\msix",
  [switch]$SkipBuild  = $false
)

$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot\..

# ── Version ──────────────────────────────────────────────────────────────────
$semver = node version.cjs
# MSIX requires 4-part version (major.minor.patch.0)
$msixVersion = "$semver.0"

Write-Host ""
Write-Host "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
Write-Host "  AgentMux MSIX Packager  v$semver"
Write-Host "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
Write-Host ""

# ── makeappx location ────────────────────────────────────────────────────────
$sdkBase = "C:\Program Files (x86)\Windows Kits\10\bin"
$makeappx = Get-ChildItem "$sdkBase\*\x64\makeappx.exe" |
  Sort-Object { [version]($_.FullName -replace '^.*\\(\d+\.\d+\.\d+\.\d+)\\.*$','$1') } |
  Select-Object -Last 1 -ExpandProperty FullName

if (-not $makeappx) {
  Write-Error "makeappx.exe not found. Install the Windows 10 SDK."
}
Write-Host "  makeappx : $makeappx"

# ── Locate Tauri release dir ─────────────────────────────────────────────────
$releaseDir = "src-tauri\target\release"
$mainExe    = "$releaseDir\agentmux.exe"

if (-not (Test-Path $mainExe)) {
  if ($SkipBuild) {
    Write-Error "agentmux.exe not found at $mainExe. Run 'task package' first."
  }
  Write-Host "  Build not found — running 'task package'..."
  task package
}

Write-Host "  release  : $releaseDir"
Write-Host "  version  : $msixVersion"
Write-Host "  publisher: $Publisher"
Write-Host ""

# ── Staging directory ─────────────────────────────────────────────────────────
$stagingDir = "$OutputDir\staging"
if (Test-Path $stagingDir) { Remove-Item $stagingDir -Recurse -Force }
New-Item -ItemType Directory -Force -Path $stagingDir | Out-Null

Write-Host "[1/5] Copying application binaries..."

# Main Tauri binary
Copy-Item "$mainExe" "$stagingDir\agentmux.exe" -Force

# WebView2 loader (bundled by Tauri)
$wv2 = "$releaseDir\WebView2Loader.dll"
if (Test-Path $wv2) { Copy-Item $wv2 "$stagingDir\" -Force }

# Sidecar binaries — Tauri strips the target triple when bundling
$sidecars = @(
  @{ src = "$releaseDir\agentmuxsrv-rs-x86_64-pc-windows-msvc.exe"; dst = "agentmuxsrv-rs.exe" },
  @{ src = "$releaseDir\wsh-x86_64-pc-windows-msvc.exe";            dst = "wsh.exe" }
)
foreach ($s in $sidecars) {
  if (Test-Path $s.src) {
    Copy-Item $s.src "$stagingDir\$($s.dst)" -Force
    Write-Host "  + $($s.dst)"
  }
}

# Resources (schema JSON files, wsh bin/* for backend deployment)
$resourcesSrc = "$releaseDir\resources"
if (Test-Path $resourcesSrc) {
  Copy-Item $resourcesSrc "$stagingDir\resources" -Recurse -Force
  Write-Host "  + resources\"
} else {
  # Fallback: copy schema directly from dist/schema
  if (Test-Path "dist\schema") {
    New-Item -ItemType Directory -Force -Path "$stagingDir\resources\schema" | Out-Null
    Copy-Item "dist\schema\*" "$stagingDir\resources\schema\" -Force
    Write-Host "  + resources\schema\ (from dist/schema)"
  }
}

Write-Host ""
Write-Host "[2/5] Copying Store assets..."

$assetsDir = "$stagingDir\Assets"
New-Item -ItemType Directory -Force -Path $assetsDir | Out-Null

$icons = @(
  "StoreLogo.png",
  "Square44x44Logo.png",
  "Square71x71Logo.png",
  "Square107x107Logo.png",
  "Square142x142Logo.png",
  "Square150x150Logo.png",
  "Square284x284Logo.png",
  "Square310x310Logo.png",
  "Wide310x150Logo.png"
)
foreach ($ico in $icons) {
  $src = "src-tauri\icons\$ico"
  if (Test-Path $src) {
    Copy-Item $src "$assetsDir\$ico" -Force
    Write-Host "  + Assets\$ico"
  } else {
    Write-Warning "  ! Missing: $src"
  }
}

Write-Host ""
Write-Host "[3/5] Writing AppxManifest.xml..."

# Read template manifest and patch version + publisher
$manifest = Get-Content "src-tauri\AppxManifest.xml" -Raw

# Patch version
$manifest = $manifest -replace '(?<=<Identity[^>]*Version=")[^"]*(?=")', $msixVersion

# Patch publisher
$manifest = $manifest -replace '(?<=<Identity[^>]*Publisher=")[^"]*(?=")', $Publisher

$manifest | Set-Content "$stagingDir\AppxManifest.xml" -Encoding UTF8
Write-Host "  Version  : $msixVersion"
Write-Host "  Publisher: $Publisher"

Write-Host ""
Write-Host "[4/5] Packing MSIX..."

$outputFile = "$OutputDir\AgentMux_${semver}_x64.msix"
New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null

& $makeappx pack /d $stagingDir /p $outputFile /overwrite /nv
if ($LASTEXITCODE -ne 0) {
  Write-Error "makeappx failed with exit code $LASTEXITCODE"
}
Write-Host "  + $outputFile"

# Clean up staging
Remove-Item $stagingDir -Recurse -Force

Write-Host ""
$size = [math]::Round((Get-Item $outputFile).Length / 1MB, 1)
Write-Host "[5/5] Done!"
Write-Host ""
Write-Host "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
Write-Host "  MSIX: $outputFile  ($size MB)"
Write-Host "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
Write-Host ""
Write-Host "  Next steps:"
Write-Host "  1. Sign the package (for local testing only — Store re-signs on ingest):"
Write-Host "       signtool sign /fd sha256 /a $outputFile"
Write-Host "  2. Upload to Partner Center:"
Write-Host "       https://partner.microsoft.com → Apps → AgentMux → Submission"
Write-Host ""

if ($Publisher -eq "CN=AgentMux Corp") {
  Write-Host ""
  Write-Warning "  Publisher is still the placeholder 'CN=AgentMux Corp'."
  Write-Host "  Update with your actual Partner Center Publisher ID before submitting."
  Write-Host "  Pass it via: pwsh -File scripts/package-msix.ps1 -Publisher 'CN=...'"
  Write-Host ""
}
