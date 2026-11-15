// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The Drone block: calls another drone as a step
//! (docs/specs/SPEC_DRONE_GENERAL_AUTOMATION_2026_11_11.md §3.5).
//!
//! Block config (`node.data`):
//!   * `drone`    required, the id of the drone to call. Written in the graph, never a
//!                `{{reference}}`: a value from outside must not pick what runs.
//!   * `version`  optional, the saved version to call. Without it the drone as it is saved now.
//!   * `inputs`   optional, `{ "name": "template" }`: what the callee reads as `{{trigger.name}}`.
//!
//! The scheduler does the calling (`scheduler/call.rs`), because the callee runs on
//! the caller's remaining budget and under its cancel.

use serde_json::Value;

use crate::backend::storage::drone_types::FlowNode;

/// What the block calls.
#[derive(Debug, Clone, PartialEq)]
pub struct CallSpec {
    pub drone: String,
    pub version: Option<i64>,
    /// Each input as the graph wrote it, to be filled in when the call is made.
    pub inputs: Vec<(String, String)>,
}

/// The block's settings, or what is wrong with them.
pub fn parse(node: &FlowNode) -> Result<CallSpec, String> {
    let id = &node.id;
    let drone = node.data.get("drone").and_then(Value::as_str).map(str::trim).unwrap_or("");
    if drone.is_empty() {
        return Err(format!("{id}: a Drone block needs `drone`, the id of the drone it calls"));
    }
    if drone.contains("{{") {
        return Err(format!("{id}: `drone` is written in the graph and cannot be a {{{{reference}}}}: a value from outside must not pick what runs"));
    }
    let version = match node.data.get("version") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_i64().filter(|n| *n > 0).ok_or_else(|| format!("{id}: `version` must be a saved version number, 1 or more"))?),
    };
    let inputs = match node.data.get("inputs") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Object(map)) => map
            .iter()
            .map(|(k, v)| match v {
                Value::String(t) => Ok((k.clone(), t.clone())),
                other => Ok((k.clone(), other.to_string())),
            })
            .collect::<Result<Vec<_>, String>>()?,
        Some(_) => return Err(format!("{id}: `inputs` must be an object of names and values")),
    };
    let mut inputs = inputs;
    inputs.sort();
    Ok(CallSpec { drone: drone.to_string(), version, inputs })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drone::types::NodePosition;
    use serde_json::json;

    fn call_node(extra: Value) -> FlowNode {
        let mut data = json!({ "kind": "drone" });
        data.as_object_mut().unwrap().extend(extra.as_object().cloned().unwrap_or_default());
        FlowNode { id: "c".to_string(), position: NodePosition::default(), data, node_type: None }
    }

    #[test]
    fn a_call_names_its_drone_and_may_pin_a_version_and_give_inputs() {
        let spec = parse(&call_node(json!({ "drone": " other ", "version": 3, "inputs": { "n": "{{var.x}}", "k": 5 } }))).unwrap();
        assert_eq!(spec.drone, "other");
        assert_eq!(spec.version, Some(3));
        assert_eq!(spec.inputs, [("k".to_string(), "5".to_string()), ("n".to_string(), "{{var.x}}".to_string())]);
        assert_eq!(parse(&call_node(json!({ "drone": "d" }))).unwrap(), CallSpec { drone: "d".into(), version: None, inputs: vec![] });
    }

    #[test]
    fn a_call_that_is_set_wrongly_says_what_to_fix() {
        assert!(parse(&call_node(json!({}))).unwrap_err().contains("needs `drone`"));
        assert!(parse(&call_node(json!({ "drone": "{{trigger.d}}" }))).unwrap_err().contains("cannot be a"));
        assert!(parse(&call_node(json!({ "drone": "d", "version": 0 }))).unwrap_err().contains("`version` must be"));
        assert!(parse(&call_node(json!({ "drone": "d", "version": "latest" }))).unwrap_err().contains("`version` must be"));
        assert!(parse(&call_node(json!({ "drone": "d", "inputs": [1] }))).unwrap_err().contains("`inputs` must be an object"));
    }
}
