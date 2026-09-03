# Entry point for a brand-new Windows machine with nothing installed yet —
# deliberately outside Task, because reaching `task init` at all already
# requires Task to be installed and working (Codex review finding on the
# fresh-PC onboarding audit, #2940: `task init` cannot be what
# detects/installs Task itself, that's circular on exactly the scenario
# this exists for).
#
# This script only gets you as far as `task` being on PATH. Everything
# after that -- Rust/Node/CMake/Ninja/git -- is `task init`'s job
# (scripts/check-toolchain.sh, run via Git Bash), once Task itself exists.
#
# Usage (PowerShell):
#   irm https://raw.githubusercontent.com/agentmuxai/agentmux/main/scripts/bootstrap.ps1 | iex
#   # or, after cloning:
#   powershell -ExecutionPolicy Bypass -File scripts\bootstrap.ps1

$ErrorActionPreference = "Stop"

if (Get-Command task -ErrorAction SilentlyContinue) {
    $version = (task --version)
    Write-Host "Task is already installed: $version"
    Write-Host "Next: task init"
    exit 0
}

if (-not (Get-Command winget -ErrorAction SilentlyContinue)) {
    Write-Host "winget is not available on this system."
    Write-Host "Install Task manually: https://taskfile.dev/installation/"
    exit 1
}

Write-Host "Task not found. Installing via winget..."
winget install Task.Task

if (Get-Command task -ErrorAction SilentlyContinue) {
    $version = (task --version)
    Write-Host ""
    Write-Host "Task installed: $version"
    Write-Host "You may need to open a new terminal for PATH changes to take effect."
    Write-Host "Next: clone the repo if you haven't, cd into it, then run: task init"
} else {
    Write-Host ""
    Write-Host "winget reported success but 'task' is still not on PATH."
    Write-Host "Open a new terminal and try again, or see: https://taskfile.dev/installation/"
    exit 1
}
