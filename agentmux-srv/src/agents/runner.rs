// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Unified agent runner. Phase 1.5 PR 0 ships the SKELETON only:
//! types are wired, the `run_agent` entry point exists, and a
//! handle type is defined — but the actual spawn / drain pipeline
//! lands in PR 1 (refactor of `blockcontroller/shell.rs`) and PR 2
//! (workflow Agent block wiring).
//!
//! Until then, `run_agent` returns a `NotImplemented` error so any
//! premature caller fails loudly rather than silently producing
//! empty output.
//!
//! See `docs/specs/SPEC_UNIFIED_AGENT_TYPES_2026_05_13.md` §4.2.

use tokio::sync::{mpsc, oneshot};

use super::types::{AgentEvent, AgentRef, AgentRunResult, AgentTask};

/// Handle returned by `run_agent`. Callers drain `events` for the
/// streaming side (UI render or per-event broker publish) and await
/// `final_result` for the structured terminal value (workflow Agent
/// block's downstream output).
///
/// Closing the `events` receiver implicitly cancels the run only if
/// the runner observes it — Phase 2 adds an explicit AbortHandle.
pub struct AgentRunHandle {
    /// `db_agent_instances.id` of the row backing this run. Empty
    /// string until the runner allocates one (PR 1).
    pub instance_id: String,
    pub final_result: oneshot::Receiver<Result<AgentRunResult, String>>,
}

/// Error returned by the runner.
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("agent runner: not yet implemented — Phase 1.5 PR 1 wires the spawn pipeline")]
    NotImplemented,
    #[error("agent runner: invalid AgentRef: {0}")]
    InvalidRef(String),
    #[error("agent runner: spawn failed: {0}")]
    Spawn(String),
}

/// Spawn an agent subprocess per the `agent_ref`, drain its output,
/// translate frames into `AgentEvent`s, broadcast each on `tx`, and
/// (when the run terminates) resolve the returned handle's
/// `final_result` with an `AgentRunResult`.
///
/// PR 0 returns `NotImplemented` — wiring lands in PR 1 (agent
/// pane refactor) and PR 2 (workflow Agent block).
pub async fn run_agent(
    _agent_ref: AgentRef,
    _task: AgentTask,
    _tx: mpsc::UnboundedSender<AgentEvent>,
) -> Result<AgentRunHandle, AgentError> {
    Err(AgentError::NotImplemented)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn run_agent_returns_not_implemented_in_pr_0() {
        // Regression guard — PR 1 replaces this with a real spawn.
        // Until then, any caller (including the workflow Agent block
        // stub) should get a clear error instead of silent empty
        // output.
        let (tx, _rx) = mpsc::unbounded_channel();
        let result = run_agent(AgentRef::default(), simple_task(), tx).await;
        match result {
            Err(AgentError::NotImplemented) => {}
            Err(other) => panic!("expected NotImplemented, got: {other}"),
            Ok(_) => panic!("expected NotImplemented error, got Ok(handle)"),
        }
    }

    fn simple_task() -> AgentTask {
        AgentTask {
            prompt: "hi".to_string(),
            context: serde_json::Map::new(),
            max_turns: None,
        }
    }
}
