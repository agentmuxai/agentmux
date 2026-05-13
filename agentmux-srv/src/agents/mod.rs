// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Unified agent runner — single entry point for spawning Claude
//! Code (or any future provider) used by BOTH the interactive agent
//! pane and the headless workflow Agent block.
//!
//! See `docs/specs/SPEC_UNIFIED_AGENT_TYPES_2026_05_13.md` for the
//! full design + 5-PR migration plan.
//!
//! This is Phase 1.5 PR 0: types + skeleton. PR 1 wires the agent
//! pane through this module; PR 2 wires the workflow Agent block.

pub mod runner;
pub mod translator;
pub mod types;

pub use runner::{run_agent, AgentError, AgentRunHandle};
pub use types::{AgentEvent, AgentRef, AgentRunResult, AgentTask, TokenCounts, Turn};
