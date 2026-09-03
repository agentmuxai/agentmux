# Entry point for a brand-new Windows machine with nothing installed yet --
# deliberately outside Task, because reaching `task init` at all already
# requires Task to be installed and working (Codex/reagent review finding on
# the fresh-PC onboarding audit, #2940/#2943: `task init` cannot be what
# detects/installs Task itself, that's circular on exactly the scenario
# this exists for).
#
# Installs Git first, then Task. Git matters here for two reasons, both
# raised in review of the first version of this script (which only
# installed Task): a stock Windows machine has no way to `git clone` this
# repo without it, and Task's own cross-platform tasks (init, dev, changeset,
# release) invoke `sh`/`bash` for their cmds -- which Git for Windows is what
# actually provides on Windows. Installing Task alone would get you to
# `task init` only to have it fail with a raw "'sh' is not recognized"
# instead of the intended diagnostic -- the same circularity this script
# exists to avoid, one layer down.
#
# Usage (PowerShell) -- this is the one entry point that works on a machine
# with NOTHING installed yet, since it doesn't need git to already exist:
#   irm https://raw.githubusercontent.com/agentmuxai/agentmux/main/scripts/bootstrap.ps1 | iex
#   # or, after cloning some other way (zip download, GitHub Desktop, etc.):
#   powershell -ExecutionPolicy Bypass -File scripts\bootstrap.ps1

$ErrorActionPreference = "Stop"

# winget updates the registry PATH but does not propagate it to this
# already-running process (reagent review finding on PR #2943: without this,
# `Get-Command task` right after `winget install Task.Task` typically still
# fails on the common success path, since $env:Path here is a snapshot taken
# at process start). Re-reading both PATH scopes from the registry after each
# install and rebuilding $env:Path is the standard fix.
function Update-SessionPath {
    $machinePath = [System.Environment]::GetEnvironmentVariable("Path", "Machine")
    $userPath = [System.Environment]::GetEnvironmentVariable("Path", "User")
    $env:Path = "$machinePath;$userPath"
}

function Install-WithWinget {
    param(
        [string]$DisplayName,
        [string]$Command,
        [string]$WingetId
    )

    if (Get-Command $Command -ErrorAction SilentlyContinue) {
        Write-Host "$DisplayName is already installed."
        return $true
    }

    if (-not (Get-Command winget -ErrorAction SilentlyContinue)) {
        Write-Host "$DisplayName not found, and winget is not available on this system."
        Write-Host "Install $DisplayName manually, then re-run this script."
        return $false
    }

    Write-Host "$DisplayName not found. Installing via winget ($WingetId)..."
    winget install --id $WingetId --silent --accept-package-agreements --accept-source-agreements
    Update-SessionPath

    if (Get-Command $Command -ErrorAction SilentlyContinue) {
        Write-Host "$DisplayName installed."
        return $true
    }

    Write-Host "winget reported success but '$Command' is still not on PATH in this session."
    Write-Host "Open a new terminal and re-run this script to confirm, or install manually."
    return $false
}

$gitOk = Install-WithWinget -DisplayName "Git" -Command "git" -WingetId "Git.Git"
$taskOk = Install-WithWinget -DisplayName "Task" -Command "task" -WingetId "Task.Task"

Write-Host ""

if ($gitOk -and $taskOk) {
    Write-Host "Git and Task are both ready: $(git --version), $(task --version)"
    Write-Host "Next: clone the repo if you haven't, cd into it, then run: task init"
    exit 0
} else {
    Write-Host "One or more tools could not be confirmed on PATH in this session."
    Write-Host "Open a NEW terminal (PATH changes from installers often don't reach this one) and re-run this script."
    exit 1
}
