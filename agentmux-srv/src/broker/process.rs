// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Process Broker — Phase A of the process-tracking consolidation.
//!
//! See `docs/specs/REPORT_PROCESS_ARCHITECTURE_STATE_AND_RETHINK_2026_07_22.md`
//! for the full rationale. AgentMux answers "is this agent/process alive and
//! what's it doing" six different, only-partially-overlapping ways today
//! (`blockcontroller::CONTROLLER_REGISTRY`, `process_tracker`, the `reactive`
//! handler's own registration list, `pidregistry`, `HealthMonitor`'s
//! output-silence heuristic, and `watchdog.rs`'s PTY-idle timers). This
//! module is the consolidation point for the two callers that most directly
//! motivated it (the Agent pane's process badge and the Swarm pane's block
//! discovery/overview), modeled on `broker::scheduler`'s Credential Broker
//! (same crate, sibling module — same "one broker, register your own
//! backing store" pattern applied to a second domain) and on
//! `agentmux-srv::reducer`'s existing discipline of computing state through
//! one arbiter with explicit transition logic rather than letting every
//! caller mutate a shared map directly (see `lifecycle_from` below, which
//! mirrors `reducer/lifecycle.rs::handle_register`'s shape: pure function,
//! explicit rule per legal state, no bare field copy).
//!
//! **Phase A scope:** read-side only. `status()`/`list()` compute a
//! normalized `ProcessStatus` by querying today's existing sources
//! (`blockcontroller` for lifecycle — authoritative, has an entry for every
//! controller type; `process_tracker` for OS-process detail — only
//! populated for `subprocess`/`persistent` controllers today) and cache the
//! result, single-flight-guarded per call. This closes the concrete,
//! reported bug directly: `agent.tracked-blocks`'s `.chain()` of
//! `process_tracker` + the `reactive` handler's registration list (two
//! structurally different registries unioned with no reconciliation) is
//! replaced by `blockcontroller::get_all_controllers()`, which is already
//! authoritative for "which blocks exist" across every controller type.
//!
//! **Deferred to later phases** (see the report's §6 open questions):
//! migrating each `Controller` impl to register with this broker directly
//! at spawn (closing the coverage gap at the write side, not just papering
//! over it at the read side); consolidating the two independent
//! activity-summary pipelines; moving the Swarm pane's client-side-only
//! fine-grained activity chip server-side. None of that is touched here.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use parking_lot::Mutex;

use crate::backend::blockcontroller::{self, BlockControllerRuntimeStatus};
use crate::backend::process_tracker::{self, TrackedProcess, TrackingConfidence};
use crate::backend::wps::{Broker as WpsBroker, WaveEvent};

/// WPS event name for a `ProcessStatus` change. Scoped `block:<id>`, same
/// convention as `agent:process-added`/`controllerstatus`. Consumers that
/// want the unified signal subscribe to this instead of the two separate
/// events the old `.chain()`-based discovery relied on.
pub const EVENT_STATUS_CHANGED: &str = "processbroker:status-changed";

/// Coarse "is it alive" question, normalized from `BlockControllerRuntimeStatus`'s
/// `turn_active`/`shellprocstatus`/`shellprocexitcode` fields into one enum —
/// the Pod-phase-equivalent signal in the Kubernetes phase/status/probe
/// analogy the report draws in §4.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lifecycle {
    Running,
    Idle,
    Done,
    Error,
    /// No controller found for this block_id at all — distinct from `Idle`,
    /// which means a controller exists and reports it's not mid-turn.
    Unknown,
}

/// The broker's public read shape — one struct every consumer (Agent pane,
/// Swarm pane, future consumers) reads instead of composing raw registry
/// data themselves.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProcessStatus {
    pub block_id: String,
    pub lifecycle: Lifecycle,
    /// OS-process-tree detail — empty for controller types `process_tracker`
    /// doesn't cover yet (today: `shell`, `acp`; report §1/§5.2). Presence of
    /// entries here is opportunistic enrichment, not a liveness signal on
    /// its own — use `lifecycle`, not `!processes.is_empty()`, to answer
    /// "is it alive."
    pub processes: Vec<TrackedProcess>,
    pub liveness_confidence: TrackingConfidence,
    pub is_agent_pane: bool,
    pub last_computed_ms: u64,
}

impl ProcessStatus {
    fn unknown(block_id: &str) -> Self {
        Self {
            block_id: block_id.to_string(),
            lifecycle: Lifecycle::Unknown,
            processes: Vec::new(),
            liveness_confidence: TrackingConfidence::None,
            is_agent_pane: false,
            last_computed_ms: now_ms(),
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Pure reducer over one block's raw controller status: decides
/// `Lifecycle` via explicit per-case rules, not a bare field copy. Isolated
/// from I/O so the transition rules are unit-testable without a real
/// controller instance — mirrors `reducer/lifecycle.rs::handle_register`'s
/// shape (compute the legal next state from the current observation,
/// rather than trusting the caller's intent unconditionally).
fn lifecycle_from(status: &BlockControllerRuntimeStatus) -> Lifecycle {
    // turn_active is the only signal that distinguishes "actively
    // generating a turn" from "process alive, idle between turns" for a
    // PersistentSubprocessController — shellprocstatus alone can't tell
    // the difference (see BlockControllerRuntimeStatus's own doc comment).
    if status.turn_active {
        return Lifecycle::Running;
    }
    match status.shellprocstatus.as_str() {
        blockcontroller::STATUS_RUNNING => Lifecycle::Idle,
        blockcontroller::STATUS_INIT => Lifecycle::Idle,
        blockcontroller::STATUS_DONE if status.shellprocexitcode == 0 => Lifecycle::Done,
        blockcontroller::STATUS_DONE => Lifecycle::Error,
        _ => Lifecycle::Unknown,
    }
}

/// Compute a block's current `ProcessStatus` from today's underlying
/// sources. Pure with respect to its two inputs — the only I/O is the two
/// read calls themselves (both cheap in-memory lock acquisitions, not
/// blocking I/O — process_tracker's registry and blockcontroller's
/// CONTROLLER_REGISTRY are both `parking_lot`/`std` sync locks guarding
/// plain maps).
fn compute_status(block_id: &str) -> ProcessStatus {
    let controller_status = blockcontroller::get_block_controller_status(block_id);
    let (processes, liveness_confidence) = process_tracker::registry::global()
        .map(|r| (r.list_block(block_id), r.confidence_of(block_id)))
        .unwrap_or((Vec::new(), TrackingConfidence::None));

    match controller_status {
        Some(status) => ProcessStatus {
            block_id: block_id.to_string(),
            lifecycle: lifecycle_from(&status),
            processes,
            liveness_confidence,
            is_agent_pane: status.is_agent_pane,
            last_computed_ms: now_ms(),
        },
        None => ProcessStatus::unknown(block_id),
    }
}

pub struct ProcessBroker {
    /// Broker-owned cache — the single source future callers read from,
    /// rather than each composing `blockcontroller`/`process_tracker`
    /// themselves. `parking_lot::Mutex`, not an async mutex: unlike the
    /// credential broker's `Entry::lock` (which guards an actual network
    /// refresh call and must not block a tokio worker thread while held),
    /// this cache's critical section is two cheap in-memory map reads plus
    /// a struct build — synchronous and fast, the same reasoning
    /// `process_tracker::registry::AgentProcessRegistry` already uses
    /// `parking_lot::Mutex` for. Coarse-grained (one mutex, not
    /// per-block_id): contention is expected to be negligible for the same
    /// reason; narrow the granularity later if profiling says otherwise.
    /// If a later phase adds a genuinely blocking liveness check (report
    /// §3.0.3 point 1 — e.g. a real health-probe round-trip), that phase
    /// should revisit this as an async lock at the same time, not before.
    cache: Mutex<HashMap<String, ProcessStatus>>,
    wps_broker: Option<Arc<WpsBroker>>,
}

impl ProcessBroker {
    pub fn new(wps_broker: Option<Arc<WpsBroker>>) -> Self {
        Self {
            cache: Mutex::new(HashMap::new()),
            wps_broker,
        }
    }

    /// Recompute and return one block's status. Single-flight-guarded by
    /// holding the cache lock across the *entire* read-compare-write
    /// sequence, not just the write — `compute_status` must run inside the
    /// critical section, not before it. reagent P1 on #2273 caught this:
    /// with the read outside the lock, two concurrent callers (e.g. an
    /// Agent pane mount and Swarm's refresh landing near-simultaneously)
    /// could interleave so the thread that *read* first but *wrote*
    /// second overwrites a fresher value the other thread already wrote —
    /// the cache regresses to a stale snapshot even though a newer read
    /// already happened. Locking around the whole sequence (safe here
    /// because `compute_status` is synchronous and cheap — see the `cache`
    /// field's own doc comment) makes "last writer wins" and "last reader
    /// wins" the same thing, which is what single-flight is supposed to
    /// guarantee.
    pub fn status(&self, block_id: &str) -> ProcessStatus {
        let mut cache = self.cache.lock();
        let fresh = compute_status(block_id);
        let changed = cache
            .get(block_id)
            .map(|prev| {
                prev.lifecycle != fresh.lifecycle || prev.processes.len() != fresh.processes.len()
            })
            .unwrap_or(true);
        cache.insert(block_id.to_string(), fresh.clone());
        drop(cache);
        if changed {
            self.emit_changed(&fresh);
        }
        fresh
    }

    /// The missing batch query (report §5.4) — replaces both
    /// `agent.tracked-blocks`'s `.chain()` of two independent registries
    /// and the N sequential single-block `GetControllerStatus` calls
    /// Swarm's `subscribeToBlockStatuses` issues today with one call.
    /// Discovery source is `blockcontroller::get_all_controllers` —
    /// authoritative for every controller type, closing the coverage gap
    /// `process_tracker` alone has for `shell`/`acp` blocks (report §1).
    pub fn list(&self) -> Vec<ProcessStatus> {
        blockcontroller::get_all_controllers()
            .into_keys()
            .map(|block_id| self.status(&block_id))
            .collect()
    }

    /// Drop a block from the cache. Call on pane close so `list()` doesn't
    /// keep serving a stale entry for a block whose controller is gone —
    /// `compute_status` would report `Lifecycle::Unknown` for it on the
    /// next `status()` call regardless, but a closed pane shouldn't appear
    /// in `list()` (sourced from `get_all_controllers()`, which already
    /// won't include it) while a stale cache entry lingers unreachable.
    pub fn forget(&self, block_id: &str) {
        self.cache.lock().remove(block_id);
    }

    fn emit_changed(&self, status: &ProcessStatus) {
        let Some(ref broker) = self.wps_broker else {
            return;
        };
        broker.publish(WaveEvent {
            event: EVENT_STATUS_CHANGED.to_string(),
            scopes: vec![format!("block:{}", status.block_id)],
            sender: String::new(),
            persist: 0,
            data: serde_json::to_value(status).ok(),
        });
    }
}

static GLOBAL: OnceLock<Arc<ProcessBroker>> = OnceLock::new();

pub fn set_global(broker: Arc<ProcessBroker>) {
    let _ = GLOBAL.set(broker);
}

pub fn global() -> Option<Arc<ProcessBroker>> {
    GLOBAL.get().cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status_with(shellprocstatus: &str, turn_active: bool, exit_code: i32) -> BlockControllerRuntimeStatus {
        BlockControllerRuntimeStatus {
            shellprocstatus: shellprocstatus.to_string(),
            turn_active,
            shellprocexitcode: exit_code,
            ..Default::default()
        }
    }

    #[test]
    fn active_turn_is_running_regardless_of_shellprocstatus() {
        let status = status_with(blockcontroller::STATUS_RUNNING, true, 0);
        assert_eq!(lifecycle_from(&status), Lifecycle::Running);
    }

    #[test]
    fn running_without_active_turn_is_idle() {
        let status = status_with(blockcontroller::STATUS_RUNNING, false, 0);
        assert_eq!(lifecycle_from(&status), Lifecycle::Idle);
    }

    #[test]
    fn init_is_idle() {
        let status = status_with(blockcontroller::STATUS_INIT, false, 0);
        assert_eq!(lifecycle_from(&status), Lifecycle::Idle);
    }

    #[test]
    fn done_with_zero_exit_code_is_done() {
        let status = status_with(blockcontroller::STATUS_DONE, false, 0);
        assert_eq!(lifecycle_from(&status), Lifecycle::Done);
    }

    #[test]
    fn done_with_nonzero_exit_code_is_error() {
        let status = status_with(blockcontroller::STATUS_DONE, false, 1);
        assert_eq!(lifecycle_from(&status), Lifecycle::Error);
    }

    #[test]
    fn unrecognized_shellprocstatus_is_unknown() {
        let status = status_with("some-future-status", false, 0);
        assert_eq!(lifecycle_from(&status), Lifecycle::Unknown);
    }

    #[test]
    fn status_of_a_block_with_no_controller_is_unknown() {
        let broker = ProcessBroker::new(None);
        let status = broker.status("no-such-block-id");
        assert_eq!(status.lifecycle, Lifecycle::Unknown);
        assert!(status.processes.is_empty());
    }

    #[test]
    fn ten_concurrent_callers_for_the_same_block_all_get_a_consistent_answer() {
        // Not a call-count assertion (unlike the credential broker's
        // analogous test — that one collapses concurrent refreshes onto
        // one actual call; this one recomputes on every call by design,
        // report §5.1, since the underlying reads are cheap in-memory
        // lookups, not an expensive operation worth collapsing). What must
        // hold is that concurrent callers across real OS threads never
        // observe a torn/partial write to the shared cache.
        let broker = Arc::new(ProcessBroker::new(None));
        let handles: Vec<_> = (0..10)
            .map(|_| {
                let broker = broker.clone();
                std::thread::spawn(move || broker.status("some-block"))
            })
            .collect();
        for h in handles {
            let status = h.join().unwrap();
            assert_eq!(status.lifecycle, Lifecycle::Unknown);
        }
    }

    #[test]
    fn forget_does_not_panic_on_an_untracked_block() {
        let broker = ProcessBroker::new(None);
        broker.forget("never-seen-this-block");
    }

    #[test]
    fn list_block_ids_exactly_match_the_controller_registrys_current_keys() {
        // CONTROLLER_REGISTRY is a process-wide static shared by every test
        // in this binary (`cargo test` runs unit tests concurrently in one
        // process by default), so this can't assert `list()` is empty —
        // another test running at the same time may have a real controller
        // registered. What must hold regardless of what else is registered:
        // list()'s block_ids exactly match get_all_controllers()'s current
        // keys, with nothing added or dropped in between the two calls.
        let broker = ProcessBroker::new(None);
        let mut from_broker: Vec<String> =
            broker.list().into_iter().map(|s| s.block_id).collect();
        let mut from_registry: Vec<String> =
            blockcontroller::get_all_controllers().into_keys().collect();
        from_broker.sort();
        from_registry.sort();
        assert_eq!(from_broker, from_registry);
    }
}
