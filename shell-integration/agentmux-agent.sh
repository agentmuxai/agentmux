# AgentMux Agent Integration
# Source this file in your .bashrc or .zshrc to enable per-pane agent colors
#
# Usage:
#   export WAVEMUX_AGENT_ID=AgentA   # Set the agent ID
#   The pane title and color will update automatically on each prompt
#
# Supported agents: AgentA (blue), AgentB (green), AgentC (orange), AgentD (purple)

# Function to send OSC 16162 E command with current WAVEMUX_AGENT_ID
__agentmux_send_agent_env() {
    if [[ -n "$WAVEMUX_AGENT_ID" ]]; then
        # Send OSC 16162;E;{JSON} to update block's cmd:env metadata
        printf '\e]16162;E;{"WAVEMUX_AGENT_ID":"%s"}\a' "$WAVEMUX_AGENT_ID"
    fi
}

# Detect shell and set up prompt hook
if [[ -n "$BASH_VERSION" ]]; then
    # Bash: Add to PROMPT_COMMAND
    if [[ -z "$PROMPT_COMMAND" ]]; then
        PROMPT_COMMAND="__agentmux_send_agent_env"
    elif [[ "$PROMPT_COMMAND" != *"__agentmux_send_agent_env"* ]]; then
        PROMPT_COMMAND="__agentmux_send_agent_env;$PROMPT_COMMAND"
    fi
elif [[ -n "$ZSH_VERSION" ]]; then
    # Zsh: Add to precmd hooks
    autoload -Uz add-zsh-hook
    add-zsh-hook precmd __agentmux_send_agent_env
fi

# Convenience function to set agent and immediately update
set_agent() {
    export WAVEMUX_AGENT_ID="$1"
    __agentmux_send_agent_env
}
