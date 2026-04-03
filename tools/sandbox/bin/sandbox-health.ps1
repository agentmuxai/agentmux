#!/usr/bin/env pwsh
<#
.SYNOPSIS
    Health check for AgentMux sandbox environment

.DESCRIPTION
    Validates sandbox configuration including Parsec, development tools,
    and AgentMux installation.

.PARAMETER OutputFormat
    Output format: 'text' (default) or 'json'

.PARAMETER Verbose
    Enable verbose output

.EXAMPLE
    sandbox-health
    Standard health check

.EXAMPLE
    sandbox-health -OutputFormat json
    JSON output for automation

.NOTES
    Part of @a5af/sandbox package (located in agentmux/tools/sandbox)

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

# Find sandbox scripts - now located in agentmux repo
$HealthScript = $null

# Check agentmux worktrees/checkouts
$SearchPaths = @(
    # Wavemux worktrees
    "D:\Code\worktrees\agentmux*\tools\sandbox\scripts\sandbox-health-impl.ps1",
    # Agent workspace agentmux checkouts
    "D:\Code\agent-workspaces\*\agentmux\tools\sandbox\scripts\sandbox-health-impl.ps1",
    # Sandbox development directory
    "D:\Code\sandbox\agentmux\tools\sandbox\scripts\sandbox-health-impl.ps1"
)

foreach ($Pattern in $SearchPaths) {
    $Matches = Get-ChildItem -Path $Pattern -ErrorAction SilentlyContinue
    if ($Matches) {
        $HealthScript = $Matches[0].FullName
        break
    }
}

if (-not $HealthScript) {
    Write-Host "ERROR: Could not find sandbox-health-impl.ps1" -ForegroundColor Red
    Write-Host "Expected in: agentmux/tools/sandbox/scripts/" -ForegroundColor Yellow
    Write-Host "" -ForegroundColor Yellow
    Write-Host "Searched locations:" -ForegroundColor Yellow
    foreach ($Pattern in $SearchPaths) {
        Write-Host "  - $Pattern" -ForegroundColor Gray
    }
    exit 3
}

Write-Host "Using: $HealthScript" -ForegroundColor Cyan

# Build parameters
$Params = @{}
if ($OutputFormat -ne 'text') { $Params['OutputFormat'] = $OutputFormat }
if ($Verbose) { $Params['Verbose'] = $true }

# Execute health check
& $HealthScript @Params
exit $LASTEXITCODE
