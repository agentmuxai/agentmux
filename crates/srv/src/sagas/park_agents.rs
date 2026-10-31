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
    obj::meta_get_string(&block.meta, "view", "") == "agent"
        && !obj::is_history_reader(&block.meta)
        && agent_of_block(state, block_id, &block).is_some()
}

/// Of the blocks being closed, the ones that are extra blocks of their agent and so are closed, not
/// parked. `batch` is each closing parkable block with its agent; `all` every agent block in a tab or
/// parked with its agent; `runs` says whether a block has the agent's live process.
///
/// Layouts from before one block per agent can hold two blocks for an agent, and parking both would
/// leave it two parked blocks. One block per agent survives the close: the one that runs the agent's
/// process, else the first of them. A block of the agent outside this close stays, so when one runs it
/// is the survivor and every closing block of that agent is extra, and when none runs the closing block
/// that runs is parked beside it. Decided once for the whole close, so a tab or workspace close that
/// covers both blocks keeps one of them.
pub(super) fn extras_to_close(batch: &[(String, String)], all: &[(String, String)], runs: &dyn Fn(&str) -> bool) -> std::collections::HashSet<String> {
    let mut extras = std::collections::HashSet::new();
    let mut decided = std::collections::HashSet::new();
    for (_, agent) in batch {
        if !decided.insert(agent.as_str()) {
            continue;
        }
        let closing: Vec<&str> = batch.iter().filter(|(_, a)| a == agent).map(|(b, _)| b.as_str()).collect();
        let outside: Vec<&str> = all.iter().filter(|(b, a)| a == agent && !closing.contains(&b.as_str())).map(|(b, _)| b.as_str()).collect();
        let keeper = if outside.is_empty() {
            closing.iter().copied().find(|b| runs(b)).or_else(|| closing.first().copied())
        } else if outside.iter().any(|b| runs(b)) {
            None
        } else {
            closing.iter().copied().find(|b| runs(b))
        };
        extras.extend(closing.into_iter().filter(|b| Some(*b) != keeper).map(str::to_string));
    }
    extras
}

/// The closing blocks among `block_ids` that [`extras_to_close`] drops.
fn extra_blocks(state: &AppState, block_ids: &[String]) -> std::collections::HashSet<String> {
    let Ok(all_blocks) = crate::server::app_api::agent_block::blocks_in_tabs_or_parked(&state.mstore) else {
        return std::collections::HashSet::new();
    };
    let agent_of = |block: &Block| agent_of_block(state, &block.oid, block).map(|i| i.id);
    let all: Vec<(String, String)> = all_blocks.iter().filter_map(|(b, _)| Some((b.oid.clone(), agent_of(b)?))).collect();
    let batch: Vec<(String, String)> = block_ids.iter().filter(|id| is_parkable(state, id)).filter_map(|id| all.iter().find(|(b, _)| b == id).cloned()).collect();
    let runs = |id: &str| {
        crate::backend::blockcontroller::get_controller(id)
            .is_some_and(|c| c.get_runtime_status().shellprocstatus == crate::backend::blockcontroller::STATUS_RUNNING)
    };
    extras_to_close(&batch, &all, &runs)
}

/// An agent that is running when its pane closes is wanted running, so a restart brings it back
/// (`restart_resume`); a block from before desired states has no record of it.
fn record_running_as_desired(state: &AppState, block_id: &str) {
    use crate::server::app_api::agent_attach::{desired_of, set_desired, DESIRED_RUNNING};
    let Ok(Some(block)) = state.mstore.get::<Block>(block_id) else {
        return;
    };
    let running = crate::backend::blockcontroller::get_controller(block_id)
        .is_some_and(|c| c.get_runtime_status().shellprocstatus == crate::backend::blockcontroller::STATUS_RUNNING);
    if running && desired_of(&block).is_none() {
        set_desired(&state.mstore, block_id, DESIRED_RUNNING);
    }
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
    let extras = if force { Default::default() } else { extra_blocks(ctx.state, block_ids) };
    let mut parked = Vec::new();
    for block_id in block_ids {
        if !is_parkable(ctx.state, block_id) {
            continue;
        }
        // An explicit detach parks whatever it is asked to; a close drops the extra block of an agent.
        if extras.contains(block_id) {
            tracing::info!(block_id = %block_id, "closing a second block of the same agent: it is closed, not parked");
            continue;
        }
        record_running_as_desired(ctx.state, block_id);
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

/// A window that closes and is kept (a closed window of the workspace): park the agent blocks of its tabs
/// (`block_ids`) so the close stops only what isn't an agent. A parked block keeps its layout leaf: parking
/// leaves the tab's tree alone, and the tree prunes only blocks that are gone, so the window reopens with
/// its agents where they were (`reattach_window_agents`). Returns the ids parked.
pub(crate) async fn park_window_agents(state: &AppState, workspace_id: &str, block_ids: &[String]) -> Vec<String> {
    if !enabled(state) {
        return Vec::new();
    }
    let saga_id = super::alloc_saga_id(state);
    let input = serde_json::json!({ "workspace_id": workspace_id, "block_count": block_ids.len() });
    if super::emit_saga_started(state, saga_id, "close_window_park", input).await.is_err() {
        return Vec::new();
    }
    let ctx = SagaCtx::new(state, saga_id);
    let parked = park_agent_blocks(&ctx, block_ids, false).await;
    super::emit_terminal(state, saga_id, super::SagaTerminal::Completed).await;
    parked
}

/// A closed window reopening: the agent blocks it parked at its close (`parked`, from its closed-window
/// record) go back into the tab they were parked from, where their layout leaf still is. A block that is
/// not parked any more (its agent was opened elsewhere, or deleted) is not brought back, and its leaf is
/// removed from the window's layout so the window doesn't show the agent twice. A block already back in
/// one of its tabs (the reopen is tried again until the window shows) is left as it is. Returns how many
/// came back.
pub(crate) async fn reattach_window_agents(state: &AppState, workspace_id: &str, parked: &[String]) -> usize {
    use crate::persist_subscriber::parked::{PARKED_FROM_TAB_META_KEY, PARKED_META_KEY};
    if parked.is_empty() {
        return 0;
    }
    let Ok(Some(ws)) = state.mstore.get::<obj::Workspace>(workspace_id) else {
        return 0;
    };
    let tabs: Vec<String> = ws.tabids.iter().chain(ws.pinnedtabids.iter()).cloned().collect();
    let saga_id = super::alloc_saga_id(state);
    let input = serde_json::json!({ "workspace_id": workspace_id, "parked": parked });
    if super::emit_saga_started(state, saga_id, "reopen_window_attach", input).await.is_err() {
        return 0;
    }
    let ctx = SagaCtx::new(state, saga_id);
    let mut back = 0;
    for block_id in parked {
        let home = match state.mstore.get::<Block>(block_id) {
            Ok(Some(b)) if obj::meta_get_bool(&b.meta, PARKED_META_KEY, false) => {
                Some(obj::meta_get_string(&b.meta, PARKED_FROM_TAB_META_KEY, "")).filter(|t| tabs.contains(t))
            }
            _ => None,
        };
        if let Some(tab_id) = home {
            match ctx.dispatch(Command::AttachBlock { block_id: block_id.clone(), dst_tab_id: tab_id.clone(), dst_index: u32::MAX }).await {
                Ok(_) => {
                    back += 1;
                    continue;
                }
                Err(reason) => tracing::warn!(block_id = %block_id, tab_id = %tab_id, "reopen window: attaching a parked agent failed: {reason}"),
            }
        }
        // Already back in one of the window's tabs (a reopen that is tried again): it keeps its leaf.
        let in_window = state.srv_state.lock().await.blocks.get(block_id).is_some_and(|b| tabs.contains(&b.tab_id));
        if in_window {
            continue;
        }
        // Not coming back: its leaf must not stay in the window's tabs.
        for tab_id in &tabs {
            let has_leaf = state
                .srv_state
                .lock()
                .await
                .tabs
                .get(tab_id)
                .and_then(|t| t.rootnode.as_ref())
                .is_some_and(|root| crate::backend::layout::find_leaf_containing_block(root, block_id).is_some());
            if has_leaf {
                let cmd = Command::LayoutDeleteNodeByBlock { tab_id: tab_id.clone(), block_id: block_id.clone(), correlation_id: String::new() };
                if let Err(reason) = ctx.dispatch(cmd).await {
                    tracing::warn!(block_id = %block_id, tab_id = %tab_id, "reopen window: removing a stale agent leaf failed: {reason}");
                }
            }
        }
    }
    super::emit_terminal(state, saga_id, super::SagaTerminal::Completed).await;
    back
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
