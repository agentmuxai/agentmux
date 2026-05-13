// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Variables block — declares workflow-scope vars. The block's
//! `data.entries` is a list of `{name, value}` pairs; each value is
//! `{{}}`-resolved against the current scope and written as the var.
//!
//! Output: `{ "vars": { name: value, ... } }` so downstream blocks can
//! read them via `{{var.name}}` OR `{{<this_block_id>.vars.name}}`.

use serde_json::{json, Value};

use crate::workflows::data_flow::ExecutionScope;
use crate::workflows::types::FlowNode;

pub async fn run(node: &FlowNode, scope: &mut ExecutionScope) -> Result<Value, String> {
    let entries = node
        .data
        .get("entries")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut written = serde_json::Map::new();
    for e in entries {
        let name = e
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "variables entry missing `name`".to_string())?;
        let raw = e.get("value").cloned().unwrap_or(Value::Null);
        let resolved = match raw {
            Value::String(s) => Value::String(scope.resolve(&s)),
            other => other,
        };
        scope.vars.insert(name.to_string(), resolved.clone());
        written.insert(name.to_string(), resolved);
    }
    Ok(json!({ "vars": written }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflows::types::NodePosition;

    #[tokio::test]
    async fn writes_string_var_with_resolution() {
        let node = FlowNode {
            id: "v1".to_string(),
            position: NodePosition::default(),
            data: json!({
                "kind": "variables",
                "entries": [
                    { "name": "greeting", "value": "hi" }
                ]
            }),
            node_type: String::new(),
        };
        let mut scope = ExecutionScope::new();
        let out = run(&node, &mut scope).await.unwrap();
        assert_eq!(scope.vars.get("greeting").unwrap(), &json!("hi"));
        assert_eq!(out["vars"]["greeting"], json!("hi"));
    }
}
