// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The persistent CLI's stdout reader task, started by `spawn_process`
//! (`spawn.rs`): control frames, session-id capture, resume-state events,
//! failure classification, the task feed and the output append.

use super::*;

/// What the stdout reader task captures from `spawn_process`.
pub(super) struct StdoutReaderCtx {
    pub(super) stdout: tokio::process::ChildStdout,
    pub(super) block_id_read: String,
    pub(super) broker_read: Option<Arc<mps::Broker>>,
    pub(super) inner_read: Arc<Mutex<PersistentInner>>,
    pub(super) mstore_read: Option<Arc<Store>>,
    pub(super) event_bus_read: Option<Arc<EventBus>>,
    pub(super) filestore_read: Option<Arc<FileStore>>,
    pub(super) health_read: Arc<TurnActivityTracker>,
    pub(super) self_ref_read: Option<std::sync::Weak<PersistentSubprocessController>>,
    pub(super) stdout_seq_read: Arc<AtomicU64>,
    pub(super) session_id_field: String,
    pub(super) my_generation_read: u64,
    pub(super) global_output_zone: Option<String>,
    pub(super) record_fence: Option<crate::backend::agent_admission::RecordFence>,
    pub(super) segment_read: super::segments::SegmentRef,
    pub(super) relocated_copy_read: Option<crate::backend::continuity_relocate::Relocated>,
    pub(super) forked_from_read: Option<String>,
}

impl PersistentSubprocessController {
    /// Body of the stdout reader task; `spawn_process` keeps its JoinHandle
    /// for the process waiter.
    pub(super) async fn run_stdout_reader(ctx: StdoutReaderCtx) {
        let StdoutReaderCtx {
            stdout,
            block_id_read,
            broker_read,
            inner_read,
            mstore_read,
            event_bus_read,
            filestore_read,
            health_read,
            self_ref_read,
            stdout_seq_read,
            session_id_field,
            my_generation_read,
            global_output_zone,
            record_fence,
            segment_read,
            mut relocated_copy_read,
            forked_from_read,
        } = ctx;
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();
        let mut stats = super::super::session_stats::SessionStatsAccumulator::new(block_id_read.clone());
        // The CLI's own `task_started`/`task_updated`/`task_notification`
        // lines drive the background-task registry, including tasks a
        // subagent owns (see `background_task_feed.rs`).
        let mut task_feed = crate::backend::background_task_feed::TaskFeed::default();

        // NOTE: OSC window-title extraction is NOT done here.
        // PersistentSubprocessController uses piped stdout with stream-json
        // NDJSON protocol. Claude Code sets window titles via process.title
        // (SetConsoleTitle on Windows; argv[0] on Unix), which does NOT
        // produce OSC escape sequences in the piped stdout stream. Inserting
        // OSC bytes into stream-json stdout would corrupt the JSON protocol.
        // block:activity events for agent panes are instead published by
        // the terminalSequence hooks path — see spec §2.5 and the future
        // SPEC_AGENT_HOOKS_TERMINAL_SEQUENCE spec.

        while let Ok(Some(line)) = lines.next_line().await {
            if line.trim().is_empty() {
                continue;
            }
            // Bump the activity counter for EVERY non-empty stdout line —
            // including control frames handled via `continue` below — so
            // the AskUserQuestion dead-air fallback can tell whether the
            // turn resumed. See `answer_question`.
            stdout_seq_read.fetch_add(1, Ordering::Relaxed);

            // Track session metadata (debounced 1 s)
            stats.record_line(line.len(), &mstore_read);

            // Set (instead of persisted immediately) when this line turns
            // out to be a terminal `result`/`is_error:true` event arriving
            // while a stale-`--resume` retry could still be confirmed for
            // this exact attempt — see `persistent_resume::ResumeState`'s
            // `held_error_line`. `false` for every other line, matching
            // today's behavior exactly.
            let mut hold_back_for_resume_retry = false;
            // Parse JSON for control-frame handling, turn-active tracking,
            // and session ID capture
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&line) {
                // Control-protocol frames (can_use_tool / AskUserQuestion) are
                // NOT conversation output — handle them and skip the blockfile
                // so the frontend stream never sees them.
                // Spec: docs/specs/SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md.
                if let Some(kind) = parsed.get("type").and_then(|v| v.as_str()) {
                    if kind == "control_request" || kind == "control_response" {
                        // The answer to our `get_context_usage`: where this
                        // process auto-compacts. Ignored from a replaced
                        // process; re-published only when it changed.
                        if let Some(usage) =
                            crate::backend::agent_context_usage::context_usage_from_control_response(&parsed)
                        {
                            let changed = {
                                let mut g = inner_read.lock().unwrap();
                                if g.spawn_generation == my_generation_read && g.context_usage.as_ref() != Some(&usage) {
                                    g.context_usage = Some(usage.clone());
                                    true
                                } else {
                                    false
                                }
                            };
                            if let (true, Some(broker)) = (changed, broker_read.as_ref()) {
                                crate::backend::agent_context_usage::publish_agent_context_usage(
                                    broker,
                                    &block_id_read,
                                    &usage,
                                );
                            }
                        }
                        // The answer to our `get_settings`: what the CLI
                        // really runs. Ignored from a replaced process.
                        if let Some(eff) = crate::backend::agent_runtime::effective_from_control_response(&parsed) {
                            let changed = {
                                let mut g = inner_read.lock().unwrap();
                                if g.spawn_generation == my_generation_read && g.spawn_runtime.is_some() {
                                    let changed = g.effective_runtime.as_ref() != Some(&eff);
                                    g.effective_runtime = Some(eff);
                                    changed
                                } else {
                                    false
                                }
                            };
                            if let (true, Some(broker)) = (changed, broker_read.as_ref()) {
                                super::status::publish_runtime_event(&inner_read, broker, &block_id_read);
                            }
                        }
                        Self::handle_control_frame(kind, &parsed, &block_id_read, &inner_read);
                        // OS notification: the agent is now blocked on the
                        // user — known here even with no pane mounted
                        // (SPEC_OS_NOTIFICATIONS_SYSTEM_2026_09_24 Phase 5).
                        if let (Some(broker), Some(asked)) = (
                            broker_read.as_ref(),
                            crate::backend::notify::sources::ask_user_question(&parsed),
                        ) {
                            if let Some(r) = crate::backend::notify::router::get(broker) {
                                r.input_waiting_nonblocking(&block_id_read, asked.text, asked.count);
                            }
                        }
                        continue;
                    }
                }
                if let Some(store) = mstore_read.as_deref() {
                    let now_ms = agentmux_common::time::now_ms();
                    task_feed.apply(
                        store,
                        broker_read.as_deref(),
                        &block_id_read,
                        &parsed,
                        now_ms,
                    );
                }
                // A pass the CLI starts by itself: a finished background
                // task's `task_notification`, then a fresh `system/init` with
                // no input from srv (turn-model spec §4.2). Recorded so the
                // agent reads busy for that pass, everywhere `turn_active`
                // is read. Ignored from a replaced process.
                if parsed.get("type").and_then(|v| v.as_str()) == Some("system") {
                    let subtype = parsed.get("subtype").and_then(|v| v.as_str());
                    let current = || inner_read.lock().unwrap().spawn_generation == my_generation_read;
                    if subtype == Some("task_notification") && current() {
                        health_read.note_cli_task_notification();
                    } else if subtype == Some("init") && current() && health_read.mark_turn_active_from_cli() {
                        if let Some(ctrl) = self_ref_read.as_ref().and_then(|w| w.upgrade()) {
                            ctrl.cli_started_pass();
                        }
                    }
                }
                // The CLI version this agent now runs, from Claude's own
                // `system/init` frame: every spawn path passes here, and
                // it is the version that actually runs, so a self-update
                // shows too. A change from the agent's last run puts a
                // notice in the pane (`backend::cli_notice`).
                if parsed.get("subtype").and_then(|v| v.as_str()) == Some("init") {
                    if let (Some(version), Some(store), Some(broker), Some(filestore)) = (
                        parsed.get("claude_code_version").and_then(|v| v.as_str()),
                        mstore_read.clone(),
                        broker_read.clone(),
                        filestore_read.clone(),
                    ) {
                        let (block_id, version) = (block_id_read.clone(), version.to_string());
                        tokio::task::spawn_blocking(move || {
                            crate::backend::cli_notice::observe_and_notify(
                                &broker, &filestore, &store, &block_id, None, "claude", &version,
                            );
                        });
                    }
                }
                // This block's CLI compacted: only its pane may send the
                // hidden memory reinjection for that boundary.
                if parsed.get("subtype").and_then(|v| v.as_str()) == Some("compact_boundary") {
                    if let Some(uuid) = parsed.get("uuid").and_then(|v| v.as_str()) {
                        crate::server::memory_delivery_handlers::record_compaction_boundary(&block_id_read, uuid);
                    }
                }
                let is_result_frame =
                    parsed.get("type").and_then(|v| v.as_str()) == Some("result");
                // Claude's turn-ending marker. Persistent mode never exits
                // between turns, so this is the only place `turn_active`
                // can go back to false without waiting for process exit —
                // see `send_message`'s matching `set_active_turn(true)`.
                if is_result_frame {
                    // A runtime-config change (model/effort/permission)
                    // arrived mid-turn and was deferred rather than
                    // killing this turn — see
                    // `request_restart_when_idle`. The turn is over now,
                    // so stop the process. Its kill arm eager-resumes the
                    // replacement at once (`respawn_after_config_restart`),
                    // which rebuilds `cli_args` from block meta, so the
                    // new flags take effect then. It used to wait for "the
                    // next message", which a jekt never is.
                    // Not `poison_resume` and not a user Stop — the
                    // session id is retained, so the respawn `--resume`s
                    // the same conversation.
                    // `set_active_turn(false)` happens under `inner` so it
                    // serializes with `request_restart_when_idle`'s own
                    // check — see that method for the interleaving this
                    // closes. `restart_pending` is committed in the SAME
                    // acquisition so no send can slip into `DeliverDirect`
                    // between the decision and the kill.
                    // Whether the deferred restart may apply: see
                    // `turn_boundary_locked`.
                    let boundary = PersistentSubprocessController::turn_boundary_locked(
                        &mut inner_read.lock().unwrap(),
                        &health_read,
                        &block_id_read,
                        my_generation_read,
                        Some(crate::backend::blockcontroller::health::PassStats::from_result_frame(&parsed)),
                    );
                    // `None`: this reader's process has been replaced. Its
                    // `result` ends nothing the current process is doing,
                    // so it must not idle or restart that process (codex
                    // P1 on #3562).
                    let boundary_is_current = boundary.is_some();
                    let deferred_restart = boundary.unwrap_or(false);
                    // The model can change under a running process (the CLI
                    // falls back to another model when one is overloaded), so
                    // ask again at every turn boundary. Cheap, and not for a
                    // process that is about to be restarted anyway.
                    if boundary_is_current && !deferred_restart {
                        let ask = inner_read.lock().unwrap().settings_readback;
                        if ask {
                            Self::push_stdin(&inner_read, crate::backend::agent_runtime::settings_request_line());
                            // The threshold moves with the model (a fallback
                            // model, a /model switch applied in place).
                            Self::push_stdin(
                                &inner_read,
                                crate::backend::agent_context_usage::context_usage_request_line(),
                            );
                        }
                    }
                    if deferred_restart {
                        tracing::info!(
                            block_id = %block_id_read,
                            "turn ended — applying the deferred runtime-config restart"
                        );
                        // Marked as a restart, not a plain stop, so the
                        // kill arm brings the replacement up itself: the
                        // next message is often a jekt, and automated
                        // delivery never spawns.
                        if let Some(ctrl) = self_ref_read.as_ref().and_then(|w| w.upgrade()) {
                            ctrl.stop_for_config_restart();
                        }
                    }
                    // Publish the flip so the Swarm view's live
                    // ControllerStatus subscription reflects "turn
                    // ended" immediately instead of only on the next
                    // unrelated status change (or process exit) — see
                    // send_message's matching publish_status() call for
                    // the turn-start side of this pair.
                    // The turn ended: any "needs your input" for it is
                    // moot (answered, or abandoned by a new message / the
                    // turn finishing). srv never drains
                    // `pending_questions` on its own, so this boundary is
                    // the reliable "resolved" signal.
                    if let (Some(broker), true) = (broker_read.as_ref(), boundary_is_current) {
                        if let Some(r) = crate::backend::notify::router::get(broker) {
                            r.resolve_nonblocking(&block_id_read, crate::backend::notify::policy::Family::Input);
                        }
                    }
                    if let (Some(broker), true) = (broker_read.as_ref(), boundary_is_current) {
                        let status = {
                            let locked = inner_read.lock().unwrap();
                            super::super::agent_runtime_status(
                                &block_id_read,
                                locked.status_version,
                                &locked.proc_status,
                                locked.proc_exit_code,
                                false,
                            )
                        };
                        super::super::publish_controller_status(broker, &status);
                    }
                    // SPEC_SUBAGENT_LIVE_RECONCILIATION_AND_RETIRE_2026_07_20
                    // Phase A: reconcile any subagent still Active for this
                    // block the instant its turn ends, not just at the next
                    // pane reopen — closes SPEC_SUBAGENT_LIFECYCLE_
                    // RECONCILIATION_2026_07_12.md's Open Question 1. A
                    // subagent runs inside the parent's own CLI process (a
                    // Task-tool call is synchronous within the parent's
                    // turn), so this is the same "turn ended" signal
                    // scan_session_subagents already reconciles against at
                    // reopen — just fired live instead of waiting.
                    // `global()` is `None` in tests that don't call
                    // `subagent_watcher::set_global` — a safe no-op, same
                    // pattern `process_tracker::registry` already uses.
                    //
                    // On a BLOCKING thread, not this one. Reconciliation
                    // used to be a pure in-memory status flip under a
                    // mutex (O(1)), but as of the completion-detection
                    // fix (#3007) it reads the parent transcript from
                    // disk — routinely tens of MB, plus per-line JSON
                    // parsing — to decide whether each member actually
                    // returned. Doing that inline would block a tokio
                    // worker thread for the whole read, and this fires on
                    // EVERY turn-end, so several blocks finishing at once
                    // could stall the runtime (reagent P1 on #3007).
                    //
                    // Fire-and-forget: the result is a status correction
                    // broadcast to whoever's listening, not something this
                    // stdout-reader task consumes, so there is nothing to
                    // await. The backfill call site (`scan_session_
                    // subagents`) needs no equivalent — it already runs in
                    // a blocking context that walks up to 200 files.
                    let session_id_snapshot = inner_read.lock().unwrap().session_id.clone();
                    if let Some(sid) = session_id_snapshot {
                        if let Some(watcher) = subagent_watcher::global() {
                            let block_id = block_id_read.clone();
                            tokio::task::spawn_blocking(move || {
                                watcher.reconcile_stale_subagents(&block_id, &sid);
                            });
                        }
                    }
                }
                // A turn WE interrupted to close the pane ends in an
                // `is_error: true` result (`error_during_execution`).
                // That is the requested stop, not a failure: treat it
                // as an ordinary end of turn — no failure banner, no
                // stale-`--resume` retry tracking (pane-close spec §9.4).
                let closing_this_generation = is_result_frame
                    && inner_read.lock().unwrap().shutdown_generation == Some(my_generation_read);
                let is_error_result = is_result_frame
                    && !closing_this_generation
                    && parsed.get("is_error").and_then(|v| v.as_bool()) == Some(true);
                // reagentx P0 on PR #2371: the real CLI's stream-json
                // protocol embeds `session_id_field` on EVERY event,
                // including the terminal `result` — so the doomed
                // attempt's own `is_error:true` line ALSO carries the
                // (stale) sid it was given. Calling
                // `try_capture_session_id` for THIS exact line would
                // resolve this generation's resume tracking (a
                // non-poisoned confirmation does — codex P1's fix,
                // needed for the genuinely-successful-resume case)
                // BEFORE the `ErrorResultLine` event below ever runs,
                // reproducing the exact bubble this PR exists to
                // suppress AND preventing PR #2360's own retry from
                // ever being confirmed. An error frame's echoed sid is
                // never genuine progress (the turn failed) — skip
                // session-id capture entirely for this exact frame;
                // every other frame type (system/init, a successful
                // result) still captures normally.
                // Set when the capture_effects loop below classifies and
                // persists a flushed OLDER turn's failure this tick — the
                // clear-on-success step further down must not immediately
                // wipe out state it just recorded (SPEC_PERSISTENT_
                // CONTROLLER_FAILURE_CLASSIFICATION_2026_08_04.md).
                let mut flushed_failure_this_tick = false;
                if !is_error_result {
                    if let Some(sid) = parsed.get(&session_id_field).and_then(|v| v.as_str()) {
                        let sid_string = sid.to_string();
                        // reagentx P0 on PR #2371: a genuinely
                        // successful terminal result (is_result_frame
                        // && !is_error, which is guaranteed true here
                        // since is_error_result already excluded the
                        // failing case) is the ONLY unambiguous proof
                        // that resuming THIS sid actually worked —
                        // any earlier frame (e.g. a "system"/init
                        // frame) echoes the same attempted sid
                        // regardless of whether the resume goes on to
                        // fail, per persistent_resume::update's own
                        // handling of `SessionCaptured`.
                        let is_confirmed_success = is_result_frame;
                        // See PersistentInner::try_capture_session_id — refuses
                        // to (re-)adopt an id the stderr reader (`stderr_reader.rs`) already
                        // confirmed unreachable, whichever task wins the race.
                        let (should_capture, capture_effects, holds_current) = {
                            let mut inner = inner_read.lock().unwrap();
                            let (adopted, effects) = inner.try_capture_session_id(&sid_string, my_generation_read, is_confirmed_success);
                            (adopted, effects, inner.holds_current_session(&sid_string, my_generation_read))
                        };
                        if should_capture {
                            tracing::info!(
                                block_id = %block_id_read,
                                session_id = %sid_string,
                                "persistent session ID captured"
                            );
                            core::persist_session_id(&block_id_read, &sid_string, &mstore_read, &event_bus_read);
                        }
                        let kept_relocated_id = super::segments::kept_relocated_id(
                            should_capture,
                            is_confirmed_success,
                            holds_current,
                            relocated_copy_read.is_some(),
                            forked_from_read.as_deref(),
                            &sid_string,
                        );
                        if should_capture {
                            super::segments::record_segment_session(&segment_read, &sid_string);
                            // The fork has its own session now; the copy it
                            // read from goes (spec §4.3 (3), I4).
                            if let Some(copy) = relocated_copy_read.take() {
                                super::segments::settle_relocated_copy(&block_id_read, &copy, forked_from_read.as_deref(), &sid_string);
                            }
                        } else if kept_relocated_id {
                            // Kept under the attempted id, the copy is the live
                            // session: recorded only while it is still this
                            // spawn's, never a replacement's at the same path.
                            if let Some(copy) = relocated_copy_read.take() {
                                if super::segments::settle_relocated_copy(&block_id_read, &copy, forked_from_read.as_deref(), &sid_string) {
                                    super::segments::record_segment_session(&segment_read, &sid_string);
                                }
                            }
                        }
                        // reagentx P0 on PR #2373: resolving tracking
                        // here can legitimately flush a held-back
                        // error line from an earlier turn on this
                        // same still-alive generation — execute it,
                        // same as every other `ResumeEffect` call
                        // site in this module. `SessionCaptured` can
                        // also now resolve tracking outright and
                        // produce an `EmitSessionOutcome` (see
                        // `persistent_resume::update`'s own handling,
                        // SPEC_AGENT_PANE_HISTORY_ALIGNMENT_2026_08_05.md
                        // §2.1) — handled explicitly below; anything
                        // else falls to the catch-all, kept exhaustive
                        // rather than assuming the effect set never
                        // grows again.
                        for effect in capture_effects {
                            match effect {
                                persistent_resume::ResumeEffect::EmitSessionOutcome {
                                    outcome,
                                    attempted_sid,
                                    actual_sid,
                                } => {
                                    // A fork reports a new id by design: that is
                                    // the resume succeeding, not a fresh start.
                                    let outcome = super::segments::forked_outcome(
                                        outcome,
                                        forked_from_read.as_deref(),
                                        &attempted_sid,
                                        actual_sid.as_deref(),
                                    );
                                    // The retry (if any led here) is now resolved one
                                    // way or the other — clear "Reconnecting…". A no-op
                                    // publish (still fine) when this outcome came from a
                                    // plain first-time resume that was never retried.
                                    publish_resume_retry_status(&broker_read, &block_id_read, "resolved");
                                    // A recovery scan can turn an
                                    // already-disclosed resume failure
                                    // into a genuine resume — retract the
                                    // banner so it can't contradict the
                                    // `resumed` divider appended just
                                    // below. See
                                    // `session_recovery::clear_resume_failed`.
                                    if matches!(outcome, persistent_resume::SessionOutcome::Resumed) {
                                        if let Some(ref store) = mstore_read {
                                            super::super::session_recovery::clear_resume_failed(
                                                store,
                                                &event_bus_read,
                                                &block_id_read,
                                            );
                                        }
                                    }
                                    if let Some(ref broker) = broker_read {
                                        let line = session_outcome_line(outcome, attempted_sid, actual_sid);
                                        super::super::shell::handle_append_block_file(
                                            broker,
                                            &block_id_read,
                                            PERSISTENT_OUTPUT_SUBJECT,
                                            line.as_bytes(),
                                            filestore_read.as_ref(),
                                            crate::backend::agent_admission::fenced_zone(global_output_zone.as_deref(), &record_fence),
                                        );
                                    }
                                }
                                persistent_resume::ResumeEffect::FlushErrorLine(line) => {
                                    if let Some(ref broker) = broker_read {
                                        super::super::shell::handle_append_block_file(
                                            broker,
                                            &block_id_read,
                                            PERSISTENT_OUTPUT_SUBJECT,
                                            line.as_bytes(),
                                            filestore_read.as_ref(),
                                            crate::backend::agent_admission::fenced_zone(global_output_zone.as_deref(), &record_fence),
                                        );
                                    }
                                    // reagentx P2 on PR #2421: this flushes
                                    // an earlier held-back turn's error,
                                    // finally confirmed final now that
                                    // session-id tracking resolved — must
                                    // get the same classify/persist/publish
                                    // treatment as the identical
                                    // FlushErrorLine handled at the
                                    // process-exit arm, or this turn's
                                    // failure silently loses its recovery
                                    // banner. No exit code exists for this
                                    // now-superseded turn.
                                    if surface_error_line(
                                        &block_id_read,
                                        None,
                                        &line,
                                        broker_read.as_deref(),
                                        &mstore_read,
                                        &event_bus_read,
                                    ) {
                                        flushed_failure_this_tick = true;
                                    }
                                }
                                other => {
                                    tracing::warn!(
                                        block_id = %block_id_read,
                                        effect = ?other,
                                        "unexpected ResumeEffect from a SessionCaptured event"
                                    );
                                }
                            }
                        }
                    }
                }
                // Issue #2368: this generation's resume tracking
                // (`persistent_resume::ResumeState`) decides whether
                // this line is still a retry candidate — if it has
                // already resolved (a session id was captured above or
                // on an earlier line, or this generation never
                // attempted an untrusted `--resume`), `update()`
                // returns a `PersistImmediately` effect and today's
                // immediate-persist behavior is unchanged; otherwise
                // the line is held back pending the retry decision at
                // process exit.
                //
                // reagentx P2 on PR #2373: hold-back is now decided
                // from the RESULTING state, not `effects.is_empty()`
                // — a still-tracking result can now ALSO carry a
                // `PersistImmediately` effect for a SUPERSEDED
                // held-back line from an earlier turn on this same
                // generation (a second `is_error:true` while tracking
                // is undecided means the first was a separate,
                // already-settled turn's error). That effect must be
                // flushed here explicitly; when NOT still tracking,
                // any effect returned is THIS exact line's own
                // `PersistImmediately`, already handled by the
                // unchanged fallthrough below — executing it here too
                // would double-persist it.
                if is_error_result {
                    let (effects, still_tracking) = {
                        let mut inner = inner_read.lock().unwrap();
                        let effects = inner.apply_resume_event(persistent_resume::ResumeEvent::ErrorResultLine {
                            generation: my_generation_read,
                            line: format!("{}\n", line),
                        });
                        let still_tracking = matches!(
                            &inner.resume,
                            persistent_resume::ResumeState::AwaitingOutcome { generation, .. }
                                if *generation == my_generation_read
                        ) || matches!(
                            &inner.resume,
                            persistent_resume::ResumeState::ConfirmedRetry { generation, .. }
                                if *generation == my_generation_read
                        );
                        (effects, still_tracking)
                    };
                    hold_back_for_resume_retry = still_tracking;
                    if still_tracking {
                        for effect in effects {
                            match effect {
                                persistent_resume::ResumeEffect::PersistImmediately(old_line)
                                | persistent_resume::ResumeEffect::FlushErrorLine(old_line) => {
                                    if let Some(ref broker) = broker_read {
                                        super::super::shell::handle_append_block_file(
                                            broker,
                                            &block_id_read,
                                            PERSISTENT_OUTPUT_SUBJECT,
                                            old_line.as_bytes(),
                                            filestore_read.as_ref(),
                                            crate::backend::agent_admission::fenced_zone(global_output_zone.as_deref(), &record_fence),
                                        );
                                    }
                                    // reagentx P1 on PR #2421 (round 2):
                                    // this is a SEPARATE, already-settled
                                    // older turn's error, superseded by
                                    // the current still-tracking line —
                                    // now confirmed final, same as the
                                    // other three FlushErrorLine/
                                    // PersistImmediately call sites this
                                    // PR wired up. `is_error_result` is
                                    // true for the rest of this tick, so
                                    // this can never collide with the
                                    // clear-on-success step below.
                                    if surface_error_line(
                                        &block_id_read,
                                        None,
                                        &old_line,
                                        broker_read.as_deref(),
                                        &mstore_read,
                                        &event_bus_read,
                                    ) {
                                        flushed_failure_this_tick = true;
                                    }
                                }
                                other => {
                                    tracing::warn!(
                                        block_id = %block_id_read,
                                        effect = ?other,
                                        "unexpected ResumeEffect from a still-tracking ErrorResultLine event"
                                    );
                                }
                            }
                        }
                    }
                }
                // Classify + surface a genuine, non-retried error result
                // (429/overloaded/auth/etc.) to the pane's failure-recovery
                // UI — SPEC_PERSISTENT_CONTROLLER_FAILURE_CLASSIFICATION.
                // Gated on `!hold_back_for_resume_retry`: when the stale-
                // `--resume` machinery above is still tracking this exact
                // error as a live retry candidate, it must stay invisible
                // to the user (per its own "must never reach the user"
                // invariant at `process_waiter.rs`'s ProcessExited/FireRetry arm) —
                // classify() only runs once this error is confirmed final.
                if is_error_result && !hold_back_for_resume_retry {
                    let failure = crate::agents::failure::classify(None, None, "", Some(&parsed));
                    surface_failure(&block_id_read, &failure, broker_read.as_deref(), &mstore_read, &event_bus_read);
                } else if is_result_frame && !is_error_result && !flushed_failure_this_tick {
                    // reagentx P1 on PR #2421: unlike host_spawn.rs, this
                    // controller never exits between turns, so nothing
                    // else ever clears a previously recorded failure —
                    // once one rate-limit/overloaded error was persisted,
                    // the pane's onMount seed logic kept re-showing that
                    // stale banner on every future reload, even after
                    // many later successful turns. A genuinely successful
                    // terminal result on this still-alive process is the
                    // signal that it's stale. persist_last_failure is a
                    // no-op when there's nothing to clear, so this is
                    // cheap on the (overwhelmingly common) already-clear
                    // path. Skipped when the capture_effects loop above
                    // just persisted a freshly-flushed OLDER failure this
                    // same tick — that state must survive, not be
                    // immediately wiped by this frame's own success.
                    core::persist_last_failure(&block_id_read, None, &mstore_read, &event_bus_read);
                    // A completed turn is when the agent's running
                    // summary may be due for an update. Background, and
                    // a no-op unless enough happened since the last one
                    // (SPEC_DURABLE_CONVERSATION_MEMORY_2026_09_23.md §4.4).
                    crate::backend::continuity_state::after_successful_turn(
                        mstore_read.clone(),
                        block_id_read.clone(),
                    );
                }
            }

            if hold_back_for_resume_retry {
                continue;
            }

            // Publish line as MPS blockfile event and write-through to FileStore
            // for persistent history (Phase 1.3).
            //
            // debug, not info: fires on EVERY output line a streaming agent
            // produces — the single largest contributor (~27%) to an
            // unrotated 406 MB launcher-log mirror on a real machine
            // (SPEC_WIN10_PAGEFILE_OOM_CRASH_2026_06_29 P1). Default
            // production filter is info, so this is now suppressed unless
            // RUST_LOG=debug is set.
            tracing::debug!(
                block_id = %block_id_read,
                line_len = line.len(),
                "persistent stdout → blockfile"
            );
            if let Some(ref broker) = broker_read {
                super::super::shell::append_output_line(
                    broker,
                    &block_id_read,
                    &line,
                    filestore_read.as_ref(),
                    crate::backend::agent_admission::fenced_zone(global_output_zone.as_deref(), &record_fence),
                );
            } else {
                tracing::warn!(block_id = %block_id_read, "persistent stdout: no broker available");
            }
        }

        tracing::info!(block_id = %block_id_read, "persistent stdout reader finished");
    }
}
