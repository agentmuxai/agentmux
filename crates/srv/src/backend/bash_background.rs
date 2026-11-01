// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A Bash call that runs long goes to the background, so the agent is free to take the next message.
//!
//! Claude Code moves a Bash call to the background by itself when it has run past its timeout, gives
//! the model the tool result ("Command did not complete within its Ns timeout and was moved to the
//! background") and ends the turn; the command's end arrives later as a task notification that
//! starts a turn. That needs no terminal: it holds in AgentMux's hosting (stream-json, no PTY). The
//! timeout is the CLI's own `BASH_DEFAULT_TIMEOUT_MS` when the model sets none; the dock already
//! promotes a call at the same age (`TOOL_PROMOTION_MS`, `activity/tool-adapter.ts`).
//!
//! Only the default is set, not `BASH_MAX_TIMEOUT_MS`. The CLI does not background a command that
//! contains `git` (or starts with `sleep`): it stops it at the timeout. With the maximum at the same
//! age, a `git push`, a clone or a commit with slow hooks would be stopped at that age whatever
//! `timeout` the model asked for. A `timeout` the model sets itself is the `PreToolUse` hook's to
//! shape (`agentmux-bashwrap`, `bash_timeout.rs`): it clamps a longer one to the default so the call
//! goes to the background too, and leaves it alone for the commands the CLI stops at the limit, which
//! it also gives five minutes when the model set none.

use std::collections::HashMap;

use super::obj::{meta_get_string, MetaMapType};

/// How long a Bash call may run before it goes to the background.
pub const BACKGROUND_AFTER_MS: u64 = 30_000;
const DEFAULT_ENV: &str = "BASH_DEFAULT_TIMEOUT_MS";
/// srv's own environment variable that changes the age, in ms; `0` turns the behaviour off.
const AFTER_ENV: &str = "AGENTMUX_BASH_BACKGROUND_AFTER_MS";

/// Set the default Bash timeout in a launch environment: a Claude agent's only. A value the
/// environment already carries (the agent's own `cmd:env`) is kept, so an operator can still choose
/// otherwise; `after_ms` of `0` sets nothing.
pub fn apply(env_vars: &mut HashMap<String, String>, provider: &str, after_ms: u64) {
    if provider != "claude" || after_ms == 0 {
        return;
    }
    env_vars.entry(DEFAULT_ENV.to_string()).or_insert_with(|| after_ms.to_string());
}

/// The age in force: srv's override, else [`BACKGROUND_AFTER_MS`].
fn after_ms(override_value: Option<&str>) -> u64 {
    override_value.and_then(|v| v.trim().parse().ok()).unwrap_or(BACKGROUND_AFTER_MS)
}

/// [`apply`] for a block's launch, reading the provider from its meta. A block with no provider is a
/// Claude one, as elsewhere in the launch code.
pub fn apply_for_spawn(env_vars: &mut HashMap<String, String>, block_meta: &MetaMapType) {
    let provider = super::providers::resolve_provider_alias(&meta_get_string(block_meta, "agentProvider", "claude"));
    let provider = if provider.is_empty() { "claude" } else { provider };
    apply(env_vars, provider, after_ms(std::env::var(AFTER_ENV).ok().as_deref()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_claude_launch_gets_the_default_timeout_and_not_the_maximum() {
        let mut env = HashMap::new();
        apply(&mut env, "claude", BACKGROUND_AFTER_MS);
        assert_eq!(env[DEFAULT_ENV], "30000");
        assert_eq!(env.len(), 1, "a model's longer timeout must still be honoured (git commands are stopped, not backgrounded)");
    }

    #[test]
    fn another_provider_is_left_alone() {
        for provider in ["codex", "gemini", "qwen", ""] {
            let mut env = HashMap::new();
            apply(&mut env, provider, BACKGROUND_AFTER_MS);
            assert!(env.is_empty(), "{provider}");
        }
    }

    #[test]
    fn the_agents_own_timeout_is_kept() {
        let mut env = HashMap::from([(DEFAULT_ENV.to_string(), "600000".to_string())]);
        apply(&mut env, "claude", BACKGROUND_AFTER_MS);
        assert_eq!(env[DEFAULT_ENV], "600000");
    }

    #[test]
    fn zero_turns_it_off_and_a_bad_value_means_the_default() {
        let mut env = HashMap::new();
        apply(&mut env, "claude", after_ms(Some("0")));
        assert!(env.is_empty());
        assert_eq!(after_ms(Some("45000")), 45_000);
        assert_eq!(after_ms(Some("soon")), BACKGROUND_AFTER_MS);
        assert_eq!(after_ms(None), BACKGROUND_AFTER_MS);
    }

    #[test]
    fn a_block_with_no_provider_or_an_alias_is_a_claude_launch() {
        for meta in [MetaMapType::new(), MetaMapType::from_iter([("agentProvider".to_string(), serde_json::json!("claude-code"))])] {
            let mut env = HashMap::new();
            apply_for_spawn(&mut env, &meta);
            if std::env::var(AFTER_ENV).is_err() {
                assert_eq!(env[DEFAULT_ENV], "30000");
            }
        }
    }
}
