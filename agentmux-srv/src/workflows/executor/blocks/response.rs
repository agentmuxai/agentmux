// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Response block — terminal output sink. Captures whatever the
//! `template` field resolves to and that becomes the workflow's run
//! `output`. There must be exactly one Response block per workflow
//! (validator enforced at save-time, frontend side).
//!
//! Output: `{ "value": <resolved-template> }` — the engine reads
//! `value` and stores it as the run-record `output` field.

use serde_json::{json, Value};

use crate::workflows::data_flow::ExecutionScope;
use crate::workflows::types::FlowNode;

pub async fn run(node: &FlowNode, scope: &ExecutionScope) -> Result<Value, String> {
    let template = node
        .data
        .get("template")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let value = scope.resolve(template);
    Ok(json!({ "value": value }))
}
