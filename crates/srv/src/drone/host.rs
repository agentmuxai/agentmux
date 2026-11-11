// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The server a running drone's blocks reach AgentMux through.
//!
//! A block is `run(node, &scope)` and has no server handle in its signature
//! (the engine's rule: Drone phase 2, docs/specs/SPEC_DRONE_REAL_WORLD_JOBS_2026_11_11.md §4.2).
//! The blocks that act on AgentMux itself (App query, Message, Task, Spawn, Notify) read it here.
//! Set once at startup, the way `server::job_actions::install` and `server::jekt_cold::install` are.

use std::sync::OnceLock;

use crate::server::AppState;

static HOST: OnceLock<AppState> = OnceLock::new();

/// Give the drone blocks the server. Later calls change nothing.
pub(crate) fn install(state: &AppState) {
    let _ = HOST.set(state.clone());
}

/// The server, or why a block can't act yet.
pub(crate) fn state() -> Result<&'static AppState, String> {
    HOST.get().ok_or_else(|| "AgentMux isn't ready for this block yet".to_string())
}
