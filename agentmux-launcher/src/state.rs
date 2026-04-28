// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase B.3 launcher state. Held inside Arc<Mutex<...>> by the IPC
// server; mutated only by the pure reducer (`crate::reducer::update`).
// Mutex is held for microseconds at a time — never across an .await
// boundary, never across I/O. Mirrors the Elm/Redux pattern: state
// is data, transitions are functions, side effects fire after the
// state mutation commits.
//
// What's here in B.3:
//   * `LifecyclePhase` (re-exported from agentmux-common::ipc so the
//     wire and internal types are the same enum)
//   * `ProcessRecord` — pid, kind, state, spawned_at
//   * `ProcessState` — Spawning / Running / Exited
//   * `State` — top-level container: lifecycle + process map +
//     monotonic version counter + monotonic client_id counter
//
// What's intentionally NOT here yet:
//   * Window state machine (B.4–B.5)
//   * Warm-pool (B.5)
//   * Event log ring buffer (B.4 — added when events first start
//     accumulating beyond handshake replies)

use std::collections::HashMap;

use agentmux_common::ipc::ClientKind;
pub use agentmux_common::ipc::LifecyclePhase;

/// Lifecycle of a single process the launcher knows about. The
/// reducer transitions through these in order — there's no skipping
/// (Spawning → Running → Exited).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    /// Spawn issued; child handle returned but process hasn't
    /// confirmed it's alive yet. In B.3 we transition straight to
    /// Running on Register because that's the first authoritative
    /// signal. B.4+ adds intermediate state for "spawned but not
    /// yet registered."
    Spawning,
    /// Process has registered with the launcher and is doing its
    /// work. Healthy.
    Running,
    /// Process exited (clean Goodbye → code=0, crash → non-zero).
    Exited { code: i32 },
}

/// One process in the launcher's canonical view. Updated by the
/// reducer; read by IPC handlers + the eventual `--diag` printer.
#[derive(Debug, Clone)]
pub struct ProcessRecord {
    pub pid: u32,
    pub kind: ClientKind,
    pub state: ProcessState,
    /// RFC3339 timestamp of the spawn (or first-register, whichever
    /// the launcher learned about first).
    pub spawned_at: String,
    /// Free-form version string of the registered binary. For log
    /// correlation across version skew during a Phase B rollout.
    pub version: String,
}

/// Top-level launcher state. Single Arc<Mutex<State>> owned by the
/// IPC server; passed into `update(state, cmd, conn)` for every
/// incoming command.
#[derive(Debug)]
pub struct State {
    pub lifecycle: LifecyclePhase,
    /// Keyed by PID. Multiple records per PID would be a bug — the
    /// reducer enforces unique-pid on insert.
    pub processes: HashMap<u32, ProcessRecord>,
    /// Monotonic counter for `Event.version`. Bumped by `bump_version()`.
    pub event_version: u64,
    /// Monotonic counter for client_id (returned in Registered events).
    pub next_client_id: u64,
}

impl Default for State {
    fn default() -> Self {
        Self {
            lifecycle: LifecyclePhase::Starting,
            processes: HashMap::new(),
            event_version: 0,
            next_client_id: 1,
        }
    }
}

impl State {
    /// Bump and return the new event version. Always called inside
    /// the reducer when constructing an Event so version numbers
    /// stay strictly monotonic.
    pub fn bump_version(&mut self) -> u64 {
        self.event_version = self.event_version.wrapping_add(1);
        self.event_version
    }

    /// Bump and return the next client_id. Client IDs are stable
    /// per launcher run; not persisted across restart.
    pub fn alloc_client_id(&mut self) -> u64 {
        let id = self.next_client_id;
        self.next_client_id = self.next_client_id.wrapping_add(1);
        id
    }
}
