// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Input into a pane over the window's connection needs proof of the sender.
//!
//! The window's input commands (`blockinput` keystrokes, `setblocktermsize`, `controllerinput`'s Stop and resize, and
//! `agent.send`) are the person's hands: what they write reads as the person's own. They are accepted from a
//! connection that has proved what it is (`RpcContext::operator_only`: the AgentMux window proven through its host, or
//! a browser the person signed in to a headless srv), and from no other. Holding the instance key, or having no agent
//! id, is not proof. An agent writes into its own terminal with the Terminal tool and reaches another agent with
//! Message send.
//!
//! For development only, a source-tree build (`task dev`) run on an isolated home can be told to let a test harness type
//! without the window's proof ([`DEV_SWITCH`]). An installed build never honours it.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

use crate::backend::rpc_types::RpcContext;

const WHY: &str = "only the person's window writes input into a pane; ask the person, or use the Terminal tool for your own pane and Message send for another agent";

/// The environment variable that lets a harness type into panes of an isolated source-tree build (`task dev`) without the window's proof.
pub(crate) const DEV_SWITCH: &str = "AGENTMUX_DEV_INPUT_WITHOUT_PROOF";

/// The refusals logged in a window of this length are one line per kind, so a stream of refused keystrokes cannot fill the log.
const LOG_EVERY_MS: u64 = 10_000;

static LAST_LOGGED: [AtomicU64; 3] = [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];

/// Whether this process honours [`DEV_SWITCH`]: only a source-tree build (`task dev`: the binary runs from `dist/cef-dev*` or
/// a cargo target folder, which `is_dev_self` answers from the binary's own path, never from the environment), on an isolated
/// home, with the variable set to `1`. Read once: the check on every keystroke is a flag read, never an environment lookup.
fn dev_switch_on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| dev_switch_in(agentmux_common::is_dev_self(), |name| std::env::var(name).ok()))
}

fn dev_switch_in(source_tree_build: bool, var: impl Fn(&str) -> Option<String>) -> bool {
    source_tree_build && var(DEV_SWITCH).as_deref() == Some("1") && var("AGENTMUX_HOME_OVERRIDE").is_some_and(|v| !v.is_empty())
}

/// Logs once at start when the dev switch is in force, so a build that types without proof is never a surprise.
pub(crate) fn log_dev_switch_at_start() {
    if dev_switch_on() {
        tracing::warn!("{DEV_SWITCH} is set: this isolated source-tree build accepts input into panes from connections without the window's proof");
    }
}

/// `Ok` when `ctx` may write `command`'s input into a pane; otherwise the refusal, logged with the kind of connection
/// and nothing of the input.
pub(crate) fn require_window_input(ctx: &RpcContext, command: &str) -> Result<(), String> {
    require_in(ctx, command, dev_switch_on())
}

fn require_in(ctx: &RpcContext, command: &str, dev_switch: bool) -> Result<(), String> {
    // An agent's connection is never let through, switch or not.
    if dev_switch && ctx.agent_id.is_empty() {
        return Ok(());
    }
    let Err(refusal) = ctx.operator_only(WHY) else { return Ok(()) };
    let kind = if !ctx.agent_id.is_empty() {
        0
    } else if ctx.operator_proven {
        1
    } else {
        2
    };
    let now = agentmux_common::time::now_ms_u64();
    let last = LAST_LOGGED[kind].load(Ordering::Relaxed);
    if now.saturating_sub(last) >= LOG_EVERY_MS && LAST_LOGGED[kind].compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed).is_ok() {
        let what = ["a connection registered as an agent", "a window whose proof has lapsed", "a connection without the window's proof"][kind];
        tracing::warn!(command, client_type = %ctx.client_type, "input into a pane refused: {what}");
    }
    Err(refusal)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent() -> RpcContext {
        RpcContext { agent_id: "korp".into(), operator_proven: true, ..Default::default() }
    }

    #[test]
    fn only_the_proven_window_may_write_input() {
        assert!(require_in(&RpcContext::operator_for_tests(), "blockinput", false).is_ok());
        let unproven = require_in(&RpcContext::default(), "blockinput", false).unwrap_err();
        assert!(unproven.starts_with("FORBIDDEN: only the person"), "{unproven}");
        assert!(unproven.contains("isn't the AgentMux window"), "{unproven}");
    }

    #[test]
    fn an_agent_is_refused_even_with_the_proof_and_even_with_the_dev_switch() {
        for dev in [false, true] {
            let e = require_in(&agent(), "agent.send", dev).unwrap_err();
            assert!(e.starts_with("FORBIDDEN: only the person") && e.contains("ask the person"), "{e}");
        }
    }

    #[test]
    fn the_dev_switch_lets_an_unregistered_connection_type_and_nothing_else() {
        assert!(require_in(&RpcContext::default(), "controllerinput", true).is_ok());
    }

    #[test]
    fn an_installed_build_a_shared_home_or_a_missing_value_never_honours_the_switch() {
        let env = |pairs: &'static [(&'static str, &'static str)]| move |name: &str| pairs.iter().find(|(k, _)| *k == name).map(|(_, v)| v.to_string());
        let all = &[(DEV_SWITCH, "1"), ("AGENTMUX_HOME_OVERRIDE", "C:/dev-home")];
        assert!(dev_switch_in(true, env(all)));
        assert!(!dev_switch_in(false, env(all)), "an installed build");
        assert!(!dev_switch_in(true, env(&[(DEV_SWITCH, "1")])), "no isolated home");
        assert!(!dev_switch_in(true, env(&[(DEV_SWITCH, "1"), ("AGENTMUX_HOME_OVERRIDE", "")])), "an empty home override");
        assert!(!dev_switch_in(true, env(&[(DEV_SWITCH, "yes"), ("AGENTMUX_HOME_OVERRIDE", "C:/dev-home")])), "any value but 1");
        assert!(!dev_switch_in(true, env(&[("AGENTMUX_HOME_OVERRIDE", "C:/dev-home")])), "unset");
    }
}
