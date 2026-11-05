// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Reading one stream-json transcript line for the rows it holds: a tool call, a thinking block or a turn's end.

use serde_json::Value;

use super::scrub;
use crate::backend::rpc_types::AuditCost;

/// A line longer than this is not read: it is a big tool result, and holds no row.
const LINE_MAX: usize = 2 * 1024 * 1024;

/// What one line of the transcript says happened.
#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    pub kind: &'static str,
    pub summary: String,
    pub tool: Option<String>,
    pub outcome: Option<String>,
    pub ms: Option<u64>,
    pub cost: Option<AuditCost>,
}

fn row(kind: &'static str, summary: String) -> Parsed {
    Parsed { kind, summary, tool: None, outcome: None, ms: None, cost: None }
}

/// The rows in `line`, none for most lines. A cheap test first: nearly every line is a message or a tool result.
pub fn parse(line: &str) -> Vec<Parsed> {
    let maybe_call = line.contains("\"tool_use\"") || line.contains("\"thinking\"");
    let maybe_end = line.contains("\"type\":\"result\"") || line.contains("\"type\": \"result\"");
    if line.len() > LINE_MAX || !(maybe_call || maybe_end) {
        return Vec::new();
    }
    let Ok(value) = serde_json::from_str::<Value>(line) else { return Vec::new() };
    match value.get("type").and_then(Value::as_str) {
        Some("assistant") => calls_in(&value),
        Some("result") => vec![turn_end(&value)],
        _ => Vec::new(),
    }
}

fn calls_in(line: &Value) -> Vec<Parsed> {
    let Some(blocks) = line.pointer("/message/content").and_then(Value::as_array) else { return Vec::new() };
    blocks
        .iter()
        .filter_map(|block| match block.get("type").and_then(Value::as_str) {
            Some("tool_use") => {
                let name = block.get("name").and_then(Value::as_str).filter(|n| !n.is_empty())?;
                let input = block.get("input").unwrap_or(&Value::Null);
                Some(Parsed { tool: Some(name.to_string()), ..row("tool", scrub::summarize(name, input)) })
            }
            // A marker only: the thought's text is never kept.
            Some("thinking") | Some("redacted_thinking") => Some(row("thought", "thinking".to_string())),
            _ => None,
        })
        .collect()
}

fn turn_end(line: &Value) -> Parsed {
    let subtype = line.get("subtype").and_then(Value::as_str).unwrap_or("success");
    let failed = line.get("is_error").and_then(Value::as_bool).unwrap_or(false) || subtype.starts_with("error");
    let outcome = if failed { if subtype == "success" { "error" } else { subtype } } else { "success" };
    let ms = line.get("duration_ms").and_then(Value::as_u64);
    let secs = ms.map(|m| format!(" in {:.1} s", m as f64 / 1000.0)).unwrap_or_default();
    let usage = line.get("usage");
    let count = |name: &str| usage.and_then(|u| u.get(name)).and_then(Value::as_u64).unwrap_or(0);
    let cost = usage.map(|_| AuditCost {
        input: count("input_tokens") + count("cache_creation_input_tokens"),
        cache_read: count("cache_read_input_tokens"),
        output: count("output_tokens"),
        usd: line.get("total_cost_usd").and_then(Value::as_f64),
        basis: "measured".to_string(),
    });
    Parsed { outcome: Some(outcome.to_string()), ms, cost, ..row("turn", format!("turn ended ({outcome}){secs}")) }
}
