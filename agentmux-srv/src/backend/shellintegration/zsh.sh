# AgentMux shell integration for zsh
# Deployed to: ~/.agentmux/shell/zsh/.zshrc
# Loaded via: ZDOTDIR=~/.agentmux/shell/zsh (zsh picks up .zshrc automatically)

# wsh has been retired — AGENTMUX is now a plain "1" sentinel.
# See docs/specs/archive/SPEC_RETIRE_WSH_2026_04_12.md.

# Source login profile (Homebrew shellenv and other login-shell setup live here)
if [ -f ~/.zprofile ]; then
    source ~/.zprofile
fi

# Source the user's real ~/.zshrc (since ZDOTDIR overrides it)
if [ -f ~/.zshrc ]; then
    source ~/.zshrc
fi

# ─── Shell Integration ────────────────────────────────────────────────────────

_agentmux_si_blocked() {
    [[ -n "$TMUX" || -n "$STY" || "$TERM" == tmux* || "$TERM" == screen* ]]
}

_agentmux_si_urlencode() {
    if (( $+functions[omz_urlencode] )); then
        omz_urlencode "$1"
    else
        local s="$1"
        s=${s//%/%25}
        s=${s// /%20}
        s=${s//#/%23}
        s=${s//\?/%3F}
        s=${s//&/%26}
        s=${s//;/%3B}
        s=${s//+/%2B}
        printf '%s' "$s"
    fi
}

_agentmux_si_osc7() {
    _agentmux_si_blocked && return
    local encoded_pwd
    encoded_pwd=$(_agentmux_si_urlencode "$PWD")
    printf '\033]7;file://%s%s\007' "$HOST" "$encoded_pwd"
}

_agentmux_si_json_escape() {
    local s="$1"
    s="${s//\\/\\\\}"
    s="${s//\"/\\\"}"
    printf '%s' "$s"
}

typeset -g _AGENTMUX_SI_LAST_AGENT=""

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

_agentmux_si_precmd() {
    _agentmux_si_blocked && return
    _agentmux_si_osc7
    _agentmux_si_agent_env
}

# ─── muxlog ───────────────────────────────────────────────────────────────────
# Discover, render & follow AgentMux logs across every running instance.
# Delegates to the shared Node core (muxlog.mjs). `muxlog help` for full usage,
# `muxlog ls` to list every instance's logs.
_AGENTMUX_MUXLOG_JS="${${(%):-%x}:A:h}/../muxlog.mjs"
muxlog() {
    if command -v node >/dev/null 2>&1 && [ -f "$_AGENTMUX_MUXLOG_JS" ]; then
        node "$_AGENTMUX_MUXLOG_JS" "$@"
        return
    fi
    # Fallback (no node / core missing): legacy pointer-based tail.
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
# live-state sibling). Delegates to the shared Node core (muxspect.mjs).
# `muxspect help` for usage. No non-Node fallback: introspection needs a real
# authenticated HTTP call.
_AGENTMUX_MUXSPECT_JS="${${(%):-%x}:A:h}/../muxspect.mjs"
muxspect() {
    if command -v node >/dev/null 2>&1 && [ -f "$_AGENTMUX_MUXSPECT_JS" ]; then
        node "$_AGENTMUX_MUXSPECT_JS" "$@"
        return
    fi
    echo "muxspect: Node unavailable or core missing at $_AGENTMUX_MUXSPECT_JS" >&2
    return 1
}

autoload -U add-zsh-hook
add-zsh-hook precmd _agentmux_si_precmd
add-zsh-hook chpwd  _agentmux_si_osc7
