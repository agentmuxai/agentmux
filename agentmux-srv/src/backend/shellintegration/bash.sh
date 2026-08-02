# AgentMux shell integration for bash
# Deployed to: ~/.agentmux/shell/bash/.bashrc
# Loaded via: bash --rcfile <this-file>

# Source /etc/profile for system-wide settings
if [ -f /etc/profile ]; then
    . /etc/profile
fi

# wsh has been retired — AGENTMUX is now a plain "1" sentinel.
# See specs/SPEC_RETIRE_WSH_2026_04_12.md.

# Source the first of ~/.bash_profile, ~/.bash_login, or ~/.profile that exists
if [ -f ~/.bash_profile ]; then
    . ~/.bash_profile
elif [ -f ~/.bash_login ]; then
    . ~/.bash_login
elif [ -f ~/.profile ]; then
    . ~/.profile
fi

# ─── Shell Integration ────────────────────────────────────────────────────────

_agentmux_si_blocked() {
    [[ -n "$TMUX" || -n "$STY" || "$TERM" == tmux* || "$TERM" == screen* ]]
}

_agentmux_si_urlencode() {
    local s="$1"
    s="${s//%/%25}"
    s="${s// /%20}"
    s="${s//#/%23}"
    s="${s//\?/%3F}"
    s="${s//&/%26}"
    s="${s//;/%3B}"
    s="${s//+/%2B}"
    printf '%s' "$s"
}

_agentmux_si_osc7() {
    _agentmux_si_blocked && return
    local encoded_pwd
    encoded_pwd=$(_agentmux_si_urlencode "$PWD")
    printf '\033]7;file://%s%s\007' "$HOSTNAME" "$encoded_pwd"
}

_agentmux_si_json_escape() {
    local s="$1"
    s="${s//\\/\\\\}"
    s="${s//\"/\\\"}"
    printf '%s' "$s"
}

_AGENTMUX_SI_LAST_AGENT=""

# Send AGENTMUX_AGENT_ID via OSC 16162;E on every prompt (only when changed)
_agentmux_si_agent_env() {
    _agentmux_si_blocked && return
    local current_agent=""
    if [[ -n "$AGENTMUX_AGENT_ID" ]]; then
        current_agent="AGENTMUX_AGENT_ID:$AGENTMUX_AGENT_ID:COLOR:$AGENTMUX_AGENT_COLOR"
    fi
    if [[ "$current_agent" != "$_AGENTMUX_SI_LAST_AGENT" ]]; then
        _AGENTMUX_SI_LAST_AGENT="$current_agent"
        if [[ -n "$AGENTMUX_AGENT_ID" ]]; then
            local escaped
            escaped=$(_agentmux_si_json_escape "$AGENTMUX_AGENT_ID")
            local payload="{\"AGENTMUX_AGENT_ID\":\"$escaped\""
            if [[ -n "$AGENTMUX_AGENT_COLOR" ]]; then
                local color_escaped
                color_escaped=$(_agentmux_si_json_escape "$AGENTMUX_AGENT_COLOR")
                payload="$payload,\"AGENTMUX_AGENT_COLOR\":\"$color_escaped\""
            fi
            payload="$payload}"
            printf '\033]16162;E;%s\007' "$payload"
        else
            printf '\033]16162;E;{}\007'
        fi
    fi
}

_agentmux_si_prompt_command() {
    _agentmux_si_osc7
    _agentmux_si_agent_env
}

# ─── muxlog ───────────────────────────────────────────────────────────────────
# Discover, render & follow AgentMux logs across every running instance.
# Delegates to the shared Node core (muxlog.mjs, deployed next to this rcfile).
#   muxlog                 tail the most-recently-active host log (follow)
#   muxlog ls              list every instance's logs (newest first)
#   muxlog srv grep <re>   search the active sidecar log (transcript excluded)
#   muxlog bridge          startup-handshake trace (debug reconnect loops)
#   muxlog help            full usage
# Resolve muxlog.mjs once, at source time (BASH_SOURCE = this rcfile).
_AGENTMUX_MUXLOG_JS="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." 2>/dev/null && pwd)/muxlog.mjs"
muxlog() {
    if command -v node >/dev/null 2>&1 && [ -f "$_AGENTMUX_MUXLOG_JS" ]; then
        node "$_AGENTMUX_MUXLOG_JS" "$@"
        return
    fi
    # Fallback (no node / core missing): legacy pointer-based tail, so logs are
    # never wholly inaccessible.
    local target="${1:-host}" action="${2:-tail}"
    [ -z "$AGENTMUX_LOG_DIR" ] && { echo "AGENTMUX_LOG_DIR not set — run inside an AgentMux terminal" >&2; return 1; }
    local ptr="$AGENTMUX_LOG_DIR/current-${target}-v${AGENTMUX_VERSION}.path"
    [ -f "$ptr" ] || { echo "muxlog: Node core unavailable and no pointer for '$target'" >&2; return 1; }
    local pc logfile; pc="$(cat "$ptr")"
    case "$pc" in /* | ?:[/\\]*) logfile="$pc" ;; *) logfile="$AGENTMUX_LOG_DIR/$pc" ;; esac
    case "$action" in tail) tail -f "$logfile" ;; cat) cat "$logfile" ;; *) grep "$action" "$logfile" ;; esac
}

# ─── muxspect ───────────────────────────────────────────────────────────────
# Live process/turn-state introspection for the CURRENT instance (muxlog's
# live-state sibling — muxlog answers "what happened", muxspect answers
# "what's happening right now"). Delegates to the shared Node core
# (muxspect.mjs, deployed next to this rcfile). `muxspect help` for usage.
# No non-Node fallback: introspection needs a real authenticated HTTP call.
_AGENTMUX_MUXSPECT_JS="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." 2>/dev/null && pwd)/muxspect.mjs"
muxspect() {
    if command -v node >/dev/null 2>&1 && [ -f "$_AGENTMUX_MUXSPECT_JS" ]; then
        node "$_AGENTMUX_MUXSPECT_JS" "$@"
        return
    fi
    echo "muxspect: Node unavailable or core missing at $_AGENTMUX_MUXSPECT_JS" >&2
    return 1
}

# Append to PROMPT_COMMAND (array-safe)
if [[ $(declare -p PROMPT_COMMAND 2>/dev/null) == "declare -a"* ]]; then
    PROMPT_COMMAND+=(_agentmux_si_prompt_command)
else
    PROMPT_COMMAND="${PROMPT_COMMAND:+$PROMPT_COMMAND$'\n'}_agentmux_si_prompt_command"
fi
