#!/usr/bin/env pwsh
<#
.SYNOPSIS
    Complete sandbox host setup for AgentMux development

.DESCRIPTION
    Orchestrates the complete setup of a Windows machine as a AgentMux
    development sandbox, including:
    - Development tools (Node.js, Go, Zig, Task, Git, VS Code)
    - Parsec remote desktop with virtual display
    - AgentMux repository clone and build
    - Instance isolation configuration

.PARAMETER SkipParsec
    Skip Parsec installation (use if already installed)

.PARAMETER SkipDevTools
    Skip development tools installation (use if already installed)

.PARAMETER SkipAgentMux
    Skip AgentMux clone and build

.PARAMETER Force
    Force reinstall of all components

.PARAMETER Verbose
    Enable verbose output

.PARAMETER AgentMuxBranch
    AgentMux branch to clone (default: main)

.EXAMPLE
    pwsh scripts/setup-sandbox-impl.ps1
    Full sandbox setup

.EXAMPLE
    pwsh scripts/setup-sandbox-impl.ps1 -SkipParsec -SkipDevTools
    Only setup AgentMux (tools already installed)

.EXAMPLE
    pwsh scripts/setup-sandbox-impl.ps1 -AgentMuxBranch agentx/feature
    Setup with specific AgentMux branch

.NOTES
    Part of @agentmuxhq/sandbox package

    Run on the SANDBOX HOST machine, not your main workstation.

    Exit Codes:
      0 = Setup completed successfully
      1 = Setup completed with warnings
      2 = Setup failed
      3 = Script error
#>

param(
    [switch]$SkipParsec,
    [switch]$SkipDevTools,
    [switch]$SkipAgentMux,
    [switch]$Force,
    [switch]$Verbose,
    [string]$AgentMuxBranch = "main"
)

$ErrorActionPreference = "Stop"

# Get script directory
$ScriptDir = $PSScriptRoot

# State tracking
$script:StepResults = @{
    DevTools = @{ Status = "PENDING"; Message = "" }
    Parsec = @{ Status = "PENDING"; Message = "" }
    AgentMux = @{ Status = "PENDING"; Message = "" }
}

function Write-Banner {
    param([string]$Text)

    $Width = 50
    $Padding = [Math]::Max(0, ($Width - $Text.Length - 2) / 2)
    $PadLeft = " " * [Math]::Floor($Padding)
    $PadRight = " " * [Math]::Ceiling($Padding)

    Write-Host ""
    Write-Host ("=" * $Width) -ForegroundColor Cyan
    Write-Host "$PadLeft $Text $PadRight" -ForegroundColor Cyan
    Write-Host ("=" * $Width) -ForegroundColor Cyan
    Write-Host ""
}

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

function Test-IsAdmin {
    $Identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $Principal = New-Object Security.Principal.WindowsPrincipal($Identity)
    return $Principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Install-DevTools {
    Write-Banner "Development Tools"

    if ($SkipDevTools) {
        Write-Status "Skipping development tools installation" "SKIP"
        $script:StepResults.DevTools.Status = "SKIPPED"
        return $true
    }

    $DevToolsScript = Join-Path $ScriptDir "install-dev-tools.ps1"

    if (-not (Test-Path $DevToolsScript)) {
        Write-Status "install-dev-tools.ps1 not found" "ERROR"
        $script:StepResults.DevTools.Status = "FAILED"
        $script:StepResults.DevTools.Message = "Script not found"
        return $false
    }

    $Params = @{}
    if ($Force) { $Params['Force'] = $true }
    if ($Verbose) { $Params['Verbose'] = $true }

    try {
        & $DevToolsScript @Params
        $ExitCode = $LASTEXITCODE

        if ($ExitCode -eq 0) {
            $script:StepResults.DevTools.Status = "OK"
            return $true
        }
        elseif ($ExitCode -eq 1) {
            $script:StepResults.DevTools.Status = "WARN"
            $script:StepResults.DevTools.Message = "Completed with warnings"
            return $true
        }
        else {
            $script:StepResults.DevTools.Status = "FAILED"
            $script:StepResults.DevTools.Message = "Exit code: $ExitCode"
            return $false
        }
    }
    catch {
        $script:StepResults.DevTools.Status = "FAILED"
        $script:StepResults.DevTools.Message = $_.Exception.Message
        return $false
    }
}

function Install-Parsec {
    Write-Banner "Parsec Remote Desktop"

    if ($SkipParsec) {
        Write-Status "Skipping Parsec installation" "SKIP"
        $script:StepResults.Parsec.Status = "SKIPPED"
        return $true
    }

    $ParsecScript = Join-Path $ScriptDir "install-parsec.ps1"

    if (-not (Test-Path $ParsecScript)) {
        Write-Status "install-parsec.ps1 not found" "ERROR"
        $script:StepResults.Parsec.Status = "FAILED"
        $script:StepResults.Parsec.Message = "Script not found"
        return $false
    }

    $Params = @{}
    if ($Force) { $Params['Force'] = $true }
    if ($Verbose) { $Params['Verbose'] = $true }

    try {
        & $ParsecScript @Params
        $ExitCode = $LASTEXITCODE

        if ($ExitCode -eq 0) {
            $script:StepResults.Parsec.Status = "OK"
            return $true
        }
        elseif ($ExitCode -eq 1) {
            $script:StepResults.Parsec.Status = "WARN"
            $script:StepResults.Parsec.Message = "Completed with warnings"
            return $true
        }
        else {
            $script:StepResults.Parsec.Status = "FAILED"
            $script:StepResults.Parsec.Message = "Exit code: $ExitCode"
            return $false
        }
    }
    catch {
        $script:StepResults.Parsec.Status = "FAILED"
        $script:StepResults.Parsec.Message = $_.Exception.Message
        return $false
    }
}

function Setup-AgentMux {
    Write-Banner "AgentMux Repository"

    if ($SkipAgentMux) {
        Write-Status "Skipping AgentMux setup" "SKIP"
        $script:StepResults.AgentMux.Status = "SKIPPED"
        return $true
    }

    $AgentMuxScript = Join-Path $ScriptDir "clone-agentmux.ps1"

    if (-not (Test-Path $AgentMuxScript)) {
        Write-Status "clone-agentmux.ps1 not found" "ERROR"
        $script:StepResults.AgentMux.Status = "FAILED"
        $script:StepResults.AgentMux.Message = "Script not found"
        return $false
    }

    $Params = @{
        Branch = $AgentMuxBranch
    }
    if ($Force) { $Params['Force'] = $true }
    if ($Verbose) { $Params['Verbose'] = $true }

    try {
        & $AgentMuxScript @Params
        $ExitCode = $LASTEXITCODE

        if ($ExitCode -eq 0) {
            $script:StepResults.AgentMux.Status = "OK"
            return $true
        }
        elseif ($ExitCode -eq 1) {
            $script:StepResults.AgentMux.Status = "WARN"
            $script:StepResults.AgentMux.Message = "Completed with warnings"
            return $true
        }
        else {
            $script:StepResults.AgentMux.Status = "FAILED"
            $script:StepResults.AgentMux.Message = "Exit code: $ExitCode"
            return $false
        }
    }
    catch {
        $script:StepResults.AgentMux.Status = "FAILED"
        $script:StepResults.AgentMux.Message = $_.Exception.Message
        return $false
    }
}

function Show-Summary {
    Write-Banner "Setup Summary"

    $AllOK = $true
    $HasWarnings = $false

    foreach ($Step in @("DevTools", "Parsec", "AgentMux")) {
        $Result = $script:StepResults[$Step]
        $Status = $Result.Status
        $Message = $Result.Message

        $StatusColor = switch ($Status) {
            "OK" { "Green" }
            "WARN" { "Yellow"; $HasWarnings = $true }
            "SKIPPED" { "Cyan" }
            "FAILED" { "Red"; $AllOK = $false }
            default { "White" }
        }

        $StatusText = "[$Status]".PadRight(10)
        Write-Host "$StatusText $Step" -ForegroundColor $StatusColor
        if ($Message) {
            Write-Host "           $Message" -ForegroundColor Gray
        }
    }

    Write-Host ""

    if ($AllOK -and -not $HasWarnings) {
        Write-Status "Sandbox setup complete!" "OK"
        Write-Host ""
        Write-Host "NEXT STEPS:" -ForegroundColor Yellow
        Write-Host "1. Launch Parsec and sign in" -ForegroundColor White
        Write-Host "2. From your main workstation, connect via Parsec" -ForegroundColor White
        Write-Host "3. Open terminal and run: cd D:\Code\sandbox\agentmux && task dev" -ForegroundColor White
        Write-Host "4. Test with: agentmux --instance=dev" -ForegroundColor White
        return 0
    }
    elseif ($AllOK) {
        Write-Status "Setup complete with warnings" "WARN"
        return 1
    }
    else {
        Write-Status "Setup failed" "ERROR"
        Write-Host ""
        Write-Host "Review the errors above and re-run with -Verbose for details" -ForegroundColor Yellow
        return 2
    }
}

# Main execution
Clear-Host
Write-Banner "AgentMux Sandbox Setup"

Write-Host "This script will configure this machine as a AgentMux development sandbox." -ForegroundColor White
Write-Host ""
Write-Host "Components to install:" -ForegroundColor Cyan
Write-Host "  - Development tools (Node.js, Go, Zig, Task, Git, VS Code)" -ForegroundColor White
Write-Host "  - Parsec remote desktop (for low-latency access)" -ForegroundColor White
Write-Host "  - AgentMux repository (branch: $AgentMuxBranch)" -ForegroundColor White
Write-Host ""

# Check admin for some operations
if (-not (Test-IsAdmin)) {
    Write-Status "Not running as Administrator" "WARN"
    Write-Host "Some installations may prompt for elevation." -ForegroundColor Yellow
    Write-Host ""
}

# Run setup steps
$DevToolsOK = Install-DevTools
$ParsecOK = Install-Parsec
$AgentMuxOK = Setup-AgentMux

# Show summary and exit
$ExitCode = Show-Summary
exit $ExitCode
