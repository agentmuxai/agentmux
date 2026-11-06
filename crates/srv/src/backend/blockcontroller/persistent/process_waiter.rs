// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The persistent CLI's process waiter task, started by `spawn_process`
//! (`spawn.rs`): the `child.wait()` arm (the process exited on its own) and
//! the kill arm (a stop or restart asked for it).

use super::*;

/// What the process waiter task captures from `spawn_process`.
pub(super) struct ProcessWaiterCtx {
    pub(super) child: tokio::process::Child,
    pub(super) kill_rx: tokio::sync::oneshot::Receiver<KillRequest>,
    pub(super) stderr_reader_handle: Option<tokio::task::JoinHandle<()>>,
    pub(super) stdout_reader_handle: tokio::task::JoinHandle<()>,
    pub(super) segment_wait: super::segments::SegmentRef,
    pub(super) global_output_zone_wait: Option<String>,
    pub(super) record_fence_wait: Option<crate::backend::agent_admission::RecordFence>,
    pub(super) block_id_wait: String,
    pub(super) inner_wait: Arc<Mutex<PersistentInner>>,
    pub(super) broker_wait: Option<Arc<mps::Broker>>,
    pub(super) mstore_wait: Option<Arc<Store>>,
    pub(super) event_bus_wait: Option<Arc<EventBus>>,
    pub(super) health_wait: Arc<TurnActivityTracker>,
    pub(super) filestore_wait: Option<Arc<FileStore>>,
    pub(super) agent_id_wait: Option<String>,
    pub(super) self_ref_wait: std::sync::Weak<PersistentSubprocessController>,
    pub(super) my_generation_wait: u64,
    pub(super) pid_wait: u32,
    pub(super) nonce_wait: u64,
    /// This spawn's registration nonce, which the stdout reader recorded its
    /// MCP report under (`backend::mcp_observed`).
    pub(super) mcp_owner_wait: u64,
}

/// What the exit steps share: the task's captures other than the child, the kill channel and the
/// reader handles, which the two arms of `run_process_waiter` consume.
struct ProcessWaiter {
    segment: super::segments::SegmentRef,
    global_output_zone: Option<String>,
    record_fence: Option<crate::backend::agent_admission::RecordFence>,
    block_id: String,
    inner: Arc<Mutex<PersistentInner>>,
    broker: Option<Arc<mps::Broker>>,
    mstore: Option<Arc<Store>>,
    event_bus: Option<Arc<EventBus>>,
    health: Arc<TurnActivityTracker>,
    filestore: Option<Arc<FileStore>>,
    agent_id: Option<String>,
    self_ref: std::sync::Weak<PersistentSubprocessController>,
    generation: u64,
    pid: u32,
    nonce: u64,
    mcp_owner: u64,
}

/// What the locked part of an exit decided, for the steps that run once the lock is dropped.
struct ExitRecord {
    is_current_generation: bool,
    effects: Vec<persistent_resume::ResumeEffect>,
    respawn_after_restart: bool,
}

impl PersistentSubprocessController {
    /// Body of the process waiter task.
    pub(super) async fn run_process_waiter(ctx: ProcessWaiterCtx) {
        let ProcessWaiterCtx {
            mut child,
            kill_rx,
            stderr_reader_handle,
            stdout_reader_handle,
            segment_wait,
            global_output_zone_wait,
            record_fence_wait,
            block_id_wait,
            inner_wait,
            broker_wait,
            mstore_wait,
            event_bus_wait,
            health_wait,
            filestore_wait,
            agent_id_wait,
            self_ref_wait,
            my_generation_wait,
            pid_wait,
            nonce_wait,
            mcp_owner_wait,
        } = ctx;
        let waiter = ProcessWaiter {
            block_id: block_id_wait,
            inner: inner_wait,
            broker: broker_wait,
            mstore: mstore_wait,
            event_bus: event_bus_wait,
            health: health_wait,
            filestore: filestore_wait,
            agent_id: agent_id_wait,
            self_ref: self_ref_wait,
            generation: my_generation_wait,
            pid: pid_wait,
            nonce: nonce_wait,
            mcp_owner: mcp_owner_wait,
            segment: segment_wait,
            global_output_zone: global_output_zone_wait,
            record_fence: record_fence_wait,
        };
        tokio::select! {
            status = child.wait() => {
                waiter.on_exited(status, stderr_reader_handle, stdout_reader_handle).await;
            }
            Ok(request) = kill_rx => {
                waiter.on_kill(request, &mut child, stderr_reader_handle, stdout_reader_handle).await;
            }
        }
    }
}

impl ProcessWaiter {
    /// The process exited on its own: `child.wait()` resolved.
    async fn on_exited(
        &self,
        status: std::io::Result<std::process::ExitStatus>,
        stderr_reader_handle: Option<tokio::task::JoinHandle<()>>,
        stdout_reader_handle: tokio::task::JoinHandle<()>,
    ) {
        let exit_code = status.map(|s| s.code().unwrap_or(-1)).unwrap_or(-1);
        super::segments::record_segment_end(
            &self.segment,
            crate::backend::continuity_segments::exit_reason(exit_code),
            crate::backend::agent_admission::fenced_zone(self.global_output_zone.as_deref(), &self.record_fence),
        );
        tracing::info!(
            block_id = %self.block_id,
            exit_code = exit_code,
            "persistent process exited"
        );

        self.settle_after_exit(stderr_reader_handle, stdout_reader_handle).await;
        self.wait_for_drain_idle().await;

        let record = self.record_exit(exit_code);
        self.publish_process_gone(record.is_current_generation);

        if record.is_current_generation {
            // The process is gone — nothing is waiting on the user
            // any more (Phase 5 notifications).
            if let Some(r) = self.broker.as_ref().and_then(crate::backend::notify::router::get) {
                r.resolve_nonblocking(&self.block_id, crate::backend::notify::policy::Family::Input);
            }
            // Notify health monitor so Stalled/Dead watchdog stops.
            self.health.set_exited(exit_code);

            self.deregister_from_muxbus();

            // Clear active pid — clean exit, no recovery needed.
            self.clear_active_pid();
        }

        self.apply_exit_effects(exit_code, record.effects);

        self.respawn_if_due(record.respawn_after_restart);
    }

    /// Give the stderr and stdout readers their bounded chance to finish after the process exited.
    async fn settle_after_exit(
        &self,
        stderr_reader_handle: Option<tokio::task::JoinHandle<()>>,
        stdout_reader_handle: tokio::task::JoinHandle<()>,
    ) {
        // Give the stderr reader a bounded chance to fully
        // drain and react to whatever it saw right before this
        // process exited — codex P1/P2 on PR #2360 (second
        // review pass): `child.wait()` resolving does NOT mean
        // the stderr reader (an independently-scheduled task)
        // has already called `poison_resume` for a "No
        // conversation found" line, or finished ITS OWN
        // subsequent `persist_session_id("")` call. Without
        // this, two failure modes were possible: (1) this task
        // could resolve the resume state (`ProcessExited`, in `record_exit`)
        // before the stderr reader ever promotes it to
        // `ConfirmedRetry`, permanently losing
        // the retry for the exact case it exists to catch, and
        // (2) a confirmed retry's fresh session id (persisted
        // by the NEW process's own stdout reader once
        // respawned) could be silently overwritten by this
        // exiting process's stderr task finally getting around
        // to persisting an empty one, corrupting continuity
        // despite the retry having succeeded. 500ms is
        // generous — the stderr pipe closes and drains almost
        // immediately once the process has genuinely exited.
        //
        // If it DOES take longer than that (e.g. `persist_session_id`
        // blocked on a slow store call), a bare `timeout()` alone
        // is not enough — codex P1 on PR #2360 (fifth review
        // pass): `timeout()` only stops WAITING for the handle,
        // it does not cancel the underlying task, which keeps
        // running in the background and can still call
        // `poison_resume`/`persist_session_id("")` AFTER this
        // task has already moved on (discarded the still-
        // tentative retry, or started a fresh child whose own
        // new session id that late write would then corrupt).
        // `abort()` on a separately-obtained `AbortHandle`
        // actually cancels it — the task stops at its next
        // yield point and can never reach either call again.
        //
        // codex P1 on PR #2371 (round 1): the stdout reader
        // performs synchronous FileStore/SQLite writes that
        // can legitimately contend for multiple seconds
        // (SQLite's own busy timeout) — a short bound (the
        // stderr reader's 500ms above) would abort it
        // mid-line under ordinary contention, discarding
        // still-unread assistant/result frames and
        // truncating the persisted transcript.
        //
        // codex P1 on PR #2371 (round 2): but an UNBOUNDED
        // await isn't safe either — if the CLI spawned a
        // background descendant that inherited its stdout
        // descriptor, killing/waiting for the direct child
        // does NOT close that descriptor, so this reader's
        // `lines.next_line()` may never see EOF, hanging this
        // exit-handling step (and everything after it —
        // health status, muxbus deregistration, the retry
        // decision itself) forever.
        //
        // 10s is the compromise: generous enough that
        // ordinary SQLite contention never triggers the
        // abort (avoiding the round-1 truncation risk), but
        // still a hard ceiling so a genuinely stuck
        // descendant-held pipe (or anything else gone wrong)
        // can't hang this task indefinitely (closing the
        // round-2 gap). Aborting our own read loop doesn't
        // require the OS pipe to actually close — it just
        // stops OUR wait, accepting we may not have drained
        // every last buffered line, the same risk profile
        // the original 500ms bound already accepted, just at
        // a bound wide enough not to fire under normal load.
        settle_reader_tasks(&self.block_id, stderr_reader_handle, stdout_reader_handle, "process exit").await;
        forget_mcp_servers(self.mcp_owner, self.broker.as_deref(), &self.block_id);
    }

    /// Wait for the queue drain to stop mid-delivery before the retry batch is read as final.
    async fn wait_for_drain_idle(&self) {
        // Wait (briefly, bounded) for the drain to finish
        // appending whatever message it's currently
        // mid-delivery on before deciding the retry batch
        // is final (`record_exit`) (#2360): `drain_queue_after_
        // successful_spawn`'s own "send, then append to the
        // retry batch" sequence has an unavoidable gap at the
        // `.await` (a mutex can't be held across it). Without
        // this wait, the `.take()` in `record_exit` could run in that
        // exact gap and dispatch a retry missing a message
        // the doomed process's channel had ALREADY accepted —
        // it stays marked "accepted" and gets persisted, but
        // is never actually delivered to any process again.
        // Same 500ms bound as the stderr-reader wait in `settle_after_exit`,
        // for the same "best effort, don't hang forever"
        // reason.
        for _ in 0..50 {
            if !self.inner.lock().unwrap().drain_send_in_flight {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }

    /// The part of an exit that needs the lock: release this generation's process state and let
    /// the resume state machine decide what the exit means.
    fn record_exit(&self, exit_code: i32) -> ExitRecord {
        let mut inner = self.inner.lock().unwrap();
        // reagentx P1 (round 6 on PR #2373, extended round 8):
        // this belated exit-handling can run AFTER a fresh
        // spawn has already superseded this generation (see
        // `respawn_once_for_leftover_queue`'s own doc comment
        // and the kill arm (`stop_child`) for the documented race that
        // makes this reachable) — `inner.spawn_generation` is
        // bumped on every NEW spawn, so a mismatch here means
        // this exit is for an already-superseded generation.
        // Everything gated on it belongs to THIS exact
        // process (its own pid/exit code/stdin/kill channel,
        // the shared `proc_status` this process last knew to
        // be true, its own health-monitor/muxbus/registry/
        // session-recovery registration) — running any of it
        // unconditionally would corrupt or tear down a newer,
        // actively-running generation's own state as if IT
        // had exited. reagentx round 8: the round-6 fix only
        // gated the field writes, missing
        // `health.set_exited` (a shared `TurnActivityTracker`
        // across generations) and the deregistration block
        // (`deregister_from_muxbus`) — both keyed by `block_id`/`agent_id`, not
        // generation, so a stale exit incorrectly marked a
        // live process's health as exited and tore down its
        // muxbus/registry/session-recovery registration while
        // it kept running.
        let is_current_generation = inner.spawn_generation == self.generation;
        if is_current_generation {
            self.release_process(&mut inner, exit_code);
        }
        // One event resolves the ENTIRE retry/error-line
        // decision — including any earlier `StopRequested`
        // (see `stop_process`), already baked into the state
        // by `persistent_resume::update` before this event
        // ever arrives. See `persistent_resume`'s module doc
        // comment for why this replaced four separate field
        // reads/writes. Safe to call unconditionally even for
        // a superseded generation — `update()`'s own
        // generation-matching arms (and `NotTracking`'s
        // `current_generation`) already no-op a stale event
        // on their own.
        let effects = inner
            .apply_resume_event(persistent_resume::ResumeEvent::ProcessExited { generation: self.generation });
        if is_current_generation {
            PersistentSubprocessController::set_status(&mut inner, STATUS_DONE);
        }
        // A committed config restart whose process exited on its
        // own before the kill was consumed: this arm won the
        // `select!`, so it must finish the restart too (codex P1 on
        // #3990). Acted on at the end of `on_exited`, like
        // `on_kill`; a stale-resume retry spawning first supersedes it.
        let respawn_after_restart = PersistentSubprocessController::config_restart_due_locked(&inner, self.generation);
        drop(inner);
        ExitRecord { is_current_generation, effects, respawn_after_restart }
    }

    /// This generation's process is gone: drop the handles and the lease the exit or the kill made
    /// stale.
    fn release_process(&self, inner: &mut PersistentInner, exit_code: i32) {
        crate::identity::token_warmup::release(&self.block_id);
        inner.proc_exit_code = exit_code;
        if let Some(ended) = inner.current_pid.take() {
            crate::backend::cli_native::pids::clear(&self.block_id, ended);
        }
        inner.stdin_tx = None;
        inner.kill_tx = None;
        // This process no longer drives the agent: release
        // its single-live-instance lease (on the last `Arc`).
        inner.agent_lease = None;
    }

    /// The process is gone: say so, whatever else the arm publishes.
    fn publish_process_gone(&self, is_current_generation: bool) {
        if is_current_generation {
            if let Some(b) = self.broker.as_ref() {
                super::status::publish_runtime_event(&self.inner, b, &self.block_id);
            }
        }
    }

    /// Deregister from muxbus so later sends fall through to the lower tiers instead of resolving to
    /// a dead block. Mirrors the shell controller's exit path.
    fn deregister_from_muxbus(&self) {
        // lower tiers instead of resolving to a dead block. Mirrors
        // the shell controller's exit path. This exact process's
        // resources are gone either way; if a retry/fallback
        // respawn has ALREADY re-registered, the guards below
        // leave its fresh registration in place.
        // All removals are compare-and-remove keyed on this
        // spawn's process-wide registration nonce (issue
        // #2363): the caller's `is_current_generation` gate
        // was read once, and a fallback/retry respawn's
        // fresh registration can land on a parallel task
        // between that read and these calls — an
        // unconditional removal here would wipe the NEW
        // spawn's entry with nothing left to re-register
        // it. Nonce, not generation: a replacement
        // controller's spawn restarts at generation 1 and
        // could collide with ours (codex P1 on PR #2500).
        let registration_was_ours = crate::backend::reactive::get_global_handler()
            .unregister_block_if_nonce(&self.block_id, self.nonce);
        sweep_stopped_registrations(&self.block_id, self.nonce, registration_was_ours);
        if let Some(ref agent_id) = self.agent_id {
            remove_registration_files_by_name(agent_id, self.nonce);
            // The cloud subscriber's agent set records no
            // per-agent identity to compare against, so the
            // in-memory registration's outcome above stands
            // in: if a newer spawn already re-registered,
            // its cloud subscription must survive too.
            // …but a registration already gone reads `false`
            // too; then nobody holds the agent and it must go,
            // or its WAN lease is renewed forever.
            // `drop_if_orphaned` re-checks after removing, so a
            // respawn registering meanwhile keeps its entry
            // (Codex P2 on #3897).
            let handler = crate::backend::reactive::get_global_handler();
            let still_held = handler.has_live_name(agent_id);
            if crate::muxbus::cloud_subscriber::drop_cloud_subscription_on_exit(
                registration_was_ours,
                still_held,
            ) {
                if let Some(sub) = crate::muxbus::cloud_subscriber::get_global_subscriber() {
                    sub.drop_if_orphaned(agent_id, |a| handler.has_live_name(a));
                }
            }
        }
    }

    /// Clear the active pid, with no recovery needed.
    fn clear_active_pid(&self) {
        // Compare-and-clear (issue #2363): only if the
        // recorded pid is still THIS process's — a fallback
        // respawn may have re-registered a fresh pid on a
        // parallel task between the caller's generation gate
        // and this call.
        if let Some(ref mstore) = self.mstore {
            super::super::session_recovery::clear_active_pid_if_pid(mstore, &self.block_id, self.pid);
        }
    }

    /// What the resume state machine decided the exit means: say how the turn ended, or retry.
    fn apply_exit_effects(&self, exit_code: i32, effects: Vec<persistent_resume::ResumeEffect>) {
        // A stale `--resume <sid>` is exactly what killed this
        // process (`retry_after_resume_failure`'s doc comment) —
        // retry the same message once, fresh, WITHOUT publishing
        // this transient failure as a completed turn first.
        // codex P2 on PR #2360 (second review pass): publishing
        // "done"/turn_active:false here would let the mounted
        // UI (trackTurnJustEnded, a deferred controller refresh)
        // treat this failed attempt as the real end of the
        // user's turn before the retry's own fresh "running"
        // status ever lands. reagentx/codex never reviewed this
        // controller type in PR #2338 (see docs/retro/
        // RETRO_STALE_RESUME_SESSION_ID_ACROSS_CHANNELS_2026_07_29.md).
        for effect in effects {
            match effect {
                // SPEC_AGENT_PANE_HISTORY_ALIGNMENT_2026_08_05.md
                // §2.1: the `ConfirmedRetry` + `ProcessExited`
                // (not-stopped) arm now bundles this alongside
                // `FireRetry` — the resume's fate (Fresh) is
                // already known here, even though the retry
                // hasn't launched yet (`fire_retry`).
                persistent_resume::ResumeEffect::EmitSessionOutcome { outcome, attempted_sid, actual_sid } => {
                    self.emit_session_outcome(outcome, attempted_sid, actual_sid);
                }
                persistent_resume::ResumeEffect::PersistImmediately(line)
                | persistent_resume::ResumeEffect::FlushErrorLine(line) => {
                    self.persist_exit_error(exit_code, &line);
                }
                persistent_resume::ResumeEffect::FireRetry { retry, held_error_line, attempted_sid } => {
                    self.fire_retry(retry, held_error_line, attempted_sid);
                }
                persistent_resume::ResumeEffect::PublishDone => self.publish_done(exit_code),
            }
        }
    }

    fn emit_session_outcome(
        &self,
        outcome: persistent_resume::SessionOutcome,
        attempted_sid: String,
        actual_sid: Option<String>,
    ) {
        publish_resume_retry_status(&self.broker, &self.block_id, "resolved");
        // Same retraction as the stdout-reader site
        // (`stdout_reader.rs`) — this arm reaches `Fresh` today, but
        // the clear is keyed on the outcome rather
        // than on which arm produced it, so it stays
        // correct if that ever changes.
        if matches!(outcome, persistent_resume::SessionOutcome::Resumed) {
            if let Some(ref store) = self.mstore {
                super::super::session_recovery::clear_resume_failed(
                    store,
                    &self.event_bus,
                    &self.block_id,
                );
            }
        }
        let line = session_outcome_line(outcome, attempted_sid, actual_sid);
        self.append_output_line(&line);
    }

    fn persist_exit_error(&self, exit_code: i32, line: &str) {
        // Safety net: a "Reconnecting…" left showing from
        // an earlier retry attempt on this same block must
        // not get stuck forever just because THIS exit
        // ended up genuinely non-retryable — a harmless
        // no-op publish in the (common) case where no
        // retry was in flight at all.
        publish_resume_retry_status(&self.broker, &self.block_id, "resolved");
        self.append_output_line(line);
        // Classify + surface this exit's error to the
        // pane's failure-recovery UI —
        // SPEC_PERSISTENT_CONTROLLER_FAILURE_CLASSIFICATION.
        // Reached only for PersistImmediately/FlushErrorLine
        // — the resume machinery has already decided this
        // exit is NOT being silently retried (contrast
        // FireRetry (`fire_retry`), which must stay invisible to
        // the user).
        surface_error_line(
            &self.block_id,
            Some(exit_code),
            line,
            self.broker.as_deref(),
            &self.mstore,
            &self.event_bus,
        );
    }

    fn fire_retry(&self, retry: persistent_resume::RetryPayload, held_error_line: Option<String>, attempted_sid: String) {
        // Issue #2368: the retry is firing and will
        // very likely succeed within milliseconds —
        // the doomed attempt's own terminal error
        // result must never reach the user, so it's
        // dropped (not flushed) as long as the retry
        // actually launches. Handed to
        // `retry_after_resume_failure` itself (codex
        // P2 on PR #2371) rather than dropped here
        // unconditionally — a retry that turns out
        // NOT to launch (this controller already
        // gone, or the fresh spawn itself failing)
        // must still flush it, or an already-
        // accepted prompt ends in total silence.
        if let Some(ctrl) = self.self_ref.upgrade() {
            tracing::warn!(
                block_id = %self.block_id,
                "stale --resume session id caused this exit — retrying now (find_recovery_session_id may still resume a real, on-disk session rather than starting blank)"
            );
            ctrl.retry_after_resume_failure(
                self.generation,
                retry.config,
                retry.messages,
                held_error_line,
                attempted_sid,
            );
        } else {
            // reagentx P2 on PR #2776: the controller
            // itself is already gone — no retry will
            // ever fire for this batch, so any
            // "Reconnecting…" left showing from an
            // earlier attempt on this same block must
            // be resolved here; nothing else will.
            publish_resume_retry_status(&self.broker, &self.block_id, "resolved");
            if let Some(line) = held_error_line {
                // The controller itself is already gone
                // (weak ref invalidated) — nothing can
                // retry this batch at all, so flush
                // directly via this task's own captured
                // broker/filestore rather than going
                // through `ctrl`.
                self.append_output_line(&line);
            }
        }
    }

    fn publish_done(&self, exit_code: i32) {
        if let Some(ref broker) = self.broker {
            let status = super::super::agent_runtime_status(
                &self.block_id,
                0,
                STATUS_DONE,
                exit_code,
                false,
            );
            super::super::publish_controller_status(broker, &status);
        }
    }

    /// Finish a committed config restart whose old process is gone.
    fn respawn_if_due(&self, respawn_after_restart: bool) {
        if respawn_after_restart {
            if let Some(ctrl) = self.self_ref.upgrade() {
                ctrl.respawn_after_config_restart(self.generation);
            }
        }
    }

    fn append_output_line(&self, line: &str) {
        if let Some(ref broker) = self.broker {
            super::super::shell::handle_append_block_file(
                broker,
                &self.block_id,
                PERSISTENT_OUTPUT_SUBJECT,
                line.as_bytes(),
                self.filestore.as_ref(),
                crate::backend::agent_admission::fenced_zone(self.global_output_zone.as_deref(), &self.record_fence),
            );
        }
    }

    /// A stop or restart asked for the process to go.
    async fn on_kill(
        &self,
        request: KillRequest,
        child: &mut tokio::process::Child,
        stderr_reader_handle: Option<tokio::task::JoinHandle<()>>,
        stdout_reader_handle: tokio::task::JoinHandle<()>,
    ) {
        let graceful_deadline = match request {
            KillRequest::Force => None,
            KillRequest::Graceful(deadline) => Some(deadline),
        };
        // Why it's going away, read before the kill changes
        // anything: `shutdown` marks a pane close for this
        // generation; `restart_pending` a runtime-config restart.
        let segment_end_reason = {
            let inner = self.inner.lock().unwrap();
            crate::backend::continuity_segments::kill_reason(
                inner.shutdown_generation == Some(self.generation),
                inner.restart_pending,
            )
        };
        tracing::info!(
            block_id = %self.block_id,
            force = graceful_deadline.is_none(),
            "persistent process kill requested"
        );
        self.stop_child(child, graceful_deadline).await;
        super::segments::record_segment_end(
            &self.segment,
            segment_end_reason,
            crate::backend::agent_admission::fenced_zone(self.global_output_zone.as_deref(), &self.record_fence),
        );

        // Mirror the child.wait() arm's bounded await+abort of
        // both reader tasks (`settle_after_exit`) before resolving the resume
        // state (`StopRequested` + `ProcessExited`, in `record_kill`)
        // (#2371). Without this, a stop racing the doomed
        // attempt's in-flight terminal error line could resolve
        // the state before the stdout reader holds the line
        // back; the lagging reader would then find `NotTracking`
        // and persist the line on its own, instead of it being
        // flushed (`flush_stop_effects`) as part of this stop.
        // codex P1 on PR #2371 (round 2): a bounded wait, not
        // an unconditional one — see the child.wait() arm's
        // identical comment (`settle_after_exit`) for the full reasoning. This
        // matters MORE here: codex flagged that every
        // remaining cleanup step (clearing current_pid/
        // kill_tx, STATUS_DONE, muxbus deregistration) runs
        // AFTER this await, so an unbounded hang here would
        // leave a user-initiated Stop making the controller
        // appear permanently alive if a descendant process
        // ever holds the stdout descriptor open.
        settle_reader_tasks(&self.block_id, stderr_reader_handle, stdout_reader_handle, "kill").await;
        forget_mcp_servers(self.mcp_owner, self.broker.as_deref(), &self.block_id);

        let record = self.record_kill();
        self.publish_process_gone(record.is_current_generation);

        // reagentx P1 on PR #2776: a user-initiated Stop can
        // land at any point, including mid stale-`--resume`
        // retry — "Reconnecting…" has no other clearing path on
        // this branch (unlike `compacting`, which several
        // turn-end transitions defensively reset), so without
        // this it would stick on-screen forever with a growing
        // counter, and a later retry on the same pane would
        // wrongly keep the stale `startedAt` (ResumeRetryStarted
        // is a no-op while already reconnecting). Unconditional
        // and un-gated by `is_current_generation` on purpose —
        // a harmless no-op when nothing was reconnecting, and
        // the pane is stopping either way, so there's no
        // "which generation" ambiguity worth encoding here.
        publish_resume_retry_status(&self.broker, &self.block_id, "resolved");

        // Flush a held-back error line now, if the resume
        // state machine produced one — the `StopRequested`
        // sent in `record_kill` guarantees `ProcessExited` resolves via
        // the "stop wins" branch (see `persistent_resume::
        // update`), so `FireRetry` is never actually possible
        // here, but it's still matched defensively rather
        // than assumed.
        self.flush_stop_effects(record.effects);

        if record.is_current_generation {
            // Notify the turn-activity tracker of the exit —
            // shared `Arc<TurnActivityTracker>` across
            // generations, gated the same way as the
            // `on_exited`.
            self.health.set_exited(-1);
            self.deregister_from_muxbus();

            // Clear active pid — user-initiated stop, no recovery needed.
            self.clear_active_pid();
        }

        self.respawn_if_due(record.respawn_after_restart);
    }

    /// Ask the child to go: close stdin and give it until the deadline for a graceful stop, or
    /// kill it now. Records how it ended in `stop_exit`.
    async fn stop_child(&self, child: &mut tokio::process::Child, graceful_deadline: Option<std::time::Instant>) {
        if let Some(deadline) = graceful_deadline {
            // Graceful: drop stdin to send EOF, then wait briefly
            {
                let mut inner = self.inner.lock().unwrap();
                inner.stdin_tx = None; // drops the sender → stdin writer exits → stdin closes
                // reagentx P0 on PR #2360 (sixth review pass,
                // round 10): must clear pending_send_messages/
                // spawning_in_progress in this SAME lock
                // acquisition as stdin_tx, not only later
                // (after child.wait()/the 5s timeout below).
                // During that window,
                // drain_queue_after_successful_spawn's
                // independently-scheduled background task
                // could observe stdin_tx.is_none() with
                // messages still queued, classify it as a
                // stall, and call
                // respawn_once_for_leftover_queue — spawning a
                // brand-new CLI process while the user's
                // graceful stop is still in progress. The
                // force-kill path doesn't have this gap (it
                // never clears stdin_tx early — all three
                // clear together (in `record_kill`), after child.kill()
                // resolves), only this graceful one, since it
                // specifically needs stdin_tx gone early to
                // trigger EOF.
                inner.pending_send_messages.clear();
                inner.spawning_in_progress = false;
            }
            // Wait until the caller's deadline (a close shares
            // one deadline across every agent it stops — spec
            // §9.2), then kill.
            let killed = tokio::select! {
                _ = child.wait() => false,
                _ = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => {
                    // With its process group: its MCP servers go too.
                    let _ = agentmux_common::process::kill_child_and_group(child).await;
                    true
                }
            };
            self.inner.lock().unwrap().stop_exit = Some((self.generation, killed));
        } else {
            let _ = agentmux_common::process::kill_child_and_group(child).await;
            self.inner.lock().unwrap().stop_exit = Some((self.generation, true));
        }
    }

    /// The part of a kill that needs the lock; the kill arm's counterpart of `record_exit`.
    fn record_kill(&self) -> ExitRecord {
        let mut inner = self.inner.lock().unwrap();
        // reagentx P1 (round 6 on PR #2373, extended round 8):
        // same reasoning as `record_exit` — this
        // graceful-stop cleanup can itself run AFTER the
        // documented race in `stop_child` (dropping `stdin_tx`
        // early to trigger EOF lets
        // `respawn_once_for_leftover_queue` spawn a brand-new
        // generation while this kill is still mid-flight) has
        // already superseded this generation. Everything
        // gated on it belongs to THIS exact kill (its own
        // pid/exit code/stdin/kill channel, its own spawn
        // claim/queue, the shared `proc_status` this stop
        // last knew to be true, its own health-monitor/
        // muxbus/registry/session-recovery registration) —
        // running any of it unconditionally would corrupt or
        // tear down a newer, actively-running generation's
        // own state as if IT had been stopped.
        let is_current_generation = inner.spawn_generation == self.generation;
        if is_current_generation {
            self.release_process(&mut inner, -1);
        }
        // A user-initiated kill overrides any resume-retry
        // decision in flight, for a REUSED controller instance
        // (`resync_controller` can reuse the same instance
        // across a kill+restart cycle) the same way an
        // in-flight Stop already does for the child.wait() arm (`record_exit`)
        // — reusing that exact `StopRequested` + `ProcessExited`
        // event pair here instead of duplicating the "stop
        // wins" logic against raw fields. codex P2 on PR #2371:
        // this also means a stashed error line is never
        // silently lost on a user-initiated stop (it may be a
        // genuine error the stop itself interrupted) — it's
        // flushed (`flush_stop_effects`) via the effects this produces, same as
        // the "genuinely done" case. Safe to call
        // unconditionally even for a superseded generation —
        // `update()`'s own generation-matching arms already
        // no-op a stale event on their own.
        inner.apply_resume_event(persistent_resume::ResumeEvent::StopRequested {
            generation: self.generation,
        });
        let effects = inner
            .apply_resume_event(persistent_resume::ResumeEvent::ProcessExited { generation: self.generation });
        // codex P1 on PR #2360 (sixth review pass, round 5):
        // an active spawn claim's own background drain
        // (`drain_queue_after_successful_spawn`) is a
        // SEPARATE, independently-scheduled task — killing
        // this process does not cancel it. Left untouched, its
        // next check would see `stdin_tx` gone with messages
        // still queued, treat that as a stall, and hand off to
        // `respawn_once_for_leftover_queue` — silently
        // reviving the agent moments after the user explicitly
        // stopped it. Clearing the queue AND releasing the
        // claim here means that same check instead sees an
        // empty queue and just concludes normally, with no
        // fallback respawn triggered. Gated the same way as
        // the process state — a NEWER generation's own claim/queue must
        // never be cleared by this stale one's cleanup.
        if is_current_generation {
            inner.pending_send_messages.clear();
            inner.spawning_in_progress = false;
            PersistentSubprocessController::set_status(&mut inner, STATUS_DONE);
        }
        // This kill was a deferred config restart of THIS
        // generation (`config_restart_due_locked`). Acted on at the
        // end of `on_kill`, after the deregistration, because
        // the replacement registers itself afresh. Only read here:
        // the spawn claim consumes the token, so a Stop landing
        // before then still wins.
        let respawn_after_restart = PersistentSubprocessController::config_restart_due_locked(&inner, self.generation);
        drop(inner);
        ExitRecord { is_current_generation, effects, respawn_after_restart }
    }

    fn flush_stop_effects(&self, effects: Vec<persistent_resume::ResumeEffect>) {
        for effect in effects {
            let line = match effect {
                persistent_resume::ResumeEffect::PersistImmediately(line)
                | persistent_resume::ResumeEffect::FlushErrorLine(line) => Some(line),
                persistent_resume::ResumeEffect::FireRetry { held_error_line, .. } => held_error_line,
                persistent_resume::ResumeEffect::PublishDone => None,
                // Not actually reachable here today — the "stop
                // wins" branch of `update()`'s ConfirmedRetry +
                // ProcessExited arm never produces this effect
                // (see SPEC_AGENT_PANE_HISTORY_ALIGNMENT_2026_08_05.md
                // §2.1) — but matched defensively, same as
                // `FireRetry` above, rather than assumed. Reuses
                // the same "append this line" path as every
                // other variant here.
                persistent_resume::ResumeEffect::EmitSessionOutcome {
                    outcome,
                    attempted_sid,
                    actual_sid,
                } => Some(session_outcome_line(outcome, attempted_sid, actual_sid)),
            };
            if let Some(line) = line {
                self.append_output_line(&line);
            }
        }
    }
}

/// The CLI is gone, so its MCP servers are too (`backend::mcp_observed`),
/// unless a newer spawn already reports its own. Here, not at the reader's
/// end of input: a reader can be aborted unfinished (`settle_reader_tasks`),
/// and a descendant holding stdout open keeps it from ending at all.
fn forget_mcp_servers(owner: u64, broker: Option<&mps::Broker>, block_id: &str) {
    if crate::backend::mcp_observed::forget_owned(block_id, owner) {
        crate::backend::mcp_observed::publish(broker, block_id);
    }
}

/// Wait for the process's reader tasks once it has ended (stderr up to 500ms,
/// stdout up to 10s), aborting any that overruns. `after` names how the process
/// ended ("process exit" or "kill") in the warning.
async fn settle_reader_tasks(
    block_id: &str,
    stderr_reader_handle: Option<tokio::task::JoinHandle<()>>,
    stdout_reader_handle: tokio::task::JoinHandle<()>,
    after: &str,
) {
    if let Some(handle) = stderr_reader_handle {
        let abort_handle = handle.abort_handle();
        if tokio::time::timeout(std::time::Duration::from_millis(500), handle).await.is_err() {
            tracing::warn!(
                block_id = %block_id,
                "stderr reader did not finish within 500ms of {after} — aborting it"
            );
            abort_handle.abort();
        }
    }
    let abort_handle = stdout_reader_handle.abort_handle();
    if tokio::time::timeout(std::time::Duration::from_secs(10), stdout_reader_handle)
        .await
        .is_err()
    {
        tracing::warn!(
            block_id = %block_id,
            "stdout reader did not finish within 10s of {after} \
             (SQLite contention, or a descendant process holding stdout open?) \
             — aborting it"
        );
        abort_handle.abort();
    }
}

/// Remove the registration files a stopped spawn left for its block, under every name they are
/// filed by (slug, former slug, display-name route). An entry a turn wrote for the block's current
/// name is filed under that name, with no nonce, so the removal keyed on the spawn's own name and
/// nonce never reaches it, and the agent then reads as running to anything that reads the files
/// (the ID update's running check). Spares a newer spawn's entries: one registered since leaves
/// the block registered in memory under a nonce that isn't ours.
fn sweep_stopped_registrations(block_id: &str, nonce: u64, registration_was_ours: bool) {
    let newer_spawn_holds_block = !registration_was_ours
        && crate::backend::reactive::get_global_handler().try_get_agent_by_block(block_id).is_some();
    if newer_spawn_holds_block {
        return;
    }
    let data_dir = crate::backend::base::get_mux_data_dir();
    crate::backend::reactive::registry::remove_for_block(&data_dir, block_id, Some(nonce));
    crate::backend::reactive::registry::remove_shared_for_block_from_env(block_id, Some(nonce));
}

/// The files a stopped spawn registered under `agent_id`, removed only if they are still its own
/// (compare-and-remove on its nonce, issue #2363: a respawn's fresh entry stays).
fn remove_registration_files_by_name(agent_id: &str, nonce: u64) {
    let data_dir = crate::backend::base::get_mux_data_dir();
    crate::backend::reactive::registry::remove_if_nonce(&data_dir, agent_id, nonce);
    crate::backend::reactive::registry::remove_shared_from_env_if_nonce(agent_id, nonce);
}
