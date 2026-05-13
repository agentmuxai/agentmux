// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Workflows pane backend — issue #753 Phase 1.
//!
//! Native DAG-of-blocks workflow engine modeled after
//! [Sim](https://github.com/simstudioai/sim). Frontend lives in
//! `frontend/app/view/workflows/`. The canvas uses `@dschz/solid-flow`
//! (xyflow core, SolidJS port).
//!
//! Phase 1 ships 5 block types: Agent, Condition, API, Response,
//! Variables. The Function block (`quickjs-rs` sandbox) is Phase 2.
//!
//! Architecture:
//!   * `types.rs`        — WorkflowDefinition, WorkflowRun, FlowNode, FlowEdge.
//!   * `storage.rs`      — wstore CRUD over db_workflow_definitions
//!                         + db_workflow_runs.
//!   * `executor/`       — DAG topological sort + per-layer concurrent runner.
//!   * `executor/blocks` — one file per block type.
//!   * `data_flow.rs`    — `{{var}}` interpolation (Mustache-style; see RFC §2 Q5).

pub mod data_flow;
pub mod executor;
pub mod storage;
pub mod types;

pub use types::{
    BlockKind, BlockState, FlowEdge, FlowNode, RunStatus, WorkflowDefinition, WorkflowRun,
};
