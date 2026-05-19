// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Drone pane backend — issue #753 Phase 1.
//!
//! Native DAG-of-blocks drone engine modeled after
//! [Sim](https://github.com/simstudioai/sim). Frontend lives in
//! `frontend/app/view/drone/`. The canvas uses `@dschz/solid-flow`
//! (xyflow core, SolidJS port).
//!
//! Phase 1 ships 5 block types: Agent, Condition, API, Response,
//! Variables. The Function block (`quickjs-rs` sandbox) is Phase 2.
//!
//! Architecture:
//!   * `types.rs`        — DroneDefinition, DroneRun, FlowNode, FlowEdge.
//!   * `storage.rs`      — wstore CRUD over db_drone_definitions
//!                         + db_drone_runs.
//!   * `executor/`       — DAG topological sort + per-layer concurrent runner.
//!   * `executor/blocks` — one file per block type.
//!   * `data_flow.rs`    — `{{var}}` interpolation (Mustache-style; see RFC §2 Q5).

pub mod data_flow;
pub mod executor;
pub mod storage;
pub mod types;

pub use types::{
    BlockKind, BlockState, FlowEdge, FlowNode, RunStatus, DroneDefinition, DroneRun,
};
