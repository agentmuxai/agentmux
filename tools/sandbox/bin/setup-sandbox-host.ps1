#!/usr/bin/env pwsh
<#
.SYNOPSIS
    Setup a Windows machine as a WaveMux development sandbox

.DESCRIPTION
    Complete setup orchestrator for WaveMux development sandbox.
    Installs development tools, Parsec remote desktop, and clones WaveMux.

.PARAMETER SkipParsec
    Skip Parsec installation

.PARAMETER SkipDevTools
    Skip development tools installation

.PARAMETER SkipWaveMux
    Skip WaveMux repository setup

.PARAMETER Force
    Force reinstall of all components

.PARAMETER Verbose
    Enable verbose output

.PARAMETER WaveMuxBranch
    WaveMux branch to clone (default: main)

.EXAMPLE
    setup-sandbox-host
    Full sandbox setup

.EXAMPLE
    setup-sandbox-host -SkipParsec
    Setup without Parsec (already installed)

.NOTES
    Part of @a5af/sandbox package (located in wavemux/tools/sandbox)

    Exit Codes:
      0 = Setup completed successfully
      1 = Setup completed with warnings
      2 = Setup failed
      3 = Script error
#>

param(
    [switch]$SkipParsec,
    [switch]$SkipDevTools,
    [switch]$SkipWaveMux,
    [switch]$Force,
    [switch]$Verbose,
    [string]$WaveMuxBranch = "main"
)

$ErrorActionPreference = "Stop"

# Find sandbox scripts - now located in wavemux repo
$SetupScript = $null

# Check wavemux worktrees/checkouts
$SearchPaths = @(
    # Wavemux worktrees
    "D:\Code\worktrees\wavemux*\tools\sandbox\scripts\setup-sandbox-impl.ps1",
    # Agent workspace wavemux checkouts
    "D:\Code\agent-workspaces\*\wavemux\tools\sandbox\scripts\setup-sandbox-impl.ps1",
    # Sandbox development directory
    "D:\Code\sandbox\wavemux\tools\sandbox\scripts\setup-sandbox-impl.ps1"
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
    Write-Host "Expected in: wavemux/tools/sandbox/scripts/" -ForegroundColor Yellow
    Write-Host "" -ForegroundColor Yellow
    Write-Host "Searched locations:" -ForegroundColor Yellow
    foreach ($Pattern in $SearchPaths) {
        Write-Host "  - $Pattern" -ForegroundColor Gray
    }
    Write-Host "" -ForegroundColor Yellow
    Write-Host "Make sure wavemux repo is checked out with tools/sandbox." -ForegroundColor Yellow
    exit 3
}

Write-Host "Using: $SetupScript" -ForegroundColor Cyan

# Build parameters
$Params = @{}
if ($SkipParsec) { $Params['SkipParsec'] = $true }
if ($SkipDevTools) { $Params['SkipDevTools'] = $true }
if ($SkipWaveMux) { $Params['SkipWaveMux'] = $true }
if ($Force) { $Params['Force'] = $true }
if ($Verbose) { $Params['Verbose'] = $true }
if ($WaveMuxBranch -ne "main") { $Params['WaveMuxBranch'] = $WaveMuxBranch }

# Execute setup
& $SetupScript @Params
exit $LASTEXITCODE
