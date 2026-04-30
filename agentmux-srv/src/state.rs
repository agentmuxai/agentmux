// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase E.1b — srv reducer state. Mirrors agentmux-launcher's State
// pattern: plain data, mutated only by the pure reducer
// (`crate::reducer::update`). Held inside Arc<Mutex<State>> by the
// pipe IPC server; mutex held only during reducer dispatch
// (sub-millisecond).
//
// What's here:
//   * `LifecyclePhase` (re-exported from agentmux-common::ipc) — E.1b
//   * `ProcessRecord` — pid, kind, state, spawned_at — E.1b
//   * `ProcessState` — Spawning / Running / Exited — E.1b
//   * `WorkspaceRecord` — workspace_id, name — E.2
//   * `State` — top-level: lifecycle + process map + workspaces +
//     monotonic counters
//
// What's intentionally NOT here yet:
//   * Tab / Block / Layout domain state — E.2b+
//   * `persistence_hwm` field — E.2c (lands with the persist
//     subscriber that mirrors pipe-event effects back to SQLite)

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

/// Phase E.2 — workspace as held by the srv reducer's canonical
/// state. Mirrors the persistent `Workspace` struct in
/// `agentmux_srv::backend::obj::Workspace` but with the reducer-
/// canonical fields the cross-process events care about.
///
/// Phase E.2b extends the record with `tab_ids` (ordered) and
/// `active_tab_id`. Tabs themselves live in `state.tabs` keyed by
/// tab_id; the workspace owns the ordering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRecord {
    pub workspace_id: String,
    pub name: String,
    /// Ordered list of tab ids in this workspace. Mirrors the
    /// persistent `Workspace.tabids` field.
    pub tab_ids: Vec<String>,
    /// The active tab in this workspace, if any. `None` when the
    /// workspace has no tabs or has not yet had one selected.
    pub active_tab_id: Option<String>,
}

/// Phase E.2b — tab as held by the srv reducer's canonical state.
/// Tabs are owned by exactly one workspace; the workspace's
/// `tab_ids` field gives the ordering. Block-level state lands in
/// E.3.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabRecord {
    pub tab_id: String,
    pub workspace_id: String,
    pub name: String,
}

#[derive(Debug)]
pub struct State {
    pub lifecycle: LifecyclePhase,
    pub processes: HashMap<u32, ProcessRecord>,
    pub event_version: u64,
    pub next_client_id: u64,
    /// Phase E.2 — workspaces canonical to the srv reducer.
    /// Bootstrapped from SQLite at startup; subsequent transitions
    /// flow through `update`. In E.2 the reducer is a session-only
    /// projection — pipe-originated mutations live only in this
    /// map until the process restarts. E.2c adds the persist
    /// subscriber that mirrors changes back to SQLite (idempotent,
    /// version-gated) and migrates HTTP/WS RPC through the reducer.
    pub workspaces: HashMap<String, WorkspaceRecord>,
    /// Phase E.2b — tabs canonical to the srv reducer. Keyed by
    /// `tab_id`; ordering within a workspace is held in
    /// `WorkspaceRecord.tab_ids`. Bootstrap-loaded from SQLite at
    /// startup alongside workspaces.
    pub tabs: HashMap<String, TabRecord>,
    // `persistence_hwm` deferred to E.2c when the persist subscriber
    // lands and there's actually something to track.
}

impl Default for State {
    fn default() -> Self {
        Self {
            lifecycle: LifecyclePhase::Starting,
            processes: HashMap::new(),
            event_version: 0,
            next_client_id: 0,
            workspaces: HashMap::new(),
            tabs: HashMap::new(),
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
