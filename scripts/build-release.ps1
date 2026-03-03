# AgentMux Release Build Script
# DEPRECATED: Use 'task package' instead
# This script will be removed in v0.27.0
# Usage: .\scripts\build-release.ps1 [-Clean] [-SkipBackend] [-SkipFrontend] [-SkipPackage]

param(
    [switch]$Clean,
    [switch]$SkipBackend,
    [switch]$SkipFrontend,
    [switch]$SkipPackage
)

$ErrorActionPreference = "Stop"

Write-Warning "DEPRECATED: This script is deprecated and will be removed in v0.27.0"
Write-Host "Please use 'task package' instead for production builds" -ForegroundColor Yellow
Write-Host "Or 'task package:portable' for portable builds" -ForegroundColor Yellow
Write-Host ""
Write-Host "Continuing with legacy build script..." -ForegroundColor Gray
Write-Host ""
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ProjectDir = Split-Path -Parent $ScriptDir

Set-Location $ProjectDir

# Get version from package.json
$PackageJson = Get-Content package.json | ConvertFrom-Json
$Version = $PackageJson.version

Write-Host ""
Write-Host "========================================" -ForegroundColor Cyan
Write-Host "  AgentMux Build v$Version" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan
Write-Host ""

# Step 1: Clean (if requested)
if ($Clean) {
    Write-Host "[1/6] Cleaning stale artifacts..." -ForegroundColor Yellow

    # Kill running processes (ignore if not running)
    $ErrorActionPreference = "SilentlyContinue"
    taskkill /F /IM AgentMux.exe 2>$null | Out-Null
    taskkill /F /IM agentmuxsrv.x64.exe 2>$null | Out-Null
    $ErrorActionPreference = "Stop"

    # Remove stale directories
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue make
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue .task
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue dist/bin

    # Remove stale lock files
    Remove-Item -Force -ErrorAction SilentlyContinue node_modules/electron/dist/wave-data/wave.lock
    Remove-Item -Force -ErrorAction SilentlyContinue node_modules/electron/dist/wave-data/wave.sock

    Write-Host "  Cleaned." -ForegroundColor Green
} else {
    Write-Host "[1/6] Clean skipped (use -Clean to force)" -ForegroundColor DarkGray
}

# Step 2: Build Backend
if (-not $SkipBackend) {
    Write-Host "[2/6] Building backend..." -ForegroundColor Yellow

    # Clear task cache to force rebuild
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue .task

    task build:backend
    if ($LASTEXITCODE -ne 0) {
        Write-Host "  Backend build FAILED" -ForegroundColor Red
        exit 1
    }
    Write-Host "  Backend built." -ForegroundColor Green
} else {
    Write-Host "[2/6] Backend skipped" -ForegroundColor DarkGray
}

# Step 3: Build Frontend
if (-not $SkipFrontend) {
    Write-Host "[3/6] Building frontend..." -ForegroundColor Yellow
    npm run build:prod
    if ($LASTEXITCODE -ne 0) {
        Write-Host "  Frontend build FAILED" -ForegroundColor Red
        exit 1
    }
    Write-Host "  Frontend built." -ForegroundColor Green
} else {
    Write-Host "[3/6] Frontend skipped" -ForegroundColor DarkGray
}

# Step 4: Version Verification
Write-Host "[4/6] Verifying versions..." -ForegroundColor Yellow

$Errors = @()

# Check wsh binary exists with correct version
$WshBinary = "dist/bin/wsh-$Version-windows.x64.exe"
if (-not (Test-Path $WshBinary)) {
    $Errors += "wsh binary not found: $WshBinary"
} else {
    # Verify wsh reports correct version
    $WshVersion = & $WshBinary version 2>&1
    $ExpectedWshVersion = "wsh v$Version"
    if ($WshVersion -ne $ExpectedWshVersion) {
        $Errors += "wsh version mismatch: got '$WshVersion', expected '$ExpectedWshVersion'"
    }
}

# Check agentmuxsrv exists
$SrvBinary = "dist/bin/agentmuxsrv.x64.exe"
if (-not (Test-Path $SrvBinary)) {
    $Errors += "agentmuxsrv binary not found: $SrvBinary"
}

# Check all wsh platform variants exist
$Platforms = @(
    "darwin.arm64", "darwin.x64",
    "linux.arm64", "linux.x64", "linux.mips", "linux.mips64",
    "windows.x64.exe", "windows.arm64.exe"
)
foreach ($Platform in $Platforms) {
    $Binary = "dist/bin/wsh-$Version-$Platform"
    if (-not (Test-Path $Binary)) {
        $Errors += "Missing wsh binary: $Binary"
    }
}

# Check frontend build exists
if (-not (Test-Path "dist/frontend/index.html")) {
    $Errors += "Frontend build missing: dist/frontend/index.html"
}

# Check main process build exists
if (-not (Test-Path "dist/main/index.js")) {
    $Errors += "Main process build missing: dist/main/index.js"
}

# Report errors
if ($Errors.Count -gt 0) {
    Write-Host ""
    Write-Host "  VERSION VERIFICATION FAILED:" -ForegroundColor Red
    foreach ($Error in $Errors) {
        Write-Host "    - $Error" -ForegroundColor Red
    }
    Write-Host ""
    exit 1
}

Write-Host "  All versions verified: v$Version" -ForegroundColor Green

# Step 5: Package
if (-not $SkipPackage) {
    Write-Host "[5/6] Packaging with electron-builder..." -ForegroundColor Yellow
    npm exec electron-builder -- -c electron-builder.config.cjs -p never --win dir
    if ($LASTEXITCODE -ne 0) {
        Write-Host "  Packaging FAILED" -ForegroundColor Red
        exit 1
    }
    Write-Host "  Packaged." -ForegroundColor Green
} else {
    Write-Host "[5/6] Packaging skipped" -ForegroundColor DarkGray
}

# Step 6: Final verification
Write-Host "[6/6] Final verification..." -ForegroundColor Yellow

if (-not $SkipPackage) {
    $ExePath = "make/win-unpacked/AgentMux.exe"
    if (-not (Test-Path $ExePath)) {
        Write-Host "  Final exe not found: $ExePath" -ForegroundColor Red
        exit 1
    }
    Write-Host "  Output: $ExePath" -ForegroundColor Green
}

# Success
Write-Host ""
Write-Host "========================================" -ForegroundColor Green
Write-Host "  BUILD SUCCESS - v$Version" -ForegroundColor Green
Write-Host "========================================" -ForegroundColor Green
Write-Host ""

if (-not $SkipPackage) {
    Write-Host "To deploy to desktop:" -ForegroundColor Cyan
    Write-Host "  xcopy /E /Y /I make\win-unpacked %USERPROFILE%\Desktop\AgentMux-$Version\" -ForegroundColor White
    Write-Host ""
}
