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

use super::{agent_started, new_tracker, TrackedProcess, TrackerHandle, TrackingConfidence};
use crate::backend::mps;

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

/// Assign a freshly-spawned child PID to `block_id`'s tracker, creating the
/// tracker if needed. The single call site every `Controller` impl should use
/// immediately after its own spawn — no-ops cleanly (with a warn log) if the
/// registry global isn't initialized (tests) or the platform tracker rejects
/// the PID, since this is opportunistic enrichment, not a liveness signal on
/// its own (see `broker::process::ProcessStatus`'s own doc comment).
pub fn track_spawned(block_id: &str, pid: u32) {
    let Some(registry) = global() else { return };
    if let Err(e) = registry.assign(block_id, pid) {
        tracing::warn!(
            block_id = %block_id,
            pid = pid,
            err = %e,
            "[process-tracker] assign_process failed"
        );
    }
}

pub struct AgentProcessRegistry {
    inner: Mutex<HashMap<String, RegistryEntry>>,
    broker: Option<Arc<mps::Broker>>,
}

struct RegistryEntry {
    tracker: Arc<dyn TrackerHandle>,
    /// Last-known PID set from the most recent poll. Used to diff
    /// against the current set so we only emit events for
    /// additions/removals — not on every poll tick.
    last_pids: HashSet<u32>,
    /// Every PID handed to `assign` — the agent CLI itself, one per
    /// (re)spawn. `agent_started` excludes them and their non-shell
    /// children (MCP servers) from what we report.
    roots: HashSet<u32>,
}

impl AgentProcessRegistry {
    pub fn new(broker: Option<Arc<mps::Broker>>) -> Self {
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
                roots: HashSet::new(),
            },
        );
        tracing::info!(
            block_id = %block_id,
            confidence = ?tracker.confidence(),
            "[process-tracker] registered tracker"
        );
        tracker
    }

    /// Assign a freshly-spawned agent/shell PID to `block_id`'s tracker
    /// (creating it if needed) and remember it as a root.
    pub fn assign(&self, block_id: &str, pid: u32) -> Result<(), String> {
        let tracker = self.ensure_tracker(block_id);
        tracker.assign_process(pid)?;
        if let Some(entry) = self.inner.lock().get_mut(block_id) {
            entry.roots.insert(pid);
        }
        Ok(())
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

    /// Processes the agent started through its tools (see
    /// [`agent_started`]) — for the RPC endpoint, Swarm and the pane-close
    /// confirmation. Excludes the CLI itself, conhost and MCP servers;
    /// `kill_tree` still takes all of them.
    pub fn list_block(&self, block_id: &str) -> Vec<TrackedProcess> {
        let (tracker, roots) = match self.inner.lock().get(block_id) {
            Some(e) => (e.tracker.clone(), e.roots.clone()),
            None => return Vec::new(),
        };
        agent_started(tracker.list_members(), &roots)
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
            let current_members = agent_started(entry.tracker.list_members(), &entry.roots);
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
        broker.publish(mps::MuxEvent {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_spawned_is_a_safe_no_op_without_a_global_registry() {
        // `GLOBAL` is only ever set by `bootstrap.rs` at real host startup —
        // never in this test binary — so this exercises the exact "tests
        // silently skip tracker registration" behavior the module doc for
        // `AgentProcessRegistry` promises. Must not panic.
        track_spawned("test-block-track-spawned-no-global", 999_999);
    }

    // This test used to be `#[ignore]`d as "environment-dependent", blaming
    // the agent-shell harness's own job object. That was wrong: the Windows
    // tracker opened the process with 0x0200 (PROCESS_SET_INFORMATION) where
    // AssignProcessToJobObject needs PROCESS_SET_QUOTA (0x0100), so EVERY
    // assignment failed with Access Denied — in the shipped app too (no
    // production log ever shows "assigned process to job"). Nested jobs are
    // fine; the access mask wasn't. Keep this test running so it can't
    // silently break again. Windows-only: elsewhere `new_tracker` is still
    // the membership-less stub, so there is nothing to observe (Codex P1 on
    // #3425).
    #[test]
    #[cfg(windows)]
    fn ensure_tracker_and_assign_process_track_a_real_short_lived_child() {
        // Uses a fresh, non-global `AgentProcessRegistry` (not `track_spawned`'s
        // `global()` path) so this doesn't touch the process-wide `GLOBAL`
        // OnceLock other tests in this binary may rely on being unset.
        //
        // Spawns a real, disposable child (rather than assigning the test
        // process's own PID) because `JobObjectTracker`'s Drop impl closes the
        // job handle, which fires `KILL_ON_JOB_CLOSE` on Windows — assigning
        // the test runner's own PID would kill the test process the moment
        // the registry (and its tracker) drops at the end of this test.
        // Alive for a few seconds, so it can't exit (and make the assignment
        // race a dead process) before `assign_process` runs; killed below.
        let mut child = if cfg!(windows) {
            std::process::Command::new("cmd")
                .args(["/C", "ping -n 2 127.0.0.1 > nul & ping -n 10 127.0.0.1 > nul"])
                .spawn()
        } else {
            std::process::Command::new("sh")
                .args(["-c", "sleep 10"])
                .spawn()
        }
        .expect("failed to spawn a disposable test child");
        let pid = child.id();

        let registry = AgentProcessRegistry::new(None);
        registry
            .assign("test-block-real-spawn", pid)
            .expect("assign should succeed for a live child we just spawned");

        let raw = registry.ensure_tracker("test-block-real-spawn").list_members();
        assert!(
            raw.iter().any(|p| p.pid == pid),
            "expected pid {pid} to be a job member after assign"
        );

        // The root itself is plumbing, not agent-started; the `ping` its shell
        // launches is. The second ping starts ~1s in, well after assignment,
        // so it can't slip through the spawn/assign race.
        let is_ping = |p: &TrackedProcess| p.command.to_ascii_lowercase().ends_with("ping.exe");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let started = loop {
            let listed = registry.list_block("test-block-real-spawn");
            if listed.iter().any(is_ping) || std::time::Instant::now() > deadline {
                break listed;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        };
        assert!(
            !started.iter().any(|p| p.pid == pid),
            "the assigned root must not be listed as agent-started"
        );
        assert!(
            started.iter().any(|p| is_ping(p) && p.parent_pid == Some(pid)),
            "expected the shell's ping child in list_block, got {started:?}"
        );

        // Reap before `registry` drops, so KILL_ON_JOB_CLOSE has nothing left
        // to terminate.
        let _ = child.kill();
        let _ = child.wait();
    }
}
