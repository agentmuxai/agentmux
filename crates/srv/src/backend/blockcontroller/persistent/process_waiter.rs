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
        } = ctx;
        tokio::select! {
            status = child.wait() => {
                let exit_code = status.map(|s| s.code().unwrap_or(-1)).unwrap_or(-1);
                super::segments::record_segment_end(
                    &segment_wait,
                    crate::backend::continuity_segments::exit_reason(exit_code),
                    crate::backend::agent_admission::fenced_zone(global_output_zone_wait.as_deref(), &record_fence_wait),
                );
                tracing::info!(
                    block_id = %block_id_wait,
                    exit_code = exit_code,
                    "persistent process exited"
                );

                // Give the stderr reader a bounded chance to fully
                // drain and react to whatever it saw right before this
                // process exited — codex P1/P2 on PR #2360 (second
                // review pass): `child.wait()` resolving does NOT mean
                // the stderr reader (an independently-scheduled task)
                // has already called `poison_resume` for a "No
                // conversation found" line, or finished ITS OWN
                // subsequent `persist_session_id("")` call. Without
                // this, two failure modes were possible: (1) this task
                // could resolve the resume state (`ProcessExited`) below
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
                settle_reader_tasks(&block_id_wait, stderr_reader_handle, stdout_reader_handle, "process exit").await;

                // Wait (briefly, bounded) for the drain to finish
                // appending whatever message it's currently
                // mid-delivery on before deciding the retry batch
                // below is final — reagentx P1 on PR #2360 (sixth
                // review pass, round 9): `drain_queue_after_
                // successful_spawn`'s own "send, then append to the
                // retry batch" sequence has an unavoidable gap at the
                // `.await` (a mutex can't be held across it). Without
                // this wait, the `.take()` below could run in that
                // exact gap and dispatch a retry missing a message
                // the doomed process's channel had ALREADY accepted —
                // it stays marked "accepted" and gets persisted, but
                // is never actually delivered to any process again.
                // Same 500ms bound as the stderr-reader wait above,
                // for the same "best effort, don't hang forever"
                // reason.
                for _ in 0..50 {
                    if !inner_wait.lock().unwrap().drain_send_in_flight {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }

                let mut inner = inner_wait.lock().unwrap();
                // reagentx P1 (round 6 on PR #2373, extended round 8):
                // this belated exit-handling can run AFTER a fresh
                // spawn has already superseded this generation (see
                // `respawn_once_for_leftover_queue`'s own doc comment
                // and the kill arm below for the documented race that
                // makes this reachable) — `inner.spawn_generation` is
                // bumped on every NEW spawn, so a mismatch here means
                // this exit is for an already-superseded generation.
                // Everything gated below belongs to THIS exact
                // process (its own pid/exit code/stdin/kill channel,
                // the shared `proc_status` this process last knew to
                // be true, its own health-monitor/muxbus/registry/
                // session-recovery registration) — running any of it
                // unconditionally would corrupt or tear down a newer,
                // actively-running generation's own state as if IT
                // had exited. reagentx round 8: the round-6 fix only
                // gated the field writes above, missing
                // `health_wait.set_exited` (a shared `TurnActivityTracker`
                // across generations) and the deregistration block
                // below — both keyed by `block_id`/`agent_id`, not
                // generation, so a stale exit incorrectly marked a
                // live process's health as exited and tore down its
                // muxbus/registry/session-recovery registration while
                // it kept running.
                let is_current_generation = inner.spawn_generation == my_generation_wait;
                if is_current_generation {
                    inner.proc_exit_code = exit_code;
                    inner.current_pid = None;
                    inner.stdin_tx = None;
                    inner.kill_tx = None;
                    // This process no longer drives the agent: release
                    // its single-live-instance lease (on the last `Arc`).
                    inner.agent_lease = None;
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
                    .apply_resume_event(persistent_resume::ResumeEvent::ProcessExited { generation: my_generation_wait });
                if is_current_generation {
                    Self::set_status(&mut inner, STATUS_DONE);
                }
                // A committed config restart whose process exited on its
                // own before the kill was consumed: this arm won the
                // `select!`, so it must finish the restart too (codex P1 on
                // #3990). Acted on at the end of this arm, like the kill
                // arm; a stale-resume retry spawning first supersedes it.
                let respawn_after_restart = Self::config_restart_due_locked(&inner, my_generation_wait);
                drop(inner);
                // The process is gone: say so, whatever else this arm publishes.
                if is_current_generation {
                    if let Some(b) = broker_wait.as_ref() {
                        super::status::publish_runtime_event(&inner_wait, b, &block_id_wait);
                    }
                }

                if is_current_generation {
                    // The process is gone — nothing is waiting on the user
                    // any more (Phase 5 notifications).
                    if let Some(r) = broker_wait.as_ref().and_then(crate::backend::notify::router::get) {
                        r.resolve_nonblocking(&block_id_wait, crate::backend::notify::policy::Family::Input);
                    }
                    // Notify health monitor so Stalled/Dead watchdog stops.
                    health_wait.set_exited(exit_code);

                    // Deregister from muxbus so later sends fall through to the
                    // lower tiers instead of resolving to a dead block. Mirrors
                    // the shell controller's exit path. This exact process's
                    // resources are gone either way; if a retry/fallback
                    // respawn has ALREADY re-registered, the guards below
                    // leave its fresh registration in place.
                    // All removals are compare-and-remove keyed on this
                    // spawn's process-wide registration nonce (issue
                    // #2363): the `is_current_generation` gate above
                    // was read once, and a fallback/retry respawn's
                    // fresh registration can land on a parallel task
                    // between that read and these calls — an
                    // unconditional removal here would wipe the NEW
                    // spawn's entry with nothing left to re-register
                    // it. Nonce, not generation: a replacement
                    // controller's spawn restarts at generation 1 and
                    // could collide with ours (codex P1 on PR #2500).
                    let registration_was_ours = crate::backend::reactive::get_global_handler()
                        .unregister_block_if_nonce(&block_id_wait, nonce_wait);
                    if let Some(ref agent_id) = agent_id_wait {
                        let data_dir = crate::backend::base::get_mux_data_dir();
                        crate::backend::reactive::registry::remove_if_nonce(
                            &data_dir,
                            agent_id,
                            nonce_wait,
                        );
                        crate::backend::reactive::registry::remove_shared_from_env_if_nonce(
                            agent_id,
                            nonce_wait,
                        );
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

                    // Clear active pid — clean exit, no recovery needed.
                    // Compare-and-clear (issue #2363): only if the
                    // recorded pid is still THIS process's — a fallback
                    // respawn may have re-registered a fresh pid on a
                    // parallel task between the generation gate above
                    // and this call.
                    if let Some(ref mstore) = mstore_wait {
                        super::super::session_recovery::clear_active_pid_if_pid(mstore, &block_id_wait, pid_wait);
                    }
                }

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
                        // below hasn't launched yet.
                        persistent_resume::ResumeEffect::EmitSessionOutcome {
                            outcome,
                            attempted_sid,
                            actual_sid,
                        } => {
                            publish_resume_retry_status(&broker_wait, &block_id_wait, "resolved");
                            // Same retraction as the stdout-reader site
                            // (`stdout_reader.rs`) — this arm reaches `Fresh` today, but
                            // the clear is keyed on the outcome rather
                            // than on which arm produced it, so it stays
                            // correct if that ever changes.
                            if matches!(outcome, persistent_resume::SessionOutcome::Resumed) {
                                if let Some(ref store) = mstore_wait {
                                    super::super::session_recovery::clear_resume_failed(
                                        store,
                                        &event_bus_wait,
                                        &block_id_wait,
                                    );
                                }
                            }
                            if let Some(ref broker) = broker_wait {
                                let line = session_outcome_line(outcome, attempted_sid, actual_sid);
                                super::super::shell::handle_append_block_file(
                                    broker,
                                    &block_id_wait,
                                    PERSISTENT_OUTPUT_SUBJECT,
                                    line.as_bytes(),
                                    filestore_wait.as_ref(),
                                    crate::backend::agent_admission::fenced_zone(global_output_zone_wait.as_deref(), &record_fence_wait),
                                );
                            }
                        }
                        persistent_resume::ResumeEffect::PersistImmediately(line)
                        | persistent_resume::ResumeEffect::FlushErrorLine(line) => {
                            // Safety net: a "Reconnecting…" left showing from
                            // an earlier retry attempt on this same block must
                            // not get stuck forever just because THIS exit
                            // ended up genuinely non-retryable — a harmless
                            // no-op publish in the (common) case where no
                            // retry was in flight at all.
                            publish_resume_retry_status(&broker_wait, &block_id_wait, "resolved");
                            if let Some(ref broker) = broker_wait {
                                super::super::shell::handle_append_block_file(
                                    broker,
                                    &block_id_wait,
                                    PERSISTENT_OUTPUT_SUBJECT,
                                    line.as_bytes(),
                                    filestore_wait.as_ref(),
                                    crate::backend::agent_admission::fenced_zone(global_output_zone_wait.as_deref(), &record_fence_wait),
                                );
                            }
                            // Classify + surface this exit's error to the
                            // pane's failure-recovery UI —
                            // SPEC_PERSISTENT_CONTROLLER_FAILURE_CLASSIFICATION.
                            // Reached only for PersistImmediately/FlushErrorLine
                            // — the resume machinery has already decided this
                            // exit is NOT being silently retried (contrast
                            // FireRetry below, which must stay invisible to
                            // the user).
                            surface_error_line(
                                &block_id_wait,
                                Some(exit_code),
                                &line,
                                broker_wait.as_deref(),
                                &mstore_wait,
                                &event_bus_wait,
                            );
                        }
                        persistent_resume::ResumeEffect::FireRetry { retry, held_error_line, attempted_sid } => {
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
                            if let Some(ctrl) = self_ref_wait.upgrade() {
                                tracing::warn!(
                                    block_id = %block_id_wait,
                                    "stale --resume session id caused this exit — retrying now (find_recovery_session_id may still resume a real, on-disk session rather than starting blank)"
                                );
                                ctrl.retry_after_resume_failure(
                                    my_generation_wait,
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
                                publish_resume_retry_status(&broker_wait, &block_id_wait, "resolved");
                                if let Some(line) = held_error_line {
                                // The controller itself is already gone
                                // (weak ref invalidated) — nothing can
                                // retry this batch at all, so flush
                                // directly via this task's own captured
                                // broker/filestore rather than going
                                // through `ctrl`.
                                if let Some(ref broker) = broker_wait {
                                    super::super::shell::handle_append_block_file(
                                        broker,
                                        &block_id_wait,
                                        PERSISTENT_OUTPUT_SUBJECT,
                                        line.as_bytes(),
                                        filestore_wait.as_ref(),
                                        crate::backend::agent_admission::fenced_zone(global_output_zone_wait.as_deref(), &record_fence_wait),
                                    );
                                }
                                }
                            }
                        }
                        persistent_resume::ResumeEffect::PublishDone => {
                            if let Some(ref broker) = broker_wait {
                                let status = super::super::agent_runtime_status(
                                    &block_id_wait,
                                    0,
                                    STATUS_DONE,
                                    exit_code,
                                    false,
                                );
                                super::super::publish_controller_status(broker, &status);
                            }
                        }
                    }
                }

                if respawn_after_restart {
                    if let Some(ctrl) = self_ref_wait.upgrade() {
                        ctrl.respawn_after_config_restart(my_generation_wait);
                    }
                }
            }
            Ok(request) = kill_rx => {
                let graceful_deadline = match request {
                    KillRequest::Force => None,
                    KillRequest::Graceful(deadline) => Some(deadline),
                };
                // Why it's going away, read before the kill changes
                // anything: `shutdown` marks a pane close for this
                // generation; `restart_pending` a runtime-config restart.
                let segment_end_reason = {
                    let inner = inner_wait.lock().unwrap();
                    crate::backend::continuity_segments::kill_reason(
                        inner.shutdown_generation == Some(my_generation_wait),
                        inner.restart_pending,
                    )
                };
                tracing::info!(
                    block_id = %block_id_wait,
                    force = graceful_deadline.is_none(),
                    "persistent process kill requested"
                );
                if let Some(deadline) = graceful_deadline {
                    // Graceful: drop stdin to send EOF, then wait briefly
                    {
                        let mut inner = inner_wait.lock().unwrap();
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
                        // clear together below, after child.kill()
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
                            let _ = child.kill().await;
                            true
                        }
                    };
                    inner_wait.lock().unwrap().stop_exit = Some((my_generation_wait, killed));
                } else {
                    let _ = child.kill().await;
                    inner_wait.lock().unwrap().stop_exit = Some((my_generation_wait, true));
                }
                super::segments::record_segment_end(
                    &segment_wait,
                    segment_end_reason,
                    crate::backend::agent_admission::fenced_zone(global_output_zone_wait.as_deref(), &record_fence_wait),
                );

                // Mirror the child.wait() arm's bounded await+abort of
                // both reader tasks (above) before resolving the resume
                // state (`StopRequested` + `ProcessExited`) below
                // (#2371). Without this, a stop racing the doomed
                // attempt's in-flight terminal error line could resolve
                // the state before the stdout reader holds the line
                // back; the lagging reader would then find `NotTracking`
                // and persist the line on its own, instead of it being
                // flushed below as part of this stop.
                // codex P1 on PR #2371 (round 2): a bounded wait, not
                // an unconditional one — see the child.wait() arm's
                // identical comment above for the full reasoning. This
                // matters MORE here: codex flagged that every
                // remaining cleanup step (clearing current_pid/
                // kill_tx, STATUS_DONE, muxbus deregistration) runs
                // AFTER this await, so an unbounded hang here would
                // leave a user-initiated Stop making the controller
                // appear permanently alive if a descendant process
                // ever holds the stdout descriptor open.
                settle_reader_tasks(&block_id_wait, stderr_reader_handle, stdout_reader_handle, "kill").await;

                let mut inner = inner_wait.lock().unwrap();
                // reagentx P1 (round 6 on PR #2373, extended round 8):
                // same reasoning as the child.wait() arm above — this
                // graceful-stop cleanup can itself run AFTER the
                // documented race just above (dropping `stdin_tx`
                // early to trigger EOF lets
                // `respawn_once_for_leftover_queue` spawn a brand-new
                // generation while this kill is still mid-flight) has
                // already superseded this generation. Everything
                // gated below belongs to THIS exact kill (its own
                // pid/exit code/stdin/kill channel, its own spawn
                // claim/queue, the shared `proc_status` this stop
                // last knew to be true, its own health-monitor/
                // muxbus/registry/session-recovery registration) —
                // running any of it unconditionally would corrupt or
                // tear down a newer, actively-running generation's
                // own state as if IT had been stopped.
                let is_current_generation = inner.spawn_generation == my_generation_wait;
                if is_current_generation {
                    inner.proc_exit_code = -1;
                    inner.current_pid = None;
                    inner.stdin_tx = None;
                    inner.kill_tx = None;
                    // This process no longer drives the agent: release
                    // its single-live-instance lease (on the last `Arc`).
                    inner.agent_lease = None;
                }
                // A user-initiated kill overrides any resume-retry
                // decision in flight, for a REUSED controller instance
                // (`resync_controller` can reuse the same instance
                // across a kill+restart cycle) the same way an
                // in-flight Stop already does for the child.wait() arm
                // — reusing that exact `StopRequested` + `ProcessExited`
                // event pair here instead of duplicating the "stop
                // wins" logic against raw fields. codex P2 on PR #2371:
                // this also means a stashed error line is never
                // silently lost on a user-initiated stop (it may be a
                // genuine error the stop itself interrupted) — it's
                // flushed below via the effects this produces, same as
                // the "genuinely done" case. Safe to call
                // unconditionally even for a superseded generation —
                // `update()`'s own generation-matching arms already
                // no-op a stale event on their own.
                inner.apply_resume_event(persistent_resume::ResumeEvent::StopRequested {
                    generation: my_generation_wait,
                });
                let effects = inner
                    .apply_resume_event(persistent_resume::ResumeEvent::ProcessExited { generation: my_generation_wait });
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
                // above — a NEWER generation's own claim/queue must
                // never be cleared by this stale one's cleanup.
                if is_current_generation {
                    inner.pending_send_messages.clear();
                    inner.spawning_in_progress = false;
                    Self::set_status(&mut inner, STATUS_DONE);
                }
                // This kill was a deferred config restart of THIS
                // generation (`config_restart_due_locked`). Acted on at the
                // end of this arm, after the deregistration below, because
                // the replacement registers itself afresh. Only read here:
                // the spawn claim consumes the token, so a Stop landing
                // before then still wins.
                let respawn_after_restart = Self::config_restart_due_locked(&inner, my_generation_wait);
                drop(inner);
                // The process is gone: say so, whatever else this arm publishes.
                if is_current_generation {
                    if let Some(b) = broker_wait.as_ref() {
                        super::status::publish_runtime_event(&inner_wait, b, &block_id_wait);
                    }
                }

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
                publish_resume_retry_status(&broker_wait, &block_id_wait, "resolved");

                // Flush a held-back error line now, if the resume
                // state machine produced one — the `StopRequested`
                // sent above guarantees `ProcessExited` resolves via
                // the "stop wins" branch (see `persistent_resume::
                // update`), so `FireRetry` is never actually possible
                // here, but it's still matched defensively rather
                // than assumed.
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
                    if let Some(ref broker) = broker_wait {
                        super::super::shell::handle_append_block_file(
                            broker,
                            &block_id_wait,
                            PERSISTENT_OUTPUT_SUBJECT,
                            line.as_bytes(),
                            filestore_wait.as_ref(),
                            crate::backend::agent_admission::fenced_zone(global_output_zone_wait.as_deref(), &record_fence_wait),
                        );
                    }
                    }
                }

                if is_current_generation {
                    // Notify the turn-activity tracker of the exit —
                    // shared `Arc<TurnActivityTracker>` across
                    // generations, gated the same way as the
                    // child.wait() arm above.
                    health_wait.set_exited(-1);

                    // Deregister from muxbus (see the clean-exit arm above).
                    // All removals are compare-and-remove keyed on this
                    // spawn's process-wide registration nonce (issue
                    // #2363): the `is_current_generation` gate above
                    // was read once, and a fallback/retry respawn's
                    // fresh registration can land on a parallel task
                    // between that read and these calls — an
                    // unconditional removal here would wipe the NEW
                    // spawn's entry with nothing left to re-register
                    // it. Nonce, not generation: a replacement
                    // controller's spawn restarts at generation 1 and
                    // could collide with ours (codex P1 on PR #2500).
                    let registration_was_ours = crate::backend::reactive::get_global_handler()
                        .unregister_block_if_nonce(&block_id_wait, nonce_wait);
                    if let Some(ref agent_id) = agent_id_wait {
                        let data_dir = crate::backend::base::get_mux_data_dir();
                        crate::backend::reactive::registry::remove_if_nonce(
                            &data_dir,
                            agent_id,
                            nonce_wait,
                        );
                        crate::backend::reactive::registry::remove_shared_from_env_if_nonce(
                            agent_id,
                            nonce_wait,
                        );
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

                    // Clear active pid — user-initiated stop, no recovery needed.
                    // Compare-and-clear (issue #2363), same as the
                    // clean-exit arm: the `is_current_generation` gate
                    // above was read once under the lock, and a fallback
                    // respawn's re-registration can land between that
                    // read and this call.
                    if let Some(ref mstore) = mstore_wait {
                        super::super::session_recovery::clear_active_pid_if_pid(mstore, &block_id_wait, pid_wait);
                    }
                }

                if respawn_after_restart {
                    if let Some(ctrl) = self_ref_wait.upgrade() {
                        ctrl.respawn_after_config_restart(my_generation_wait);
                    }
                }
            }
        }
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
