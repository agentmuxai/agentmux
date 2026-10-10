// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which agent tool call each running Bash command is: `agentmux-bashwrap`,
//! running a call as Claude Code's shell prefix, reports its own pid with the
//! call's id and description (`prefix.rs`, `{op:"call"}` on the wrapper's
//! publish route). Tower labels that process, the root of the call's process
//! tree, with the description ("started by").
//! docs/specs/SPEC_TOWER_AGENT_CENTRIC_VIEWS_2026_10_08.md §5.3.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// A call's report is kept this long: a background `task dev` runs for hours.
const KEEP_MS: u64 = 12 * 60 * 60 * 1000;
/// At most this many reports, the newest kept.
const MAX_CALLS: usize = 4096;
/// A process is the reporting wrapper only if it started at most this long
/// before the report (the report is sent as the wrapper starts its command):
/// a later process reusing the pid never inherits the label.
const START_SLACK_MS: u64 = 60_000;
/// And at most this long after it: a start time the OS gives in whole
/// seconds (Linux's boot time) can land just after the report.
const CLOCK_SLACK_MS: u64 = 2_000;

/// One tool call a wrapper reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    pub block_id: String,
    pub tool_id: String,
    pub description: String,
    /// When it was reported, unix ms.
    pub at_ms: u64,
}

fn calls() -> &'static Mutex<HashMap<u32, Call>> {
    static CALLS: OnceLock<Mutex<HashMap<u32, Call>>> = OnceLock::new();
    CALLS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Note a wrapper's report: `pid` runs `call`.
pub fn record(pid: u32, call: Call) {
    let mut map = calls().lock().unwrap_or_else(|e| e.into_inner());
    let cutoff = call.at_ms.saturating_sub(KEEP_MS);
    map.retain(|_, c| c.at_ms >= cutoff);
    if map.len() >= MAX_CALLS {
        if let Some(oldest) = map.iter().min_by_key(|(_, c)| c.at_ms).map(|(p, _)| *p) {
            map.remove(&oldest);
        }
    }
    map.insert(pid, call);
}

/// The call the process with `pid`, started at `started_at_ms`, runs, if it
/// is the wrapper that reported one. A process whose start is unknown is
/// trusted on its pid alone.
pub fn lookup(pid: u32, started_at_ms: Option<u64>) -> Option<Call> {
    let map = calls().lock().unwrap_or_else(|e| e.into_inner());
    let call = map.get(&pid)?;
    match started_at_ms {
        Some(started) if started > call.at_ms + CLOCK_SLACK_MS || call.at_ms.saturating_sub(started) > START_SLACK_MS => None,
        _ => Some(call.clone()),
    }
}

/// A `{op:"call"}` report from the wrapper's publish, if `data` is one:
/// `(pid, call)`, with the block from the publish's `block:<id>` scope.
pub fn parse_report(scopes: &[String], data: &serde_json::Value) -> Option<(u32, Call)> {
    if data.get("op").and_then(|v| v.as_str()) != Some("call") {
        return None;
    }
    let pid = u32::try_from(data.get("pid")?.as_u64()?).ok()?;
    let tool_id = data.get("tool_id")?.as_str()?.to_string();
    let description = data.get("description")?.as_str()?.trim().to_string();
    if tool_id.is_empty() || description.is_empty() {
        return None;
    }
    let block_id = scopes.iter().find_map(|s| s.strip_prefix("block:")).unwrap_or("").to_string();
    let at_ms = data
        .get("timestamp")
        .and_then(|v| v.as_u64())
        .unwrap_or_else(|| agentmux_common::time::now_ms() as u64);
    Some((pid, Call { block_id, tool_id, description, at_ms }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_a_wrappers_report_and_nothing_else() {
        let scopes = vec!["block:b1".to_string()];
        let (pid, call) = parse_report(
            &scopes,
            &json!({"op": "call", "tool_id": "toolu_1", "pid": 4242, "description": " Run the srv tests ", "timestamp": 1000}),
        )
        .unwrap();
        assert_eq!(pid, 4242);
        assert_eq!(
            call,
            Call { block_id: "b1".into(), tool_id: "toolu_1".into(), description: "Run the srv tests".into(), at_ms: 1000 }
        );
        for other in [
            json!({"op": "chunk", "tool_id": "t", "content": "x"}),
            json!({"op": "pid", "tool_id": "t", "pid": 1}),
            json!({"op": "call", "tool_id": "", "pid": 1, "description": "d"}),
            json!({"op": "call", "tool_id": "t", "pid": 1, "description": "  "}),
            json!({"op": "call", "tool_id": "t", "description": "d"}),
        ] {
            assert!(parse_report(&scopes, &other).is_none(), "{other}");
        }
    }

    #[test]
    fn a_process_is_the_reporting_wrapper_only_if_it_started_before_the_report() {
        let call = |at_ms| Call { block_id: "b".into(), tool_id: "t".into(), description: "Build".into(), at_ms };
        record(910_001, call(500_000));
        assert_eq!(lookup(910_001, Some(499_000)).unwrap().description, "Build");
        assert!(lookup(910_001, None).is_some(), "an unknown start is trusted on the pid");
        assert!(lookup(910_001, Some(501_000)).is_some(), "a start a second late is the clock's");
        assert!(lookup(910_001, Some(500_000 + CLOCK_SLACK_MS + 1)).is_none(), "started after the report: the pid was reused");
        assert!(lookup(910_001, Some(500_000 - START_SLACK_MS - 1)).is_none(), "far older than the report");
        assert!(lookup(910_002, Some(499_000)).is_none());
    }
}
