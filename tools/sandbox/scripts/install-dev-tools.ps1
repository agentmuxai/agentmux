#!/usr/bin/env pwsh
<#
.SYNOPSIS
    Install development tools required for WaveMux development

.DESCRIPTION
    Installs Node.js, Go, Zig, Task, Git, and VS Code using winget.
    Checks for existing installations and validates versions.

.PARAMETER Force
    Reinstall even if already present

.PARAMETER Verbose
    Enable verbose output

.EXAMPLE
    pwsh scripts/install-dev-tools.ps1
    Install all development tools

.EXAMPLE
    pwsh scripts/install-dev-tools.ps1 -Force
    Force reinstall all tools

.NOTES
    Part of @agentmuxhq/sandbox package

    Exit Codes:
      0 = All tools installed successfully
      1 = Some tools failed to install
      2 = Critical failure
#>

param(
    [switch]$Force,
    [switch]$Verbose
)

$ErrorActionPreference = "Stop"

# Tool definitions
$Tools = @(
    @{
        Name = "Node.js"
        WingetId = "OpenJS.NodeJS.LTS"
        Command = "node"
        VersionArg = "--version"
        MinVersion = "18.0.0"
        Required = $true
    },
    @{
        Name = "Go"
        WingetId = "GoLang.Go"
        Command = "go"
        VersionArg = "version"
        MinVersion = "1.21.0"
        Required = $true
    },
    @{
        Name = "Git"
        WingetId = "Git.Git"
        Command = "git"
        VersionArg = "--version"
        MinVersion = "2.0.0"
        Required = $true
    },
    @{
        Name = "VS Code"
        WingetId = "Microsoft.VisualStudioCode"
        Command = "code"
        VersionArg = "--version"
        MinVersion = "1.0.0"
        Required = $false
    },
    @{
        Name = "Task"
        WingetId = "Task.Task"
        Command = "task"
        VersionArg = "--version"
        MinVersion = "3.0.0"
        Required = $true
    }
)

# State tracking
$script:Installed = @()
$script:Skipped = @()
$script:Failed = @()

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

function Test-CommandExists {
    param([string]$Command)

    $null = Get-Command $Command -ErrorAction SilentlyContinue
    return $?
}

function Get-ToolVersion {
    param([string]$Command, [string]$VersionArg)

    try {
        $Output = & $Command $VersionArg 2>&1
        # Extract version number (handles various formats)
        if ($Output -match '(\d+\.\d+\.\d+)') {
            return $Matches[1]
        }
        if ($Output -match 'v(\d+\.\d+\.\d+)') {
            return $Matches[1]
        }
        if ($Output -match '(\d+\.\d+)') {
            return "$($Matches[1]).0"
        }
        return $null
    }
    catch {
        return $null
    }
}

function Compare-Versions {
    param([string]$Current, [string]$Minimum)

    try {
        $CurrentParts = $Current.Split('.') | ForEach-Object { [int]$_ }
        $MinimumParts = $Minimum.Split('.') | ForEach-Object { [int]$_ }

        for ($i = 0; $i -lt [Math]::Max($CurrentParts.Length, $MinimumParts.Length); $i++) {
            $C = if ($i -lt $CurrentParts.Length) { $CurrentParts[$i] } else { 0 }
            $M = if ($i -lt $MinimumParts.Length) { $MinimumParts[$i] } else { 0 }

            if ($C -gt $M) { return 1 }
            if ($C -lt $M) { return -1 }
        }
        return 0
    }
    catch {
        return 0
    }
}

function Test-WingetAvailable {
    return Test-CommandExists "winget"
}

function Install-ToolWithWinget {
    param(
        [hashtable]$Tool
    )

    $Name = $Tool.Name
    $WingetId = $Tool.WingetId
    $Command = $Tool.Command
    $VersionArg = $Tool.VersionArg
    $MinVersion = $Tool.MinVersion

    Write-Host ""
    Write-Host "=== $Name ===" -ForegroundColor Cyan

    # Check if already installed
    if (Test-CommandExists $Command) {
        $CurrentVersion = Get-ToolVersion -Command $Command -VersionArg $VersionArg

        if ($CurrentVersion) {
            $Comparison = Compare-Versions -Current $CurrentVersion -Minimum $MinVersion

            if ($Comparison -ge 0) {
                if (-not $Force) {
                    Write-Status "$Name $CurrentVersion already installed (>= $MinVersion)" "SKIP"
                    $script:Skipped += $Name
                    return $true
                }
                Write-Status "$Name $CurrentVersion found, forcing reinstall" "WARN"
            }
            else {
                Write-Status "$Name $CurrentVersion found but < $MinVersion, upgrading" "WARN"
            }
        }
    }

    # Install with winget
    Write-Status "Installing $Name via winget..." "INFO"

    try {
        $InstallArgs = @("install", "--id", $WingetId, "--silent", "--accept-package-agreements", "--accept-source-agreements")

        if ($Verbose) {
            & winget @InstallArgs
        }
        else {
            $null = & winget @InstallArgs 2>&1
        }

        if ($LASTEXITCODE -eq 0 -or $LASTEXITCODE -eq $null) {
            # Refresh PATH
            $env:Path = [System.Environment]::GetEnvironmentVariable("Path", "Machine") + ";" + [System.Environment]::GetEnvironmentVariable("Path", "User")

            # Verify installation
            Start-Sleep -Seconds 2

            if (Test-CommandExists $Command) {
                $NewVersion = Get-ToolVersion -Command $Command -VersionArg $VersionArg
                Write-Status "$Name $NewVersion installed successfully" "OK"
                $script:Installed += $Name
                return $true
            }
            else {
                Write-Status "$Name installed but not in PATH - may need terminal restart" "WARN"
                $script:Installed += $Name
                return $true
            }
        }
        else {
            Write-Status "Failed to install $Name (exit code: $LASTEXITCODE)" "ERROR"
            $script:Failed += $Name
            return $false
        }
    }
    catch {
        Write-Status "Error installing $Name : $_" "ERROR"
        $script:Failed += $Name
        return $false
    }
}

function Install-Zig {
    Write-Host ""
    Write-Host "=== Zig ===" -ForegroundColor Cyan

    # Check if already installed
    if (Test-CommandExists "zig") {
        $ZigVersion = & zig version 2>&1
        if (-not $Force) {
            Write-Status "Zig $ZigVersion already installed" "SKIP"
            $script:Skipped += "Zig"
            return $true
        }
        Write-Status "Zig $ZigVersion found, forcing reinstall" "WARN"
    }

    # Try scoop first
    if (Test-CommandExists "scoop") {
        Write-Status "Installing Zig via scoop..." "INFO"
        try {
            & scoop install zig
            if ($LASTEXITCODE -eq 0) {
                Write-Status "Zig installed via scoop" "OK"
                $script:Installed += "Zig"
                return $true
            }
        }
        catch {
            Write-Status "Scoop install failed, trying direct download" "WARN"
        }
    }

    # Direct download
    Write-Status "Downloading Zig directly..." "INFO"

    $ZigDir = "$env:LOCALAPPDATA\zig"
    $ZigUrl = "https://ziglang.org/download/0.13.0/zig-windows-x86_64-0.13.0.zip"
    $ZipPath = "$env:TEMP\zig.zip"

    try {
        # Download
        Invoke-WebRequest -Uri $ZigUrl -OutFile $ZipPath -UseBasicParsing

        # Extract
        if (Test-Path $ZigDir) {
            Remove-Item -Path $ZigDir -Recurse -Force
        }
        Expand-Archive -Path $ZipPath -DestinationPath "$env:LOCALAPPDATA" -Force
        Rename-Item -Path "$env:LOCALAPPDATA\zig-windows-x86_64-0.13.0" -NewName "zig" -ErrorAction SilentlyContinue

        # Add to PATH
        $UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
        if ($UserPath -notlike "*$ZigDir*") {
            [Environment]::SetEnvironmentVariable("Path", "$UserPath;$ZigDir", "User")
            $env:Path = "$env:Path;$ZigDir"
        }

        # Cleanup
        Remove-Item -Path $ZipPath -Force -ErrorAction SilentlyContinue

        Write-Status "Zig installed to $ZigDir" "OK"
        $script:Installed += "Zig"
        return $true
    }
    catch {
        Write-Status "Failed to install Zig: $_" "ERROR"
        $script:Failed += "Zig"
        return $false
    }
}

# Main execution
Write-Host "==========================================" -ForegroundColor Cyan
Write-Host "  WaveMux Development Tools Installer" -ForegroundColor Cyan
Write-Host "==========================================" -ForegroundColor Cyan
Write-Host ""

# Check winget
if (-not (Test-WingetAvailable)) {
    Write-Status "winget not available - please install App Installer from Microsoft Store" "ERROR"
    exit 2
}

Write-Status "winget available" "OK"

# Install tools via winget
foreach ($Tool in $Tools) {
    Install-ToolWithWinget -Tool $Tool
}

# Install Zig (special case - not in winget)
Install-Zig

# Summary
Write-Host ""
Write-Host "==========================================" -ForegroundColor Cyan
Write-Host "  Installation Summary" -ForegroundColor Cyan
Write-Host "==========================================" -ForegroundColor Cyan

if ($script:Installed.Count -gt 0) {
    Write-Status "Installed: $($script:Installed -join ', ')" "OK"
}

if ($script:Skipped.Count -gt 0) {
    Write-Status "Skipped (already installed): $($script:Skipped -join ', ')" "SKIP"
}

if ($script:Failed.Count -gt 0) {
    Write-Status "Failed: $($script:Failed -join ', ')" "ERROR"

    $RequiredFailed = $script:Failed | Where-Object {
        $Tool = $Tools | Where-Object { $_.Name -eq $_ }
        $Tool.Required -eq $true -or $_ -eq "Zig"
    }

    if ($RequiredFailed.Count -gt 0) {
        Write-Host ""
        Write-Status "Critical tools failed to install. Please install manually." "ERROR"
        exit 2
    }

    exit 1
}

Write-Host ""
Write-Status "All development tools ready!" "OK"
Write-Host ""
Write-Host "NOTE: You may need to restart your terminal for PATH changes to take effect." -ForegroundColor Yellow

exit 0
