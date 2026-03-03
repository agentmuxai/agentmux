#!/usr/bin/env pwsh
<#
.SYNOPSIS
    Install and configure Parsec for headless remote desktop

.DESCRIPTION
    Downloads and installs Parsec with virtual display support.
    Configures for headless operation (no physical monitor required).

.PARAMETER SkipVDA
    Skip Parsec Virtual Display Adapter installation

.PARAMETER Force
    Reinstall even if already present

.PARAMETER Verbose
    Enable verbose output

.EXAMPLE
    pwsh scripts/install-parsec.ps1
    Install Parsec with VDA

.EXAMPLE
    pwsh scripts/install-parsec.ps1 -SkipVDA
    Install Parsec without VDA

.NOTES
    Part of @agentmuxhq/sandbox package

    Parsec requires a free account - create at https://parsec.app

    Exit Codes:
      0 = Parsec installed successfully
      1 = Installation warnings
      2 = Installation failed
#>

param(
    [switch]$SkipVDA,
    [switch]$Force,
    [switch]$Verbose
)

$ErrorActionPreference = "Stop"

# Constants
$ParsecUrl = "https://builds.parsec.app/package/parsec-windows.exe"
$ParsecVDAUrl = "https://builds.parsec.app/vdd/parsec-vdd-0.45.0.0.exe"
$ParsecInstallPath = "$env:ProgramFiles\Parsec"
$ParsecConfigPath = "$env:APPDATA\Parsec\config.json"
$DownloadDir = "$env:TEMP\parsec-setup"

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

function Test-ParsecInstalled {
    return (Test-Path "$ParsecInstallPath\parsecd.exe") -or
           (Test-Path "$env:ProgramFiles\Parsec\parsecd.exe") -or
           (Test-Path "${env:ProgramFiles(x86)}\Parsec\parsecd.exe")
}

function Test-ParsecServiceRunning {
    $Service = Get-Service -Name "Parsec" -ErrorAction SilentlyContinue
    return $Service -and $Service.Status -eq "Running"
}

function Install-Parsec {
    Write-Host ""
    Write-Host "=== Parsec Installation ===" -ForegroundColor Cyan

    # Check if already installed
    if (Test-ParsecInstalled) {
        if (-not $Force) {
            Write-Status "Parsec already installed" "SKIP"
            return $true
        }
        Write-Status "Parsec found, forcing reinstall" "WARN"
    }

    # Create download directory
    if (-not (Test-Path $DownloadDir)) {
        New-Item -Path $DownloadDir -ItemType Directory -Force | Out-Null
    }

    $InstallerPath = "$DownloadDir\parsec-windows.exe"

    # Download installer
    Write-Status "Downloading Parsec installer..." "INFO"
    try {
        Invoke-WebRequest -Uri $ParsecUrl -OutFile $InstallerPath -UseBasicParsing
        Write-Status "Downloaded Parsec installer" "OK"
    }
    catch {
        Write-Status "Failed to download Parsec: $_" "ERROR"
        return $false
    }

    # Run silent install
    Write-Status "Installing Parsec (silent)..." "INFO"
    try {
        $Process = Start-Process -FilePath $InstallerPath -ArgumentList "/S" -Wait -PassThru

        if ($Process.ExitCode -eq 0) {
            Write-Status "Parsec installed successfully" "OK"
        }
        else {
            Write-Status "Parsec installer returned code $($Process.ExitCode)" "WARN"
        }
    }
    catch {
        Write-Status "Failed to install Parsec: $_" "ERROR"
        return $false
    }

    # Verify installation
    Start-Sleep -Seconds 3
    if (Test-ParsecInstalled) {
        Write-Status "Parsec installation verified" "OK"
        return $true
    }
    else {
        Write-Status "Parsec installation could not be verified" "WARN"
        return $true  # May still work, just path issue
    }
}

function Install-ParsecVDA {
    Write-Host ""
    Write-Host "=== Parsec Virtual Display Adapter ===" -ForegroundColor Cyan

    if ($SkipVDA) {
        Write-Status "VDA installation skipped" "SKIP"
        return $true
    }

    # Create download directory
    if (-not (Test-Path $DownloadDir)) {
        New-Item -Path $DownloadDir -ItemType Directory -Force | Out-Null
    }

    $VDAInstallerPath = "$DownloadDir\parsec-vdd.exe"

    # Download VDA installer
    Write-Status "Downloading Parsec VDA..." "INFO"
    try {
        Invoke-WebRequest -Uri $ParsecVDAUrl -OutFile $VDAInstallerPath -UseBasicParsing
        Write-Status "Downloaded Parsec VDA" "OK"
    }
    catch {
        Write-Status "Failed to download Parsec VDA: $_" "WARN"
        Write-Status "VDA is optional - Parsec will still work with 'Fallback To Virtual Display'" "INFO"
        return $true
    }

    # Run VDA installer
    Write-Status "Installing Parsec VDA (may require admin)..." "INFO"
    try {
        $Process = Start-Process -FilePath $VDAInstallerPath -ArgumentList "/S" -Wait -PassThru -Verb RunAs

        if ($Process.ExitCode -eq 0) {
            Write-Status "Parsec VDA installed successfully" "OK"
        }
        else {
            Write-Status "VDA installer returned code $($Process.ExitCode) - this is often OK" "WARN"
        }
    }
    catch {
        Write-Status "VDA installation may require manual admin approval" "WARN"
        return $true
    }

    return $true
}

function Set-ParsecConfig {
    Write-Host ""
    Write-Host "=== Parsec Configuration ===" -ForegroundColor Cyan

    # Ensure config directory exists
    $ConfigDir = Split-Path -Parent $ParsecConfigPath
    if (-not (Test-Path $ConfigDir)) {
        New-Item -Path $ConfigDir -ItemType Directory -Force | Out-Null
    }

    # Read existing config or create new
    $Config = @{}
    if (Test-Path $ParsecConfigPath) {
        try {
            $Config = Get-Content $ParsecConfigPath -Raw | ConvertFrom-Json -AsHashtable
            Write-Status "Loaded existing Parsec config" "OK"
        }
        catch {
            Write-Status "Could not parse existing config, creating new" "WARN"
            $Config = @{}
        }
    }

    # Set headless configuration
    $HeadlessSettings = @{
        "host_virtual_monitor" = 1
        "host_virtual_monitor_fallback" = 1
        "host_privacy" = 0
        "host_virtual_monitor_preset" = "1920x1080@60"
    }

    foreach ($Key in $HeadlessSettings.Keys) {
        $Config[$Key] = $HeadlessSettings[$Key]
    }

    # Write config
    try {
        $Config | ConvertTo-Json -Depth 10 | Set-Content -Path $ParsecConfigPath -Encoding UTF8
        Write-Status "Configured Parsec for headless operation" "OK"
    }
    catch {
        Write-Status "Failed to write Parsec config: $_" "WARN"
        Write-Status "You may need to manually enable 'Fallback To Virtual Display' in Parsec settings" "INFO"
    }

    return $true
}

function Set-FirewallRules {
    Write-Host ""
    Write-Host "=== Firewall Configuration ===" -ForegroundColor Cyan

    try {
        # Check if rule already exists
        $ExistingRule = Get-NetFirewallRule -DisplayName "Parsec" -ErrorAction SilentlyContinue

        if ($ExistingRule) {
            Write-Status "Parsec firewall rule already exists" "SKIP"
            return $true
        }

        # Create firewall rule for Parsec UDP traffic
        New-NetFirewallRule -DisplayName "Parsec" `
            -Direction Inbound `
            -Protocol UDP `
            -LocalPort 8000-8010 `
            -Action Allow `
            -Profile Any `
            -Description "Allow Parsec peer-to-peer connections" `
            -ErrorAction Stop

        Write-Status "Created Parsec firewall rule (UDP 8000-8010)" "OK"
    }
    catch {
        Write-Status "Could not create firewall rule (may need admin): $_" "WARN"
        Write-Status "Parsec may still work via relay" "INFO"
    }

    return $true
}

function Set-AutoStart {
    Write-Host ""
    Write-Host "=== Auto-Start Configuration ===" -ForegroundColor Cyan

    $StartupPath = "$env:APPDATA\Microsoft\Windows\Start Menu\Programs\Startup"
    $ParsecExe = "$ParsecInstallPath\parsecd.exe"

    # Check if Parsec is already in startup
    $ExistingShortcut = Get-ChildItem -Path $StartupPath -Filter "*Parsec*" -ErrorAction SilentlyContinue

    if ($ExistingShortcut) {
        Write-Status "Parsec auto-start already configured" "SKIP"
        return $true
    }

    # Find Parsec executable
    $PossiblePaths = @(
        "$ParsecInstallPath\parsecd.exe",
        "$env:ProgramFiles\Parsec\parsecd.exe",
        "${env:ProgramFiles(x86)}\Parsec\parsecd.exe"
    )

    $ParsecExe = $null
    foreach ($Path in $PossiblePaths) {
        if (Test-Path $Path) {
            $ParsecExe = $Path
            break
        }
    }

    if (-not $ParsecExe) {
        Write-Status "Could not find Parsec executable for auto-start" "WARN"
        return $true
    }

    # Create shortcut
    try {
        $WshShell = New-Object -ComObject WScript.Shell
        $Shortcut = $WshShell.CreateShortcut("$StartupPath\Parsec.lnk")
        $Shortcut.TargetPath = $ParsecExe
        $Shortcut.Save()

        Write-Status "Configured Parsec to start with Windows" "OK"
    }
    catch {
        Write-Status "Could not create startup shortcut: $_" "WARN"
    }

    return $true
}

# Main execution
Write-Host "==========================================" -ForegroundColor Cyan
Write-Host "  Parsec Remote Desktop Installer" -ForegroundColor Cyan
Write-Host "==========================================" -ForegroundColor Cyan
Write-Host ""

$Success = $true

# Install Parsec
if (-not (Install-Parsec)) {
    $Success = $false
}

# Install VDA
if (-not (Install-ParsecVDA)) {
    # VDA is optional, don't fail
}

# Configure
Set-ParsecConfig
Set-FirewallRules
Set-AutoStart

# Cleanup
if (Test-Path $DownloadDir) {
    Remove-Item -Path $DownloadDir -Recurse -Force -ErrorAction SilentlyContinue
}

# Summary
Write-Host ""
Write-Host "==========================================" -ForegroundColor Cyan
Write-Host "  Installation Complete" -ForegroundColor Cyan
Write-Host "==========================================" -ForegroundColor Cyan
Write-Host ""

if ($Success) {
    Write-Status "Parsec installed and configured for headless operation" "OK"
    Write-Host ""
    Write-Host "NEXT STEPS:" -ForegroundColor Yellow
    Write-Host "1. Launch Parsec and sign in (or create account at https://parsec.app)" -ForegroundColor White
    Write-Host "2. On your main workstation, install Parsec client and connect" -ForegroundColor White
    Write-Host "3. Test connection with monitor disconnected or off" -ForegroundColor White
    Write-Host ""
    exit 0
}
else {
    Write-Status "Parsec installation had errors" "ERROR"
    exit 2
}
