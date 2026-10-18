// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Warden's History section: `history.agents` lists every agent whose history
//! the global transcript store holds (`backend::history_registry`), and
//! `history.agent_sessions` one agent's sessions from its record. The
//! window's, not agent APIs: a connection registered as an agent is refused.

use super::tower_pane::not_an_agent_api;
use super::*;
use crate::backend::history_registry::{self, AgentInfo};

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
}
