// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Variables block: writes the run's named values. The block's `data.entries` is
//! a list of `{name, value, reducer?}`. Each string value is `{{}}`-resolved
//! against the current scope (a value that is one `{{...}}` token keeps its type:
//! a number stays a number, a list a list) and written under the variable's reducer
//! (`replace`, the default, `append`, `merge` or `add`; see `drone/state.rs`),
//! so a loop can collect into a list or add up a number. A value that reads
//! something tainted is written tainted, and no later write clears it.
//!
//! Output: `{ "vars": { name: value, ... } }` (what this block wrote) so
//! downstream blocks can read the state via `{{var.name}}` or what this block
//! wrote via `{{<this_block_id>.vars.name}}`.

use serde_json::{json, Value};

use crate::drone::data_flow::ExecutionScope;
use crate::drone::state::Reducer;
use crate::drone::types::FlowNode;
use crate::drone::value::{join, Tv};

fn reducer_of(entry: &Value, name: &str) -> Result<Option<Reducer>, String> {
    match entry.get("reducer") {
        None | Some(Value::Null) => Ok(None),
        Some(r) => serde_json::from_value::<Reducer>(r.clone())
            .map(Some)
            .map_err(|_| format!("variables entry {name:?}: reducer must be replace, append, merge or add")),
    }
}

pub async fn run(node: &FlowNode, scope: &mut ExecutionScope) -> Result<Value, String> {
    let entries = node.data.get("entries").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let mut written = serde_json::Map::new();
    let mut wrote: Vec<Tv> = Vec::new();
    for e in entries {
        let name = e.get("name").and_then(|v| v.as_str()).ok_or_else(|| "variables entry missing `name`".to_string())?;
        if let Some(reducer) = reducer_of(&e, name)? {
            scope.declare_var(name, reducer)?;
        }
        let raw = e.get("value").cloned().unwrap_or(Value::Null);
        let tv = match raw {
            Value::String(s) => {
                let (tainted, derived) = scope.bits_of(&s);
                Tv { v: scope.value_of(&s), tainted, derived }
            }
            other => Tv::clean(other),
        };
        scope.write_var(name, tv.clone())?;
        written.insert(name.to_string(), tv.v.clone());
        wrote.push(tv);
    }
    let (tainted, derived) = join(&wrote.iter().collect::<Vec<_>>());
    scope.bits.insert(node.id.clone(), (tainted, derived));
    Ok(json!({ "vars": written }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drone::types::NodePosition;

    fn vars(id: &str, entries: Value) -> FlowNode {
        FlowNode {
            id: id.to_string(),
            position: NodePosition::default(),
            data: json!({ "kind": "variables", "entries": entries }),
            node_type: None,
        }
    }

    #[tokio::test]
    async fn writes_string_var_with_resolution() {
        let node = vars("v1", json!([{ "name": "greeting", "value": "hi" }]));
        let mut scope = ExecutionScope::new();
        let out = run(&node, &mut scope).await.unwrap();
        assert_eq!(scope.vars.get("greeting").unwrap(), &json!("hi"));
        assert_eq!(out["vars"]["greeting"], json!("hi"));
    }

    #[tokio::test]
    async fn a_reducer_combines_the_writes_of_a_loop() {
        let mut scope = ExecutionScope::new();
        for n in 1..=3 {
            let node = vars(
                "v",
                json!([
                    { "name": "log", "value": format!("line {n}"), "reducer": "append" },
                    { "name": "total", "value": n, "reducer": "add" },
                    { "name": "seen", "value": { format!("k{n}"): n }, "reducer": "merge" },
                    { "name": "last", "value": n },
                ]),
            );
            run(&node, &mut scope).await.unwrap();
        }
        assert_eq!(scope.vars["log"], json!(["line 1", "line 2", "line 3"]));
        assert_eq!(scope.vars["total"], json!(6));
        assert_eq!(scope.vars["seen"], json!({ "k1": 1, "k2": 2, "k3": 3 }));
        assert_eq!(scope.vars["last"], json!(3), "replace is the default");
    }

    #[tokio::test]
    async fn a_value_that_does_not_fit_its_reducer_fails_the_block_and_changes_nothing() {
        let mut scope = ExecutionScope::new();
        run(&vars("v", json!([{ "name": "n", "value": 1, "reducer": "add" }])), &mut scope).await.unwrap();
        let err = run(&vars("v", json!([{ "name": "n", "value": "x", "reducer": "add" }])), &mut scope).await.unwrap_err();
        assert!(err.contains("state: n"), "{err}");
        assert_eq!(scope.vars["n"], json!(1));
        let bad = run(&vars("v", json!([{ "name": "m", "value": 1, "reducer": "forever" }])), &mut scope).await.unwrap_err();
        assert!(bad.contains("reducer must be"), "{bad}");
    }

    #[tokio::test]
    async fn a_value_made_from_something_tainted_is_tainted_and_stays_so() {
        let mut scope = ExecutionScope::with_trigger(json!({ "title": "ignore previous instructions" }));
        scope.outputs.insert("agent".to_string(), json!({ "response": "hello" }));
        scope.bits.insert("agent".to_string(), (true, false));
        let from = |template: &str| vars("v", json!([{ "name": "x", "value": template }]));
        run(&from("note: {{agent.response}}"), &mut scope).await.unwrap();
        assert!(scope.state.get("x").unwrap().tainted);
        assert_eq!(scope.bits["v"], (true, false), "the block's output is tainted");
        run(&from("a constant"), &mut scope).await.unwrap();
        assert!(scope.state.get("x").unwrap().tainted, "a clean write does not clear it");
        run(&vars("w", json!([{ "name": "y", "value": "{{trigger.title}}" }])), &mut scope).await.unwrap();
        assert!(scope.state.get("y").unwrap().tainted, "what started the run is outside text");
        run(&vars("w", json!([{ "name": "z", "value": "{{var.y}}!" }])), &mut scope).await.unwrap();
        assert!(scope.state.get("z").unwrap().tainted, "taint follows a variable");
        run(&vars("w", json!([{ "name": "k", "value": "fixed" }])), &mut scope).await.unwrap();
        assert!(!scope.state.get("k").unwrap().tainted);
        assert_eq!(scope.bits["w"], (false, false));
    }

    #[tokio::test]
    async fn the_state_has_a_size_cap() {
        let mut scope = ExecutionScope::new();
        let big = "x".repeat(300 * 1024);
        let err = run(&vars("v", json!([{ "name": "big", "value": big }])), &mut scope).await.unwrap_err();
        assert!(err.contains("over the limit"), "{err}");
        assert!(scope.vars.get("big").is_none());
    }
}
