// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Agents that outlive their pane (docs/specs/SPEC_ALWAYS_AVAILABLE_AGENTS_2026_10_27.md §8, §9 P1):
// with `agents:always_available` on, closing a pane, a tab or a workspace parks each agent's block
// (`Command::ParkBlock`) before the close tears anything down, and the close then skips it: its
// process keeps running and its block, with the transcript and session, stays for the next open.
// Off, nothing here does anything and every close deletes the block as it always has.

use agentmux_common::ipc::Command;

use super::SagaCtx;
use crate::backend::obj::{self, Block};
use crate::server::AppState;

/// Whether closing parks an agent's block (the `agents:always_available` setting).
pub(crate) fn enabled(state: &AppState) -> bool {
    crate::backend::always_available::enabled(&state.config_watcher.get_full_config().settings.extra)
}

/// The agent a block belongs to: the instance bound to the block (a block launched from a template
/// keeps the template's id in `agentId` and binds its agent that way), else the agent `agentId` names.
pub(crate) fn agent_of_block(state: &AppState, block_id: &str, block: &Block) -> Option<crate::backend::storage::agents::AgentInstance> {
    if let Ok(Some(bound)) = state.mstore.instance_get_by_block_id(block_id) {
        return Some(bound);
    }
    let named = obj::meta_get_string(&block.meta, "agentId", "");
    if named.is_empty() {
        return None;
    }
    state.mstore.instance_get(&named).ok().flatten()
}

/// An agent's pane block of an agent that still exists. A terminal, a browser or a drawer is not
/// one, and neither is the block of a deleted agent: nothing would ever open it again.
pub(crate) fn is_parkable(state: &AppState, block_id: &str) -> bool {
    let Ok(Some(block)) = state.mstore.get::<Block>(block_id) else {
        return false;
    };
    obj::meta_get_string(&block.meta, "view", "") == "agent" && agent_of_block(state, block_id, &block).is_some()
}

/// Whether a block that is closing now is kept: parkable, with the setting on.
pub(crate) fn keeps_block(state: &AppState, block_id: &str) -> bool {
    enabled(state) && is_parkable(state, block_id)
}

/// Park the agent blocks among `block_ids` (each in the tab the reducer has it in) and return the ids
/// parked. A park the reducer refuses leaves that block to the close, which deletes it as before.
/// `force` parks with the setting off too: an explicit detach.
pub(crate) async fn park_agent_blocks(ctx: &SagaCtx<'_>, block_ids: &[String], force: bool) -> Vec<String> {
    if !force && !enabled(ctx.state) {
        return Vec::new();
    }
    let mut parked = Vec::new();
    for block_id in block_ids {
        if !is_parkable(ctx.state, block_id) {
            continue;
        }
        let tab_id = match ctx.state.srv_state.lock().await.blocks.get(block_id) {
            Some(b) if !b.tab_id.is_empty() => b.tab_id.clone(),
            _ => continue,
        };
        match ctx.dispatch(Command::ParkBlock { tab_id: tab_id.clone(), block_id: block_id.clone() }).await {
            Ok(_) => parked.push(block_id.clone()),
            Err(reason) => tracing::warn!(
                tab_id = %tab_id,
                block_id = %block_id,
                "[saga] park of an agent's block failed — it is closed instead: {}",
                reason
            ),
        }
    }
    parked
}

/// Delete every parked block of the agent `agent_id`, stopping its process first: deleting the agent
/// is the one thing that deletes its block. A block is the agent's through `agent_of_block`, so
/// deleting a template leaves the blocks of the agents launched from it. Returns how many went. A block that is shown in a tab is
/// not touched, as before.
pub(crate) async fn delete_parked_blocks_of(state: &AppState, agent_id: &str) -> usize {
    let parked: Vec<String> = match state.mstore.get_all::<Block>() {
        Ok(blocks) => blocks
            .into_iter()
            .filter(|b| {
                obj::meta_get_bool(&b.meta, crate::persist_subscriber::parked::PARKED_META_KEY, false)
                    && agent_of_block(state, &b.oid, b).is_some_and(|i| i.id == agent_id)
            })
            .map(|b| b.oid)
            .collect(),
        Err(e) => {
            tracing::warn!(agent_id, "delete agent: could not list its parked blocks: {e}");
            return 0;
        }
    };
    let mut deleted = 0;
    for block_id in parked {
        match super::delete_block::run(state, String::new(), block_id.clone()).await {
            Ok(_) => deleted += 1,
            Err(e) => tracing::warn!(agent_id, block_id = %block_id, "delete agent: its parked block stays: {e}"),
        }
    }
    deleted
}

/// `block_ids` without the parked ones: what the close still stops and deletes.
pub(crate) fn not_parked(block_ids: &[String], parked: &[String]) -> Vec<String> {
    block_ids.iter().filter(|b| !parked.contains(b)).cloned().collect()
}
