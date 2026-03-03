# Package portable build for AgentMux
param(
    [string]$Version = "",
    [string]$OutputDir = "dist"
)

$ErrorActionPreference = "Stop"

# Get version from package.json if not provided
if ($Version -eq "") {
    $PackageJson = Get-Content "$PSScriptRoot\..\package.json" -Raw | ConvertFrom-Json
    $Version = $PackageJson.version
}

$RepoRoot = Split-Path $PSScriptRoot
$BuildDir = "$RepoRoot\target\release"
$PortableDir = "$OutputDir\agentmux-$Version-x64-portable"
$ZipPath = "$OutputDir\agentmux-$Version-x64-portable.zip"

Write-Host "Packaging AgentMux $Version Portable Build..." -ForegroundColor Cyan

# Verify binaries exist
$RequiredFiles = @(
    "$BuildDir\agentmux.exe",
    "$RepoRoot\dist\bin\agentmuxsrv-rs.x64.exe",
    "$RepoRoot\dist\bin\wsh-$Version-windows.x64.exe"
)

foreach ($File in $RequiredFiles) {
    if (-not (Test-Path $File)) {
        throw "Required file not found: $File. Run build first."
    }
}

# Clean previous build
if (Test-Path $PortableDir) {
    Remove-Item $PortableDir -Recurse -Force
}
if (Test-Path $ZipPath) {
    Remove-Item $ZipPath -Force
}

# Create output directory
if (-not (Test-Path $OutputDir)) {
    New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null
}

# Create portable directory
New-Item -ItemType Directory -Force -Path $PortableDir | Out-Null
New-Item -ItemType Directory -Force -Path "$PortableDir\bin" | Out-Null

# Copy main executable
Write-Host "  Copying agentmux.exe..." -ForegroundColor Gray
Copy-Item "$BuildDir\agentmux.exe" "$PortableDir\agentmux.exe" -Force

# Copy backend
Write-Host "  Copying agentmuxsrv-rs.x64.exe..." -ForegroundColor Gray
Copy-Item "$RepoRoot\dist\bin\agentmuxsrv-rs.x64.exe" "$PortableDir\agentmuxsrv-rs.x64.exe" -Force

# Copy wsh to bin subdirectory (backend expects it there)
Write-Host "  Copying wsh-$Version-windows.x64.exe to bin/..." -ForegroundColor Gray
Copy-Item "$RepoRoot\dist\bin\wsh-$Version-windows.x64.exe" "$PortableDir\bin\wsh-$Version-windows.x64.exe" -Force

# Create README
Write-Host "  Creating README.txt..." -ForegroundColor Gray
$ReadmeContent = @"
AgentMux v$Version - Portable Edition

Quick Start:
1. Extract this ZIP to any folder
2. Run agentmux.exe
3. Data will be stored in the Data subfolder

Requirements:
- Windows 10/11 x64
- No installation needed
- No admin rights required

Files:
- agentmux.exe: Main application (Tauri frontend)
- agentmuxsrv-rs.x64.exe: Backend server (auto-launched)
- bin/wsh-$Version-windows.x64.exe: Shell integration binary

Support: https://github.com/agentmuxhq/agentmux

Build Date: $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')
"@
$ReadmeContent | Out-File "$PortableDir\README.txt" -Encoding UTF8

# Get file sizes before cleanup
$ExeSize = (Get-Item "$PortableDir\agentmux.exe").Length / 1MB
$BackendSize = (Get-Item "$PortableDir\agentmuxsrv-rs.x64.exe").Length / 1MB
$WshSize = (Get-Item "$PortableDir\bin\wsh-$Version-windows.x64.exe").Length / 1MB

# Create ZIP
Write-Host "  Creating ZIP archive..." -ForegroundColor Gray
Compress-Archive -Path "$PortableDir\*" -DestinationPath $ZipPath -Force
$ZipSize = (Get-Item $ZipPath).Length / 1MB

# Cleanup temp directory
Remove-Item $PortableDir -Recurse -Force

# Show results
Write-Host ""
Write-Host "[SUCCESS] Portable build created:" -ForegroundColor Green
Write-Host "  File: $ZipPath" -ForegroundColor White
Write-Host "  Size: $([math]::Round($ZipSize, 2)) MB (compressed)" -ForegroundColor White
Write-Host ""
Write-Host "  Contents:" -ForegroundColor Gray
Write-Host "    agentmux.exe:            $([math]::Round($ExeSize, 2)) MB" -ForegroundColor Gray
Write-Host "    agentmuxsrv-rs.x64.exe:  $([math]::Round($BackendSize, 2)) MB" -ForegroundColor Gray
Write-Host "    wsh-$Version-*.exe:      $([math]::Round($WshSize, 2)) MB" -ForegroundColor Gray
Write-Host "    README.txt" -ForegroundColor Gray
Write-Host ""
Write-Host "Extract and run agentmux.exe to test." -ForegroundColor Cyan
