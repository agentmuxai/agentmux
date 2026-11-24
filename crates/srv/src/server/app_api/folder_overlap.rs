// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A host agent that shares a folder with a container agent: which container agents, and the person going ahead
//! with one (`backend::folder_overlap`). Anyone may ask which; only the AgentMux window goes ahead or says no to an
//! agent's ask for the go-ahead (`folder_overlap_ask`). A caller that is not the window, an agent's connection or one
//! that has not proven it is the window, is refused with what to do instead. Each answer is recorded as the person's.

use super::*;
use crate::backend::folder_overlap as overlap;
use crate::backend::folder_overlap_asks::{asks, FolderAsk};

/// What a caller is told when it is not the window.
pub(crate) const ONLY_THE_PERSON_GOES_AHEAD: &str = "only the person goes ahead with a host agent sharing a folder with a container agent, from the AgentMux window: give the agents separate workspaces, or bind the shared folder read-only into the container agent (a volume ending in :ro)";

fn check(state: &AppState, cmd: CommandFolderOverlapCheckData) -> Result<FolderOverlapCheckResult, String> {
    let def = state
        .mstore
        .agent_def_get(&cmd.agent_id)
        .map_err(|e| format!("folderoverlapcheck: {e}"))?
        .ok_or_else(|| format!("folderoverlapcheck: agent {} not found", cmd.agent_id))?;
    let folder = cmd.folder.as_deref().map(str::trim).filter(|dir| !dir.is_empty()).map(crate::backend::base::expand_home_dir_safe);
    // No folder named: the one `agent.open` runs the agent in, so the window asks about what srv would refuse.
    let folder = folder.unwrap_or_else(|| overlap::open_folder(&def));
    let overlaps = overlap::unchosen(overlap::of(&state.mstore, &def, Some(&folder)));
    Ok(FolderOverlapCheckResult { overlaps })
}

/// Tells every window the asks now waiting.
pub(crate) fn publish_asks(state: &AppState) {
    let event = crate::backend::mps_events::FolderOverlapAsksEvent { asks: asks().list(agentmux_common::time::now_ms_u64()) };
    state.broker.publish_event::<crate::backend::mps_events::FolderOverlapAsks>(&event, vec![], 0);
}

/// Restart resume left host agent `agent_id` (its parked block `block_id`) stopped for want of the person's go-ahead: one
/// ask per pair it shares a folder with, with no asking agent, which the window shows as a card for the agent's block.
pub(crate) fn ask_for_resumed(state: &AppState, agent_id: &str, block_id: &str) {
    let Ok(Some(host)) = state.mstore.agent_def_get(agent_id) else { return };
    let now = agentmux_common::time::now_ms_u64();
    for o in overlap::unchosen(overlap::of(&state.mstore, &host, Some(&overlap::open_folder(&host)))) {
        let ask = FolderAsk {
            host_id: o.host_id,
            host_name: o.host_name,
            container_id: o.container_id,
            container_name: o.container_name,
            agent: String::new(),
            block_id: block_id.to_string(),
            requested_ms: now,
        };
        asks().ask(ask, now);
    }
    publish_asks(state);
}

/// An agent is deleted: what it asked, and what was asked about it, goes with it.
pub(crate) fn forget_asks_of(state: &AppState, id: &str) {
    asks().forget_agent(id);
    publish_asks(state);
}

/// The name of agent `id`, or its id when it has none on record.
fn name_of(state: &AppState, id: &str) -> String {
    state.mstore.agent_def_get(id).ok().flatten().map(|def| def.name).unwrap_or_else(|| id.to_string())
}

/// Records what the person declined, or what an agent asked, in the audit log: `by` is the asking agent's slug, and
/// `None` for the person's own window. The go-ahead is recorded by `backend::folder_overlap::accept_as`, which writes
/// its row before the choice it records.
pub(crate) fn audit(state: &AppState, by: Option<&str>, action: &str, host_id: &str, container_id: &str, block_id: &str) {
    let (host, container) = (name_of(state, host_id), name_of(state, container_id));
    let note = overlap::row_note(action, (host_id, &host), (container_id, &container));
    tracing::info!(action, "{note}");
    let request_id = uuid::Uuid::new_v4().to_string();
    state.reactive_handler.log_fleet_action_audit(by, &host, block_id, action, true, None, &request_id, Some(&note));
}

fn accept(state: &AppState, cmd: CommandFolderOverlapAcceptData) -> Result<FolderOverlapAcceptResult, String> {
    if !overlap::is_a_pair(&state.mstore, &cmd.host_id, &cmd.container_id) {
        return Err("folderoverlapaccept: needs a host agent and a container agent".to_string());
    }
    // The go-ahead is the window's: its audit row is written first, then the choice it records.
    overlap::accept_as((&cmd.host_id, &name_of(state, &cmd.host_id)), (&cmd.container_id, &name_of(state, &cmd.container_id)))?;
    // An agent's ask for this pair is answered, whichever pane or window the person went ahead in.
    let now = agentmux_common::time::now_ms_u64();
    let left_stopped_in = asks().list(now).into_iter().find(|a| a.agent.is_empty() && a.host_id == cmd.host_id && a.container_id == cmd.container_id).map(|a| a.block_id);
    asks().answer(&cmd.host_id, &cmd.container_id, true, now);
    publish_asks(state);
    if let Some(block_id) = left_stopped_in {
        resume_when_nothing_waits(state, &cmd.host_id, block_id);
    }
    Ok(FolderOverlapAcceptResult { chosen: true })
}

/// Restart resume left the host agent stopped for the pair just gone ahead with: once no pair holds it, it comes back.
fn resume_when_nothing_waits(state: &AppState, host_id: &str, block_id: String) {
    let waits = state.mstore.agent_def_get(host_id).ok().flatten().and_then(|def| overlap::waiting_reason(&state.mstore, &def)).is_some();
    if waits {
        return;
    }
    let (state, host_id) = (state.clone(), host_id.to_string());
    tokio::spawn(async move { crate::server::restart_resume::resume_after_go_ahead(&state, &host_id, &block_id).await });
}

fn decline(state: &AppState, cmd: CommandFolderOverlapDeclineData) -> Result<FolderOverlapDeclineResult, String> {
    if !overlap::is_a_pair(&state.mstore, &cmd.host_id, &cmd.container_id) {
        return Err("folderoverlapdecline: needs a host agent and a container agent".to_string());
    }
    // The pair stays as it was: no choice is kept, so every refusal for it stands.
    let was_asked = asks().answer(&cmd.host_id, &cmd.container_id, false, agentmux_common::time::now_ms_u64());
    publish_asks(state);
    audit(state, None, "folder-overlap.decline", &cmd.host_id, &cmd.container_id, "");
    Ok(FolderOverlapDeclineResult { declined: was_asked })
}

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let check_state = state.clone();
    engine.register_typed(COMMAND_FOLDER_OVERLAP_CHECK, move |cmd: CommandFolderOverlapCheckData, _ctx| {
        let state = check_state.clone();
        async move { check(&state, cmd) }
    });
    let accept_state = state.clone();
    engine.register_typed(COMMAND_FOLDER_OVERLAP_ACCEPT, move |cmd: CommandFolderOverlapAcceptData, ctx| {
        let state = accept_state.clone();
        async move {
            ctx.operator_only(ONLY_THE_PERSON_GOES_AHEAD)?;
            accept(&state, cmd)
        }
    });
    let decline_state = state.clone();
    engine.register_typed(COMMAND_FOLDER_OVERLAP_DECLINE, move |cmd: CommandFolderOverlapDeclineData, ctx| {
        let state = decline_state.clone();
        async move {
            ctx.operator_only(ONLY_THE_PERSON_GOES_AHEAD)?;
            decline(&state, cmd)
        }
    });
    engine.register_typed(COMMAND_FOLDER_OVERLAP_ASKS, |_req: serde_json::Value, _ctx| async move {
        Ok(FolderOverlapAsksResult { asks: asks().list(agentmux_common::time::now_ms_u64()) })
    });
}
