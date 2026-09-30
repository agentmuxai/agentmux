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
    let mstore_afh = state.mstore.clone();
    let broker_afh = state.broker.clone();
    engine.register_typed(
        COMMAND_APPEND_AGENT_HISTORY,
        move |cmd: CommandAppendAgentHistoryData, _ctx| {
            let mstore = mstore_afh.clone();
            let broker = broker_afh.clone();
            async move {
                let entry = mstore.agent_history_append(&cmd.agent_id, &cmd.entry)
                    .map_err(|e| format!("appendagenthistory: {e}"))?;
                broker.publish(crate::backend::mps::MuxEvent {
                    event: "agenthistory:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(entry)
            }
        },
    );

    // listagenthistory → return history entries with pagination
    let mstore_lfh = state.mstore.clone();
    engine.register_typed(
        COMMAND_LIST_AGENT_HISTORY,
        move |cmd: CommandListAgentHistoryData, _ctx| {
            let mstore = mstore_lfh.clone();
            async move {
                let entries = mstore.agent_history_list(
                    &cmd.agent_id,
                    cmd.session_date.as_deref(),
                    cmd.limit,
                    cmd.offset,
                ).map_err(|e| format!("listagenthistory: {e}"))?;
                Ok(entries)
            }
        },
    );

    // searchagenthistory → search history entries by query
    let mstore_sfh = state.mstore.clone();
    engine.register_typed(
        COMMAND_SEARCH_AGENT_HISTORY,
        move |cmd: CommandSearchAgentHistoryData, _ctx| {
            let mstore = mstore_sfh.clone();
            async move {
                let entries = mstore.agent_history_search(&cmd.agent_id, &cmd.query, cmd.limit)
                    .map_err(|e| format!("searchagenthistory: {e}"))?;
                Ok(entries)
            }
        },
    );

}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::rpc_types::{CommandListAgentHistoryData, CommandSearchAgentHistoryData};

    /// All three history commands record their request and response types.
    #[tokio::test]
    async fn register_typed_records_every_history_command() {
        let state = crate::server::tests::test_state();
        let (engine, _rx) = WshRpcEngine::new();
        register(&engine, &state);
        let schema = engine.schema_json();
        let rows = schema.as_array().unwrap();
        let find = |cmd: &str| {
            rows.iter()
                .find(|r| r["command"] == cmd)
                .unwrap_or_else(|| panic!("{cmd} missing from the schema"))
        };

        let append = find(COMMAND_APPEND_AGENT_HISTORY);
        assert_eq!(append["requestName"], "CommandAppendAgentHistoryData");
        assert_eq!(append["responseName"], "AgentHistory");

        for (cmd, req) in [
            (COMMAND_LIST_AGENT_HISTORY, "CommandListAgentHistoryData"),
            (COMMAND_SEARCH_AGENT_HISTORY, "CommandSearchAgentHistoryData"),
        ] {
            let row = find(cmd);
            assert_eq!(row["requestName"], req, "{cmd} request");
            let resp = row["responseName"].as_str().unwrap();
            assert!(
                resp.starts_with("alloc::vec::Vec<") && resp.contains("AgentHistory"),
                "{cmd} should answer Vec<AgentHistory>, got {resp:?}",
            );
        }
    }

    /// The pagination fields are `serde(default)` on `i64`, which ts-rs cannot
    /// mark optional -- the stubs derive `ListAgentHistoryInput` /
    /// `SearchAgentHistoryInput` to restore it. Pin the server half: drop a
    /// default and those derived TS types become a lie.
    #[test]
    fn the_pagination_fields_may_be_omitted() {
        let list: CommandListAgentHistoryData =
            serde_json::from_value(serde_json::json!({ "agent_id": "a1" }))
                .expect("limit and offset are both defaulted");
        assert_eq!(list.limit, 50, "listagenthistory defaults limit to 50");
        assert_eq!(list.offset, 0);
        assert!(list.session_date.is_none());

        let search: CommandSearchAgentHistoryData =
            serde_json::from_value(serde_json::json!({ "agent_id": "a1", "query": "x" }))
                .expect("limit is defaulted");
        assert_eq!(search.limit, 50, "searchagenthistory shares the same default");
    }
}
