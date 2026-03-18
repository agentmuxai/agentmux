# AgentMux shell integration for PowerShell (pwsh / powershell.exe)
# Deployed to: ~/.agentmux/shell/pwsh/wavepwsh.ps1
# Loaded via: pwsh -ExecutionPolicy Bypass -NoExit -File <this-file>

# Add wsh to PATH via the AGENTMUX executable path (portable mode support)
if ($env:AGENTMUX -and $env:AGENTMUX -ne "1") {
    $agentmuxAppDir = Split-Path -Parent $env:AGENTMUX
    if ($agentmuxAppDir -and (Test-Path $agentmuxAppDir)) {
        $env:PATH = $agentmuxAppDir + [System.IO.Path]::PathSeparator + $env:PATH
    }
    Remove-Variable -Name agentmuxAppDir -ErrorAction SilentlyContinue
}

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
        $current_agent = "AGENTMUX_AGENT_ID:$($env:AGENTMUX_AGENT_ID)"
    }
    if ($current_agent -ne $Global:_AGENTMUX_SI_LAST_AGENT) {
        $Global:_AGENTMUX_SI_LAST_AGENT = $current_agent
        if ($env:AGENTMUX_AGENT_ID) {
            $escaped = _agentmux_si_json_escape $env:AGENTMUX_AGENT_ID
            Write-Host -NoNewline "${ESC}]16162;E;{`"AGENTMUX_AGENT_ID`":`"$escaped`"}`a"
        } else {
            Write-Host -NoNewline "${ESC}]16162;E;{}`a"
        }
    }
}

function Global:_agentmux_si_prompt {
    _agentmux_si_osc7
    _agentmux_si_agent_env
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
