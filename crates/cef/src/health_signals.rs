// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Performance health signals (low RAM, low page file, slow backend): the
//! host keeps each kind's current level so a window opened mid-episode can
//! catch up, and pushes every change to the open windows as `health-signal`.
//! docs/reports/REPORT_PERFORMANCE_INDICATORS_TO_STATUS_BAR_2026_10_08.md §3.2.

use std::collections::BTreeMap;

use crate::state::AppState;

pub const EVENT: &str = "health-signal";

/// Fold `payload` into `active` (kind → its latest non-normal payload) and
/// return what to push. A non-normal payload gets `since_ms`, the start of its
/// episode, kept across warn ↔ critical; a normal one removes the kind.
pub(crate) fn record(
    active: &mut BTreeMap<String, serde_json::Value>,
    mut payload: serde_json::Value,
    now_ms: u64,
) -> serde_json::Value {
    let kind = payload["kind"].as_str().unwrap_or_default().to_string();
    if payload["level"] == "normal" {
        active.remove(&kind);
        return payload;
    }
    let since = active.get(&kind).and_then(|p| p["since_ms"].as_u64()).unwrap_or(now_ms);
    payload["since_ms"] = since.into();
    active.insert(kind, payload.clone());
    payload
}

/// Record a level and push it to every top-level window. UI thread only (the
/// emit runs JavaScript); callers on other threads post a task.
pub fn emit(state: &AppState, payload: serde_json::Value) {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default();
    let payload = record(&mut state.health_signals.lock(), payload, now_ms);
    crate::events::emit_event_to_top_level_windows(state, EVENT, &payload);
}

/// Every kind that is not normal right now, for a window that just opened.
pub fn snapshot(state: &AppState) -> serde_json::Value {
    serde_json::Value::Array(state.health_signals.lock().values().cloned().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_episode_keeps_its_start_across_levels_and_ends_at_normal() {
        let mut active = BTreeMap::new();
        let first = record(&mut active, json!({ "kind": "backend", "level": "warn", "avg_ms": 1200 }), 1_000);
        assert_eq!(first["since_ms"], 1_000);
        let worse = record(&mut active, json!({ "kind": "backend", "level": "critical", "avg_ms": 2900 }), 5_000);
        assert_eq!(worse["since_ms"], 1_000, "escalating is the same episode");
        assert_eq!(active["backend"]["level"], "critical");

        let cleared = record(&mut active, json!({ "kind": "backend", "level": "normal" }), 9_000);
        assert!(cleared.get("since_ms").is_none());
        assert!(active.is_empty());

        let again = record(&mut active, json!({ "kind": "backend", "level": "warn" }), 12_000);
        assert_eq!(again["since_ms"], 12_000, "a new episode starts afresh");
    }

    #[test]
    fn kinds_are_tracked_apart() {
        let mut active = BTreeMap::new();
        record(&mut active, json!({ "kind": "ram", "level": "warn" }), 1);
        record(&mut active, json!({ "kind": "pagefile", "level": "critical" }), 2);
        record(&mut active, json!({ "kind": "ram", "level": "normal" }), 3);
        assert_eq!(active.keys().collect::<Vec<_>>(), vec!["pagefile"]);
    }
}
