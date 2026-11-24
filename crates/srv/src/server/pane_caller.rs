// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Whose pane a window command acts on, for a connection registered as an agent.
//!
//! An agent acts as itself: on its own pane (or a sub-block of it) a window command runs, and the action record names
//! the agent. It does not act as the person, and it does not act as another agent: a pane another agent runs, or one
//! no agent runs (the person's terminal), is refused with what to do instead. A connection that is not registered as
//! an agent (the window, or one nothing proved) is not affected.

use crate::backend::obj::Block;
use crate::backend::rpc_types::RpcContext;
use crate::backend::storage::store::Store;

/// The slug of the agent that runs `block_id`: the agent on the pane, or on the pane a sub-block hangs from.
fn pane_agent(mstore: &Store, block_id: &str) -> Option<String> {
    let direct = |id: &str| crate::backend::spawn_env::identity::block_agent_slug(mstore, id);
    direct(block_id).or_else(|| {
        let block: Block = mstore.get(block_id).ok().flatten()?;
        direct(block.parentoref.strip_prefix("block:")?)
    })
}

/// Whether the connection `ctx` may `doing` on `block_id`: always, unless it is registered as an agent and the pane is
/// not the one that agent runs. `Err` is the refusal, saying what the agent can do instead.
pub(crate) fn agent_acts_on_own_pane(mstore: &Store, ctx: &RpcContext, block_id: &str, doing: &str) -> Result<(), String> {
    if ctx.agent_id.is_empty() {
        return Ok(());
    }
    match pane_agent(mstore, block_id) {
        Some(owner) if owner.eq_ignore_ascii_case(&ctx.agent_id) => Ok(()),
        Some(_) => Err(format!(
            "FORBIDDEN: {doing} on another agent's pane is that agent's own act; do it on your own pane, or ask that agent with Message send"
        )),
        None => Err(format!(
            "FORBIDDEN: {doing} on a pane no agent runs is the person's act; do it on your own pane, or ask the person"
        )),
    }
}

/// Whether the connection `ctx` may delete the block `block_id` as a sub-block: always, unless it is registered as an agent
/// and the block is not a sub-block of the pane that agent runs. The pane itself is not a sub-block of anything, so an agent
/// never deletes its own pane this way.
pub(crate) fn agent_deletes_own_sub_block(mstore: &Store, ctx: &RpcContext, block_id: &str) -> Result<(), String> {
    agent_acts_on_own_pane(mstore, ctx, block_id, "deleting a sub-block")?;
    if ctx.agent_id.is_empty() {
        return Ok(());
    }
    let hangs_from_a_pane = mstore.get::<Block>(block_id).ok().flatten().is_some_and(|b| b.parentoref.starts_with("block:"));
    if hangs_from_a_pane {
        Ok(())
    } else {
        Err("FORBIDDEN: deleting a pane is the person's act; an agent releases its shell drawer with Terminal close and closes a pane it opened with Pane close".to_string())
    }
}

/// Whether the connection `ctx` may write the meta of `oref` (`block:<id>`, `tab:<id>`, ...): always, unless it is registered
/// as an agent. An agent writes the meta of its own pane; a tab, window, workspace or any other object holds the person's
/// layout, names and settings, so it is refused with what to do instead.
pub(crate) fn agent_writes_meta_of(mstore: &Store, ctx: &RpcContext, oref: &str) -> Result<(), String> {
    if ctx.agent_id.is_empty() {
        return Ok(());
    }
    match oref.split_once(':') {
        Some(("block", block_id)) => agent_acts_on_own_pane(mstore, ctx, block_id, "writing pane meta"),
        Some((kind, _)) => Err(format!(
            "FORBIDDEN: writing a {kind}'s meta is the person's act (the layout, names and settings of their window); an agent changes its own pane with the Pane and UI tools; to change this, ask the person"
        )),
        None => Err("FORBIDDEN: writing this object's meta is the person's act; an agent changes its own pane with the Pane and UI tools; to change this, ask the person".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::agents::test_agent_def;

    /// Two agents with a pane each, and a sub-block under the first one's pane, in a fresh store.
    fn fleet() -> Store {
        let store = Store::open_in_memory().unwrap();
        for id in ["pane-agent-a", "pane-agent-b"] {
            let mut def = test_agent_def(id, id, "claude", "agent", 1, "");
            def.slug = id.to_string();
            store.agent_def_insert(&mut def).unwrap();
        }
        let mut block = |oid: &str, agent: Option<&str>, parent: &str| {
            let mut b = Block { oid: oid.to_string(), parentoref: parent.to_string(), ..Default::default() };
            if let Some(agent) = agent {
                b.meta.insert("agentId".to_string(), serde_json::json!(agent));
            }
            store.insert(&mut b).unwrap();
        };
        block("pane-a", Some("pane-agent-a"), "");
        block("pane-b", Some("pane-agent-b"), "");
        block("pane-a-shell", None, "block:pane-a");
        block("persons-terminal", None, "");
        store
    }

    fn agent(slug: &str) -> RpcContext {
        RpcContext { agent_id: slug.to_string(), ..Default::default() }
    }

    #[test]
    fn an_agent_acts_on_its_own_pane_and_on_a_sub_block_of_it() {
        let store = fleet();
        for block in ["pane-a", "pane-a-shell"] {
            assert_eq!(agent_acts_on_own_pane(&store, &agent("pane-agent-a"), block, "x"), Ok(()), "{block}");
        }
        assert_eq!(agent_acts_on_own_pane(&store, &agent("PANE-AGENT-A"), "pane-a", "x"), Ok(()), "the slug is read without case");
    }

    #[test]
    fn an_agent_is_refused_on_another_agents_pane_and_on_one_no_agent_runs_and_told_what_to_do() {
        let store = fleet();
        let other = agent_acts_on_own_pane(&store, &agent("pane-agent-a"), "pane-b", "restart a controller").unwrap_err();
        assert!(other.starts_with("FORBIDDEN") && other.contains("another agent's pane") && other.contains("Message send"), "{other}");
        for block in ["persons-terminal", "no-such-pane"] {
            let refused = agent_acts_on_own_pane(&store, &agent("pane-agent-a"), block, "restart a controller").unwrap_err();
            assert!(refused.starts_with("FORBIDDEN") && refused.contains("no agent runs") && refused.contains("ask the person"), "{refused}");
        }
    }

    #[test]
    fn the_window_and_an_unproven_connection_are_not_affected() {
        let store = fleet();
        for ctx in [RpcContext::operator_for_tests(), RpcContext::default()] {
            for block in ["pane-a", "pane-b", "persons-terminal"] {
                assert_eq!(agent_acts_on_own_pane(&store, &ctx, block, "x"), Ok(()), "{block}");
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/pane_caller_tests.rs"]
mod handler_tests;
