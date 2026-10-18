// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `copyforkhistory`: a fork's pane starts with its parent's history
//! (`agent_session::fork_history`). Forks only; see "Fork, spawn and worker"
//! in `mod.rs`.

use std::sync::Arc;

use crate::backend::agent_session::fork_history::{copy_parent_history, ForkHistory};
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{CommandCopyForkHistoryData, CopyForkHistoryResult, COMMAND_COPY_FORK_HISTORY};
use crate::backend::storage::store::Store;

use super::super::AppState;

pub(super) fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let mstore = state.mstore.clone();
    engine.register_typed(COMMAND_COPY_FORK_HISTORY, move |cmd: CommandCopyForkHistoryData, _ctx| {
        let mstore = mstore.clone();
        async move { tokio::task::spawn_blocking(move || copy_fork_history(&mstore, &cmd)).await.map_err(|e| format!("copyforkhistory: {e}"))? }
    });
}

/// Refuses anything but a real fork of `source_id` (its `parent_id`, and a
/// branch label, which only `forkagentdefinition` sets) on a provider that
/// names its sessions.
pub(super) fn copy_fork_history(mstore: &Store, cmd: &CommandCopyForkHistoryData) -> Result<CopyForkHistoryResult, String> {
    let fork = mstore
        .agent_def_get(&cmd.fork_id)
        .map_err(|e| format!("copyforkhistory: {e}"))?
        .ok_or_else(|| format!("copyforkhistory: no agent {}", cmd.fork_id))?;
    if fork.parent_id != cmd.source_id || fork.branch_label.is_empty() {
        return Err(format!("copyforkhistory: {} is not a fork of {}", cmd.fork_id, cmd.source_id));
    }
    let field = crate::backend::providers::get_provider(crate::backend::providers::resolve_provider_alias(&cmd.provider))
        .map(|p| p.session_id_field)
        .unwrap_or("");
    if field.is_empty() {
        return Err(format!("copyforkhistory: {} has no sessions to continue", cmd.provider));
    }
    let gfs = crate::backend::agent_session::global_transcript_store()
        .ok_or_else(|| "copyforkhistory: no transcript store".to_string())?;
    let result = |copied: bool, reason: &str, bytes: u64| CopyForkHistoryResult { copied, reason: reason.to_string(), bytes };
    Ok(match copy_parent_history(gfs, &cmd.source_id, &cmd.fork_id, &cmd.session_id, field)? {
        ForkHistory::Copied { bytes } => result(true, "", bytes),
        ForkHistory::NoParentHistory => result(false, "no_parent_history", 0),
        ForkHistory::NotTheLastSession => result(false, "not_the_last_session", 0),
        ForkHistory::ForkHasHistory => result(false, "fork_has_history", 0),
    })
}
