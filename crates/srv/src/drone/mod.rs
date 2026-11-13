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
//!   * `storage.rs`      — mstore CRUD over db_drone_definitions
//!                         + db_drone_runs.
//!   * `executor/`       — DAG topological sort + per-layer concurrent runner.
//!   * `executor/blocks` — one file per block type.
//!   * `data_flow.rs`    — `{{var}}` interpolation (Mustache-style; see RFC §2 Q5).

pub mod checkpoint;
pub mod data_flow;
pub mod executor;
pub(crate) mod host;
pub mod state;
pub mod storage;
pub mod transform;
pub mod types;
pub mod value;
pub mod versions;

// The engine's modules A, B and C (docs/specs/SPEC_DRONE_GENERAL_AUTOMATION_2026_11_11.md
// section 5, E1): their bodies are written by their own PRs, and their acceptance
// tests are `ignore`d until then (see the head of each file).
#[cfg(test)]
mod state_acceptance_tests;
#[cfg(test)]
mod transform_acceptance_tests;
#[cfg(test)]
mod versions_acceptance_tests;
