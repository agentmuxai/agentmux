# AgentMux shell integration for PowerShell (pwsh / powershell.exe)
# Deployed to: ~/.agentmux/shell/pwsh/wavepwsh.ps1
# Loaded via: pwsh -ExecutionPolicy Bypass -NoExit -File <this-file>

# wsh has been retired — AGENTMUX is now a plain "1" sentinel.
# See docs/specs/archive/SPEC_RETIRE_WSH_2026_04_12.md.

# ─── Shell Integration ────────────────────────────────────────────────────────

# PS5 (Windows PowerShell 5.1) does not support `e as ESC — use [char]0x1B instead
if ($PSVersionTable.PSVersion.Major -ge 7) { $ESC = "`e" } else { $ESC = [char]0x1B }

function Global:_agentmux_si_blocked {
    return ($env:TMUX -or $env:STY -or $env:TERM -like "tmux*" -or $env:TERM -like "screen*")
}

function Global:_agentmux_si_osc7 {
    if (_agentmux_si_blocked) { return }
    $hostname = if ($env:COMPUTERNAME) { $env:COMPUTERNAME } else { $env:HOSTNAME }
    $encoded = [System.Uri]::EscapeDataString($PWD.Path)
    Write-Host -NoNewline "${ESC}]7;file://$hostname/$encoded`a"
}

function Global:_agentmux_si_json_escape {
    param([string]$s)
    $s = $s.Replace('\', '\\')
    $s = $s.Replace('"', '\"')
    return $s
}

$Global:_AGENTMUX_SI_LAST_AGENT = ""

# Send AGENTMUX_AGENT_ID via OSC 16162;E on every prompt (only when changed)
function Global:_agentmux_si_agent_env {
    if (_agentmux_si_blocked) { return }
    $current_agent = ""
    if ($env:AGENTMUX_AGENT_ID) {
        $current_agent = "AGENTMUX_AGENT_ID:$($env:AGENTMUX_AGENT_ID):COLOR:$($env:AGENTMUX_AGENT_COLOR)"
    }
    if ($current_agent -ne $Global:_AGENTMUX_SI_LAST_AGENT) {
        $Global:_AGENTMUX_SI_LAST_AGENT = $current_agent
        if ($env:AGENTMUX_AGENT_ID) {
            $escaped = _agentmux_si_json_escape $env:AGENTMUX_AGENT_ID
            $payload = "{`"AGENTMUX_AGENT_ID`":`"$escaped`""
            if ($env:AGENTMUX_AGENT_COLOR) {
                $colorEscaped = _agentmux_si_json_escape $env:AGENTMUX_AGENT_COLOR
                $payload += ",`"AGENTMUX_AGENT_COLOR`":`"$colorEscaped`""
            }
            $payload += "}"
            Write-Host -NoNewline "${ESC}]16162;E;${payload}`a"
        } else {
            Write-Host -NoNewline "${ESC}]16162;E;{}`a"
        }
    }
}

function Global:_agentmux_si_prompt {
    _agentmux_si_osc7
    _agentmux_si_agent_env
}

# ─── muxlog ───────────────────────────────────────────────────────────────────
# Discover, render & follow AgentMux logs across every running instance.
# Delegates to the shared Node core (muxlog.mjs). `muxlog help` for usage,
# `muxlog ls` to list every instance's logs.
$global:AgentmuxMuxlogJs = Join-Path $PSScriptRoot "..\muxlog.mjs"
function Global:muxlog {
    if ((Get-Command node -ErrorAction SilentlyContinue) -and (Test-Path $global:AgentmuxMuxlogJs)) {
        node $global:AgentmuxMuxlogJs @args
        return
    }
    # Fallback (no node / core missing): legacy pointer-based tail.
    $Target = if ($args.Count -ge 1) { $args[0] } else { "host" }
    $Action = if ($args.Count -ge 2) { $args[1] } else { "tail" }
    if (-not $env:AGENTMUX_LOG_DIR) { Write-Error "AGENTMUX_LOG_DIR not set - run inside an AgentMux terminal"; return }
    $ptr = Join-Path $env:AGENTMUX_LOG_DIR "current-$Target-v$($env:AGENTMUX_VERSION).path"
    if (-not (Test-Path $ptr)) { Write-Error "muxlog: Node core unavailable and no pointer for '$Target'"; return }
    $pc = Get-Content $ptr
    $logfile = if ($pc -match '^([A-Za-z]:[\\/]|[\\/])') { $pc } else { Join-Path $env:AGENTMUX_LOG_DIR $pc }
    switch ($Action) {
        "tail" { Get-Content $logfile -Wait -Tail 50 }
        "cat"  { Get-Content $logfile }
        default { Select-String $Action $logfile }
    }
}

# ─── muxspect ───────────────────────────────────────────────────────────────
# Live process/turn-state introspection for the CURRENT instance (muxlog's
# live-state sibling). Delegates to the shared Node core (muxspect.mjs).
# `muxspect help` for usage. No non-Node fallback: introspection needs a real
# authenticated HTTP call.
$global:AgentmuxMuxspectJs = Join-Path $PSScriptRoot "..\muxspect.mjs"
function Global:muxspect {
    if ((Get-Command node -ErrorAction SilentlyContinue) -and (Test-Path $global:AgentmuxMuxspectJs)) {
        node $global:AgentmuxMuxspectJs @args
        return
    }
    Write-Error "muxspect: Node unavailable or core missing at $global:AgentmuxMuxspectJs"
}

# ─── muxopen ────────────────────────────────────────────────────────────────
# Launch an agent into a pane from the terminal (no GUI). Constructive
# sibling of the stop verbs. `muxopen help` for usage.
$global:AgentmuxMuxopenJs = Join-Path $PSScriptRoot "..\muxopen.mjs"
function Global:muxopen {
    if ((Get-Command node -ErrorAction SilentlyContinue) -and (Test-Path $global:AgentmuxMuxopenJs)) {
        node $global:AgentmuxMuxopenJs @args
        return
    }
    Write-Error "muxopen: Node unavailable or core missing at $global:AgentmuxMuxopenJs"
}

# Hook into the prompt function
if (Test-Path Function:\prompt) {
    $global:_agentmux_original_prompt = $function:prompt
    function Global:prompt {
        _agentmux_si_prompt
        & $global:_agentmux_original_prompt
    }
} else {
    function Global:prompt {
        _agentmux_si_prompt
        "PS $($executionContext.SessionState.Path.CurrentLocation)$('>' * ($nestedPromptLevel + 1)) "
    }
}
