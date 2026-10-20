// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Warden's History section: `history.agents` lists every agent whose history
//! the global transcript store holds (`backend::history_registry`), and
//! `history.agent_sessions` one agent's sessions from its record. The Agent
//! History view's `history.locate` finds the line to load up to for a time or
//! a session's end. The window's, not agent APIs: a connection registered as
//! an agent is refused.

use super::tower_pane::not_an_agent_api;
use super::*;
use crate::backend::agent_session::{OUTPUT_FILE, TSIDX_FILE};
use crate::backend::blockcontroller::shell::{extend_output_idx, lines_before_byte, stamps};
use crate::backend::history_registry::{self, AgentInfo};
use crate::backend::storage::filestore::FileStore;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let mstore = state.mstore.clone();
    engine.register_typed(COMMAND_HISTORY_AGENTS, move |_req: HistoryAgentsReq, ctx| {
        let mstore = mstore.clone();
        async move {
            not_an_agent_api(&ctx)?;
            let Some(store) = crate::backend::agent_session::global_transcript_store().cloned() else {
                return Err("history.agents: no transcript store".to_string());
            };
            tokio::task::spawn_blocking(move || {
                let defs: std::collections::HashMap<String, AgentInfo> = mstore
                    .agent_def_list()
                    .map_err(|e| format!("history.agents: {e}"))?
                    .into_iter()
                    // `parent_id` also names the template an agent was made from; only
                    // a fork has a branch label (`forkagentdefinition`).
                    .map(|d| {
                        let fork_of = if d.branch_label.is_empty() { String::new() } else { d.parent_id };
                        (d.id, AgentInfo { name: d.name, provider: d.provider, fork_of })
                    })
                    .collect();
                let agents = history_registry::agents(&store, |id| defs.get(id).cloned()).map_err(|e| format!("history.agents: {e}"))?;
                Ok(HistoryAgents { agents })
            })
            .await
            .map_err(|e| format!("history.agents: {e}"))?
        }
    });

    let history = state.history_service.clone();
    engine.register_typed(COMMAND_HISTORY_AGENT_SESSIONS, move |req: HistoryAgentSessionsReq, ctx| {
        let history = history.clone();
        async move {
            not_an_agent_api(&ctx)?;
            if !crate::backend::agent_session::is_valid_definition_id(&req.definition_id) {
                return Err(format!("history.agent_sessions: invalid agent id {:?}", req.definition_id));
            }
            // The first read of a long record scans it: off the async workers.
            tokio::task::spawn_blocking(move || history.agent_sessions(&req.definition_id))
                .await
                .map_err(|e| format!("history.agent_sessions: {e}"))
        }
    });

    let filestore = state.filestore.clone();
    let global_store = state.global_transcript_store.clone();
    let mstore = state.mstore.clone();
    engine.register_typed(COMMAND_HISTORY_LOCATE, move |req: HistoryLocateReq, ctx| {
        let (filestore, global_store, mstore) = (filestore.clone(), global_store.clone(), mstore.clone());
        async move {
            not_an_agent_api(&ctx)?;
            let source = super::blockfile::history_output_source(
                &filestore,
                &global_store,
                &mstore,
                &req.block_id,
                OUTPUT_FILE,
                req.archive_zone.as_deref(),
            )?;
            let (store, zone) = source.unwrap_or_else(|| (filestore.clone(), req.block_id.clone()));
            tokio::task::spawn_blocking(move || locate(&store, &zone, req.at_ms, req.at_byte))
                .await
                .map_err(|e| format!("history.locate: {e}"))?
        }
    });
}

/// The line a reader showing `zone`'s transcript up to `at_byte`, or up to the
/// last line received at or before `at_ms`, stops at. Neither: the end.
fn locate(fs: &FileStore, zone: &str, at_ms: Option<u64>, at_byte: Option<u64>) -> Result<HistoryLocation, String> {
    let size = fs.stat(zone, OUTPUT_FILE).map_err(|e| format!("history.locate: {e}"))?.map_or(0, |f| f.size.max(0) as u64);
    let byte = match (at_byte, at_ms) {
        (Some(byte), _) => byte.min(size),
        (None, Some(ms)) => {
            let ts_size = fs.stat(zone, TSIDX_FILE).map_err(|e| format!("history.locate: {e}"))?.map_or(0, |f| f.size);
            // History recorded before the receive-time sidecar existed has no
            // times: say so, rather than land at the end.
            if ts_size <= 0 && size > 0 {
                return Err("history.locate: this conversation has no recorded times; jump to a session instead".to_string());
            }
            let read = |off: i64, len: i64| fs.read_at(zone, TSIDX_FILE, off, len).map(|(_, data)| Some(data));
            stamps::byte_after_time(&read, ts_size, ms as i64)
                .map_err(|e| format!("history.locate: {e}"))?
                .ok_or_else(|| "history.locate: the timestamps can't be read yet".to_string())?
                .unwrap_or(size)
        }
        (None, None) => size,
    };
    // The index is a cache of `output`: bring it up to date first.
    extend_output_idx(fs, zone);
    let (line, total) = lines_before_byte(fs, zone, byte).ok_or_else(|| "history.locate: the line index isn't ready; try again".to_string())?;
    Ok(HistoryLocation { line, total })
}

#[cfg(test)]
#[path = "history_agents_tests.rs"]
mod tests;
