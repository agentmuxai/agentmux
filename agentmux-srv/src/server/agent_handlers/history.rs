// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0


use std::sync::Arc;


use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{
    COMMAND_APPEND_AGENT_HISTORY, COMMAND_LIST_AGENT_HISTORY, COMMAND_SEARCH_AGENT_HISTORY,
    CommandAppendAgentHistoryData, CommandListAgentHistoryData, CommandSearchAgentHistoryData,
};

use super::super::AppState;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    // appendagenthistory → append a history entry, broadcast agenthistory:changed
    let wstore_afh = state.wstore.clone();
    let broker_afh = state.broker.clone();
    engine.register_handler(
        COMMAND_APPEND_AGENT_HISTORY,
        Box::new(move |data, _ctx| {
            let wstore = wstore_afh.clone();
            let broker = broker_afh.clone();
            Box::pin(async move {
                let cmd: CommandAppendAgentHistoryData = serde_json::from_value(data)
                    .map_err(|e| format!("appendagenthistory: {e}"))?;
                let entry = wstore.agent_history_append(&cmd.agent_id, &cmd.entry)
                    .map_err(|e| format!("appendagenthistory: {e}"))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "agenthistory:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(serde_json::to_value(&entry).unwrap_or_default()))
            })
        }),
    );

    // listagenthistory → return history entries with pagination
    let wstore_lfh = state.wstore.clone();
    engine.register_handler(
        COMMAND_LIST_AGENT_HISTORY,
        Box::new(move |data, _ctx| {
            let wstore = wstore_lfh.clone();
            Box::pin(async move {
                let cmd: CommandListAgentHistoryData = serde_json::from_value(data)
                    .map_err(|e| format!("listagenthistory: {e}"))?;
                let entries = wstore.agent_history_list(
                    &cmd.agent_id,
                    cmd.session_date.as_deref(),
                    cmd.limit,
                    cmd.offset,
                ).map_err(|e| format!("listagenthistory: {e}"))?;
                Ok(Some(serde_json::to_value(&entries).unwrap_or_default()))
            })
        }),
    );

    // searchagenthistory → search history entries by query
    let wstore_sfh = state.wstore.clone();
    engine.register_handler(
        COMMAND_SEARCH_AGENT_HISTORY,
        Box::new(move |data, _ctx| {
            let wstore = wstore_sfh.clone();
            Box::pin(async move {
                let cmd: CommandSearchAgentHistoryData = serde_json::from_value(data)
                    .map_err(|e| format!("searchagenthistory: {e}"))?;
                let entries = wstore.agent_history_search(&cmd.agent_id, &cmd.query, cmd.limit)
                    .map_err(|e| format!("searchagenthistory: {e}"))?;
                Ok(Some(serde_json::to_value(&entries).unwrap_or_default()))
            })
        }),
    );

}
