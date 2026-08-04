// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `AgentProcessRegistry` — app-wide map from block_id → tracker handle.
//!
//! Created once at host startup, passed into each `SubprocessController`
//! / `PersistentSubprocessController` instance so it can wrap spawns
//! in its per-block job. Polled periodically from a background task
//! that emits `agent:process-added` / `agent:process-exited` events to
//! the frontend's swarm activity panel.
//!
//! Centralizing this here (rather than one tracker per controller)
//! means the lifetime of the tracker matches the lifetime of the pane
//! — multiple turns on the same block share the same job, so
//! descendants from turn N are still visible on turn N+1.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};

use parking_lot::Mutex;

use super::{new_tracker, TrackedProcess, TrackerHandle, TrackingConfidence};
use crate::backend::wps;

/// Host-wide registry, set once at startup. Exposed as a global so
/// `SubprocessController` / `PersistentSubprocessController` can reach
/// it without threading an `Arc` through every constructor + test site.
/// Tests that don't initialize it see `None` and silently skip
/// tracker registration — the job-object spawn path no-ops cleanly.
static GLOBAL: OnceLock<Arc<AgentProcessRegistry>> = OnceLock::new();

pub fn set_global(registry: Arc<AgentProcessRegistry>) {
    let _ = GLOBAL.set(registry);
}

pub fn global() -> Option<Arc<AgentProcessRegistry>> {
    GLOBAL.get().cloned()
}

pub struct AgentProcessRegistry {
    inner: Mutex<HashMap<String, RegistryEntry>>,
    broker: Option<Arc<wps::Broker>>,
}

struct RegistryEntry {
    tracker: Arc<dyn TrackerHandle>,
    /// Last-known PID set from the most recent poll. Used to diff
    /// against the current set so we only emit events for
    /// additions/removals — not on every poll tick.
    last_pids: HashSet<u32>,
}

impl AgentProcessRegistry {
    pub fn new(broker: Option<Arc<wps::Broker>>) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            broker,
        }
    }

    /// Ensure a tracker exists for this block. Idempotent — calling
    /// twice for the same block returns the existing tracker so the
    /// job survives controller re-creation (e.g. on /clear).
    pub fn ensure_tracker(&self, block_id: &str) -> Arc<dyn TrackerHandle> {
        let mut map = self.inner.lock();
        if let Some(entry) = map.get(block_id) {
            return entry.tracker.clone();
        }
        let tracker = new_tracker(block_id);
        map.insert(
            block_id.to_string(),
            RegistryEntry {
                tracker: tracker.clone(),
                last_pids: HashSet::new(),
            },
        );
        tracing::info!(
            block_id = %block_id,
            confidence = ?tracker.confidence(),
            "[process-tracker] registered tracker"
        );
        tracker
    }

    /// Drop a block's tracker — call when the pane closes. The tracker's
    /// Drop impl kills the whole process tree (via `KILL_ON_JOB_CLOSE`
    /// on Windows, `cgroup.kill` on Linux, `killpg` on macOS).
    pub fn remove(&self, block_id: &str) {
        let mut map = self.inner.lock();
        if map.remove(block_id).is_some() {
            tracing::info!(block_id = %block_id, "[process-tracker] dropped tracker on pane close");
        }
    }

    /// Current members of a block's tracker, for the RPC endpoint.
    pub fn list_block(&self, block_id: &str) -> Vec<TrackedProcess> {
        self.inner
            .lock()
            .get(block_id)
            .map(|e| e.tracker.list_members())
            .unwrap_or_default()
    }

    /// Confidence of a block's tracker — drives the "tracking is
    /// best-effort on macOS" badge in the swarm UI.
    pub fn confidence_of(&self, block_id: &str) -> TrackingConfidence {
        self.inner
            .lock()
            .get(block_id)
            .map(|e| e.tracker.confidence())
            .unwrap_or(TrackingConfidence::None)
    }

    /// Kill the entire process tree for a given block. Returns `true`
    /// if a tracker was found (the kill was dispatched). Does NOT
    /// synchronously wait for descendants to actually exit — the
    /// poller's next tick will pick up the state changes and emit
    /// `agent:process-exited` events the frontend can react to.
    pub fn kill_tree(&self, block_id: &str) -> bool {
        let tracker = self.inner.lock().get(block_id).map(|e| e.tracker.clone());
        match tracker {
            Some(t) => {
                t.kill_tree();
                true
            }
            None => false,
        }
    }

    /// Kill a single PID if it's a member of the given block's tree.
    /// Returns `true` if the tracker was found and the PID matched.
    pub fn kill_pid(&self, block_id: &str, pid: u32) -> bool {
        let tracker = self.inner.lock().get(block_id).map(|e| e.tracker.clone());
        match tracker {
            Some(t) => t.kill_pid(pid),
            None => false,
        }
    }

    /// Poll every tracked block's membership and diff against the
    /// last-known set. Emits `agent:process-added` / `-exited` events
    /// for each delta.
    ///
    /// Called by a background Tokio task on a ~2s interval.
    pub fn poll_and_emit(&self) {
        let mut map = self.inner.lock();
        for (block_id, entry) in map.iter_mut() {
            let current_members = entry.tracker.list_members();
            let current_pids: HashSet<u32> = current_members.iter().map(|p| p.pid).collect();

            for added_pid in current_pids.difference(&entry.last_pids) {
                if let Some(p) = current_members.iter().find(|p| p.pid == *added_pid) {
                    self.emit(
                        "agent:process-added",
                        block_id,
                        serde_json::json!({ "block_id": block_id, "process": p }),
                    );
                }
            }
            for removed_pid in entry.last_pids.difference(&current_pids) {
                self.emit(
                    "agent:process-exited",
                    block_id,
                    serde_json::json!({ "block_id": block_id, "pid": removed_pid }),
                );
            }

            entry.last_pids = current_pids;
        }
    }

    fn emit(&self, event_name: &str, block_id: &str, data: serde_json::Value) {
        let Some(ref broker) = self.broker else { return };
        broker.publish(wps::WaveEvent {
            event: event_name.to_string(),
            scopes: vec![format!("block:{}", block_id)],
            sender: String::new(),
            persist: 0,
            data: Some(data),
        });
    }
}

/// Spawn the polling task. Drops when the registry's `Arc` refcount
/// hits zero (host shutdown). ~2s cadence balances latency (new
/// processes show up fast) with CPU overhead (job queries are cheap
/// but not free).
pub fn spawn_poller(registry: Arc<AgentProcessRegistry>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            registry.poll_and_emit();
        }
    });
}
