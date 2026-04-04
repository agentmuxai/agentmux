# AgentMux shell integration for zsh
# Deployed to: ~/.agentmux/shell/zsh/.zshrc
# Loaded via: ZDOTDIR=~/.agentmux/shell/zsh (zsh picks up .zshrc automatically)

# Add wsh to PATH via the AGENTMUX executable path (portable mode support)
if [ -n "$AGENTMUX" ] && [ "$AGENTMUX" != "1" ]; then
    _agentmux_app_dir="$(dirname "$AGENTMUX")"
    export PATH="$_agentmux_app_dir:$PATH"
    unset _agentmux_app_dir
fi

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

autoload -U add-zsh-hook
add-zsh-hook precmd _agentmux_si_precmd
add-zsh-hook chpwd  _agentmux_si_osc7
