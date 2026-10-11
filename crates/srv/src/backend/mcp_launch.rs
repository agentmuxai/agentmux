// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The MCP servers an agent launches with, put into its `.mcp.json`: the same
//! rows on both launch paths, `agent.open` (`agent_config::build_mcp_config_from_refs`)
//! and the picker's `writeagentconfig` ([`overlay_mcp_servers`]).
//! `SPEC_CONNECTORS_MCP_WHAT_AGENTS_RUN_2026_10_10.md` §2.2.

use serde_json::Value;

/// Whether a row is written into `.mcp.json` at all: not the reserved
/// `agentmux` name, and a config that is JSON. [`insert_server_rows`] skips
/// the others, so a server that is never launched is never "missing".
pub(crate) fn is_launched(server: &crate::backend::storage::McpServer) -> bool {
    server.name != "agentmux" && serde_json::from_str::<Value>(&server.config).is_ok()
}

/// The catalog (global) servers AgentMux launches an agent with that its CLI
/// doesn't report (`reported`, from `backend::mcp_observed`), by name. That
/// is a server bound or switched on after the agent started: `.mcp.json` is
/// written at launch, so the agent needs a restart to get it. A server the
/// CLI reports as failed is reported, so not missing. The agent's own private
/// servers are left out: the pane matches by name against catalog rows, and a
/// private server can share a name with a catalog one.
pub fn missing_servers(
    expected: &[crate::backend::storage::McpServer],
    reported: &[crate::backend::mcp_observed::ObservedServer],
) -> Vec<String> {
    let mut missing: Vec<String> = expected
        .iter()
        .filter(|s| s.is_global && is_launched(s) && !reported.iter().any(|r| r.name == s.name))
        .map(|s| s.name.clone())
        .collect();
    missing.sort();
    missing.dedup();
    missing
}

/// [`missing_servers`] for `agent_id`: what AgentMux would launch it with now
/// (`Store::effective_mcp_servers`, which gives a container agent only what
/// is switched on for containers) against what its CLI reported.
pub fn missing_for_agent(
    refs: &crate::backend::storage::store::Store,
    catalog: &crate::backend::storage::store::Store,
    agent_id: &str,
    container: bool,
    reported: &[crate::backend::mcp_observed::ObservedServer],
) -> Vec<String> {
    missing_servers(&refs.effective_mcp_servers(catalog, agent_id, container), reported)
}

/// What a pane needs a restart for: [`missing_for_agent`] for a pane running
/// an agent definition. `definition_type` is that definition's kind, `None`
/// when `agent_id` names no definition: a quick-launch pane's "agent id" is
/// its provider's name, and no `.mcp.json` is written for it, so there is
/// nothing to restart for. `pane_mode` is the pane's own `agentMode`, which
/// `writeagentconfig` launched it by and the picker's runtime override can
/// set differently from the definition's.
pub fn missing_for_pane(
    refs: &crate::backend::storage::store::Store,
    catalog: &crate::backend::storage::store::Store,
    agent_id: &str,
    pane_mode: Option<&str>,
    definition_type: Option<&str>,
    reported: &[crate::backend::mcp_observed::ObservedServer],
) -> Vec<String> {
    let Some(definition_type) = definition_type else { return Vec::new() };
    let container = pane_mode.filter(|m| !m.is_empty()).unwrap_or(definition_type) == "container";
    missing_for_agent(refs, catalog, agent_id, container, reported)
}

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

    fn reported(name: &str, status: &str) -> crate::backend::mcp_observed::ObservedServer {
        crate::backend::mcp_observed::ObservedServer {
            name: name.to_string(),
            status: status.to_string(),
            source: "project".to_string(),
            tools: 0,
        }
    }

    /// A server launched after the agent started isn't reported; one the CLI
    /// reports as failed is (it was launched), and so is the built-in.
    #[test]
    fn a_server_the_agent_does_not_report_is_missing() {
        let expected = [row("git", "{}"), row("fetch", "{}"), row("memory", "{}")];
        let now = [reported("agentmux", "connected"), reported("git", "connected"), reported("fetch", "failed")];
        assert_eq!(missing_servers(&expected, &now), vec!["memory"]);
        assert!(missing_servers(&[], &now).is_empty());
    }

    /// Through the real stores: a catalog server switched on for all agents
    /// is missing from an agent that started before, until it reports it.
    #[test]
    fn an_agent_that_started_before_a_server_was_switched_on_is_missing_it() {
        use crate::backend::storage::{store::Store, McpServerReach};
        let store = Store::open_in_memory().unwrap();
        store.mcp_server_upsert_unique_global(&row("git", r#"{"command":"uvx"}"#)).unwrap();
        let before = [reported("agentmux", "connected")];
        assert!(missing_for_agent(&store, &store, "agent-1", false, &before).is_empty(), "not switched on yet");

        store.mcp_server_set_reach("id-git", McpServerReach { all_agents: true, in_containers: false }, 1).unwrap();
        assert_eq!(missing_for_agent(&store, &store, "agent-1", false, &before), vec!["git"]);
        let after = [reported("agentmux", "connected"), reported("git", "connected")];
        assert!(missing_for_agent(&store, &store, "agent-1", false, &after).is_empty());
        // A container pane never got it (not switched on for containers), so
        // isn't told to restart for it.
        assert!(missing_for_agent(&store, &store, "agent-1", true, &before).is_empty());
    }

    /// A pane is judged as what it was launched as: its own mode wins over
    /// the definition's, and a pane with no definition (quick launch, which
    /// writes no `.mcp.json`) has nothing to restart for.
    #[test]
    fn a_pane_is_judged_by_how_it_was_launched() {
        use crate::backend::storage::{store::Store, McpServerReach};
        let store = Store::open_in_memory().unwrap();
        store.mcp_server_upsert_unique_global(&row("git", "{}")).unwrap();
        store.mcp_server_set_reach("id-git", McpServerReach { all_agents: true, in_containers: false }, 1).unwrap();
        let none = [reported("agentmux", "connected")];
        let missing = |mode: Option<&str>, def: Option<&str>| missing_for_pane(&store, &store, "a", mode, def, &none);

        assert_eq!(missing(None, Some("host")), vec!["git"], "a host agent that started before");
        assert!(missing(None, None).is_empty(), "quick launch: no definition, no restart can help");
        assert!(missing(Some("container"), Some("host")).is_empty(), "a host definition launched as a container");
        assert_eq!(missing(Some("host"), Some("container")), vec!["git"], "the reverse override");
        assert!(missing(Some(""), Some("container")).is_empty(), "no pane mode: the definition's");
    }

    /// What `insert_server_rows` skips is never launched, so never missing:
    /// the two stay in step.
    /// An agent's private server isn't a catalog row, even when a catalog
    /// server of the same name exists: it is never listed, so the pane can't
    /// show it under that row.
    #[test]
    fn a_private_server_is_never_missing_from_the_catalog_view() {
        let mut private = row("git", "{}");
        private.is_global = false;
        let catalog_only = row("fetch", "{}");
        assert_eq!(missing_servers(&[private.clone(), catalog_only], &[]), vec!["fetch"]);
        assert!(missing_servers(&[private], &[]).is_empty());
    }

    #[test]
    fn a_server_that_is_never_launched_is_never_missing() {
        let rows = [row("git", "{}"), row("agentmux", "{}"), row("broken", "{not json")];
        let mut written = serde_json::Map::new();
        insert_server_rows(&mut written, &rows);
        let launched: Vec<&str> = rows.iter().filter(|s| is_launched(s)).map(|s| s.name.as_str()).collect();
        assert_eq!(written.keys().map(String::as_str).collect::<Vec<_>>(), launched);
        assert_eq!(missing_servers(&rows, &[]), vec!["git"]);
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
