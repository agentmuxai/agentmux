// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase E.1b — srv reducer state. Mirrors agentmux-launcher's State
// pattern: plain data, mutated only by the pure reducer
// (`crate::reducer::update`). Held inside Arc<Mutex<State>> by the
// pipe IPC server; mutex held only during reducer dispatch
// (sub-millisecond).
//
// What's here in E.1b:
//   * `LifecyclePhase` (re-exported from agentmux-common::ipc)
//   * `ProcessRecord` — pid, kind, state, spawned_at
//   * `ProcessState` — Spawning / Running / Exited
//   * `State` — top-level: lifecycle + process map + monotonic counters
//
// What's intentionally NOT here yet:
//   * Domain state (workspaces, tabs, blocks, layouts) — E.2+
//   * SQLite-bootstrap path / persistence HWM — E.2 (when there's
//     domain state to persist)

use std::collections::HashMap;

use agentmux_common::ipc::ClientKind;
pub use agentmux_common::ipc::LifecyclePhase;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    Spawning,
    Running,
    Exited { code: i32 },
}

#[derive(Debug, Clone)]
pub struct ProcessRecord {
    pub pid: u32,
    pub kind: ClientKind,
    pub state: ProcessState,
    pub spawned_at: String,
    pub version: String,
}

#[derive(Debug)]
pub struct State {
    pub lifecycle: LifecyclePhase,
    pub processes: HashMap<u32, ProcessRecord>,
    pub event_version: u64,
    pub next_client_id: u64,
}

impl Default for State {
    fn default() -> Self {
        Self {
            lifecycle: LifecyclePhase::Starting,
            processes: HashMap::new(),
            event_version: 0,
            next_client_id: 0,
        }
    }
}

impl State {
    /// Increment + return the monotonic event-version counter.
    /// Every emitted Event carries a version; subscribers detect
    /// gaps for resync (Phase D.3).
    pub fn bump_version(&mut self) -> u64 {
        self.event_version = self.event_version.saturating_add(1);
        self.event_version
    }

    /// Allocate a fresh client_id for a new Register reply.
    pub fn alloc_client_id(&mut self) -> u64 {
        self.next_client_id = self.next_client_id.saturating_add(1);
        self.next_client_id
    }
}
