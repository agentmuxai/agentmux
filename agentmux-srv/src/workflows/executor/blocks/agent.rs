// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Agent block — one-shot Claude Code spawn driven by an `AgentRef`
//! and a per-call task template.
//!
//! Phase 1.5 PR 2: replaces the original stub
//! (`{ response: "[stub]" }`) with a real invocation of
//! `agents::runner::run_agent`. The runner spawns
//! `claude --print --output-format=stream-json`, drains its stdout
//! through `ClaudeTranslator`, and produces a structured
//! `AgentRunResult` that this function flattens into the
//! snake_case workflow-block output shape.
//!
//! Block config (`node.data`):
//!   * `task`         — required. Mustache-style template resolved
//!                      against `scope.outputs + scope.vars` before
//!                      spawning.
//!   * `agent_ref`    — optional. Object matching `AgentRef` shape
//!                      (camelCase keys): identityId, memoryId,
//!                      instanceName, workingDirectory. All fields
//!                      optional; missing = blank agent.
//!   * `max_turns`    — optional. Hard cap on claude turns.
//!
//! Output (snake_case to match other workflow blocks — see spec
//! §4.5):
//!
//!   ```json
//!   {
//!     "response": "<text>",
//!     "tokens": { "input": .., "output": .., "cache_creation": .., "cache_read": .. },
//!     "cost_usd": 0.001,
//!     "status": "done"
//!   }
//!   ```
//!
//! Downstream blocks read `{{<this_block_id>.response}}` for the
//! agent's reply and `{{<this_block_id>.cost_usd}}` for accounting.

use serde_json::{json, Value};

use crate::agents::runner::{run_agent, AgentError};
use crate::agents::types::{AgentRef, AgentTask};
use crate::workflows::data_flow::ExecutionScope;
use crate::workflows::types::FlowNode;

pub async fn run(node: &FlowNode, scope: &ExecutionScope) -> Result<Value, String> {
    let task_raw = node
        .data
        .get("task")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "agent block missing `task`".to_string())?;
    let prompt = scope.resolve(task_raw);

    let agent_ref: AgentRef = match node.data.get("agent_ref") {
        Some(v) => serde_json::from_value(v.clone())
            .map_err(|e| format!("agent block: invalid agent_ref: {e}"))?,
        None => AgentRef::default(),
    };

    let max_turns = node
        .data
        .get("max_turns")
        .and_then(|v| v.as_u64())
        .map(|n| n as u32);

    let task = AgentTask {
        prompt,
        // The runner doesn't currently use the `context` map (claude
        // takes the prompt on argv); leave it empty. Phase 2 may
        // surface scope vars as a `system` message section.
        context: serde_json::Map::new(),
        max_turns,
    };

    // Forward AgentEvents into a local channel and discard for now.
    // Phase 1.5 PR 3 will re-emit them on the `workflowrun:<id>`
    // broker so the inspector pane can render the live stream.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let handle = run_agent(agent_ref, task, tx).await.map_err(|e| match e {
        AgentError::Spawn(msg) => format!("agent block: spawn failed: {msg}"),
        AgentError::InvalidRef(msg) => format!("agent block: invalid agent ref: {msg}"),
    })?;

    // Drain the event channel concurrently so the runner's sender
    // can make progress. Drop the events for now — the captured
    // accumulator from `final_result` is the authoritative output.
    tokio::spawn(async move {
        while rx.recv().await.is_some() {}
    });

    let result = handle
        .final_result
        .await
        .map_err(|e| format!("agent block: runner cancelled: {e}"))?
        .map_err(|e| format!("agent block: agent run failed: {e}"))?;

    // Manually flatten to snake_case to match other workflow block
    // outputs (the AgentRunResult's serde camelCase is for the IPC
    // seam with the frontend, NOT for workflow templates — see spec
    // §4.5 NOTE).
    Ok(json!({
        "response": result.response,
        "tokens": {
            "input": result.tokens.input,
            "output": result.tokens.output,
            "cache_creation": result.tokens.cache_creation,
            "cache_read": result.tokens.cache_read,
        },
        "cost_usd": result.cost_usd,
        "status": "done",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflows::types::NodePosition;

    fn mk_node(data: Value) -> FlowNode {
        FlowNode {
            id: "a1".to_string(),
            position: NodePosition::default(),
            data,
            node_type: String::new(),
        }
    }

    #[tokio::test]
    async fn rejects_missing_task() {
        let node = mk_node(json!({ "kind": "agent" }));
        let scope = ExecutionScope::new();
        let err = run(&node, &scope).await.expect_err("must error");
        assert!(err.contains("missing `task`"), "got: {err}");
    }

    #[tokio::test]
    async fn rejects_malformed_agent_ref() {
        let node = mk_node(json!({
            "kind": "agent",
            "task": "hi",
            "agent_ref": "not an object"
        }));
        let scope = ExecutionScope::new();
        let err = run(&node, &scope).await.expect_err("must error");
        assert!(err.contains("invalid agent_ref"), "got: {err}");
    }

    #[tokio::test]
    async fn surfaces_spawn_failure() {
        // Force a known-nonexistent claude binary so the spawn fails
        // deterministically. This verifies the AgentError → block
        // error path without needing claude on PATH.
        let _guard = EnvGuard::set(
            "AGENTMUX_CLAUDE_BIN",
            "/definitely/does/not/exist/claude-xyz-test",
        );

        let node = mk_node(json!({
            "kind": "agent",
            "task": "anything"
        }));
        let scope = ExecutionScope::new();
        let err = run(&node, &scope).await.expect_err("must error");
        assert!(
            err.contains("spawn failed") || err.contains("agent block: spawn"),
            "got: {err}"
        );
    }

    /// RAII helper to set + restore an env var, since tests run in the
    /// same process and we don't want to leak state to siblings.
    struct EnvGuard {
        key: String,
        prior: Option<String>,
    }
    impl EnvGuard {
        fn set(key: &str, value: &str) -> Self {
            let prior = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self {
                key: key.to_string(),
                prior,
            }
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.prior {
                Some(v) => std::env::set_var(&self.key, v),
                None => std::env::remove_var(&self.key),
            }
        }
    }
}
