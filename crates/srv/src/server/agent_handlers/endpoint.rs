// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `undoagentendpoint`: the Undo on the notice for a changed model server (`backend/endpoint_notice.rs`).
//! It puts the agent's environment back as it was, while the notice's window is open and the environment is
//! unchanged since (`app_api::agent_env::undo_endpoint`).

use std::sync::Arc;

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{CommandUndoAgentEndpointData, UndoAgentEndpointResult, COMMAND_UNDO_AGENT_ENDPOINT};

use super::super::AppState;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let state = state.clone();
    engine.register_typed(COMMAND_UNDO_AGENT_ENDPOINT, move |cmd: CommandUndoAgentEndpointData, _ctx| {
        let state = state.clone();
        async move {
            let result = tokio::task::spawn_blocking(move || crate::server::app_api::agent_env::undo_endpoint(&state, &cmd.notice_id, "you"))
                .await
                .map_err(|e| format!("undoagentendpoint: {e}"))?;
            Ok(match result {
                Ok(()) => UndoAgentEndpointResult { restored: true, reason: None },
                Err(why) => UndoAgentEndpointResult { restored: false, reason: Some(why) },
            })
        }
    });
}
