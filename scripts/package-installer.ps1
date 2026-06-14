#!/usr/bin/env pwsh
<#
.SYNOPSIS
  Build the AgentMux Windows .exe installer (Inno Setup) from a portable build.

.DESCRIPTION
  Native-CEF Windows installer (the Tauri-era tauri-action installer is gone).
  Wraps `packaging/windows/agentmux.iss`: resolves a portable build, reads the
  version from package.json, and runs ISCC with the right defines. Output is a
  per-user `AgentMux-<ver>-x64-setup.exe`.

  Get a portable first with `task package:release` (or `task package`). By default
  this picks the newest `agentmux-*-x64-portable` folder on the Desktop.

.PARAMETER PortableDir  Portable folder to package. Default: newest on the Desktop.
.PARAMETER OutputDir    Where to write the setup .exe. Default: the Desktop.

.EXAMPLE  pwsh -File scripts/package-installer.ps1
.EXAMPLE  pwsh -File scripts/package-installer.ps1 -PortableDir C:\path\to\portable
#>
param(
  [string]$PortableDir = "",
  [string]$OutputDir   = ""
)
$ErrorActionPreference = "Stop"
$repo    = Split-Path -Parent $PSScriptRoot
$desktop = [Environment]::GetFolderPath("Desktop")
if (-not $OutputDir) { $OutputDir = $desktop }

Write-Host "`n=== AgentMux Windows installer (Inno Setup) ===`n"

# ── locate ISCC ──────────────────────────────────────────────────────────────
$iscc = (Get-Command iscc -ErrorAction SilentlyContinue).Source
if (-not $iscc) {
  $iscc = Get-ChildItem @(
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "${env:ProgramFiles}\Inno Setup 6\ISCC.exe",
    "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe"
  ) -ErrorAction SilentlyContinue | Select-Object -First 1 -ExpandProperty FullName
}
if (-not $iscc) {
  throw "ISCC.exe (Inno Setup 6) not found. Install it from https://jrsoftware.org/isdl.php (or: winget install JRSoftware.InnoSetup)."
}
Write-Host "  iscc     : $iscc"

# ── version (from package.json) ──────────────────────────────────────────────
$version = (Get-Content -Raw (Join-Path $repo "package.json") | ConvertFrom-Json).version
Write-Host "  version  : $version"

# ── resolve portable dir ─────────────────────────────────────────────────────
if (-not $PortableDir) {
  $PortableDir = Get-ChildItem $desktop -Directory -Filter "agentmux-*-x64-portable" -ErrorAction SilentlyContinue |
                 Sort-Object LastWriteTime -Descending | Select-Object -First 1 -ExpandProperty FullName
}
if (-not $PortableDir -or -not (Test-Path $PortableDir)) {
  throw "No portable folder found. Run `task package:release` first (or pass -PortableDir)."
}
if (-not (Test-Path (Join-Path $PortableDir "agentmux.exe")))       { throw "portable missing agentmux.exe: $PortableDir" }
if (-not (Test-Path (Join-Path $PortableDir "runtime\libcef.dll"))) { throw "portable missing runtime\libcef.dll: $PortableDir" }
Write-Host "  portable : $PortableDir"
Write-Host "  output   : $OutputDir`n"

# ── build ────────────────────────────────────────────────────────────────────
$iss = Join-Path $repo "packaging\windows\agentmux.iss"
& $iscc /Qp `
  "/DAppVersion=$version" `
  "/DSourceDir=$PortableDir" `
  "/DOutputDir=$OutputDir" `
  $iss
if ($LASTEXITCODE -ne 0) { throw "ISCC failed (exit $LASTEXITCODE)" }

$setup = Join-Path $OutputDir "AgentMux-$version-x64-setup.exe"
if (-not (Test-Path $setup)) { throw "expected installer not produced: $setup" }
$mb = [math]::Round((Get-Item $setup).Length / 1MB, 1)
Write-Host "`n[SUCCESS] $setup ($mb MB)"
