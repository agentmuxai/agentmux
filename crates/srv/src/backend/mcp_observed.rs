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
//!
//! Each report belongs to the spawn that sent it, by the spawn's
//! process-wide registration nonce (`persistent::next_registration_nonce`),
//! which only grows. A report is kept only over one from an older or the
//! same spawn, and a spawn's exit clears only its own: so a replaced process,
//! even one of a replaced controller whose `spawn_generation` restarted,
//! never writes over or clears a newer process's servers. A spawn that has
//! exited records nothing more: its reader can still be running after the
//! exit is handled (an abort doesn't stop it mid-poll), and a late report
//! would stay with nothing left to clear it.

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
    /// The spawn that reported it (module doc).
    #[serde(skip)]
    pub owner: u64,
}

#[derive(Default)]
struct State {
    reports: HashMap<String, Observation>,
    /// Per pane, the newest spawn that has exited: none of its reports, or
    /// an older spawn's, is kept any more.
    ended: HashMap<String, u64>,
}

/// At most this many panes' exit markers (`State::ended`) are kept, the
/// oldest spawns going first: a marker matters only while its reader may
/// still be finishing, seconds after the exit.
const MAX_ENDED: usize = 4096;

static OBSERVED: LazyLock<Mutex<State>> = LazyLock::new(|| Mutex::new(State::default()));

fn observed() -> std::sync::MutexGuard<'static, State> {
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

/// Keep what spawn `owner` of a pane reported, unless a newer spawn already
/// reported or this one has exited. True when it changed, so the caller
/// tells the panes (`mcp:observed`); a resumed turn's init repeats it.
pub fn record(block_id: &str, owner: u64, servers: Vec<ObservedServer>, at_ms: u64) -> bool {
    let mut state = observed();
    if state.ended.get(block_id).is_some_and(|&ended| owner <= ended) {
        return false;
    }
    let changed = match state.reports.get(block_id) {
        Some(o) if o.owner > owner => return false,
        Some(o) => o.servers != servers || o.owner != owner,
        None => true,
    };
    state.reports.insert(block_id.to_string(), Observation { servers, at_ms, owner });
    changed
}

/// Record an init frame's servers for spawn `owner`. True when they changed:
/// the caller then publishes (`publish`).
pub fn record_init(block_id: &str, owner: u64, frame: &Value) -> bool {
    match parse_init(frame) {
        Some(servers) => record(block_id, owner, servers, agentmux_common::time::now_ms_u64()),
        None => false,
    }
}

/// Spawn `owner` exited: drop its report, or an older spawn's still shown
/// (a replacement that exits before reporting leaves its predecessor's),
/// never a newer spawn's; and keep none it sends later. True when there was
/// one to drop.
pub fn forget_owned(block_id: &str, owner: u64) -> bool {
    let mut state = observed();
    let ended = state.ended.entry(block_id.to_string()).or_insert(0);
    *ended = (*ended).max(owner);
    if state.ended.len() > MAX_ENDED {
        // Nonces only grow, so the smallest are the oldest exits.
        let mut owners: Vec<u64> = state.ended.values().copied().collect();
        owners.sort_unstable();
        let cutoff = owners[state.ended.len() - MAX_ENDED];
        state.ended.retain(|_, &mut o| o >= cutoff);
    }
    if state.reports.get(block_id).is_some_and(|o| o.owner <= owner) {
        state.reports.remove(block_id);
        return true;
    }
    false
}

/// Tell the panes a report changed or went (`mcp:observed`).
pub fn publish(broker: Option<&crate::backend::mps::Broker>, block_id: &str) {
    let Some(broker) = broker else { return };
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
    observed().reports.clone()
}

/// Drop everything kept for panes whose block is gone (`exists` says which
/// still are): their reports, and their exit markers.
pub fn prune(exists: impl Fn(&str) -> bool) {
    let mut state = observed();
    state.reports.retain(|block, _| exists(block));
    state.ended.retain(|block, _| exists(block));
}

/// Drop a pane's report and exit marker, whoever sent it. True when there
/// was a report. Tests only: srv drops gone panes with `prune`.
#[cfg(test)]
fn forget(block_id: &str) -> bool {
    let mut state = observed();
    state.ended.remove(block_id);
    state.reports.remove(block_id).is_some()
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
        assert!(record(block, 7, servers.clone(), 1));
        assert!(!record(block, 7, servers.clone(), 2), "same servers again");
        let mut fewer = servers;
        fewer.pop();
        assert!(record(block, 7, fewer, 3));
        assert_eq!(all().get(block).map(|o| o.at_ms), Some(3));
        forget(block);
        assert!(!all().contains_key(block));
    }

    #[test]
    fn an_init_frame_is_recorded_once_and_forgotten_once() {
        let block = "mcp-observed-test-ended";
        assert!(record_init(block, 3, &init_frame()));
        assert!(!record_init(block, 3, &init_frame()), "the same report again");
        assert!(!record_init(block, 3, &json!({"type": "system"})), "not an init frame");
        assert!(forget_owned(block, 3));
        assert!(!forget_owned(block, 3), "nothing left to forget");
    }

    /// A replaced spawn (an older nonce), even under a new controller whose
    /// generation restarted, never writes over or clears the newer one's.
    #[test]
    fn an_older_spawn_never_overwrites_or_clears_a_newer_ones_report() {
        let block = "mcp-observed-test-owner";
        assert!(record_init(block, 10, &init_frame()));
        assert!(!record_init(block, 9, &json!({"mcp_servers": []})), "older spawn's late init");
        assert_eq!(all().get(block).map(|o| o.servers.len()), Some(4));
        assert!(!forget_owned(block, 9), "older spawn's exit");
        assert!(all().contains_key(block));
        assert!(forget_owned(block, 10));
        forget(block);
    }

    /// A replacement that exits before reporting takes its predecessor's
    /// report with it; a newer spawn's report stays.
    #[test]
    fn an_exit_clears_an_older_spawns_report_but_not_a_newer_ones() {
        let block = "mcp-observed-test-older";
        record_init(block, 40, &init_frame());
        assert!(forget_owned(block, 41), "the replacement exited without reporting");
        assert!(!all().contains_key(block));
        record_init(block, 43, &init_frame());
        assert!(!forget_owned(block, 42), "an older spawn's exit");
        assert!(all().contains_key(block));
        forget(block);
    }

    #[test]
    fn a_deleted_panes_report_and_exit_marker_both_go() {
        let (gone, kept) = ("mcp-observed-test-gone", "mcp-observed-test-kept");
        record_init(gone, 30, &init_frame());
        forget_owned(gone, 30);
        record_init(kept, 31, &init_frame());
        prune(|block| block != gone);
        assert!(!observed().ended.contains_key(gone));
        assert!(all().contains_key(kept));
        forget(kept);
    }

    /// A reader still running after its process's exit was handled (an
    /// aborted task finishing its poll) can't bring the servers back; the
    /// next spawn can.
    #[test]
    fn an_exited_spawns_late_report_is_not_kept() {
        let block = "mcp-observed-test-late";
        assert!(!forget_owned(block, 20), "exit handled before any report");
        assert!(!record_init(block, 20, &init_frame()), "the late init");
        assert!(!all().contains_key(block));
        assert!(record_init(block, 21, &init_frame()), "the next spawn");
        forget(block);
    }
}
