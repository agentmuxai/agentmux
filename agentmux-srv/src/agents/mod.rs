// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Unified agent runner — single entry point for spawning Claude
//! Code (or any future provider) used by BOTH the interactive agent
//! pane and the headless drone Agent block.
//!
//! The full design + 5-PR migration plan lived in a Phase-1.5 lineage spec
//! that was retired in #1928 ("remove legacy pre-Drone Workflows lineage
//! specs"); this module is what shipped from it.
//!
//! This is Phase 1.5 PR 0: types + skeleton. PR 1 wires the agent
//! pane through this module; PR 2 wires the drone Agent block.

pub mod failure;
pub mod runner;
pub mod translator;
pub mod types;

pub use runner::{run_agent, AgentError, AgentRunHandle};
pub use types::{AgentEvent, AgentRef, AgentRunResult, AgentTask, TokenCounts, AgentTurn};
