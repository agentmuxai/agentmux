// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Agent block — references a Forge agent definition + a per-call
//! task prompt. RFC #753 §2 Q4: Agent block is a *reference* to a
//! Forge agent (`forge_agent_id`), not its own definition.
//!
//! Phase 1 placeholder: the executor does not yet spawn live Forge
//! subprocesses inside a workflow run (that's Phase 1 PR-3 polish —
//! out of scope for this MVP commit). Instead the block returns a
//! synthetic `{ response, status: "stub" }` echoing the resolved task.
//! This keeps the canvas and DAG end-to-end demoable while the real
//! Forge integration lands.
//!
//! TODO(#753 follow-up): replace stub with `RpcApi::AgentInputCommand`
//! against a fresh AgentInstance row scoped to this run.

use serde_json::{json, Value};

use crate::workflows::data_flow::ExecutionScope;
use crate::workflows::types::FlowNode;

pub async fn run(node: &FlowNode, scope: &ExecutionScope) -> Result<Value, String> {
    let task_raw = node
        .data
        .get("task")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let task = scope.resolve(task_raw);
    let forge_agent_id = node
        .data
        .get("forge_agent_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    Ok(json!({
        "response": format!("[stub agent={}] task={}", forge_agent_id, task),
        "status": "stub",
        "tokens": { "in": 0, "out": 0 },
    }))
}
