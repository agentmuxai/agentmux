// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The Once block: "do this once for this key" (docs/specs/SPEC_DRONE_REAL_WORLD_JOBS_2026_11_11.md §4.2).
//!
//! `{ "kind": "once", "key": "{{trigger.run_id}}" }`. The first run of the drone to reach it with a key goes on;
//! a later run with the same key takes the other branch. It answers like a Condition does: `result` is true for the
//! first and false for every later one, and the edges off its `true` and `false` handles carry the run on by it. The
//! keys are kept in this machine's store, per drone, until the drone is deleted. The cloud form (across machines)
//! comes with the cloud phases.

use serde_json::{json, Value};

use crate::drone::data_flow::ExecutionScope;
use crate::drone::host;
use crate::drone::storage::DroneStore;
use crate::drone::types::FlowNode;
use agentmux_common::time::now_ms;

/// The longest key. A key is a short name for the thing done once (a run id, a commit, an issue number).
const MAX_KEY: usize = 200;

/// The key the block names, with its templates filled in.
fn key_of(node: &FlowNode, scope: &ExecutionScope) -> Result<String, String> {
    let raw = node.data.get("key").and_then(Value::as_str).unwrap_or_default();
    let key = scope.resolve(raw).trim().to_string();
    if key.is_empty() {
        return Err("once block: `key` is empty once filled in, so there is nothing to do once for".to_string());
    }
    // A reference the run cannot fill stays as written; one key for every run would make the first the only one.
    if key.contains("{{") {
        return Err(format!("once block: `key` refers to something this run does not have ({key})"));
    }
    if key.chars().count() > MAX_KEY {
        return Err(format!("once block: `key` is at most {MAX_KEY} characters"));
    }
    Ok(key)
}

pub async fn run(node: &FlowNode, scope: &ExecutionScope) -> Result<Value, String> {
    let key = key_of(node, scope)?;
    if scope.drone_id.is_empty() {
        return Err("once block: a key is kept per saved drone, and this run is not of one".to_string());
    }
    let state = host::state()?;
    let first = state.id_store.drone_once_claim(&scope.drone_id, &key, now_ms()).map_err(|e| format!("once block: {e}"))?;
    Ok(json!({ "result": first, "first": first, "key": key }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(key: &str) -> FlowNode {
        serde_json::from_value(json!({ "id": "o", "position": { "x": 0, "y": 0 }, "data": { "kind": "once", "key": key } })).unwrap()
    }

    #[test]
    fn the_key_is_filled_in_and_must_have_text() {
        let mut scope = ExecutionScope::for_drone("d1", json!({ "run_id": 42 }));
        scope.drone_id = "d1".into();
        assert_eq!(key_of(&node("run-{{trigger.run_id}}"), &scope).unwrap(), "run-42");
        let err = key_of(&node(""), &scope).unwrap_err();
        assert!(err.contains("nothing to do once for"), "{err}");
        let err = key_of(&node("{{trigger.missing}}"), &scope).unwrap_err();
        assert!(err.contains("does not have"), "{err}");
        let long = "x".repeat(MAX_KEY + 1);
        assert!(key_of(&node(&long), &scope).unwrap_err().contains("at most"));
    }

    #[tokio::test]
    async fn the_first_run_with_a_key_is_first_and_a_later_one_is_not_and_another_drone_has_its_own() {
        let state = crate::server::tests::test_state();
        host::install(&state);
        let run = |drone: &str, key: &str| {
            let scope = ExecutionScope::for_drone(drone, Value::Null);
            let n = node(key);
            async move { run(&n, &scope).await.unwrap() }
        };
        assert_eq!(run("d1", "a").await["result"], true);
        assert_eq!(run("d1", "a").await["result"], false, "the same key again");
        assert_eq!(run("d1", "b").await["result"], true, "another key");
        assert_eq!(run("d2", "a").await["result"], true, "another drone's keys are its own");
        state.id_store.drone_delete("d1").unwrap();
        assert_eq!(run("d1", "a").await["result"], true, "deleting a drone forgets its keys");
    }
}
