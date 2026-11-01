// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! One pane per agent, for the layouts that already hold more (task #5419). An older build, an import or a
//! restore can leave two panes of one agent. The one that keeps the agent is the one running it, else the
//! one it launched last ([`super::agent_block::agent_blocks_by_uid`]); every other becomes a closed pane
//! that says where the agent is and offers Go there and Move here (`open_in_other_pane`, the recovery the
//! duplicate-pane spec built), so what a person types there is never taken in silence.
//!
//! Runs once at startup and after a layout is opened. Each duplicate is one log line and one entry in
//! Warden's Audit (`agent.reconcile_duplicate_pane`, by `agentmux`); a pane already closed this way for the
//! same keeper is left alone, so a second run changes nothing.

use super::agent_block::{agent_blocks_by_uid, blocks_in_tabs_or_parked, describe_tab, AgentBlock};
use super::agent_attach::{set_desired, DESIRED_STOPPED};
use crate::backend::blockcontroller;
use crate::backend::obj::{self, Block};
use crate::backend::rpc::action_record::{ActionRecord, Target};
use crate::backend::rpc::actor::Actor;
use crate::backend::storage::store::Store;
use crate::server::AppState;
use std::collections::BTreeSet;

/// The command named in the audit.
pub(crate) const AUDIT_COMMAND: &str = "agent.reconcile_duplicate_pane";

/// An agent's pane that is not the agent's: `keeper` holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Duplicate {
    pub agent: String,
    pub block_id: String,
    pub tab_id: Option<String>,
    pub keeper: String,
    pub keeper_tab_id: Option<String>,
}

/// The agents (by the id their blocks resolve to) that have a pane or a parked block.
fn agents_with_blocks(mstore: &Store) -> Result<BTreeSet<String>, String> {
    let mut agents = BTreeSet::new();
    for (block, _) in blocks_in_tabs_or_parked(mstore)? {
        if obj::meta_get_string(&block.meta, "view", "") != "agent" {
            continue;
        }
        let bound = obj::meta_get_string(&block.meta, "agentInstanceId", "");
        let id = if bound.is_empty() { obj::meta_get_string(&block.meta, "agentId", "") } else { bound };
        if !id.is_empty() {
            agents.insert(id);
        }
    }
    Ok(agents)
}

/// Every pane that is a second (or later) pane of an agent. A template's id is skipped: the panes launched
/// from it are different agents, told apart by their instance.
pub(crate) fn find_duplicates(mstore: &Store) -> Result<Vec<Duplicate>, String> {
    let mut out = Vec::new();
    for agent in agents_with_blocks(mstore)? {
        if mstore.instance_get(&agent).ok().flatten().is_none() {
            continue;
        }
        let mut panes = agent_blocks_by_uid(mstore, &agent)?.into_iter();
        let Some(AgentBlock { block: keeper, tab_id: keeper_tab_id }) = panes.next() else { continue };
        for AgentBlock { block, tab_id } in panes {
            out.push(Duplicate { agent: agent.clone(), block_id: block.oid, tab_id, keeper: keeper.oid.clone(), keeper_tab_id: keeper_tab_id.clone() });
        }
    }
    Ok(out)
}

fn place(mstore: &Store, tab_id: &Option<String>) -> String {
    match tab_id {
        Some(tab) => describe_tab(mstore, tab),
        None => "no tab (parked)".to_string(),
    }
}

/// Whether the pane already says it is closed for `keeper`.
fn already_closed_for(block: &Block, keeper: &str) -> bool {
    block
        .meta
        .get(blockcontroller::core::META_LAST_FAILURE)
        .is_some_and(|f| f.get("code").and_then(|c| c.as_str()) == Some("open_in_other_pane") && f.get("holderBlockId").and_then(|h| h.as_str()) == Some(keeper))
}

/// Close one duplicate: it says where the agent is, and the agent is not wanted running by it.
fn close_duplicate(state: &AppState, d: &Duplicate) {
    let mstore = &state.mstore;
    let name = mstore.instance_get(&d.agent).ok().flatten().and_then(|_| mstore.agent_def_get(&d.agent).ok().flatten()).map(|def| def.name).unwrap_or_else(|| "This agent".to_string());
    let (here, there) = (place(mstore, &d.tab_id), place(mstore, &d.keeper_tab_id));
    tracing::warn!(
        agent = %d.agent, block_id = %d.block_id, kept = %d.keeper,
        "agent_panes.duplicate: {name} had a second pane in {here}; its pane in {there} is kept and this one is closed"
    );
    let mut failure = crate::agents::failure::classify(None, None, &crate::backend::blockcontroller::persistent::held_elsewhere_error(&d.keeper, false), None);
    failure.title = format!("{name} moved to {there}");
    failure.detail = format!(
        "{name} is open in {there}, so this pane is closed. Go there to keep working in it, or move it here: the other pane stops and this one resumes the same conversation."
    );
    crate::server::agent_takeover::surface_classified_failure(state, &d.block_id, &failure);
    set_desired(mstore, &d.block_id, DESIRED_STOPPED);
    crate::backend::reactive::get_global_handler().audit_action(&ActionRecord {
        actor: Actor::Srv,
        command: AUDIT_COMMAND.to_string(),
        target: Target { agent: Some(name), block_id: Some(d.block_id.clone()), subject: Some(d.keeper.clone()) },
        request_id: String::new(),
        error: None,
    });
}

/// Close every duplicate pane there is. Returns how many it closed now.
pub(crate) fn reconcile_duplicate_panes(state: &AppState) -> usize {
    let duplicates = match find_duplicates(&state.mstore) {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!("agent_panes.duplicate: could not look for duplicate agent panes: {e}");
            return 0;
        }
    };
    let mut closed = 0;
    for d in &duplicates {
        let Ok(Some(block)) = state.mstore.get::<Block>(&d.block_id) else { continue };
        if already_closed_for(&block, &d.keeper) {
            continue;
        }
        close_duplicate(state, d);
        closed += 1;
    }
    if closed > 0 {
        tracing::warn!("agent_panes.duplicate: closed {closed} duplicate agent pane(s)");
    }
    closed
}

/// At startup, before any window shows the panes.
pub(crate) fn install(state: &AppState) {
    let state = state.clone();
    tokio::task::spawn_blocking(move || {
        reconcile_duplicate_panes(&state);
    });
}
