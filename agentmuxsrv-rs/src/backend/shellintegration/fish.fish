# AgentMux shell integration for fish
# Deployed to: ~/.agentmux/shell/fish/wave.fish
# Loaded via: fish -C "source <this-file>"

# Add wsh to PATH via the AGENTMUX executable path (portable mode support)
if set -q AGENTMUX; and test "$AGENTMUX" != "1"
    set _agentmux_app_dir (dirname "$AGENTMUX")
    if test -d "$_agentmux_app_dir"
        fish_add_path --prepend "$_agentmux_app_dir"
    end
    set -e _agentmux_app_dir
end

# ─── Shell Integration ────────────────────────────────────────────────────────

function _agentmux_si_blocked
    test -n "$TMUX"; or test -n "$STY"
end

function _agentmux_si_osc7
    _agentmux_si_blocked; and return
    set -l encoded (string escape --style url -- $PWD)
    printf '\033]7;file://%s%s\007' (hostname) "$encoded"
end

function _agentmux_si_json_escape
    set -l s $argv[1]
    set s (string replace -a '\\' '\\\\' -- $s)
    set s (string replace -a '"' '\\"' -- $s)
    printf '%s' $s
end

set -g _AGENTMUX_SI_LAST_AGENT ""

# Send AGENTMUX_AGENT_ID via OSC 16162;E on every prompt (only when changed)
function _agentmux_si_agent_env
    _agentmux_si_blocked; and return
    set -l current_agent ""
    if set -q AGENTMUX_AGENT_ID; and test -n "$AGENTMUX_AGENT_ID"
        set current_agent "AGENTMUX_AGENT_ID:$AGENTMUX_AGENT_ID"
    end
    if test "$current_agent" != "$_AGENTMUX_SI_LAST_AGENT"
        set -g _AGENTMUX_SI_LAST_AGENT "$current_agent"
        if set -q AGENTMUX_AGENT_ID; and test -n "$AGENTMUX_AGENT_ID"
            set -l escaped (_agentmux_si_json_escape "$AGENTMUX_AGENT_ID")
            printf '\033]16162;E;{"AGENTMUX_AGENT_ID":"%s"}\007' "$escaped"
        else
            printf '\033]16162;E;{}\007'
        end
    end
end

function _agentmux_si_prompt --on-event fish_prompt
    _agentmux_si_osc7
    _agentmux_si_agent_env
end
