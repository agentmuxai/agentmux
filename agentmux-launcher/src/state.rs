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

use std::collections::{HashMap, HashSet};

use agentmux_common::ipc::{ClientKind, WindowKind};
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

/// Phase B.4 read-only mirror of one host-owned window. The launcher
/// learns about windows via `Command::ReportWindowOpened`; the host
/// remains authoritative until B.5 flips the direction. `opened_at`
/// is the launcher's clock at the time the report arrived (RFC3339)
/// — useful for correlating launcher logs with host logs across
/// version skew.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowMirror {
    pub label: String,
    pub kind: WindowKind,
    /// Set only for `Subwindow`; identifies the FullInstance that
    /// owns this window so the eventual cascade-close logic (B.5)
    /// has the parent linkage.
    pub parent_label: Option<String>,
    pub opened_at: String,
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
    /// Read-only window mirror (Phase B.4). Keyed by label. Source of
    /// truth still lives in `agentmux-cef::AppState.browsers` /
    /// `window_meta`; this is a passive copy fed by host
    /// `ReportWindow*` commands. B.5 inverts the dependency: host
    /// queries this map instead of maintaining its own.
    pub windows: HashMap<String, WindowMirror>,
    /// Phase B.4 follow-up — pre-warmed pool inventory. Tracked
    /// separately from `windows` because pool entries are not
    /// user-visible until promote. On promote the host emits
    /// `ReportPoolWindowRemoved` + `ReportWindowOpened` so the same
    /// label transitions atomically (from launcher's perspective)
    /// from `pool` to `windows`. On pre-promote destroy: only
    /// `ReportPoolWindowRemoved`.
    pub pool: HashSet<String>,
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
            windows: HashMap::new(),
            pool: HashSet::new(),
            event_version: 0,
            next_client_id: 1,
        }
    }
}

impl State {
    /// Bump and return the new event version. Always called inside
    /// the reducer when constructing an Event so version numbers
    /// stay strictly monotonic.
    ///
    /// Strict (non-wrapping) add: Phase D's GetSnapshot resync
    /// protocol relies on monotonicity (`event.version >
    /// snapshot.version`), and a wrap to 0 would silently break
    /// that contract. u64 at one event/ns would take 584 years to
    /// overflow — never going to happen in practice; if it ever
    /// does, the panic is the right failure mode.
    /// (gemini MEDIUM PR #574 round-1.)
    pub fn bump_version(&mut self) -> u64 {
        self.event_version += 1;
        self.event_version
    }

    /// Bump and return the next client_id. Client IDs are stable
    /// per launcher run; not persisted across restart. Same strict-
    /// add reasoning as bump_version.
    pub fn alloc_client_id(&mut self) -> u64 {
        let id = self.next_client_id;
        self.next_client_id += 1;
        id
    }
}
