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

# A working `git` on PATH does NOT imply `sh`/`bash` are too. Per this
# repo's own CLAUDE.md ("Gap B"): a standard Git for Windows install adds
# `Git\cmd` (git.exe and other shims) to PATH, not `Git\bin` where
# sh.exe/bash.exe live. Two independent reviewers on PR #2943 caught that my
# first fix here assumed installing Git was sufficient to unblock `task
# init`'s `sh scripts/check-toolchain.sh` -- it wasn't.
#
# My first attempt at a real fix derived Git's bin directory from wherever
# `Get-Command git` currently resolved (e.g. going up from .../cmd or
# .../bin). Testing it on this machine caught a second, worse bug in that
# approach before it shipped: here `git` resolves via `Git\mingw64\bin`
# (some other PATH entry wins ahead of the standard shim), so deriving from
# it computed `Git\mingw64\bin` as the target -- which contains a git.exe
# but NOT sh.exe (that's under `Git\usr\bin`, a sibling of `mingw64`, not a
# child of it). The derivation is exactly the kind of "looks right, isn't"
# logic this whole onboarding effort exists to catch.
#
# scripts/dev-agent.cmd already solves this exact problem, for the exact
# same reason (go-task spawning bash on Windows), by hardcoding the standard
# Git for Windows install location instead of deriving it -- confirmed
# real on this machine: sh.exe and bash.exe genuinely exist under
# `Git\bin`, and sh.exe also under `Git\usr\bin`, regardless of what `git`
# currently resolves to via other PATH entries. Matching that precedent
# here rather than reinventing a fragile derivation.
function Ensure-GitShellOnPath {
    if (Get-Command sh -ErrorAction SilentlyContinue) {
        return $true
    }

    $candidates = @(
        "$env:ProgramFiles\Git\bin",
        "$env:ProgramFiles\Git\usr\bin",
        "${env:ProgramFiles(x86)}\Git\bin",
        "${env:ProgramFiles(x86)}\Git\usr\bin"
    )
    $found = $candidates | Where-Object { Test-Path (Join-Path $_ "sh.exe") }

    if (-not $found) {
        Write-Host "Could not locate sh.exe under any standard Git for Windows install path ($($candidates -join ', '))."
        Write-Host "If Git is installed somewhere else, add its bin directory to PATH manually."
        return $false
    }

    foreach ($dir in $found) {
        if ($env:Path -notlike "*$dir*") {
            $env:Path = "$dir;$env:Path"
        }
    }

    # Persist to the User PATH too, so a new terminal also has it without
    # re-running this script -- Update-SessionPath only fixes the current
    # process; without this, `task init` would work now but break again
    # after closing this terminal.
    $currentUserPath = [System.Environment]::GetEnvironmentVariable("Path", "User")
    $toAdd = $found | Where-Object { $currentUserPath -notlike "*$_*" }
    if ($toAdd) {
        [System.Environment]::SetEnvironmentVariable("Path", ($currentUserPath, ($toAdd -join ";") -join ";"), "User")
    }

    return [bool](Get-Command sh -ErrorAction SilentlyContinue)
}

$gitOk = Install-WithWinget -DisplayName "Git" -Command "git" -WingetId "Git.Git"
$shellOk = $false
if ($gitOk) {
    $shellOk = Ensure-GitShellOnPath
    if (-not $shellOk) {
        Write-Host "Git is installed but its shell (sh.exe/bash.exe) could not be found or added to PATH."
        Write-Host "task init and other Task cross-platform commands need this -- see CLAUDE.md's Gap A/B notes."
    }
}
$taskOk = Install-WithWinget -DisplayName "Task" -Command "task" -WingetId "Task.Task"

Write-Host ""

if ($gitOk -and $shellOk -and $taskOk) {
    Write-Host "Git (with its shell on PATH) and Task are both ready: $(git --version), $(task --version)"
    Write-Host "Next: clone the repo if you haven't, cd into it, then run: task init"
    exit 0
} else {
    Write-Host "One or more tools (or Git's shell) could not be confirmed on PATH in this session."
    Write-Host "Open a NEW terminal (PATH changes from installers often don't reach this one) and re-run this script."
    exit 1
}
