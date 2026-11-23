// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A host agent that shares a folder with a container agent: which container agents, and the person going ahead
//! with one (`backend::folder_overlap`). Anyone may ask which; only the AgentMux window goes ahead or says no to an
//! agent's ask for the go-ahead (`folder_overlap_ask`). A caller that is not the window, an agent's connection or one
//! that has not proven it is the window, is refused with what to do instead. Each answer is recorded as the person's.

use super::*;
use crate::backend::folder_overlap as overlap;
use crate::backend::folder_overlap_asks::asks;

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

/// An agent is deleted: what it asked, and what was asked about it, goes with it.
pub(crate) fn forget_asks_of(state: &AppState, id: &str) {
    asks().forget_agent(id);
    publish_asks(state);
}

/// Records what the person answered, or what an agent asked, in the audit log: `by` is the asking agent's slug, and
/// `None` for the person's own window.
pub(crate) fn audit(state: &AppState, by: Option<&str>, action: &str, host_id: &str, container_id: &str, block_id: &str) {
    let name_of = |id: &str| state.mstore.agent_def_get(id).ok().flatten().map(|def| def.name).unwrap_or_else(|| id.to_string());
    let (host, container) = (name_of(host_id), name_of(container_id));
    let note = format!("{action}: host agent {host} ({host_id}) and container agent {container} ({container_id})");
    tracing::info!(action, "{note}");
    let request_id = uuid::Uuid::new_v4().to_string();
    state.reactive_handler.log_fleet_action_audit(by, &host, block_id, action, true, None, &request_id, Some(&note));
}

fn accept(state: &AppState, cmd: CommandFolderOverlapAcceptData) -> Result<FolderOverlapAcceptResult, String> {
    if !overlap::is_a_pair(&state.mstore, &cmd.host_id, &cmd.container_id) {
        return Err("folderoverlapaccept: needs a host agent and a container agent".to_string());
    }
    overlap::accept(&cmd.host_id, &cmd.container_id)?;
    // An agent's ask for this pair is answered, whichever pane or window the person went ahead in.
    asks().answer(&cmd.host_id, &cmd.container_id, true, agentmux_common::time::now_ms_u64());
    publish_asks(state);
    audit(state, None, "folder-overlap.go-ahead", &cmd.host_id, &cmd.container_id, "");
    Ok(FolderOverlapAcceptResult { chosen: true })
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
