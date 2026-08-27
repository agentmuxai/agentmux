// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Process Broker — Phase A of the process-tracking consolidation.
//!
//! See `docs/specs/REPORT_PROCESS_ARCHITECTURE_STATE_AND_RETHINK_2026_07_22.md`
//! for the full rationale. AgentMux answers "is this agent/process alive and
//! what's it doing" six different, only-partially-overlapping ways today
//! (`blockcontroller::CONTROLLER_REGISTRY`, `process_tracker`, the `reactive`
//! handler's own registration list, `pidregistry`, `TurnActivityTracker`'s
//! turn-active bookkeeping, and `watchdog.rs`'s PTY-idle timers). This
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
    /// Controller type string (`"shell"`, `"cmd"`, `"subprocess"`,
    /// `"persistent"`, `"acp"`, `"tsunami"`) — see `is_agent()` for why this
    /// matters beyond being informational.
    pub controller_type: String,
    pub is_agent_pane: bool,
    pub last_computed_ms: u64,
    /// The raw `BlockControllerRuntimeStatus` `compute_status` already reads
    /// to derive `lifecycle`/`is_agent_pane` above — exposed so callers that
    /// want the finer-grained fields (`shellprocstatus`, `spawn_ts_ms`,
    /// `turn_active` itself, not just the `lifecycle` it was folded into)
    /// don't have to read the controller registry a second time and risk a
    /// second, possibly-inconsistent snapshot (codex P2 on PR #2380 — the
    /// `muxspect describe` route originally did exactly that).
    pub controller_status: Option<BlockControllerRuntimeStatus>,
}

impl ProcessStatus {
    fn unknown(block_id: &str) -> Self {
        Self {
            block_id: block_id.to_string(),
            lifecycle: Lifecycle::Unknown,
            processes: Vec::new(),
            liveness_confidence: TrackingConfidence::None,
            controller_type: String::new(),
            is_agent_pane: false,
            last_computed_ms: now_ms(),
            controller_status: None,
        }
    }

    /// True if this block is an agent pane rather than a plain terminal.
    ///
    /// `is_agent_pane` alone is NOT sufficient: `subprocess`/`persistent`/
    /// `acp` controllers exist *only* for agent CLIs (one-shot-per-turn,
    /// long-lived stream-json, and JSON-RPC/ACP agents respectively — see
    /// `blockcontroller/mod.rs`'s controller-type doc), but
    /// `SubprocessController` unconditionally reports `is_agent_pane:
    /// false` in its runtime status regardless (reagent/codex P1 on
    /// #2273 — filtering on the flag alone would wrongly exclude every
    /// `subprocess`-type agent pane). Only `shell`/`cmd` controllers can
    /// legitimately be either a plain terminal or an agent running inside
    /// one, so `is_agent_pane` (set dynamically at PTY spawn time — see
    /// `shell/lifecycle.rs`) is the correct signal for exactly those two
    /// types, and only those two.
    pub fn is_agent(&self) -> bool {
        match self.controller_type.as_str() {
            blockcontroller::BLOCK_CONTROLLER_SUBPROCESS
            | blockcontroller::BLOCK_CONTROLLER_PERSISTENT
            | blockcontroller::BLOCK_CONTROLLER_ACP => true,
            _ => self.is_agent_pane,
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
    let controller = blockcontroller::get_controller(block_id);
    let controller_status = controller.as_ref().map(|c| c.get_runtime_status());
    let (processes, liveness_confidence) = process_tracker::registry::global()
        .map(|r| (r.list_block(block_id), r.confidence_of(block_id)))
        .unwrap_or((Vec::new(), TrackingConfidence::None));

    match (controller, controller_status) {
        (Some(controller), Some(status)) => ProcessStatus {
            block_id: block_id.to_string(),
            lifecycle: lifecycle_from(&status),
            processes,
            liveness_confidence,
            controller_type: controller.controller_type().to_string(),
            is_agent_pane: status.is_agent_pane,
            last_computed_ms: now_ms(),
            controller_status: Some(status),
        },
        _ => ProcessStatus::unknown(block_id),
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
        // Never cache (or emit a change event for) a block with no real
        // controller — `list()`'s own callers only ever pass real,
        // discovered block_ids (from `get_all_controllers()`), but this
        // broker is also reachable directly with caller-supplied input via
        // `muxspect describe` (agentmux-srv/src/server/muxspect_handlers.rs)
        // — the first RPC-adjacent surface to call `status()` with an
        // arbitrary string rather than one already known to exist. Without
        // this guard, repeated queries for distinct nonexistent block_ids
        // would grow this cache unboundedly, since `forget()` is only ever
        // called from real controller teardown and never sees IDs that were
        // never real to begin with (reagent + codex, independently, on
        // PR #2380).
        if fresh.lifecycle == Lifecycle::Unknown {
            drop(cache);
            return fresh;
        }
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
    ///
    /// Returns EVERY controller-backed block — plain `shell`/`cmd`
    /// terminals included, not just agent panes. Use `list_agent_panes()`
    /// for the agent-scoped subset (which is what `agent.tracked-blocks`
    /// actually needs — see that method's doc comment).
    pub fn list(&self) -> Vec<ProcessStatus> {
        blockcontroller::get_all_controllers()
            .into_keys()
            .map(|block_id| self.status(&block_id))
            .collect()
    }

    /// Agent-scoped subset of `list()`. `agent.tracked-blocks`'s original
    /// (pre-broker) implementation was implicitly agent-only — the two
    /// registries it unioned (`process_tracker`, populated only by
    /// agent-CLI controller types, and `reactive`'s registration list,
    /// populated only via `register_agent`) never included a plain
    /// terminal. `list()` alone loses that scoping because
    /// `get_all_controllers()` is controller-type-agnostic by design
    /// (reagent/codex P1 on #2273 — an earlier version of this PR fed
    /// `list()` straight into `agent.tracked-blocks`, which made every
    /// open terminal pane show up in Swarm mislabeled as an "Agent").
    /// See `ProcessStatus::is_agent` for the exact classification rule.
    pub fn list_agent_panes(&self) -> Vec<ProcessStatus> {
        self.list().into_iter().filter(|s| s.is_agent()).collect()
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

    /// Test-only: current cache size, to prove unknown/nonexistent
    /// block_ids never get inserted (see `status()`'s own guard and the
    /// regression test below).
    #[cfg(test)]
    fn cache_len(&self) -> usize {
        self.cache.lock().len()
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

    fn process_status_with(controller_type: &str, is_agent_pane: bool) -> ProcessStatus {
        ProcessStatus {
            block_id: "test-block".to_string(),
            lifecycle: Lifecycle::Idle,
            processes: Vec::new(),
            liveness_confidence: TrackingConfidence::None,
            controller_type: controller_type.to_string(),
            is_agent_pane,
            last_computed_ms: 0,
            controller_status: None,
        }
    }

    #[test]
    fn subprocess_controller_is_always_agent_even_when_is_agent_pane_is_false() {
        // reagent/codex P1 on #2273: SubprocessController unconditionally
        // reports is_agent_pane: false in its own runtime status (see
        // subprocess.rs) despite existing only for agent CLIs — is_agent()
        // must not rely on the flag for this controller type.
        let status = process_status_with(blockcontroller::BLOCK_CONTROLLER_SUBPROCESS, false);
        assert!(status.is_agent());
    }

    #[test]
    fn persistent_and_acp_controllers_are_always_agent() {
        for ty in [
            blockcontroller::BLOCK_CONTROLLER_PERSISTENT,
            blockcontroller::BLOCK_CONTROLLER_ACP,
        ] {
            let status = process_status_with(ty, false);
            assert!(status.is_agent(), "{ty} should always classify as agent");
        }
    }

    #[test]
    fn shell_and_cmd_controllers_defer_to_the_real_is_agent_pane_flag() {
        for ty in [
            blockcontroller::BLOCK_CONTROLLER_SHELL,
            blockcontroller::BLOCK_CONTROLLER_CMD,
        ] {
            assert!(
                !process_status_with(ty, false).is_agent(),
                "{ty} with is_agent_pane=false must not classify as agent (this is the exact regression reagent/codex caught — a plain terminal wrongly showing up in Swarm as an agent)"
            );
            assert!(
                process_status_with(ty, true).is_agent(),
                "{ty} with is_agent_pane=true (agent running inside a terminal) must classify as agent"
            );
        }
    }

    #[test]
    fn unknown_block_is_never_classified_as_agent() {
        let status = ProcessStatus::unknown("no-such-block");
        assert!(!status.is_agent());
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
    fn unknown_block_ids_are_never_cached_unbounded_growth_guard() {
        // reagent + codex, independently, on PR #2380: `muxspect describe`
        // is the first caller to reach `status()` with arbitrary,
        // caller-supplied block_ids rather than ones already known to be
        // real (list()'s own callers only ever pass IDs from
        // get_all_controllers()). Without this guard, a loop probing
        // distinct nonexistent block_ids would grow the cache forever,
        // since forget() only ever fires from real controller teardown.
        let broker = ProcessBroker::new(None);
        for i in 0..50 {
            let _ = broker.status(&format!("garbage-block-{i}"));
        }
        assert_eq!(broker.cache_len(), 0, "unknown block_ids must never be cached");
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
