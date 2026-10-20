// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `copyforkhistory`: a fork's pane starts with its parent's history
//! (`agent_session::fork_history`). Forks only; see "Fork, spawn and worker"
//! in `mod.rs`.

use std::sync::Arc;

use crate::backend::agent_session::fork_history::{copy_older_session_history, copy_parent_history, ForkHistory};
use crate::backend::history::HistoryService;
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{CommandCopyForkHistoryData, CopyForkHistoryResult, COMMAND_COPY_FORK_HISTORY};
use crate::backend::storage::store::Store;

use super::super::AppState;

pub(super) fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let mstore = state.mstore.clone();
    let history = state.history_service.clone();
    engine.register_typed(COMMAND_COPY_FORK_HISTORY, move |cmd: CommandCopyForkHistoryData, _ctx| {
        let mstore = mstore.clone();
        let history = history.clone();
        async move { tokio::task::spawn_blocking(move || copy_fork_history(&mstore, &history, &cmd)).await.map_err(|e| format!("copyforkhistory: {e}"))? }
    });
}

/// Refuses anything but a real fork of `source_id` (its `parent_id`, and a
/// branch label, which only `forkagentdefinition` sets) on a provider that
/// names its sessions.
pub(super) fn copy_fork_history(mstore: &Store, history: &HistoryService, cmd: &CommandCopyForkHistoryData) -> Result<CopyForkHistoryResult, String> {
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
        // Either way the session may be in the record: after a New conversation
        // the current zone is empty and the older sessions are in an archive.
        ForkHistory::NoParentHistory => older_session(gfs, history, cmd, field, "no_parent_history")?,
        ForkHistory::NotTheLastSession => older_session(gfs, history, cmd, field, "not_the_last_session")?,
        ForkHistory::ForkHasHistory => result(false, "fork_has_history", 0),
    })
}

/// A fork of an older session: the parent's history up to where that session
/// ends, found in the agent's record. A session the record doesn't know has no
/// history to copy, and the answer is `reason`, what the current zone said.
fn older_session(
    gfs: &crate::backend::storage::filestore::FileStore,
    history: &HistoryService,
    cmd: &CommandCopyForkHistoryData,
    field: &str,
    reason: &str,
) -> Result<CopyForkHistoryResult, String> {
    let not_last = || CopyForkHistoryResult { copied: false, reason: reason.to_string(), bytes: 0 };
    let sessions = history.agent_sessions(&cmd.source_id).sessions;
    let Some(session) = sessions.iter().find(|s| s.session_id == cmd.session_id && !s.zone.is_empty() && s.end_byte > 0) else {
        return Ok(not_last());
    };
    Ok(match copy_older_session_history(gfs, &cmd.source_id, &cmd.fork_id, &cmd.session_id, field, &session.zone, session.end_byte as i64)? {
        ForkHistory::Copied { bytes } => CopyForkHistoryResult { copied: true, reason: String::new(), bytes },
        ForkHistory::NoParentHistory => CopyForkHistoryResult { copied: false, reason: "no_parent_history".to_string(), bytes: 0 },
        ForkHistory::ForkHasHistory => CopyForkHistoryResult { copied: false, reason: "fork_has_history".to_string(), bytes: 0 },
        ForkHistory::NotTheLastSession => not_last(),
    })
}
