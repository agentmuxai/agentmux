# Package AgentMux CEF as a portable directory + ZIP (Windows x64).
# Usage: pwsh scripts/package-cef-portable.ps1 [-OutputDir <path>]
#
# Default output: ~/Desktop/agentmux-cef-{version}-x64-portable/
#
# Exits with code 1 on any error. All steps are logged — no silent failures.

param(
    [string]$OutputDir = (Join-Path $HOME "Desktop")
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# ── Helpers ──────────────────────────────────────────────────────────────────

function Log-Step  { param([string]$msg) Write-Host "  → $msg" }
function Log-Ok    { param([string]$msg) Write-Host "  ✓ $msg" -ForegroundColor Green }
function Log-Warn  { param([string]$msg) Write-Host "  ⚠ $msg" -ForegroundColor Yellow }
function Log-Error { param([string]$msg) Write-Host "  ✗ $msg" -ForegroundColor Red }

function Assert-File {
    param([string]$Path, [string]$Label)
    if (-not (Test-Path $Path)) {
        Log-Error "Required file not found: $Label"
        Log-Error "  Expected at: $Path"
        Log-Error "  Run 'task build:backend' and 'task cef:build' first."
        exit 1
    }
    Log-Ok "Found: $Label"
}

function Get-VersionFromBinary {
    param([string]$Path, [string]$Version)
    $bytes = [System.IO.File]::ReadAllBytes($Path)
    $ascii = [System.Text.Encoding]::ASCII.GetString($bytes)
    if ($ascii -match [regex]::Escape($Version)) { return $Version }
    return $null
}

function Copy-Required {
    param([string]$Src, [string]$Dst, [string]$Label)
    if (-not (Test-Path $Src)) {
        Log-Error "Cannot copy $Label — source not found: $Src"
        exit 1
    }
    Copy-Item $Src $Dst -Force
    Log-Ok "Copied: $Label"
}

function Copy-Optional {
    param([string]$Src, [string]$Dst, [string]$Label)
    if (Test-Path $Src) {
        Copy-Item $Src $Dst -Force
        Log-Ok "Copied: $Label"
    } else {
        Log-Warn "Optional file not found (skipped): $Label"
    }
}

# ── Setup ────────────────────────────────────────────────────────────────────

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$Base      = Split-Path -Parent $ScriptDir

Push-Location $Base
try {

$VERSION = (Get-Content "$Base\package.json" -Raw | ConvertFrom-Json).version
if (-not $VERSION) {
    Log-Error "Could not read version from package.json"
    exit 1
}

$PORTABLE = Join-Path $OutputDir "agentmux-cef-$VERSION-x64-portable"
$ZIPPATH  = Join-Path $OutputDir "agentmux-cef-$VERSION-x64-portable.zip"

Write-Host ""
Write-Host "AgentMux CEF Portable Builder" -ForegroundColor Cyan
Write-Host "  Version : $VERSION"
Write-Host "  Output  : $PORTABLE"
Write-Host ""

# ── 1. Verify required source files ──────────────────────────────────────────

Write-Host "[1/6] Verifying source files..."
Assert-File "$Base\target\release\agentmux-cef.exe"        "agentmux-cef.exe"
Assert-File "$Base\target\release\agentmux-launcher.exe"   "agentmux-launcher.exe"
Assert-File "$Base\dist\bin\agentmuxsrv-rs.x64.exe"        "agentmuxsrv-rs.x64.exe"
Assert-File "$Base\dist\frontend\index.html"               "dist/frontend/index.html"
Assert-File "$Base\dist\cef\libcef.dll"                    "dist/cef/libcef.dll"

$wshPath = "$Base\dist\bin\wsh-$VERSION-windows.x64.exe"
if (Test-Path $wshPath) {
    Log-Ok "Found: wsh-$VERSION-windows.x64.exe"
} else {
    Log-Warn "wsh-$VERSION-windows.x64.exe not found — wsh will be absent from portable"
    Log-Warn "  Run 'task build:wsh' to include it"
}

# ── 2. Verify embedded versions in binaries ───────────────────────────────────

Write-Host ""
Write-Host "[2/6] Verifying embedded versions in binaries..."

$cefVer = Get-VersionFromBinary "$Base\target\release\agentmux-cef.exe" $VERSION
if (-not $cefVer) {
    Log-Error "agentmux-cef.exe does not contain version $VERSION"
    Log-Error "  Rebuild with: cargo build --release -p agentmux-cef"
    exit 1
}
Log-Ok "agentmux-cef.exe contains $VERSION"

$srvVer = Get-VersionFromBinary "$Base\dist\bin\agentmuxsrv-rs.x64.exe" $VERSION
if (-not $srvVer) {
    Log-Error "agentmuxsrv-rs.x64.exe does not contain version $VERSION"
    Log-Error "  Rebuild with: task build:backend"
    exit 1
}
Log-Ok "agentmuxsrv-rs.x64.exe contains $VERSION"

# agentmux-launcher is a tiny pass-through binary (sets DLL path, spawns CEF host).
# It intentionally embeds no version string — no check needed.

# ── 3. Create directory structure ─────────────────────────────────────────────

Write-Host ""
Write-Host "[3/6] Creating portable directory..."

if (Test-Path $PORTABLE) {
    Log-Step "Removing previous: $PORTABLE"
    Remove-Item $PORTABLE -Recurse -Force
}
if (Test-Path $ZIPPATH) {
    Log-Step "Removing previous ZIP"
    Remove-Item $ZIPPATH -Force
}

New-Item -ItemType Directory -Force -Path "$PORTABLE\runtime\locales"  | Out-Null
New-Item -ItemType Directory -Force -Path "$PORTABLE\runtime\frontend" | Out-Null
Log-Ok "Created directory structure"

# ── 4. Copy files ─────────────────────────────────────────────────────────────

Write-Host ""
Write-Host "[4/6] Copying files..."

# Launcher → root as agentmux.exe (user-facing entry point)
Copy-Required "$Base\target\release\agentmux-launcher.exe" "$PORTABLE\agentmux.exe"     "launcher → agentmux.exe"

# Runtime binaries
Copy-Required "$Base\target\release\agentmux-cef.exe"      "$PORTABLE\runtime\"         "agentmux-cef.exe"
Copy-Required "$Base\dist\bin\agentmuxsrv-rs.x64.exe"      "$PORTABLE\runtime\"         "agentmuxsrv-rs.x64.exe"

# wsh (optional)
if (Test-Path $wshPath) {
    Copy-Item $wshPath "$PORTABLE\runtime\wsh.exe" -Force
    Log-Ok "Copied: wsh.exe"
}

# Frontend
Copy-Item "$Base\dist\frontend\*" "$PORTABLE\runtime\frontend\" -Recurse -Force
Log-Ok "Copied: dist/frontend → runtime/frontend/"

# CEF core (required)
Copy-Required "$Base\dist\cef\libcef.dll"  "$PORTABLE\runtime\" "libcef.dll"

# CEF optional runtime files
Copy-Optional "$Base\dist\cef\chrome_elf.dll"           "$PORTABLE\runtime\" "chrome_elf.dll"
Copy-Optional "$Base\dist\cef\icudtl.dat"               "$PORTABLE\runtime\" "icudtl.dat"
Copy-Optional "$Base\dist\cef\v8_context_snapshot.bin"  "$PORTABLE\runtime\" "v8_context_snapshot.bin"
Copy-Optional "$Base\dist\cef\libEGL.dll"               "$PORTABLE\runtime\" "libEGL.dll"
Copy-Optional "$Base\dist\cef\libGLESv2.dll"            "$PORTABLE\runtime\" "libGLESv2.dll"
Copy-Optional "$Base\dist\cef\d3dcompiler_47.dll"       "$PORTABLE\runtime\" "d3dcompiler_47.dll"
Copy-Optional "$Base\dist\cef\chrome_100_percent.pak"   "$PORTABLE\runtime\" "chrome_100_percent.pak"
Copy-Optional "$Base\dist\cef\chrome_200_percent.pak"   "$PORTABLE\runtime\" "chrome_200_percent.pak"
Copy-Optional "$Base\dist\cef\resources.pak"            "$PORTABLE\runtime\" "resources.pak"
Copy-Optional "$Base\dist\cef\locales\en-US.pak"        "$PORTABLE\runtime\locales\" "locales/en-US.pak"

# README
@"
AgentMux v$VERSION - Portable Edition

Quick Start:
  1. Extract this folder (or ZIP) anywhere
  2. Run agentmux.exe

Requirements:
  - Windows 10/11 x64
  - No installation needed
  - No admin rights required
"@ | Set-Content "$PORTABLE\README.txt" -Encoding UTF8
Log-Ok "Created README.txt"

# ── 5. Size report ────────────────────────────────────────────────────────────

Write-Host ""
Write-Host "[5/6] Computing size..."
$dirBytes = (Get-ChildItem $PORTABLE -Recurse -File | Measure-Object -Property Length -Sum).Sum
$dirMB    = [math]::Round($dirBytes / 1MB, 1)
Log-Ok "Directory: $dirMB MB"

# ── 6. ZIP ────────────────────────────────────────────────────────────────────

Write-Host ""
Write-Host "[6/6] Creating ZIP..."
Log-Step "Compressing to $ZIPPATH"
Compress-Archive -Path "$PORTABLE\*" -DestinationPath $ZIPPATH -Force
$zipMB = [math]::Round((Get-Item $ZIPPATH).Length / 1MB, 1)
Log-Ok "ZIP: $zipMB MB"

# ── Done ──────────────────────────────────────────────────────────────────────

Write-Host ""
Write-Host "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━" -ForegroundColor Cyan
Write-Host " SUCCESS  AgentMux CEF Portable v$VERSION" -ForegroundColor Green
Write-Host "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━" -ForegroundColor Cyan
Write-Host "  Directory : $PORTABLE ($dirMB MB)"
Write-Host "  ZIP       : $ZIPPATH ($zipMB MB)"
Write-Host ""

} finally {
    Pop-Location
}
