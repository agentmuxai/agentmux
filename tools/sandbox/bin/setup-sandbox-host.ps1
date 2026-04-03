#!/usr/bin/env pwsh
<#
.SYNOPSIS
    Setup a Windows machine as a AgentMux development sandbox

.DESCRIPTION
    Complete setup orchestrator for AgentMux development sandbox.
    Installs development tools, Parsec remote desktop, and clones AgentMux.

.PARAMETER SkipParsec
    Skip Parsec installation

.PARAMETER SkipDevTools
    Skip development tools installation

.PARAMETER SkipAgentMux
    Skip AgentMux repository setup

.PARAMETER Force
    Force reinstall of all components

.PARAMETER Verbose
    Enable verbose output

.PARAMETER AgentMuxBranch
    AgentMux branch to clone (default: main)

.EXAMPLE
    setup-sandbox-host
    Full sandbox setup

.EXAMPLE
    setup-sandbox-host -SkipParsec
    Setup without Parsec (already installed)

.NOTES
    Part of @a5af/sandbox package (located in agentmux/tools/sandbox)

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

# Find sandbox scripts - now located in agentmux repo
$SetupScript = $null

# Check agentmux worktrees/checkouts
$SearchPaths = @(
    # Wavemux worktrees
    "D:\Code\worktrees\agentmux*\tools\sandbox\scripts\setup-sandbox-impl.ps1",
    # Agent workspace agentmux checkouts
    "D:\Code\agent-workspaces\*\agentmux\tools\sandbox\scripts\setup-sandbox-impl.ps1",
    # Sandbox development directory
    "D:\Code\sandbox\agentmux\tools\sandbox\scripts\setup-sandbox-impl.ps1"
)

foreach ($Pattern in $SearchPaths) {
    $Matches = Get-ChildItem -Path $Pattern -ErrorAction SilentlyContinue
    if ($Matches) {
        $SetupScript = $Matches[0].FullName
        break
    }
}

if (-not $SetupScript) {
    Write-Host "ERROR: Could not find setup-sandbox-impl.ps1" -ForegroundColor Red
    Write-Host "Expected in: agentmux/tools/sandbox/scripts/" -ForegroundColor Yellow
    Write-Host "" -ForegroundColor Yellow
    Write-Host "Searched locations:" -ForegroundColor Yellow
    foreach ($Pattern in $SearchPaths) {
        Write-Host "  - $Pattern" -ForegroundColor Gray
    }
    Write-Host "" -ForegroundColor Yellow
    Write-Host "Make sure agentmux repo is checked out with tools/sandbox." -ForegroundColor Yellow
    exit 3
}

Write-Host "Using: $SetupScript" -ForegroundColor Cyan

# Build parameters
$Params = @{}
if ($SkipParsec) { $Params['SkipParsec'] = $true }
if ($SkipDevTools) { $Params['SkipDevTools'] = $true }
if ($SkipAgentMux) { $Params['SkipAgentMux'] = $true }
if ($Force) { $Params['Force'] = $true }
if ($Verbose) { $Params['Verbose'] = $true }
if ($AgentMuxBranch -ne "main") { $Params['AgentMuxBranch'] = $AgentMuxBranch }

# Execute setup
& $SetupScript @Params
exit $LASTEXITCODE
