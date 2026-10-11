// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The MCP servers an agent launches with, put into its `.mcp.json`: the same
//! rows on both launch paths, `agent.open` (`agent_config::build_mcp_config_from_refs`)
//! and the picker's `writeagentconfig` ([`overlay_mcp_servers`]).
//! `SPEC_CONNECTORS_MCP_WHAT_AGENTS_RUN_2026_10_10.md` §2.2.

use serde_json::Value;

/// Put server rows into an `mcpServers` map, each under its name. A row
/// named `agentmux` is skipped: the built-in entry always wins.
pub(crate) fn insert_server_rows(mcp_servers: &mut serde_json::Map<String, Value>, servers: &[crate::backend::storage::McpServer]) {
    for server in servers {
        if server.name == "agentmux" {
            continue; // synthetic entry wins; user cannot override the key
        }
        match serde_json::from_str::<Value>(&server.config) {
            Ok(cfg) => {
                mcp_servers.insert(server.name.clone(), cfg);
            }
            Err(e) => {
                tracing::warn!(
                    id = %server.id,
                    name = %server.name,
                    error = %e,
                    "agent_config: invalid JSON in mcp_server.config; skipping entry"
                );
            }
        }
    }
}

/// Lay an agent's effective MCP servers (`Store::effective_mcp_servers`)
/// over a `.mcp.json` the frontend built (the `agentmux` entry and the
/// agent's legacy blob), so a picker launch gets the same catalog servers
/// as `agent.open`. Rows win over the blob on a name collision, as in
/// `agent_config::build_mcp_config_from_refs`; the `agentmux` entry is never replaced.
/// `None` when `mcp_json` isn't a JSON object with an `mcpServers` object:
/// the caller writes it unchanged.
/// `SPEC_CONNECTORS_MCP_WHAT_AGENTS_RUN_2026_10_10.md` §2.2.
pub fn overlay_mcp_servers(mcp_json: &str, servers: &[crate::backend::storage::McpServer]) -> Option<String> {
    let mut root = serde_json::from_str::<Value>(mcp_json).ok()?;
    let map = root.get_mut("mcpServers")?.as_object_mut()?;
    insert_server_rows(map, servers);
    serde_json::to_string_pretty(&root).ok()
}

/// [`overlay_mcp_servers`] with `agent_id`'s effective servers
/// (`Store::effective_mcp_servers`): `refs` holds the agent binds, `catalog`
/// the catalog and its reach switches. The picker path's whole MCP step.
pub fn overlay_for_agent(
    mcp_json: &str,
    refs: &crate::backend::storage::store::Store,
    catalog: &crate::backend::storage::store::Store,
    agent_id: &str,
    container: bool,
) -> Option<String> {
    overlay_mcp_servers(mcp_json, &refs.effective_mcp_servers(catalog, agent_id, container))
}

/// The picker path's MCP step (`writeagentconfig`): lay the agent's
/// effective servers over the `.mcp.json` among the frontend's files, which
/// has only the `agentmux` entry and the agent's legacy blob. Nothing to do
/// without one, or without an agent id (an older frontend sends none).
pub fn add_to_files(
    files: &mut [crate::backend::rpc_types::AgentConfigFile],
    refs: &crate::backend::storage::store::Store,
    catalog: &crate::backend::storage::store::Store,
    agent_id: Option<&str>,
    container: bool,
) {
    let Some(agent_id) = agent_id.filter(|id| !id.is_empty()) else { return };
    let Some(file) = files.iter_mut().find(|f| f.path == ".mcp.json") else { return };
    if let Some(merged) = overlay_for_agent(&file.content, refs, catalog, agent_id, container) {
        file.content = merged;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::McpServer;

    fn row(name: &str, config: &str) -> McpServer {
        McpServer {
            id: format!("id-{name}"),
            name: name.to_string(),
            transport: "stdio".to_string(),
            config: config.to_string(),
            is_global: true,
            created_at: 1,
            updated_at: 1,
        }
    }

    fn names(json: &str) -> Vec<String> {
        let v: Value = serde_json::from_str(json).unwrap();
        v["mcpServers"].as_object().unwrap().keys().cloned().collect()
    }

    #[test]
    fn rows_are_added_beside_the_frontends_entries() {
        let built = r#"{"mcpServers":{"agentmux":{"command":"agentmux-mcp"},"legacy":{"command":"old"}}}"#;
        let out = overlay_mcp_servers(built, &[row("git", r#"{"command":"uvx","args":["mcp-server-git"]}"#)]).unwrap();
        assert_eq!(names(&out), vec!["agentmux", "git", "legacy"]);
    }

    #[test]
    fn a_row_wins_over_the_blob_but_never_over_the_built_in_entry() {
        let built = r#"{"mcpServers":{"agentmux":{"command":"agentmux-mcp"},"git":{"command":"old"}}}"#;
        let rows = [row("git", r#"{"command":"new"}"#), row("agentmux", r#"{"command":"evil"}"#)];
        let v: Value = serde_json::from_str(&overlay_mcp_servers(built, &rows).unwrap()).unwrap();
        assert_eq!(v["mcpServers"]["git"]["command"], "new");
        assert_eq!(v["mcpServers"]["agentmux"]["command"], "agentmux-mcp");
    }

    #[test]
    fn a_row_with_broken_config_is_skipped() {
        let built = r#"{"mcpServers":{"agentmux":{}}}"#;
        let out = overlay_mcp_servers(built, &[row("bad", "{not json")]).unwrap();
        assert_eq!(names(&out), vec!["agentmux"]);
    }

    /// A picker launch gets a catalog server switched on for all agents, and
    /// a container launch only once it is switched on for containers too.
    #[test]
    fn a_picker_launch_gets_the_catalog_servers_that_reach_it() {
        use crate::backend::storage::{store::Store, McpServerReach};
        let store = Store::open_in_memory().unwrap();
        store.mcp_server_upsert_unique_global(&row("git", r#"{"command":"uvx"}"#)).unwrap();
        let built = r#"{"mcpServers":{"agentmux":{"command":"agentmux-mcp"}}}"#;
        let launch = |container| names(&overlay_for_agent(built, &store, &store, "agent-1", container).unwrap());

        assert_eq!(launch(false), vec!["agentmux"], "in the catalog alone reaches no one");
        let all = McpServerReach { all_agents: true, in_containers: false };
        store.mcp_server_set_reach("id-git", all, 1).unwrap();
        assert_eq!(launch(false), vec!["agentmux", "git"]);
        assert_eq!(launch(true), vec!["agentmux"]);
        store.mcp_server_set_reach("id-git", McpServerReach { in_containers: true, ..all }, 2).unwrap();
        assert_eq!(launch(true), vec!["agentmux", "git"]);
    }

    #[test]
    fn content_that_isnt_an_mcp_json_is_left_to_the_caller() {
        assert!(overlay_mcp_servers("not json", &[]).is_none());
        assert!(overlay_mcp_servers(r#"{"servers":{}}"#, &[]).is_none());
    }
}
