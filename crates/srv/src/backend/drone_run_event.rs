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
}
