// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The MCP servers each agent's CLI says it has, per pane: what Connectors
//! shows, as opposed to what AgentMux wrote into the agent's `.mcp.json`
//! (`SPEC_CONNECTORS_MCP_WHAT_AGENTS_RUN_2026_10_10.md` §3).
//!
//! Claude Code's `system/init` frame lists every MCP server it loaded, from
//! every source, with a status and the CLI's own source label:
//! `{"name":"stub","status":"connected","source":"project"}`. The label is
//! `project` (`.mcp.json`, which is where AgentMux's servers go), `user`
//! (the account's config), `local`, `dynamic` (`--mcp-config`, plugins),
//! `managed`, `enterprise` or `claudeai` (claude.ai connectors). Its tool
//! list names each server's tools `mcp__<server>__<tool>`, the server name
//! with everything but letters, digits, `_` and `-` made `_`. Checked
//! against the real CLI by `scripts/cli-contract/mcp-servers-contract.mjs`.
//!
//! Only names, statuses and counts: the frame carries no server config, so
//! nothing secret passes through here. The health of the `agentmux` entry
//! and its repair are `mcp_health`'s; this only records.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use serde::Serialize;
use serde_json::Value;

/// One MCP server as the agent's CLI reported it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ObservedServer {
    pub name: String,
    /// `connected`, `failed`, `needs-auth` or `pending`, as the CLI says it.
    pub status: String,
    /// The CLI's source label (module doc), `""` when it gave none.
    pub source: String,
    /// How many of the agent's tools come from this server.
    pub tools: u32,
}

/// The servers one pane's CLI last reported, and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Observation {
    pub servers: Vec<ObservedServer>,
    pub at_ms: u64,
}

static OBSERVED: LazyLock<Mutex<HashMap<String, Observation>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn observed() -> std::sync::MutexGuard<'static, HashMap<String, Observation>> {
    OBSERVED.lock().unwrap_or_else(|e| e.into_inner())
}

/// The prefix of a server's tool names in the CLI's tool list.
fn tool_prefix(server: &str) -> String {
    let name: String = server
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect();
    format!("mcp__{name}__")
}

/// The servers in an init frame, or `None` when it has no `mcp_servers`
/// (not an init frame, or another provider's).
pub fn parse_init(frame: &Value) -> Option<Vec<ObservedServer>> {
    let servers = frame.get("mcp_servers")?.as_array()?;
    let tools: Vec<&str> = frame
        .get("tools")
        .and_then(|t| t.as_array())
        .map(|t| t.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    let text = |v: &Value, key: &str| v.get(key).and_then(|s| s.as_str()).unwrap_or("").to_string();
    Some(
        servers
            .iter()
            .filter_map(|s| {
                let name = text(s, "name");
                if name.is_empty() {
                    return None;
                }
                let prefix = tool_prefix(&name);
                let count = tools.iter().filter(|t| t.starts_with(&prefix)).count();
                Some(ObservedServer {
                    status: text(s, "status"),
                    source: text(s, "source"),
                    tools: u32::try_from(count).unwrap_or(u32::MAX),
                    name,
                })
            })
            .collect(),
    )
}

/// Keep what a pane's CLI reported. True when it changed, so the caller
/// tells the panes (`mcp:observed`); a resumed turn's init repeats it.
pub fn record(block_id: &str, servers: Vec<ObservedServer>, at_ms: u64) -> bool {
    let mut map = observed();
    let changed = map.get(block_id).map(|o| o.servers != servers).unwrap_or(true);
    map.insert(block_id.to_string(), Observation { servers, at_ms });
    changed
}

/// Record an init frame and, if anything changed, publish `mcp:observed`.
pub fn observe_init(block_id: &str, frame: &Value, broker: Option<&crate::backend::mps::Broker>) {
    let Some(servers) = parse_init(frame) else { return };
    if record(block_id, servers, agentmux_common::time::now_ms_u64()) {
        if let Some(broker) = broker {
            publish(broker, block_id);
        }
    }
}

/// A pane's CLI exited: its servers are gone. Publishes `mcp:observed` when
/// there was a report to drop.
pub fn process_ended(block_id: &str, broker: Option<&crate::backend::mps::Broker>) {
    if observed().remove(block_id).is_none() {
        return;
    }
    if let Some(broker) = broker {
        publish(broker, block_id);
    }
}

fn publish(broker: &crate::backend::mps::Broker, block_id: &str) {
    broker.publish(crate::backend::mps::MuxEvent {
        event: "mcp:observed".to_string(),
        scopes: vec![],
        sender: String::new(),
        persist: 0,
        data: Some(serde_json::json!({ "block_id": block_id })),
    });
}

/// Every pane's last report, by block id.
pub fn all() -> HashMap<String, Observation> {
    observed().clone()
}

/// Drop a pane's report (its block is gone), quietly.
pub fn forget(block_id: &str) {
    observed().remove(block_id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The frame the real CLI (2.1.288) sent with a working server, one with
    /// a space and a dot in its name, and one that exited at once.
    fn init_frame() -> Value {
        json!({
            "type": "system",
            "subtype": "init",
            "mcp_servers": [
                {"name": "stub", "status": "connected", "source": "project"},
                {"name": "my server.v2", "status": "connected", "source": "project"},
                {"name": "broken", "status": "failed", "source": "project"},
                {"name": "claude.ai Gmail", "status": "needs-auth", "source": "claudeai"}
            ],
            "tools": ["Bash", "mcp__my_server_v2__alpha", "mcp__my_server_v2__beta", "mcp__stub__alpha", "mcp__stub__beta", "mcp__stubby__x"]
        })
    }

    #[test]
    fn an_init_frame_gives_each_server_its_status_source_and_tool_count() {
        let servers = parse_init(&init_frame()).unwrap();
        let got: Vec<(&str, &str, &str, u32)> =
            servers.iter().map(|s| (s.name.as_str(), s.status.as_str(), s.source.as_str(), s.tools)).collect();
        assert_eq!(
            got,
            vec![
                ("stub", "connected", "project", 2),
                ("my server.v2", "connected", "project", 2),
                ("broken", "failed", "project", 0),
                ("claude.ai Gmail", "needs-auth", "claudeai", 0),
            ]
        );
    }

    #[test]
    fn a_frame_without_mcp_servers_is_not_an_observation() {
        assert!(parse_init(&json!({"type": "system", "subtype": "init"})).is_none());
        assert!(parse_init(&json!({"type": "assistant"})).is_none());
    }

    #[test]
    fn a_repeated_report_is_not_a_change() {
        let block = "mcp-observed-test-block";
        let servers = parse_init(&init_frame()).unwrap();
        assert!(record(block, servers.clone(), 1));
        assert!(!record(block, servers.clone(), 2), "same servers again");
        let mut fewer = servers;
        fewer.pop();
        assert!(record(block, fewer, 3));
        assert_eq!(all().get(block).map(|o| o.at_ms), Some(3));
        forget(block);
        assert!(!all().contains_key(block));
    }

    #[test]
    fn a_process_that_ended_has_no_servers() {
        let block = "mcp-observed-test-ended";
        record(block, parse_init(&init_frame()).unwrap(), 1);
        process_ended(block, None);
        assert!(!all().contains_key(block));
        process_ended(block, None);
    }
}
