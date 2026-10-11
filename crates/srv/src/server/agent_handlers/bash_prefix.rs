// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The Claude agent's shell prefix: `agentmux-bashwrap` as the CLI's
//! `CLAUDE_CODE_SHELL_PREFIX`, which streams Bash output without the hook
//! rewriting the call (docs/specs/SPEC_BASH_STREAMING_VIA_SHELL_PREFIX_2026_10_10.md).

/// Set the prefix in an agent's spawn environment
/// (`input::build_persistent_spawn_env`). Not for a pane in the MCP
/// self-heal's safe profile: its MCP server failed to start and reconnecting
/// didn't help, so its CLI runs without the prefix, as with
/// `AGENTMUX_BASHWRAP_MODE=rewrite` (`backend::mcp_health`).
pub(crate) fn apply_for_spawn(env_vars: &mut std::collections::HashMap<String, String>, block_id: &str) {
    let mode = mode_for(block_id);
    apply_bash_shell_prefix(env_vars, crate::backend::tool_store::bashwrap_path(), mode.as_deref());
}

/// `rewrite` for a pane in the safe profile, else srv's own setting.
fn mode_for(block_id: &str) -> Option<String> {
    if crate::backend::mcp_health::is_safe_profile(block_id) {
        tracing::info!(block_id, "MCP self-heal safe profile: starting the agent without the shell prefix");
        return Some("rewrite".to_string());
    }
    std::env::var(BASHWRAP_MODE_ENV).ok()
}

/// srv's own environment variable that turns the shell prefix off:
/// `rewrite` keeps the hook rewriting commands, as before the prefix.
const BASHWRAP_MODE_ENV: &str = "AGENTMUX_BASHWRAP_MODE";

/// Claude Code runs every Bash tool command through `CLAUDE_CODE_SHELL_PREFIX`
/// when it is set: pointed at `agentmux-bashwrap`, that streams the command
/// without the hook rewriting the call, so the CLI's permission rules and
/// labels see the model's own command
/// (docs/specs/SPEC_BASH_STREAMING_VIA_SHELL_PREFIX_2026_10_10.md §2.4).
/// Only Claude Code reads the variable. A prefix the agent's own `cmd:env`
/// sets is kept (the hook then rewrites, as it does without a prefix); no
/// prefix is set without the wrapper, or when srv runs with
/// `AGENTMUX_BASHWRAP_MODE=rewrite`.
fn apply_bash_shell_prefix(
    env_vars: &mut std::collections::HashMap<String, String>,
    bashwrap: Option<std::path::PathBuf>,
    mode: Option<&str>,
) {
    const PREFIX_ENV: &str = "CLAUDE_CODE_SHELL_PREFIX";
    if env_vars.contains_key(PREFIX_ENV) || mode.is_some_and(|m| m.eq_ignore_ascii_case("rewrite")) {
        return;
    }
    if let Some(path) = bashwrap {
        env_vars.insert(PREFIX_ENV.to_string(), path.to_string_lossy().into_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::apply_bash_shell_prefix;

    const PREFIX: &str = "CLAUDE_CODE_SHELL_PREFIX";

    #[test]
    fn the_shell_prefix_names_the_bundled_wrapper() {
        let mut env = std::collections::HashMap::new();
        let wrapper = std::path::PathBuf::from("/opt/agentmux/tools/bin/agentmux-bashwrap");
        apply_bash_shell_prefix(&mut env, Some(wrapper.clone()), None);
        assert_eq!(env.get(PREFIX).map(String::as_str), Some(wrapper.to_str().unwrap()));
    }

    #[test]
    fn no_shell_prefix_without_the_wrapper_or_when_switched_off() {
        let mut env = std::collections::HashMap::new();
        apply_bash_shell_prefix(&mut env, None, None);
        assert!(!env.contains_key(PREFIX));
        let wrapper = Some(std::path::PathBuf::from("/x/agentmux-bashwrap"));
        apply_bash_shell_prefix(&mut env, wrapper.clone(), Some("rewrite"));
        apply_bash_shell_prefix(&mut env, wrapper, Some("REWRITE"));
        assert!(!env.contains_key(PREFIX));
    }

    #[test]
    fn an_agents_own_shell_prefix_is_kept() {
        let mut env = std::collections::HashMap::from([(PREFIX.to_string(), "/home/me/log-commands.sh".to_string())]);
        apply_bash_shell_prefix(&mut env, Some("/x/agentmux-bashwrap".into()), None);
        assert_eq!(env[PREFIX], "/home/me/log-commands.sh");
    }

    /// A pane whose MCP server failed and wouldn't reconnect starts without
    /// the prefix; other panes are unaffected.
    #[test]
    fn a_pane_in_the_safe_profile_gets_no_prefix() {
        use crate::backend::mcp_health::{on_reconnect, on_status, Status};
        let block = format!("safe-profile-test-{}", uuid::Uuid::new_v4());
        on_status(&block, "p1", Status::Failed, None, true, 0);
        on_reconnect(&block, "p1", Err("Connection closed".into()), 1);
        on_reconnect(&block, "p1", Err("Connection closed".into()), 2);
        assert_eq!(super::mode_for(&block).as_deref(), Some("rewrite"));
        if std::env::var("AGENTMUX_BASHWRAP_MODE").is_err() {
            assert_eq!(super::mode_for("another-pane"), None);
        }
    }
}
