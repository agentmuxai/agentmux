// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A drone run's event stream. It lives in the core so `backend/mps_events.rs` names it without reaching up into
//! `drone/`; the executor re-exports it under its old path. The wire shape is pinned by
//! `sagas/prompt_events_tests.rs`.

use serde_json::Value;

/// Streaming events emitted to the frontend during a run.
#[derive(Debug, Clone, serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunEvent {
    RunStarted { run_id: String, drone_id: String },
    BlockStarted { run_id: String, block_id: String },
    BlockDone {
        run_id: String,
        block_id: String,
        #[ts(type = "unknown")]
        output: Value,
    },
    BlockError {
        run_id: String,
        block_id: String,
        error: String,
    },
    RunDone {
        run_id: String,
        #[ts(type = "unknown")]
        output: Value,
    },
    RunFailed { run_id: String, error: String },
    /// The run was taken up again after AgentMux restarted, from checkpoint
    /// `step`: the blocks before it are not run again.
    Resumed { run_id: String, step: u32 },
    /// The run's state was kept as checkpoint `step` (0 is the start, before any
    /// block); `block_id` is the block it follows.
    StepCheckpointed {
        run_id: String,
        step: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        block_id: Option<String>,
    },
    /// A loop's guard sent the run round again: `iteration` is the pass that is
    /// starting (the first pass has no event).
    LoopIteration { run_id: String, block_id: String, iteration: u32 },
    /// A limit (a loop's iterations, cost or time, a for-each's items, the run's
    /// dollar cap) stopped something; `block_id` is the block it was met at.
    /// The run may go on (`on_cap: exit_loop`) or end, as the reason says.
    CapReached { run_id: String, block_id: String, reason: String },
    /// The run was stopped on purpose (`cancel_run`): a person, an agent, the
    /// job's timeout or the job's next run. `reason` says which, in plain words.
    RunCancelled { run_id: String, reason: String },
}
