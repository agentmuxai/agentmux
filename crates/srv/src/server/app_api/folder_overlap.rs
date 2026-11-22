// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A host agent that shares a folder with a container agent: which container agents, and the person going ahead
//! with one (`backend::folder_overlap`). Anyone may ask which; only the AgentMux window goes ahead. A caller that
//! is not the window, an agent's connection or one that has not proven it is the window, is refused with what to
//! do instead.

use super::*;
use crate::backend::folder_overlap as overlap;

/// What a caller is told when it is not the window.
pub(crate) const ONLY_THE_PERSON_GOES_AHEAD: &str = "only the person goes ahead with a host agent sharing a folder with a container agent, from the AgentMux window: give the agents separate workspaces, or bind the shared folder read-only into the container agent (a volume ending in :ro)";

fn check(state: &AppState, cmd: CommandFolderOverlapCheckData) -> Result<FolderOverlapCheckResult, String> {
    let def = state
        .mstore
        .agent_def_get(&cmd.agent_id)
        .map_err(|e| format!("folderoverlapcheck: {e}"))?
        .ok_or_else(|| format!("folderoverlapcheck: agent {} not found", cmd.agent_id))?;
    let folder = cmd.folder.as_deref().map(str::trim).filter(|dir| !dir.is_empty()).map(crate::backend::base::expand_home_dir_safe);
    let overlaps = overlap::unchosen(overlap::of(&state.mstore, &def, folder.as_deref()));
    Ok(FolderOverlapCheckResult { overlaps })
}

fn accept(state: &AppState, cmd: CommandFolderOverlapAcceptData) -> Result<FolderOverlapAcceptResult, String> {
    if !overlap::is_a_pair(&state.mstore, &cmd.host_id, &cmd.container_id) {
        return Err("folderoverlapaccept: needs a host agent and a container agent".to_string());
    }
    overlap::accept(&cmd.host_id, &cmd.container_id)?;
    Ok(FolderOverlapAcceptResult { chosen: true })
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
}
