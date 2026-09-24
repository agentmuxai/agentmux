// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Stop/restart and session-identity queries: `stop_process`, the kill request
//! path, `session_id`, `needs_spawn`, and the deferred-restart flag.

use super::*;

impl PersistentSubprocessController {
    pub fn stop_process(&self, force: bool) -> Result<(), String> {
        let request = if force {
            KillRequest::Force
        } else {
            KillRequest::Graceful(std::time::Instant::now() + super::super::SHUTDOWN_GRACE)
        };
        self.request_stop(request)
    }

    /// Another process already running on session `sid`: `(block id, closing)`.
    /// Checks live persistent controllers in the registry, then blocks
    /// mid-close (out of the registry, process not yet exited).
    pub(super) fn session_held_elsewhere(&self, sid: &str) -> Option<(String, bool)> {
        for (block_id, ctrl) in super::super::get_all_controllers() {
            if block_id == self.block_id {
                continue;
            }
            if let Some(other) = ctrl.as_any().downcast_ref::<PersistentSubprocessController>() {
                let g = other.inner.lock().unwrap();
                if g.current_pid.is_some() && g.session_id.as_deref() == Some(sid) {
                    return Some((block_id, false));
                }
            }
        }
        let store = self.mstore.as_ref()?;
        super::super::closing_blocks_still_running()
            .into_iter()
            .filter(|block_id| *block_id != self.block_id)
            .find(|block_id| {
                store
                    .get::<crate::backend::obj::Block>(block_id)
                    .ok()
                    .flatten()
                    .is_some_and(|b| crate::backend::obj::meta_get_string(&b.meta, core::META_SESSION_ID, "") == sid)
            })
            .map(|block_id| (block_id, true))
    }

    pub(super) fn request_stop(&self, request: KillRequest) -> Result<(), String> {
        Self::request_stop_on(&self.inner, request)
    }

    /// [`request_stop`] over just the shared state, for the `'static`
    /// future [`Controller::shutdown`] returns.
    pub(super) fn request_stop_on(inner_arc: &Arc<Mutex<PersistentInner>>, request: KillRequest) -> Result<(), String> {
        Self::request_stop_inner(inner_arc, request, false);
        Ok(())
    }

    /// A kill for teardown: drains the deferred queue in the SAME `inner`
    /// acquisition that issues the kill, and returns what it drained for the
    /// caller to report. Draining in a second acquisition left a window
    /// after the kill request where the stdout reader could handle the
    /// turn's `result`, flush a deferred message into the process being
    /// killed, and leave the report an empty queue: lost, with no stranded
    /// warning (codex P2 on #3562). Once this lock is released, a boundary
    /// flush finds the queue empty.
    pub(super) fn request_stop_draining_deferred(&self, request: KillRequest) -> Vec<String> {
        Self::request_stop_inner(&self.inner, request, true)
    }

    fn request_stop_inner(
        inner_arc: &Arc<Mutex<PersistentInner>>,
        request: KillRequest,
        drain_deferred: bool,
    ) -> Vec<String> {
        let (kill_tx, drained) = {
            let mut inner = inner_arc.lock().unwrap();
            // Recorded unconditionally, not only when `kill_tx` is already
            // `None` — codex P1 on PR #2360 (round 16, commit ce1642d90):
            // `stop_process` can race a process that already exited (a
            // confirmed stale-`--resume` death) and is about to be
            // silently retried — sending through `kill_tx` is futile in
            // that window regardless of whether it's already `None` (the
            // exit-handler cleared it) or still `Some` (`tokio::select!`
            // already committed to the `child.wait()` exit arm before
            // this call reached the lock, so the `kill_rx` arm will never
            // be polled again even if the send succeeds). Recording this
            // as a `StopRequested` event — resolved by
            // `persistent_resume::update` once `ProcessExited` arrives —
            // is the only way the exit-handler's retry decision can know
            // the user explicitly asked to stop.
            let generation = inner.spawn_generation;
            inner.apply_resume_event(persistent_resume::ResumeEvent::StopRequested { generation });
            inner.stop_pending = true;
            let drained = if drain_deferred {
                inner.deferred_deliveries.drain(..).collect()
            } else {
                Vec::new()
            };
            (inner.kill_tx.take(), drained)
        };
        if let Some(tx) = kill_tx {
            let _ = tx.send(request);
        }
        drained
    }

    pub fn session_id(&self) -> Option<String> {
        self.inner.lock().unwrap().session_id.clone()
    }

    /// True when this controller is registered but has **no live process and no
    /// spawn already in flight** — the state a lazily-registered controller sits
    /// in between `register` ("spawns on first message") and its first
    /// `send_message`.
    ///
    /// In that state `inject_message` cannot deliver: it has no spawn config and
    /// only writes to a live `stdin_tx`, so it returns "persistent process not
    /// running". The caller can, however, start a *turn* — `run_agent_turn`
    /// re-reads the spawn config from block metadata and calls `send_message`,
    /// which does spawn. This predicate is what lets the reactive delivery path
    /// tell those two situations apart. See
    /// `docs/reports/REPORT_JEKT_DELIVERY_DROPS_UNSPAWNED_PERSISTENT_AGENTS_2026_09_03.md`.
    ///
    /// **The invariant this must uphold:** `needs_spawn() == true` implies a
    /// subsequent `send_message` takes `decide_send_action`'s `BecomeSpawner`
    /// branch — i.e. it really spawns and delivers. It must never be true in a
    /// state where `send_message` would instead return `SendAction::Queued`,
    /// because `Queued` returns `Ok(())` having delivered nothing, the reactive
    /// path maps that to `Ok(true)`, and the caller is told the message landed
    /// when it is still sitting in the queue. `cloud_subscriber` only retries
    /// on `!success`, so a false success is a permanently lost message.
    ///
    /// That is why all three exclusions are here, and each mirrors one of
    /// `decide_send_action`'s own guards:
    ///
    /// - `stdin_tx.is_none()` — a live process is steerable; `inject_message`
    ///   handles it and no turn start is wanted.
    /// - `!spawning_in_progress` — a caller has already claimed this spawn
    ///   round (see that field's doc comment on the concurrent-spawn TOCTOU
    ///   race). Reporting "needs spawn" here would invite a second, racing
    ///   spawn and orphan a child.
    /// - `!drain_claim` — a `RetryFlush` drain owns the round. This one is
    ///   subtle and was missed on the first cut (reagent P1 on PR #2960): when
    ///   a drain's target process dies mid-flush, the drain **deliberately
    ///   retains** its claim for the fallback respawn while the exit handler
    ///   clears `stdin_tx`. Without this term, that window reports "needs
    ///   spawn", `decide_send_action` then returns `Queued` on the
    ///   still-held claim, and the message is silently queued while the caller
    ///   is told it was delivered.
    ///
    /// Both excluded windows are retryable rather than lost: the caller gets
    /// the original delivery error back, which is what lets it come again.
    ///
    /// Racy by nature, and safe to be: the process can exit the instant after
    /// this returns `false`. It is a routing hint, not a guarantee — the spawn
    /// claim inside `send_message` is what actually serialises spawners. What
    /// it must not do is report `true` for a state that cannot spawn.
    pub fn needs_spawn(&self) -> bool {
        let inner = self.inner.lock().unwrap();
        inner.stdin_tx.is_none() && !inner.spawning_in_progress && !inner.drain_claim
    }

    /// Ask this controller to restart itself once the current turn ends,
    /// instead of being torn down and replaced right now.
    ///
    /// Returns `true` when the restart was deferred (a turn is in flight, so
    /// the caller must NOT replace the controller), `false` when the pane is
    /// idle and an immediate replace is safe — the caller then proceeds
    /// exactly as before.
    ///
    /// Called from `resync_controller`'s forced-replace path. See
    /// [`PersistentInner::restart_when_idle`] for what the old
    /// kill-immediately behaviour destroyed.
    ///
    /// Deliberately keyed on the health monitor's `is_active_turn` rather
    /// than on `stdin_tx.is_some()`: a live process between turns is idle and
    /// should be replaced immediately (that's the common `/model` case, and
    /// deferring it would leave the change unapplied until the next turn
    /// happened to end). It's specifically an IN-FLIGHT turn that must not be
    /// interrupted.
    pub fn request_restart_when_idle(&self) -> bool {
        // The turn-active read and the flag write happen under ONE acquisition
        // of `inner`, and the turn-end consumer clears `active_turn` under that
        // same lock — so the two serialize (codex P2 on PR #2858). Interleaved,
        // this would otherwise observe an active turn, have the consumer run to
        // completion (seeing `restart_when_idle` still false, so restarting
        // nothing), and then set the flag — leaving it stuck until the NEXT
        // turn ended, so the user's next prompt ran with the old config.
        //
        // Lock order is `inner` -> health monitor here and in the consumer;
        // `TurnActivityTracker` never holds its own lock across an `inner`
        // acquisition, so the order can't invert.
        let mut inner = self.inner.lock().unwrap();
        if !self.health_monitor.is_active_turn() {
            return false;
        }
        inner.restart_when_idle = true;
        drop(inner);
        tracing::info!(
            block_id = %self.block_id,
            "runtime-config change arrived mid-turn — deferring the restart to the end of this turn \
             instead of killing the in-flight message"
        );
        true
    }
}
