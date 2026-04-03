#!/usr/bin/env pwsh
<#
.SYNOPSIS
    Health check for WaveMux sandbox environment

.DESCRIPTION
    Validates that the sandbox is properly configured:
    - Parsec service and VDA status
    - Development tools installation
    - WaveMux repository and build status
    - Instance isolation

.PARAMETER OutputFormat
    Output format: 'text' (default) or 'json'

.PARAMETER Verbose
    Enable verbose output

.EXAMPLE
    pwsh scripts/sandbox-health-impl.ps1
    Standard health check

.EXAMPLE
    pwsh scripts/sandbox-health-impl.ps1 -OutputFormat json
    JSON output for automation

.NOTES
    Part of @agentmuxhq/sandbox package

    Exit Codes:
      0 = All checks passed
      1 = Warnings found
      2 = Errors found
      3 = Health check failed
#>

param(
    [ValidateSet('text', 'json')]
    [string]$OutputFormat = 'text',

    [switch]$Verbose
)

$ErrorActionPreference = "Stop"

# Constants
$WaveMuxDir = "D:\Code\sandbox\wavemux"
$InstanceName = "dev"
$InstanceDir = "$env:USERPROFILE\.wavemux-$InstanceName"

# Health data
$script:HealthData = @{
    timestamp = (Get-Date).ToString('o')
    hostname = $env:COMPUTERNAME
    checks = @{}
    summary = @{
        total = 0
        passed = 0
        warnings = 0
        errors = 0
    }
}

function Add-CheckResult {
    param(
        [string]$Name,
        [string]$Status,  # OK, WARN, ERROR
        [string]$Message,
        [string]$Details = ""
    )

    $script:HealthData.checks[$Name] = @{
        status = $Status
        message = $Message
        details = $Details
    }

    $script:HealthData.summary.total++

    switch ($Status) {
        "OK" { $script:HealthData.summary.passed++ }
        "WARN" { $script:HealthData.summary.warnings++ }
        "ERROR" { $script:HealthData.summary.errors++ }
    }

    if ($OutputFormat -eq 'text') {
        $Color = switch ($Status) {
            "OK" { "Green" }
            "WARN" { "Yellow" }
            "ERROR" { "Red" }
        }

        $StatusText = "[$Status]".PadRight(8)
        Write-Host "$StatusText $Name" -ForegroundColor $Color

        if ($Verbose -and $Details) {
            Write-Host "         $Details" -ForegroundColor Gray
        }
    }
}

function Test-ParsecService {
    $Service = Get-Service -Name "Parsec" -ErrorAction SilentlyContinue

    if (-not $Service) {
        Add-CheckResult -Name "Parsec Service" -Status "ERROR" -Message "Not installed" -Details "Parsec service not found"
        return
    }

    if ($Service.Status -eq "Running") {
        Add-CheckResult -Name "Parsec Service" -Status "OK" -Message "Running" -Details "Service status: $($Service.Status)"
    }
    else {
        Add-CheckResult -Name "Parsec Service" -Status "WARN" -Message "Not running" -Details "Service status: $($Service.Status)"
    }
}

function Test-ParsecVDA {
    # Check for Parsec VDA in device manager
    $VDA = Get-PnpDevice -FriendlyName "*Parsec*" -ErrorAction SilentlyContinue

    if ($VDA) {
        if ($VDA.Status -eq "OK") {
            Add-CheckResult -Name "Parsec VDA" -Status "OK" -Message "Active" -Details "Virtual display adapter present"
        }
        else {
            Add-CheckResult -Name "Parsec VDA" -Status "WARN" -Message "Present but status: $($VDA.Status)" -Details "May need driver reinstall"
        }
    }
    else {
        # Check Parsec config for virtual display fallback
        $ConfigPath = "$env:APPDATA\Parsec\config.json"
        if (Test-Path $ConfigPath) {
            $Config = Get-Content $ConfigPath -Raw | ConvertFrom-Json
            if ($Config.host_virtual_monitor_fallback -eq 1) {
                Add-CheckResult -Name "Parsec VDA" -Status "OK" -Message "Fallback enabled" -Details "Using virtual display fallback"
            }
            else {
                Add-CheckResult -Name "Parsec VDA" -Status "WARN" -Message "Not installed" -Details "Consider enabling 'Fallback To Virtual Display' in Parsec settings"
            }
        }
        else {
            Add-CheckResult -Name "Parsec VDA" -Status "WARN" -Message "Not configured" -Details "Parsec config not found"
        }
    }
}

function Test-DevTool {
    param(
        [string]$Name,
        [string]$Command,
        [string]$VersionArg,
        [string]$MinVersion,
        [bool]$Required = $true
    )

    $Exe = Get-Command $Command -ErrorAction SilentlyContinue

    if (-not $Exe) {
        $Status = if ($Required) { "ERROR" } else { "WARN" }
        Add-CheckResult -Name $Name -Status $Status -Message "Not installed" -Details "Command '$Command' not found in PATH"
        return
    }

    try {
        $VersionOutput = & $Command $VersionArg 2>&1
        $Version = if ($VersionOutput -match '(\d+\.\d+\.\d+)') { $Matches[1] }
                   elseif ($VersionOutput -match 'v(\d+\.\d+\.\d+)') { $Matches[1] }
                   else { "unknown" }

        Add-CheckResult -Name $Name -Status "OK" -Message "v$Version" -Details "Path: $($Exe.Source)"
    }
    catch {
        Add-CheckResult -Name $Name -Status "WARN" -Message "Installed but version check failed" -Details $_.Exception.Message
    }
}

function Test-WaveMuxRepo {
    if (-not (Test-Path $WaveMuxDir)) {
        Add-CheckResult -Name "WaveMux Repo" -Status "ERROR" -Message "Not cloned" -Details "Expected at $WaveMuxDir"
        return
    }

    if (-not (Test-Path "$WaveMuxDir\.git")) {
        Add-CheckResult -Name "WaveMux Repo" -Status "ERROR" -Message "Not a git repo" -Details "$WaveMuxDir exists but is not a git repository"
        return
    }

    # Get current branch
    Push-Location $WaveMuxDir
    try {
        $Branch = & git rev-parse --abbrev-ref HEAD 2>&1
        $LastCommit = & git log -1 --format="%h %s" 2>&1

        Add-CheckResult -Name "WaveMux Repo" -Status "OK" -Message "Branch: $Branch" -Details "Last commit: $LastCommit"
    }
    catch {
        Add-CheckResult -Name "WaveMux Repo" -Status "WARN" -Message "Present but git status failed" -Details $_.Exception.Message
    }
    finally {
        Pop-Location
    }
}

function Test-WaveMuxBuild {
    if (-not (Test-Path $WaveMuxDir)) {
        Add-CheckResult -Name "WaveMux Build" -Status "ERROR" -Message "Repo not found" -Details "Cannot check build without repo"
        return
    }

    # Check for built binaries
    $BinDir = "$WaveMuxDir\dist\bin"
    if (-not (Test-Path $BinDir)) {
        Add-CheckResult -Name "WaveMux Build" -Status "WARN" -Message "Not built" -Details "Run 'task build:backend' to build"
        return
    }

    $Binaries = Get-ChildItem -Path $BinDir -Filter "agentmux-wsh-*" -ErrorAction SilentlyContinue

    if ($Binaries.Count -eq 0) {
        Add-CheckResult -Name "WaveMux Build" -Status "WARN" -Message "No binaries found" -Details "dist/bin exists but no agentmux-wsh binaries"
        return
    }

    Add-CheckResult -Name "WaveMux Build" -Status "OK" -Message "$($Binaries.Count) binaries" -Details "Found: $($Binaries.Name -join ', ')"
}

function Test-WaveMuxInstance {
    if (Test-Path $InstanceDir) {
        $Files = Get-ChildItem -Path $InstanceDir -ErrorAction SilentlyContinue
        Add-CheckResult -Name "Dev Instance" -Status "OK" -Message "Configured" -Details "Instance dir: $InstanceDir ($($Files.Count) files)"
    }
    else {
        Add-CheckResult -Name "Dev Instance" -Status "WARN" -Message "Not initialized" -Details "Instance directory not found. Run WaveMux with --instance=$InstanceName to create."
    }
}

function Test-NodeModules {
    if (-not (Test-Path $WaveMuxDir)) {
        Add-CheckResult -Name "Dependencies" -Status "ERROR" -Message "Repo not found"
        return
    }

    $NodeModules = "$WaveMuxDir\node_modules"
    if (-not (Test-Path $NodeModules)) {
        Add-CheckResult -Name "Dependencies" -Status "ERROR" -Message "Not installed" -Details "Run 'npm install' in $WaveMuxDir"
        return
    }

    $PackageCount = (Get-ChildItem -Path $NodeModules -Directory).Count
    Add-CheckResult -Name "Dependencies" -Status "OK" -Message "$PackageCount packages" -Details "node_modules present"
}

# Main execution
if ($OutputFormat -eq 'text') {
    Write-Host ""
    Write-Host "==========================================" -ForegroundColor Cyan
    Write-Host "  WaveMux Sandbox Health Check" -ForegroundColor Cyan
    Write-Host "==========================================" -ForegroundColor Cyan
    Write-Host "  Host: $env:COMPUTERNAME" -ForegroundColor Gray
    Write-Host "  Time: $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')" -ForegroundColor Gray
    Write-Host "==========================================" -ForegroundColor Cyan
    Write-Host ""
    Write-Host "Remote Access" -ForegroundColor Cyan
    Write-Host "-------------" -ForegroundColor Cyan
}

# Parsec checks
Test-ParsecService
Test-ParsecVDA

if ($OutputFormat -eq 'text') {
    Write-Host ""
    Write-Host "Development Tools" -ForegroundColor Cyan
    Write-Host "-----------------" -ForegroundColor Cyan
}

# Dev tool checks
Test-DevTool -Name "Node.js" -Command "node" -VersionArg "--version" -MinVersion "18.0.0"
Test-DevTool -Name "npm" -Command "npm" -VersionArg "--version" -MinVersion "9.0.0"
Test-DevTool -Name "Go" -Command "go" -VersionArg "version" -MinVersion "1.21.0"
Test-DevTool -Name "Zig" -Command "zig" -VersionArg "version" -MinVersion "0.11.0"
Test-DevTool -Name "Task" -Command "task" -VersionArg "--version" -MinVersion "3.0.0"
Test-DevTool -Name "Git" -Command "git" -VersionArg "--version" -MinVersion "2.0.0"
Test-DevTool -Name "VS Code" -Command "code" -VersionArg "--version" -MinVersion "1.0.0" -Required $false

if ($OutputFormat -eq 'text') {
    Write-Host ""
    Write-Host "WaveMux" -ForegroundColor Cyan
    Write-Host "-------" -ForegroundColor Cyan
}

# WaveMux checks
Test-WaveMuxRepo
Test-WaveMuxBuild
Test-NodeModules
Test-WaveMuxInstance

# Output results
if ($OutputFormat -eq 'json') {
    $script:HealthData | ConvertTo-Json -Depth 10
}
else {
    Write-Host ""
    Write-Host "==========================================" -ForegroundColor Cyan
    Write-Host "  Summary" -ForegroundColor Cyan
    Write-Host "==========================================" -ForegroundColor Cyan

    $Summary = $script:HealthData.summary
    Write-Host "  Total:    $($Summary.total)" -ForegroundColor White
    Write-Host "  Passed:   $($Summary.passed)" -ForegroundColor Green
    Write-Host "  Warnings: $($Summary.warnings)" -ForegroundColor Yellow
    Write-Host "  Errors:   $($Summary.errors)" -ForegroundColor Red
    Write-Host ""
}

# Determine exit code
if ($script:HealthData.summary.errors -gt 0) {
    exit 2
}
elseif ($script:HealthData.summary.warnings -gt 0) {
    exit 1
}
else {
    exit 0
}
