# AgentMux shell integration for fish
# Deployed to: ~/.agentmux/shell/fish/wave.fish
# Loaded via: fish -C "source <this-file>"

# wsh has been retired — AGENTMUX is now a plain "1" sentinel.
# See specs/SPEC_RETIRE_WSH_2026_04_12.md.

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
        set current_agent "AGENTMUX_AGENT_ID:$AGENTMUX_AGENT_ID:COLOR:$AGENTMUX_AGENT_COLOR"
    end
    if test "$current_agent" != "$_AGENTMUX_SI_LAST_AGENT"
        set -g _AGENTMUX_SI_LAST_AGENT "$current_agent"
        if set -q AGENTMUX_AGENT_ID; and test -n "$AGENTMUX_AGENT_ID"
            set -l escaped (_agentmux_si_json_escape "$AGENTMUX_AGENT_ID")
            set -l payload "{\"AGENTMUX_AGENT_ID\":\"$escaped\""
            if set -q AGENTMUX_AGENT_COLOR; and test -n "$AGENTMUX_AGENT_COLOR"
                set -l color_escaped (_agentmux_si_json_escape "$AGENTMUX_AGENT_COLOR")
                set payload "$payload,\"AGENTMUX_AGENT_COLOR\":\"$color_escaped\""
            end
            set payload "$payload}"
            printf '\033]16162;E;%s\007' "$payload"
        else
            printf '\033]16162;E;{}\007'
        end
    end
end

# ─── muxlog helper ────────────────────────────────────────────────────────────
function muxlog
    set -l target (test (count $argv) -ge 1; and echo $argv[1]; or echo "host")
    set -l action (test (count $argv) -ge 2; and echo $argv[2]; or echo "tail")
    if not set -q AGENTMUX_LOG_DIR
        echo "AGENTMUX_LOG_DIR not set — run inside an AgentMux terminal" >&2
        return 1
    end
    set -l ptr "$AGENTMUX_LOG_DIR/current-$target-v$AGENTMUX_VERSION.path"
    if not test -f "$ptr"
        echo "Unknown log target '$target'. Check $AGENTMUX_LOG_DIR for current-*.path files." >&2
        return 1
    end
    set -l ptr_content (cat "$ptr")
    # Pointer content may be a basename (legacy: resolve under
    # AGENTMUX_LOG_DIR) or an absolute path (post-2026-05 host fix:
    # global pointer writes the absolute path so discovery works
    # from outside the instance dir).
    set -l logfile
    if string match -q '/*' "$ptr_content"; or string match -q '?:[/\\]*' "$ptr_content"
        set logfile "$ptr_content"
    else
        set logfile "$AGENTMUX_LOG_DIR/$ptr_content"
    end
    switch $action
        case tail
            tail -f "$logfile"
        case cat
            cat "$logfile"
        case '*'
            grep "$action" "$logfile"
    end
end

function _agentmux_si_prompt --on-event fish_prompt
    _agentmux_si_osc7
    _agentmux_si_agent_env
end
