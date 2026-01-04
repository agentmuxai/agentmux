# =============================================================================
# VS Code Bridge - Windows Service Installer
# =============================================================================
# Installs vscode-bridge as a Windows Task Scheduler task that:
#   - Starts automatically on user login
#   - Restarts if it crashes
#   - Runs in the background
#
# Usage:
#   .\install-service.ps1           # Install and start
#   .\install-service.ps1 -Uninstall # Remove service
# =============================================================================

param(
    [switch]$Uninstall
)

$TaskName = "VSCodeBridge"
$ScriptPath = Join-Path $PSScriptRoot "index.js"

# Uninstall
if ($Uninstall) {
    Write-Host "Removing VS Code Bridge service..." -ForegroundColor Yellow

    # Stop any running instance
    Get-Process -Name "node" -ErrorAction SilentlyContinue |
        Where-Object { $_.CommandLine -like "*vscode-bridge*" } |
        Stop-Process -Force -ErrorAction SilentlyContinue

    # Remove scheduled task
    Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false -ErrorAction SilentlyContinue

    Write-Host "VS Code Bridge service removed." -ForegroundColor Green
    exit 0
}

# Check if Node.js is available
if (-not (Get-Command node -ErrorAction SilentlyContinue)) {
    Write-Host "Error: Node.js not found. Please install Node.js first." -ForegroundColor Red
    exit 1
}

# Check if script exists
if (-not (Test-Path $ScriptPath)) {
    Write-Host "Error: index.js not found at $ScriptPath" -ForegroundColor Red
    exit 1
}

Write-Host "Installing VS Code Bridge service..." -ForegroundColor Yellow

# Remove existing task if present
Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false -ErrorAction SilentlyContinue

# Create the scheduled task
$NodePath = (Get-Command node).Source
$Action = New-ScheduledTaskAction -Execute $NodePath -Argument "`"$ScriptPath`"" -WorkingDirectory $PSScriptRoot

# Trigger: At logon
$Trigger = New-ScheduledTaskTrigger -AtLogon

# Settings: Restart on failure, don't stop on idle, run indefinitely
$Settings = New-ScheduledTaskSettingsSet `
    -AllowStartIfOnBatteries `
    -DontStopIfGoingOnBatteries `
    -StartWhenAvailable `
    -RestartCount 3 `
    -RestartInterval (New-TimeSpan -Minutes 1) `
    -ExecutionTimeLimit (New-TimeSpan -Days 9999)

# Register the task (runs as current user)
Register-ScheduledTask `
    -TaskName $TaskName `
    -Action $Action `
    -Trigger $Trigger `
    -Settings $Settings `
    -Description "VS Code Bridge - Opens files in VS Code from container agents" `
    -RunLevel Limited | Out-Null

Write-Host "VS Code Bridge service installed." -ForegroundColor Green

# Start the task immediately
Write-Host "Starting VS Code Bridge..." -ForegroundColor Yellow
Start-ScheduledTask -TaskName $TaskName

# Wait a moment and check health
Start-Sleep -Seconds 2

try {
    $health = Invoke-RestMethod -Uri "http://localhost:3101/health" -TimeoutSec 5
    Write-Host "VS Code Bridge is running!" -ForegroundColor Green
    Write-Host "  Status: $($health.status)" -ForegroundColor Cyan
    Write-Host "  Version: $($health.version)" -ForegroundColor Cyan
    Write-Host "  Workspaces: $($health.workspacesBase)" -ForegroundColor Cyan
} catch {
    Write-Host "Warning: Could not verify health endpoint. Service may still be starting." -ForegroundColor Yellow
    Write-Host "  Check manually: curl http://localhost:3101/health" -ForegroundColor Yellow
}

Write-Host ""
Write-Host "Service will auto-start on login and restart on failure." -ForegroundColor Gray
Write-Host "To uninstall: .\install-service.ps1 -Uninstall" -ForegroundColor Gray
