// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Runtime status: the `BlockControllerRuntimeStatus` snapshot, its publication
//! to the frontend, the status heartbeat, and turn-activity marking.

use super::*;

impl PersistentSubprocessController {
    pub(super) fn set_status(inner: &mut PersistentInner, status: &str) {
        inner.proc_status = status.to_string();
        inner.status_version += 1;
    }

    pub(super) fn get_status_snapshot(&self) -> BlockControllerRuntimeStatus {
        let inner = self.inner.lock().unwrap();
        BlockControllerRuntimeStatus {
            blockid: self.block_id.clone(),
            version: inner.status_version,
            shellprocstatus: inner.proc_status.clone(),
            shellprocconnname: "local".to_string(),
            shellprocexitcode: inner.proc_exit_code,
            shellprocpid: None,
            shellprocname: String::new(),
            spawn_ts_ms: None,
            is_agent_pane: true,
            turn_active: self.health_monitor.is_active_turn(),
        }
    }

    pub(super) fn publish_status(&self) {
        if let Some(ref broker) = self.broker {
            let status = self.get_status_snapshot();
            super::super::publish_controller_status(broker, &status);
        }
    }

    /// Periodic (low-frequency) republish of the current controllerstatus
    /// while a turn is active — a self-healing backstop independent of
    /// `publish_controller_status`'s `persist: 1` (which only helps a
    /// reconnecting subscriber) and the frontend's focus-triggered reconcile
    /// (which only fires on a background→foreground transition). If a single
    /// live push is missed for some other reason (e.g. a throttled/backgrounded
    /// renderer coalescing WS messages) while the window stays foregrounded
    /// and connected the whole time, nothing else corrects it until the turn
    /// actually ends. This shrinks that window from "forever" to at most one
    /// heartbeat interval. `HEARTBEAT_SECS` is well below the frontend's own
    /// `STUCK_THRESHOLD_MS` (45s, diagnostic-only) and `LIVENESS_RECOVERY_MS`
    /// (180s, force-recovery) so a missed push self-heals long before either
    /// of those fire. See REPORT_LOGIN_PERSIST_FAILURE_AND_STUCK_WORKING_2026_07_27.md
    /// §4 item 5. Duplicates `get_status_snapshot`'s field construction
    /// rather than calling it, since the spawned task only holds cloned
    /// `Arc`s, not `&self`; worth factoring out if a second controller type
    /// needs the same heartbeat.
    ///
    /// A latent duplicate-loop race is an existing, already-accepted
    /// contract (reagent P2 on the PR that introduced this function): if a
    /// turn ends and a new one starts again within one
    /// `HEARTBEAT_SECS` window, the old loop hasn't yet woken up to observe
    /// `is_active_turn() == false` and break, so both the old and new loop
    /// can run concurrently for that window. Harmless — `publish_status`
    /// republishing the same (or a slightly stale) snapshot twice is
    /// idempotent from the frontend's point of view — so this is accepted
    /// rather than fixed, matching the pre-existing pattern.
    pub(super) fn spawn_status_heartbeat(&self) {
        const HEARTBEAT_SECS: u64 = 20;
        self.spawn_status_heartbeat_with_interval(tokio::time::Duration::from_secs(HEARTBEAT_SECS));
    }

    /// Interval parameterized out of `spawn_status_heartbeat` so a test can
    /// drive it with a short, real (not virtual-clock) interval instead of
    /// waiting out `HEARTBEAT_SECS` — this crate doesn't enable tokio's
    /// `test-util` feature (needed for `start_paused`/`time::advance`), and
    /// adding it crate-wide for one test wasn't judged worth it.
    pub(super) fn spawn_status_heartbeat_with_interval(&self, heartbeat_interval: tokio::time::Duration) {
        let inner = Arc::clone(&self.inner);
        let block_id = self.block_id.clone();
        let broker = self.broker.clone();
        let health_monitor = Arc::clone(&self.health_monitor);
        tokio::spawn(async move {
            let Some(broker) = broker else { return };
            let mut interval = tokio::time::interval(heartbeat_interval);
            interval.tick().await; // first tick is immediate; publish_status() already ran at turn start
            loop {
                interval.tick().await;
                let still_active = health_monitor.is_active_turn();
                let status = {
                    let g = inner.lock().unwrap();
                    BlockControllerRuntimeStatus {
                        blockid: block_id.clone(),
                        version: g.status_version,
                        shellprocstatus: g.proc_status.clone(),
                        shellprocconnname: "local".to_string(),
                        shellprocexitcode: g.proc_exit_code,
                        shellprocpid: None,
                        shellprocname: String::new(),
                        spawn_ts_ms: None,
                        is_agent_pane: true,
                        turn_active: still_active,
                    }
                };
                super::super::publish_controller_status(&broker, &status);
                // Publish the final `turn_active: false` snapshot BEFORE
                // exiting, not just break silently — reagent P1: this
                // heartbeat exists specifically to backstop a missed live
                // turn-end push (the exact stuck-"Working"-forever bug this
                // PR fixes). Breaking without this last publish meant the
                // one case it's most needed for — the terminal push being
                // the one that got dropped — was the one case it didn't
                // help.
                if !still_active {
                    break;
                }
            }
        });
    }

    /// Marks a turn active (re-arming the status heartbeat only if it was
    /// previously idle) and publishes the resulting status flip. Shared by
    /// `send_message` and `retry_after_resume_failure` — both represent "a
    /// user message is about to be delivered," just via different spawn
    /// paths. The heartbeat is re-armed conditionally: a mid-turn steering
    /// send already has one running, so re-spawning on every call would
    /// leak duplicate heartbeat tasks.
    pub(super) fn mark_turn_active_and_publish(&self) {
        let was_active = self.health_monitor.mark_turn_active_for_queued();
        if !was_active {
            self.spawn_status_heartbeat();
        }
        self.publish_status();
    }
}
