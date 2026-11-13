// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Transform block: shapes a list or an object with a chain of operations in a
//! restricted expression language, and runs no code
//! (docs/specs/SPEC_DRONE_GENERAL_AUTOMATION_2026_11_11.md §3.9; the evaluator
//! is `drone/transform.rs`).
//!
//! `data.input` is a template for what to shape: one `{{...}}` token gives the
//! value itself (a list, an object), anything else gives text. `data.ops` is the
//! chain, such as `[{"op": "filter", "expr": "item.state == 'open'"}, {"op":
//! "count"}]`. The output is `{ "value": <result> }`, and its taint bits follow
//! the chain: tainted in, tainted out, except a `count`, `sum`, `min`, `max` or
//! `exists`, which cannot carry text and come out derived.

use serde_json::{json, Value};

use crate::drone::data_flow::ExecutionScope;
use crate::drone::transform::{evaluate, Transform, TransformOp};
use crate::drone::types::FlowNode;
use crate::drone::value::Tv;

pub async fn run(node: &FlowNode, scope: &mut ExecutionScope) -> Result<Value, String> {
    let template = node.data.get("input").and_then(Value::as_str).ok_or_else(|| "Transform block missing `input`".to_string())?;
    let ops: Vec<TransformOp> = serde_json::from_value(node.data.get("ops").cloned().unwrap_or_else(|| json!([])))
        .map_err(|e| format!("transform: `ops` is not a list of operations: {e}"))?;
    let (tainted, derived) = scope.bits_of(template);
    let input = Tv { v: scope.value_of(template), tainted, derived };
    let out = evaluate(&Transform { ops }, &input).map_err(|e| e.to_string())?;
    scope.bits.insert(node.id.clone(), (out.tainted, out.derived));
    Ok(json!({ "value": out.v }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drone::types::NodePosition;

    fn node(data: Value) -> FlowNode {
        FlowNode { id: "t".to_string(), position: NodePosition::default(), data, node_type: None }
    }

    fn scope() -> ExecutionScope {
        let mut scope = ExecutionScope::with_trigger(json!({ "prs": [
            { "n": 1, "state": "open", "author": "a" },
            { "n": 2, "state": "closed", "author": "b" },
            { "n": 3, "state": "open", "author": "c" },
        ]}));
        scope.outputs.insert("api".to_string(), json!({ "body": { "items": [3, 1, 2] } }));
        scope
    }

    #[tokio::test]
    async fn shapes_the_list_a_template_names() {
        let mut s = scope();
        let n = node(json!({ "kind": "transform", "input": "{{trigger.prs}}", "ops": [
            { "op": "filter", "expr": "item.state == 'open'" },
            { "op": "map", "expr": "item.n" },
        ]}));
        assert_eq!(run(&n, &mut s).await.unwrap(), json!({ "value": [1, 3] }));
        let sorted = node(json!({ "kind": "transform", "input": "{{api.body.items}}", "ops": [{ "op": "sort", "by": "item" }] }));
        assert_eq!(run(&sorted, &mut s).await.unwrap(), json!({ "value": [1, 2, 3] }));
    }

    #[tokio::test]
    async fn taint_follows_the_chain_and_a_count_is_derived() {
        let mut s = scope();
        let list = node(json!({ "kind": "transform", "input": "{{trigger.prs}}", "ops": [{ "op": "pick", "fields": ["n"] }] }));
        run(&list, &mut s).await.unwrap();
        assert_eq!(s.bits["t"], (true, false), "a list from the trigger is tainted, and so is what is picked from it");
        let count = node(json!({ "kind": "transform", "input": "{{trigger.prs}}", "ops": [{ "op": "count" }] }));
        assert_eq!(run(&count, &mut s).await.unwrap(), json!({ "value": 3 }));
        assert_eq!(s.bits["t"], (false, true), "a count of a tainted list is a number made in the run");
        let clean = node(json!({ "kind": "transform", "input": "{{api.body.items}}", "ops": [{ "op": "count" }] }));
        run(&clean, &mut s).await.unwrap();
        assert_eq!(s.bits["t"], (false, false), "nothing tainted was read");
    }

    #[tokio::test]
    async fn a_bad_chain_or_a_missing_input_fails_the_block_naming_the_step() {
        let mut s = scope();
        let no_input = node(json!({ "kind": "transform", "ops": [] }));
        assert!(run(&no_input, &mut s).await.unwrap_err().contains("missing `input`"));
        let bad_op = node(json!({ "kind": "transform", "input": "{{trigger.prs}}", "ops": [{ "op": "launch" }] }));
        assert!(run(&bad_op, &mut s).await.unwrap_err().contains("not a list of operations"));
        let bad_expr = node(json!({ "kind": "transform", "input": "{{trigger.prs}}", "ops": [{ "op": "map", "expr": "item +" }] }));
        assert!(run(&bad_expr, &mut s).await.unwrap_err().contains("step 0"));
    }
}
