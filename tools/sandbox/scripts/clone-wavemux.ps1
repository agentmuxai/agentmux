#!/usr/bin/env pwsh
<#
.SYNOPSIS
    Clone and configure AgentMux for sandbox development

.DESCRIPTION
    Clones AgentMux repository, installs dependencies, builds backend,
    and configures for isolated instance operation.

.PARAMETER Branch
    Git branch to clone (default: main)

.PARAMETER TargetDir
    Directory to clone into (default: D:\Code\sandbox\agentmux)

.PARAMETER Force
    Remove existing and re-clone

.PARAMETER Verbose
    Enable verbose output

.EXAMPLE
    pwsh scripts/clone-agentmux.ps1
    Clone main branch to default location

.EXAMPLE
    pwsh scripts/clone-agentmux.ps1 -Branch agentx/feature
    Clone specific branch

.NOTES
    Part of @agentmuxhq/sandbox package

    Exit Codes:
      0 = AgentMux cloned and built successfully
      1 = Warnings during setup
      2 = Setup failed
#>

param(
    [string]$Branch = "main",
    [string]$TargetDir = "$env:USERPROFILE\agentmux",
    [switch]$Force,
    [switch]$Verbose
)

$ErrorActionPreference = "Stop"

# Constants
$AgentMuxRepo = "https://github.com/agentmuxhq/agentmux.git"
$InstanceName = "dev"

function Write-Status {
    param([string]$Message, [string]$Status = "INFO")

    $Color = switch ($Status) {
        "OK" { "Green" }
        "WARN" { "Yellow" }
        "ERROR" { "Red" }
        "SKIP" { "Cyan" }
        default { "White" }
    }

    $Prefix = switch ($Status) {
        "OK" { "[OK]" }
        "WARN" { "[WARN]" }
        "ERROR" { "[ERROR]" }
        "SKIP" { "[SKIP]" }
        default { "[INFO]" }
    }

    Write-Host "$Prefix $Message" -ForegroundColor $Color
}

function Test-Prerequisites {
    Write-Host ""
    Write-Host "=== Checking Prerequisites ===" -ForegroundColor Cyan

    $Missing = @()

    # Git
    if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
        $Missing += "git"
    }

    # Node.js
    if (-not (Get-Command node -ErrorAction SilentlyContinue)) {
        $Missing += "node"
    }

    # npm
    if (-not (Get-Command npm -ErrorAction SilentlyContinue)) {
        $Missing += "npm"
    }

    # Go
    if (-not (Get-Command go -ErrorAction SilentlyContinue)) {
        $Missing += "go"
    }

    # Task
    if (-not (Get-Command task -ErrorAction SilentlyContinue)) {
        $Missing += "task"
    }

    if ($Missing.Count -gt 0) {
        Write-Status "Missing prerequisites: $($Missing -join ', ')" "ERROR"
        Write-Status "Run install-dev-tools.ps1 first" "INFO"
        return $false
    }

    Write-Status "All prerequisites available" "OK"
    return $true
}

function Clone-Repository {
    Write-Host ""
    Write-Host "=== Cloning AgentMux ===" -ForegroundColor Cyan

    # Check if already exists
    if (Test-Path $TargetDir) {
        if ($Force) {
            Write-Status "Removing existing directory..." "WARN"
            Remove-Item -Path $TargetDir -Recurse -Force
        }
        else {
            # Check if it's a valid git repo
            if (Test-Path "$TargetDir\.git") {
                Write-Status "AgentMux already cloned at $TargetDir" "SKIP"

                # Update instead
                Write-Status "Pulling latest changes..." "INFO"
                Push-Location $TargetDir
                try {
                    & git fetch origin
                    & git checkout $Branch 2>$null
                    if ($LASTEXITCODE -ne 0) {
                        # Branch might be remote only
                        & git checkout -b $Branch "origin/$Branch" 2>$null
                    }
                    & git pull origin $Branch
                    Write-Status "Updated to latest $Branch" "OK"
                }
                catch {
                    Write-Status "Could not update: $_" "WARN"
                }
                finally {
                    Pop-Location
                }
                return $true
            }
            else {
                Write-Status "Directory exists but is not a git repo. Use -Force to remove." "ERROR"
                return $false
            }
        }
    }

    # Create parent directory
    $ParentDir = Split-Path -Parent $TargetDir
    if (-not (Test-Path $ParentDir)) {
        New-Item -Path $ParentDir -ItemType Directory -Force | Out-Null
    }

    # Clone
    Write-Status "Cloning $AgentMuxRepo (branch: $Branch)..." "INFO"
    try {
        & git clone --branch $Branch $AgentMuxRepo $TargetDir

        if ($LASTEXITCODE -ne 0) {
            # Branch might not exist, try cloning main first
            Write-Status "Branch $Branch not found, cloning main and checking out..." "WARN"
            & git clone $AgentMuxRepo $TargetDir
            Push-Location $TargetDir
            & git checkout -b $Branch
            Pop-Location
        }

        Write-Status "Cloned AgentMux to $TargetDir" "OK"
        return $true
    }
    catch {
        Write-Status "Failed to clone: $_" "ERROR"
        return $false
    }
}

function Install-Dependencies {
    Write-Host ""
    Write-Host "=== Installing Dependencies ===" -ForegroundColor Cyan

    Push-Location $TargetDir
    try {
        Write-Status "Running npm install..." "INFO"
        & npm install

        if ($LASTEXITCODE -ne 0) {
            Write-Status "npm install failed" "ERROR"
            return $false
        }

        Write-Status "Dependencies installed" "OK"
        return $true
    }
    catch {
        Write-Status "Failed to install dependencies: $_" "ERROR"
        return $false
    }
    finally {
        Pop-Location
    }
}

function Build-Backend {
    Write-Host ""
    Write-Host "=== Building Backend ===" -ForegroundColor Cyan

    Push-Location $TargetDir
    try {
        Write-Status "Running task build:backend..." "INFO"
        & task build:backend

        if ($LASTEXITCODE -ne 0) {
            Write-Status "Backend build failed" "ERROR"
            return $false
        }

        Write-Status "Backend built successfully" "OK"
        return $true
    }
    catch {
        Write-Status "Failed to build backend: $_" "ERROR"
        return $false
    }
    finally {
        Pop-Location
    }
}

function Create-DesktopShortcut {
    Write-Host ""
    Write-Host "=== Creating Desktop Shortcut ===" -ForegroundColor Cyan

    $DesktopPath = [Environment]::GetFolderPath("Desktop")
    $ShortcutPath = "$DesktopPath\AgentMux-Dev.lnk"

    # Find AgentMux executable
    $AgentMuxExe = $null
    $PossiblePaths = @(
        "$TargetDir\make\AgentMux-win32-x64\AgentMux.exe",
        "$TargetDir\dist\AgentMux-win32-x64\AgentMux.exe",
        "$TargetDir\out\AgentMux-win32-x64\AgentMux.exe"
    )

    foreach ($Path in $PossiblePaths) {
        if (Test-Path $Path) {
            $AgentMuxExe = $Path
            break
        }
    }

    if (-not $AgentMuxExe) {
        Write-Status "AgentMux executable not found (not packaged yet)" "WARN"
        Write-Status "Run 'task package' to create executable, then re-run this script" "INFO"

        # Create a shortcut to task dev instead
        Write-Status "Creating shortcut to 'task dev' instead..." "INFO"

        try {
            $WshShell = New-Object -ComObject WScript.Shell
            $Shortcut = $WshShell.CreateShortcut($ShortcutPath)
            $Shortcut.TargetPath = "pwsh"
            $Shortcut.Arguments = "-NoExit -Command `"cd '$TargetDir'; task dev`""
            $Shortcut.WorkingDirectory = $TargetDir
            $Shortcut.Description = "AgentMux Development Server"
            $Shortcut.Save()

            Write-Status "Created 'AgentMux-Dev' shortcut (runs task dev)" "OK"
        }
        catch {
            Write-Status "Could not create shortcut: $_" "WARN"
        }

        return $true
    }

    try {
        $WshShell = New-Object -ComObject WScript.Shell
        $Shortcut = $WshShell.CreateShortcut($ShortcutPath)
        $Shortcut.TargetPath = $AgentMuxExe
        $Shortcut.Arguments = "--instance=$InstanceName"
        $Shortcut.WorkingDirectory = Split-Path -Parent $AgentMuxExe
        $Shortcut.Description = "AgentMux Development Instance"
        $Shortcut.Save()

        Write-Status "Created 'AgentMux-Dev' desktop shortcut" "OK"
    }
    catch {
        Write-Status "Could not create shortcut: $_" "WARN"
    }

    return $true
}

function Initialize-DevInstance {
    Write-Host ""
    Write-Host "=== Initializing Dev Instance ===" -ForegroundColor Cyan

    $InstanceDir = "$env:USERPROFILE\.agentmux-$InstanceName"

    if (Test-Path $InstanceDir) {
        Write-Status "Instance directory already exists: $InstanceDir" "SKIP"
        return $true
    }

    try {
        New-Item -Path $InstanceDir -ItemType Directory -Force | Out-Null
        Write-Status "Created instance directory: $InstanceDir" "OK"

        # Create basic config
        $ConfigPath = "$InstanceDir\config.json"
        @{
            instance = $InstanceName
            version = "dev"
            created = (Get-Date).ToString("o")
        } | ConvertTo-Json | Set-Content -Path $ConfigPath

        Write-Status "Instance configured for isolation" "OK"
    }
    catch {
        Write-Status "Could not initialize instance: $_" "WARN"
    }

    return $true
}

# Main execution
Write-Host "==========================================" -ForegroundColor Cyan
Write-Host "  AgentMux Sandbox Setup" -ForegroundColor Cyan
Write-Host "==========================================" -ForegroundColor Cyan
Write-Host ""
Write-Host "Target: $TargetDir" -ForegroundColor White
Write-Host "Branch: $Branch" -ForegroundColor White
Write-Host "Instance: $InstanceName" -ForegroundColor White
Write-Host ""

$Success = $true

# Check prerequisites
if (-not (Test-Prerequisites)) {
    exit 2
}

# Clone repository
if (-not (Clone-Repository)) {
    exit 2
}

# Install dependencies
if (-not (Install-Dependencies)) {
    $Success = $false
}

# Build backend
if (-not (Build-Backend)) {
    $Success = $false
}

# Create shortcut
Create-DesktopShortcut

# Initialize dev instance
Initialize-DevInstance

# Summary
Write-Host ""
Write-Host "==========================================" -ForegroundColor Cyan
Write-Host "  Setup Complete" -ForegroundColor Cyan
Write-Host "==========================================" -ForegroundColor Cyan
Write-Host ""

if ($Success) {
    Write-Status "AgentMux ready for development!" "OK"
    Write-Host ""
    Write-Host "QUICK START:" -ForegroundColor Yellow
    Write-Host "  cd $TargetDir" -ForegroundColor White
    Write-Host "  task dev                    # Start dev server" -ForegroundColor White
    Write-Host "  agentmux --instance=dev      # Run isolated instance" -ForegroundColor White
    Write-Host ""
    exit 0
}
else {
    Write-Status "Setup completed with warnings" "WARN"
    exit 1
}
