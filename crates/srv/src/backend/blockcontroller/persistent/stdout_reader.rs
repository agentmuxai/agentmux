// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The persistent CLI's stdout reader task, started by `spawn_process`
//! (`spawn.rs`): control frames, session-id capture, resume-state events,
//! API retry state, the API's reachability, failure classification, the
//! task feed and the output append.

use super::*;

// What the reader does when the loop guard says the agent is stuck. Declared here, not in `mod.rs`, which is
// at its size ratchet.
#[path = "loop_stop.rs"]
pub mod loop_stop;

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
    /// This spawn's process-wide registration nonce: whose MCP report this
    /// reader records (`backend::mcp_observed`).
    pub(super) mcp_owner_read: u64,
    pub(super) global_output_zone: Option<String>,
    pub(super) record_fence: Option<crate::backend::agent_admission::RecordFence>,
    pub(super) segment_read: super::segments::SegmentRef,
    pub(super) relocated_copy_read: Option<crate::backend::continuity_relocate::Relocated>,
    pub(super) forked_from_read: Option<String>,
    /// The user turns the stdin writer sent (`backend::cli_native::sent`).
    pub(super) sent_prompts: crate::backend::cli_native::sent::SentPrompts,
    /// The model API this pane's CLI calls, which its frames report on (`connectivity`).
    pub(super) model_api: crate::connectivity::Service,
}

/// The stdout reader task: what its context carries, and what it builds up across lines.
struct StdoutReader {
    block_id_read: String,
    broker_read: Option<Arc<mps::Broker>>,
    inner_read: Arc<Mutex<PersistentInner>>,
    mstore_read: Option<Arc<Store>>,
    event_bus_read: Option<Arc<EventBus>>,
    filestore_read: Option<Arc<FileStore>>,
    health_read: Arc<TurnActivityTracker>,
    self_ref_read: Option<std::sync::Weak<PersistentSubprocessController>>,
    stdout_seq_read: Arc<AtomicU64>,
    session_id_field: String,
    my_generation_read: u64,
    mcp_owner_read: u64,
    global_output_zone: Option<String>,
    record_fence: Option<crate::backend::agent_admission::RecordFence>,
    segment_read: super::segments::SegmentRef,
    relocated_copy_read: Option<crate::backend::continuity_relocate::Relocated>,
    forked_from_read: Option<String>,
    model_api: crate::connectivity::Service,
    stats: super::super::session_stats::SessionStatsAccumulator,
    task_feed: crate::backend::background_task_feed::TaskFeed,
    api_retry: crate::backend::agent_api_retry::ApiRetryReader,
    parked_notices_due: bool,
    login_failed_in_band: bool,
    cli_native: crate::backend::cli_native::reader::CliNativeReader,
    /// What this turn has done, for the loop guard (`agents::loop_guard`).
    loop_guard: crate::agents::loop_guard::LoopGuard,
    /// The guard ended this turn, and why: the interrupt's result frame is our stop, not a failure.
    ended_by_guard: Option<String>,
}

/// What one parsed stdout line decided for the loop.
#[derive(Default)]
struct FrameOutcome {
    /// A control frame: handled, and not conversation output.
    skip: bool,
    hold_back_for_resume_retry: bool,
    release_parked_notices: bool,
    /// Why we ended this result frame's turn (the loop guard, the spend cap): the frame is shown with that
    /// text, not the CLI's generic error.
    ended_because: Option<String>,
}

/// What a frame says about its turn, before the turn-end work.
struct FrameVerdict {
    is_result_frame: bool,
    inband_error: &'static str,
    early_failure: Option<crate::agents::failure::AgentFailure>,
}

impl PersistentSubprocessController {
    /// Body of the stdout reader task; `spawn_process` keeps its JoinHandle
    /// for the process waiter.
    pub(super) async fn run_stdout_reader(ctx: StdoutReaderCtx) {
        let (stdout, mut reader) = StdoutReader::from_ctx(ctx);
        let mut lines = BufReader::new(stdout).lines();

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
            reader.handle_line(line);
        }

        tracing::info!(block_id = %reader.block_id_read, "persistent stdout reader finished");
    }
}

impl StdoutReader {
    fn from_ctx(ctx: StdoutReaderCtx) -> (tokio::process::ChildStdout, Self) {
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
            mcp_owner_read,
            global_output_zone,
            record_fence,
            segment_read,
            relocated_copy_read,
            forked_from_read,
            sent_prompts,
            model_api,
        } = ctx;
        let stats = super::super::session_stats::SessionStatsAccumulator::new(block_id_read.clone());
        // The CLI's own `task_started`/`task_updated`/`task_notification`
        // lines drive the background-task registry, including tasks a
        // subagent owns (see `background_task_feed.rs`).
        let task_feed = crate::backend::background_task_feed::TaskFeed::default();
        // Clears its retry when dropped, so also when this task is aborted.
        let api_retry =
            crate::backend::agent_api_retry::ApiRetryReader::new(&block_id_read, my_generation_read, broker_read.clone());
        // Messages other agents send this one through the CLI's own messaging, and the
        // user turns the CLI replays because it is launched with `--replay-user-messages`
        // (`backend::cli_native`).
        let cli_native = crate::backend::cli_native::reader::CliNativeReader::new(&block_id_read, sent_prompts);
        let reader = Self {
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
            mcp_owner_read,
            global_output_zone,
            record_fence,
            segment_read,
            relocated_copy_read,
            forked_from_read,
            model_api,
            stats,
            task_feed,
            api_retry,
            // Set by a compaction boundary: the memory card the hook finished
            // before the boundary was written waits for the first line after the
            // boundary that isn't the CLI's own user-turn text (its summary, a
            // command's output), and is written just ahead of it
            // (SPEC_CONTEXT_DELIVERY_2026_09_30.md §9 C2).
            parked_notices_due: false,
            // The turn's synthetic assistant message said the login failed; its result frame follows.
            login_failed_in_band: false,
            cli_native,
            loop_guard: Default::default(),
            ended_by_guard: None,
        };
        (stdout, reader)
    }

    /// `line` rewritten when it is a `result` frame from an agent on a model
    /// route: the harness prices its tokens with Claude's rates, which on a
    /// route are wrong by orders of magnitude, so the provider's own session
    /// total replaces both totals the frame reports (#30143). `None` for any
    /// other frame, and for an agent whose responses never carried a charge.
    fn route_cost_frame(&self, line: &str, parsed: &serde_json::Value) -> Option<String> {
        if parsed.get("type").and_then(serde_json::Value::as_str) != Some("result") {
            return None;
        }
        let mstore = self.mstore_read.as_ref()?;
        let agent = crate::identity::resolver::block_agent_id(mstore, &self.block_id_read)?;
        crate::backend::model_gateway::spend::rewrite_result_total(line, &agent)
    }

    /// Settles a model change waiting on its first turn with the `result` frame that ended it; whether the change was
    /// undone, which needs the harness restarted on the old place.
    fn settle_route_trial(&self, parsed: &serde_json::Value) -> bool {
        // A replaced process's late frame ends nothing the current one is doing.
        if self.inner_read.lock().unwrap().spawn_generation != self.my_generation_read {
            return false;
        }
        let Some(mstore) = self.mstore_read.as_ref() else { return false };
        let Some(agent) = crate::identity::resolver::block_agent_id(mstore, &self.block_id_read) else { return false };
        crate::backend::model_gateway::trial::settle(mstore, &agent, &self.block_id_read, parsed).is_some()
    }

    /// One line of the CLI's stdout.
    fn handle_line(&mut self, line: String) {
        if line.trim().is_empty() {
            return;
        }
        // A turn the CLI found cut off with a tool call pending (we
        // interrupted it at a restart or an upgrade) is told plainly, not
        // as the CLI's raw diagnostic; the diagnostic stays in the log.
        let mut line = match crate::agents::failure::plain_cut_off(&line) {
            Some((diagnostic, plain)) => {
                tracing::info!(block_id = %self.block_id_read, diagnostic = %diagnostic, "a turn was cut off mid-tool; shown as an interrupted turn");
                plain
            }
            None => line,
        };
        // A replayed user turn is not output of the conversation: the echo of a turn srv wrote is
        // recorded by srv and the pane already, and a peer message is audited here instead. The
        // CLI's own replayed frames (a local command's output) stay.
        if line.contains("\"isReplay\"") {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
                use crate::backend::cli_native::reader::Disposition;
                if self.cli_native.observe(&v, &crate::backend::cli_native::live::LiveEnv) == Disposition::Drop {
                    return;
                }
            }
        }
        // Bump the activity counter for EVERY non-empty stdout line —
        // including control frames, which `handle_frame` skips — so
        // the AskUserQuestion dead-air fallback can tell whether the
        // turn resumed. See `answer_question`.
        self.stdout_seq_read.fetch_add(1, Ordering::Relaxed);
        self.health_read.note_output();

        // Track session metadata (debounced 1 s)
        self.stats.record_line(line.len(), &self.mstore_read);

        // Set (instead of persisted immediately) when this line turns
        // out to be a terminal `result`/`is_error:true` event arriving
        // while a stale-`--resume` retry could still be confirmed for
        // this exact attempt — see `persistent_resume::ResumeState`'s
        // `held_error_line`. `false` for every other line, matching
        // today's behavior exactly.
        let mut outcome = FrameOutcome::default();
        // Parse JSON for control-frame handling, turn-active tracking,
        // and session ID capture
        if let Ok(mut parsed) = serde_json::from_str::<serde_json::Value>(&line) {
            if let Some(rewritten) = self.route_cost_frame(&line, &parsed) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&rewritten) {
                    parsed = v;
                    line = rewritten;
                }
            }
            outcome = self.handle_frame(&line, &parsed);
            if outcome.skip {
                return;
            }
            if let Some(why) = outcome.ended_because.take() { line = loop_stop::plain_end_frame(&line, &why); }
        }

        if outcome.hold_back_for_resume_retry {
            return;
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
            block_id = %self.block_id_read,
            line_len = line.len(),
            "persistent stdout → blockfile"
        );
        if let Some(ref broker) = self.broker_read {
            if outcome.release_parked_notices {
                for notice in crate::backend::memory::delivery_state::take_parked_notices(&self.block_id_read) {
                    crate::backend::transcript::append::append_output_line(
                        broker,
                        &self.block_id_read,
                        &notice,
                        self.filestore_read.as_ref(),
                        crate::backend::agent_admission::fenced_zone(self.global_output_zone.as_deref(), &self.record_fence),
                    );
                }
            }
            crate::backend::transcript::append::append_output_line(
                broker,
                &self.block_id_read,
                &line,
                self.filestore_read.as_ref(),
                crate::backend::agent_admission::fenced_zone(self.global_output_zone.as_deref(), &self.record_fence),
            );
        } else {
            tracing::warn!(block_id = %self.block_id_read, "persistent stdout: no broker available");
        }
    }

    /// Everything a parsed line sets in motion: control frames, observers, turn end, session-id
    /// capture, resume tracking and failure classification.
    fn handle_frame(&mut self, line: &str, parsed: &serde_json::Value) -> FrameOutcome {
        self.cli_native.observe(parsed, &crate::backend::cli_native::live::LiveEnv);
        // Control-protocol frames (can_use_tool / AskUserQuestion) are
        // NOT conversation output — handle them and skip the blockfile
        // so the frontend stream never sees them.
        // Spec: docs/specs/SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md.
        if let Some(kind) = parsed.get("type").and_then(|v| v.as_str()) {
            if kind == "control_request" || kind == "control_response" {
                self.observe_control_frame(kind, parsed);
                return FrameOutcome { skip: true, ..FrameOutcome::default() };
            }
        }
        let release_parked_notices = self.observe_frame(parsed);
        let FrameVerdict { is_result_frame, inband_error, early_failure } = self.frame_verdict(parsed);
        let ended_because = self.watch_for_loops(parsed, is_result_frame).or_else(|| (is_result_frame && loop_stop::take_cap_end(&self.block_id_read)).then(|| loop_stop::CAP_ENDED.to_string()));
        // Claude's turn-ending marker. Persistent mode never exits
        // between turns, so this is the only place `turn_active`
        // can go back to false without waiting for process exit —
        // see `send_message`'s matching `set_active_turn(true)`.
        if is_result_frame {
            self.on_turn_end(parsed);
        }
        // A turn WE interrupted to close the pane ends in an
        // `is_error: true` result (`error_during_execution`).
        // That is the requested stop, not a failure: treat it
        // as an ordinary end of turn — no failure banner, no
        // stale-`--resume` retry tracking (pane-close spec §9.4).
        let closing_this_generation = is_result_frame
            && self.inner_read.lock().unwrap().shutdown_generation == Some(self.my_generation_read);
        let is_error_result = is_result_frame
            && !closing_this_generation
            && ended_because.is_none()
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
        // BEFORE the `ErrorResultLine` event (`track_error_result`) ever runs,
        // reproducing the exact bubble this PR exists to
        // suppress AND preventing PR #2360's own retry from
        // ever being confirmed. An error frame's echoed sid is
        // never genuine progress (the turn failed) — skip
        // session-id capture entirely for this exact frame;
        // every other frame type (system/init, a successful
        // result) still captures normally.
        // Set when `apply_capture_effects` classifies and
        // persists a flushed OLDER turn's failure this tick — the
        // clear-on-success step (`surface_turn_outcome`) must not immediately
        // wipe out state it just recorded (SPEC_PERSISTENT_
        // CONTROLLER_FAILURE_CLASSIFICATION_2026_08_04.md).
        let mut flushed_failure_this_tick = false;
        if !is_error_result {
            self.capture_session_id(parsed, is_result_frame, &mut flushed_failure_this_tick);
        }
        let hold_back_for_resume_retry =
            is_error_result && self.track_error_result(line, &mut flushed_failure_this_tick);
        self.surface_turn_outcome(
            parsed,
            is_result_frame,
            is_error_result,
            hold_back_for_resume_retry,
            early_failure.is_none(),
            inband_error,
            flushed_failure_this_tick,
        );
        FrameOutcome { skip: false, hold_back_for_resume_retry, release_parked_notices, ended_because: ended_because.filter(|_| parsed.get("is_error").and_then(|v| v.as_bool()) == Some(true)) }
    }

    /// Feeds the frame to the loop guard and ends the turn when it says the agent is stuck. `true` for the
    /// result frame of a turn the guard ended: what it was stuck doing.
    fn watch_for_loops(&mut self, parsed: &serde_json::Value, is_result_frame: bool) -> Option<String> {
        if is_result_frame {
            self.loop_guard.new_turn();
            return self.ended_by_guard.take();
        }
        if self.ended_by_guard.is_some() {
            return None;
        }
        if let Some(stuck) = self.loop_guard.observe(parsed) {
            self.ended_by_guard = loop_stop::end_the_turn(&self.inner_read, self.my_generation_read, &self.block_id_read, self.mstore_read.as_ref(), &stuck);
        }
        None
    }

    /// The answers to our own control requests, and the question the agent is blocked on.
    fn observe_control_frame(&mut self, kind: &str, parsed: &serde_json::Value) {
        // The answer to our `get_context_usage`: where this
        // process auto-compacts. Ignored from a replaced
        // process; re-published only when it changed.
        if let Some(usage) =
            crate::backend::agent_context_usage::context_usage_from_control_response(parsed)
        {
            let changed = {
                let mut g = self.inner_read.lock().unwrap();
                if g.spawn_generation == self.my_generation_read && g.context_usage.as_ref() != Some(&usage) {
                    g.context_usage = Some(usage.clone());
                    true
                } else {
                    false
                }
            };
            if let (true, Some(broker)) = (changed, self.broker_read.as_ref()) {
                crate::backend::agent_context_usage::publish_agent_context_usage(
                    broker,
                    &self.block_id_read,
                    &usage,
                );
            }
        }
        // The answer to our `get_settings`: what the CLI
        // really runs. Ignored from a replaced process.
        if let Some(eff) = crate::backend::agent_runtime::effective_from_control_response(parsed) {
            let changed = {
                let mut g = self.inner_read.lock().unwrap();
                if g.spawn_generation == self.my_generation_read && g.spawn_runtime.is_some() {
                    let changed = g.effective_runtime.as_ref() != Some(&eff);
                    g.effective_runtime = Some(eff);
                    changed
                } else {
                    false
                }
            };
            if let (true, Some(broker)) = (changed, self.broker_read.as_ref()) {
                super::status::publish_runtime_event(&self.inner_read, broker, &self.block_id_read);
            }
        }
        // The answer to a self-heal `mcp_status`/`mcp_reconnect`.
        if let Some(ctrl) = self.self_ref_read.as_ref().and_then(|w| w.upgrade()) {
            ctrl.observe_mcp_frame(parsed, self.my_generation_read);
        }
        PersistentSubprocessController::handle_control_frame(kind, parsed, &self.block_id_read, &self.inner_read);
        // OS notification: the agent is now blocked on the
        // user — known here even with no pane mounted
        // (SPEC_OS_NOTIFICATIONS_SYSTEM_2026_09_24 Phase 5).
        if let (Some(broker), Some(asked)) = (
            self.broker_read.as_ref(),
            crate::backend::notify::sources::ask_user_question(parsed),
        ) {
            if let Some(r) = crate::backend::notify::router::get(broker) {
                r.input_waiting_nonblocking(&self.block_id_read, asked.text, asked.count);
            }
        }
    }

    /// What each kind of frame tells the rest of srv about the CLI. Returns whether the parked
    /// memory notices are due ahead of this line.
    fn observe_frame(&mut self, parsed: &serde_json::Value) -> bool {
        let mut release_parked_notices = false;
        self.api_retry.observe(parsed, || self.inner_read.lock().unwrap().spawn_generation);
        crate::connectivity::observe_claude_frame(parsed, &self.model_api);
        if let Some(store) = self.mstore_read.as_deref() {
            let now_ms = agentmux_common::time::now_ms();
            self.task_feed.apply(
                store,
                self.broker_read.as_deref(),
                &self.block_id_read,
                parsed,
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
            let current = || self.inner_read.lock().unwrap().spawn_generation == self.my_generation_read;
            if subtype == Some("task_notification") && current() {
                self.health_read.note_cli_task_notification(parsed.get("summary").and_then(|v| v.as_str()));
            } else if subtype == Some("init") && current() && self.health_read.mark_turn_active_from_cli() {
                if let Some(ctrl) = self.self_ref_read.as_ref().and_then(|w| w.upgrade()) {
                    ctrl.cli_started_pass();
                }
            }
        }
        // Whether its `agentmux` MCP server connected, from the same
        // frame: one that didn't is repaired (`backend::mcp_health`).
        if parsed.get("subtype").and_then(|v| v.as_str()) == Some("init") {
            if let Some(ctrl) = self.self_ref_read.as_ref().and_then(|w| w.upgrade()) {
                ctrl.observe_mcp_frame(parsed, self.my_generation_read);
            }
            // Every server it reports, for Connectors (`backend::mcp_observed`);
            // a replaced process's late init never writes over a newer one's.
            if crate::backend::mcp_observed::record_init(&self.block_id_read, self.mcp_owner_read, parsed) {
                crate::backend::mcp_observed::publish(self.broker_read.as_deref(), &self.block_id_read);
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
                self.mstore_read.clone(),
                self.broker_read.clone(),
                self.filestore_read.clone(),
            ) {
                let (block_id, version) = (self.block_id_read.clone(), version.to_string());
                tokio::task::spawn_blocking(move || {
                    crate::backend::cli_notice::observe_and_notify(
                        &broker, &filestore, &store, &block_id, None, "claude", &version,
                    );
                });
            }
        }
        // The latest `rate_limit_event`: a usage limit stored later
        // reads its reset from it (`agents::limit_reset`).
        crate::agents::limit_reset::observe_frame(&self.block_id_read, parsed);
        crate::backend::context_reading::observe_claude_frame(&self.block_id_read, parsed);
        // This block's CLI compacted: only its pane may send the
        // hidden memory reinjection for that boundary.
        if parsed.get("subtype").and_then(|v| v.as_str()) == Some("compact_boundary") {
            if let Some(uuid) = parsed.get("uuid").and_then(|v| v.as_str()) {
                crate::backend::memory::delivery_state::record_compaction_boundary(&self.block_id_read, uuid);
            }
            self.parked_notices_due = true;
        } else if self.parked_notices_due
            && !matches!(parsed.get("type").and_then(|v| v.as_str()), Some("user") | Some("system"))
        {
            release_parked_notices = true;
            self.parked_notices_due = false;
        }
        release_parked_notices
    }

    /// Whether the frame ends the turn, and a quota or sign-in failure it carries.
    fn frame_verdict(&mut self, parsed: &serde_json::Value) -> FrameVerdict {
        let is_result_frame =
            parsed.get("type").and_then(|v| v.as_str()) == Some("result");
        if parsed.get("type").and_then(|v| v.as_str()) == Some("assistant") {
            self.login_failed_in_band = crate::agents::limit_reset::is_login_failure_frame(parsed);
        }
        let inband_error = if is_result_frame && self.login_failed_in_band { "authentication_failed" } else { "" };
        if is_result_frame {
            self.login_failed_in_band = false;
        }
        // A quota or sign-in failure is stored, and a successful turn's
        // stale failure cleared, before the turn's end is published
        // (`on_turn_end`): whoever reacts to that end (the Supervisor) reads the
        // block's state from them (#5389). The steps after
        // leave both alone, except to restore an older turn's flushed
        // failure.
        let result_is_error = is_result_frame && parsed.get("is_error").and_then(|v| v.as_bool()) == Some(true);
        let early_failure = (result_is_error && self.inner_read.lock().unwrap().shutdown_generation != Some(self.my_generation_read))
            .then(|| crate::agents::failure::classify(None, None, inband_error, Some(parsed)))
            .filter(|f| matches!(f.code, crate::agents::failure::FailureClass::UsageLimit | crate::agents::failure::FailureClass::Auth));
        if let Some(failure) = &early_failure {
            surface_failure(&self.block_id_read, failure, self.broker_read.as_deref(), &self.mstore_read, &self.event_bus_read);
        } else if is_result_frame && !result_is_error {
            core::persist_last_failure(&self.block_id_read, None, &self.mstore_read, &self.event_bus_read);
            if let Some(mstore) = &self.mstore_read {
                crate::backend::route_credits::turn_succeeded(mstore, self.broker_read.as_ref(), &self.block_id_read);
            }
        }
        FrameVerdict { is_result_frame, inband_error, early_failure }
    }

    /// A turn ended: free what waited on it, apply a deferred restart, and publish the flip.
    fn on_turn_end(&mut self, parsed: &serde_json::Value) {
        // The first turn on an expired identity is over: others waiting on its refresh may go. Not a
        // replaced process's late frame, which would free its replacement's hold.
        if self.inner_read.lock().unwrap().spawn_generation == self.my_generation_read {
            crate::identity::token_warmup::release(&self.block_id_read);
        }
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
        // The first turn on a changed model route or model decides the change: refused, the agent is put back and
        // restarted on the place it came from, by the same deferred restart a runtime-config change uses. It is asked
        // for before the boundary, which commits it in the acquisition that ends the turn.
        if self.settle_route_trial(parsed) {
            PersistentSubprocessController::restart_at_boundary_locked(&mut self.inner_read.lock().unwrap());
        }
        let boundary = PersistentSubprocessController::turn_boundary_locked(
            &mut self.inner_read.lock().unwrap(),
            &self.health_read,
            &self.block_id_read,
            self.my_generation_read,
            Some(crate::backend::blockcontroller::health::PassStats::from_result_frame(parsed)),
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
            let ask = self.inner_read.lock().unwrap().settings_readback;
            if ask {
                PersistentSubprocessController::push_stdin(&self.inner_read, crate::backend::agent_runtime::settings_request_line());
                // The threshold moves with the model (a fallback
                // model, a /model switch applied in place).
                PersistentSubprocessController::push_stdin(
                    &self.inner_read,
                    crate::backend::agent_context_usage::context_usage_request_line(),
                );
            }
        }
        if deferred_restart {
            tracing::info!(
                block_id = %self.block_id_read,
                "turn ended — applying the deferred runtime-config restart"
            );
            // Marked as a restart, not a plain stop, so the
            // kill arm brings the replacement up itself: the
            // next message is often a jekt, and automated
            // delivery never spawns.
            if let Some(ctrl) = self.self_ref_read.as_ref().and_then(|w| w.upgrade()) {
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
        self.publish_turn_ended(boundary_is_current);
        self.reconcile_subagents();
    }

    fn publish_turn_ended(&self, boundary_is_current: bool) {
        if let (Some(broker), true) = (self.broker_read.as_ref(), boundary_is_current) {
            if let Some(r) = crate::backend::notify::router::get(broker) {
                r.resolve_nonblocking(&self.block_id_read, crate::backend::notify::policy::Family::Input);
            }
        }
        if let (Some(broker), true) = (self.broker_read.as_ref(), boundary_is_current) {
            let status = {
                let locked = self.inner_read.lock().unwrap();
                super::super::agent_runtime_status(
                    &self.block_id_read,
                    locked.status_version,
                    &locked.proc_status,
                    locked.proc_exit_code,
                    false,
                )
            };
            super::super::publish_controller_status(broker, &status);
        }
    }

    fn reconcile_subagents(&self) {
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
        let session_id_snapshot = self.inner_read.lock().unwrap().session_id.clone();
        if let Some(sid) = session_id_snapshot {
            if let Some(watcher) = subagent_watcher::global() {
                let block_id = self.block_id_read.clone();
                tokio::task::spawn_blocking(move || {
                    watcher.reconcile_stale_subagents(&block_id, &sid);
                });
            }
        }
    }

    /// Adopt the session id the frame carries, and act on what that settles for resume tracking.
    fn capture_session_id(
        &mut self,
        parsed: &serde_json::Value,
        is_result_frame: bool,
        flushed_failure_this_tick: &mut bool,
    ) {
        if let Some(sid) = parsed.get(&self.session_id_field).and_then(|v| v.as_str()) {
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
                let mut inner = self.inner_read.lock().unwrap();
                let (adopted, effects) = inner.try_capture_session_id(&sid_string, self.my_generation_read, is_confirmed_success);
                (adopted, effects, inner.holds_current_session(&sid_string, self.my_generation_read))
            };
            if should_capture {
                tracing::info!(
                    block_id = %self.block_id_read,
                    session_id = %sid_string,
                    "persistent session ID captured"
                );
                core::persist_session_id(&self.block_id_read, &sid_string, &self.mstore_read, &self.event_bus_read);
            }
            let kept_relocated_id = super::segments::kept_relocated_id(
                should_capture,
                is_confirmed_success,
                holds_current,
                self.relocated_copy_read.is_some(),
                self.forked_from_read.as_deref(),
                &sid_string,
            );
            if should_capture {
                super::segments::record_segment_session(&self.segment_read, &sid_string);
                // The fork has its own session now; the copy it
                // read from goes (spec §4.3 (3), I4).
                if let Some(copy) = self.relocated_copy_read.take() {
                    super::segments::settle_relocated_copy(&self.block_id_read, &copy, self.forked_from_read.as_deref(), &sid_string);
                }
            } else if kept_relocated_id {
                // Kept under the attempted id, the copy is the live
                // session: recorded only while it is still this
                // spawn's, never a replacement's at the same path.
                if let Some(copy) = self.relocated_copy_read.take() {
                    if super::segments::settle_relocated_copy(&self.block_id_read, &copy, self.forked_from_read.as_deref(), &sid_string) {
                        super::segments::record_segment_session(&self.segment_read, &sid_string);
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
            // §2.1) — handled explicitly in this match; anything
            // else falls to the catch-all, kept exhaustive
            // rather than assuming the effect set never
            // grows again.
            self.apply_capture_effects(capture_effects, flushed_failure_this_tick);
        }
    }

    fn apply_capture_effects(
        &mut self,
        capture_effects: Vec<persistent_resume::ResumeEffect>,
        flushed_failure_this_tick: &mut bool,
    ) {
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
                        self.forked_from_read.as_deref(),
                        &attempted_sid,
                        actual_sid.as_deref(),
                    );
                    // The retry (if any led here) is now resolved one
                    // way or the other — clear "Reconnecting…". A no-op
                    // publish (still fine) when this outcome came from a
                    // plain first-time resume that was never retried.
                    publish_resume_retry_status(&self.broker_read, &self.block_id_read, "resolved");
                    // A recovery scan can turn an
                    // already-disclosed resume failure
                    // into a genuine resume — retract the
                    // banner so it can't contradict the
                    // `resumed` divider appended just
                    // after it. See
                    // `session_recovery::clear_resume_failed`.
                    if matches!(outcome, persistent_resume::SessionOutcome::Resumed) {
                        if let Some(ref store) = self.mstore_read {
                            super::super::session_recovery::clear_resume_failed(
                                store,
                                &self.event_bus_read,
                                &self.block_id_read,
                            );
                        }
                    }
                    if let Some(ref broker) = self.broker_read {
                        let line = session_outcome_line(outcome, attempted_sid, actual_sid);
                        super::super::shell::handle_append_block_file(
                            broker,
                            &self.block_id_read,
                            PERSISTENT_OUTPUT_SUBJECT,
                            line.as_bytes(),
                            self.filestore_read.as_ref(),
                            crate::backend::agent_admission::fenced_zone(self.global_output_zone.as_deref(), &self.record_fence),
                        );
                    }
                }
                persistent_resume::ResumeEffect::FlushErrorLine(line) => {
                    if let Some(ref broker) = self.broker_read {
                        super::super::shell::handle_append_block_file(
                            broker,
                            &self.block_id_read,
                            PERSISTENT_OUTPUT_SUBJECT,
                            line.as_bytes(),
                            self.filestore_read.as_ref(),
                            crate::backend::agent_admission::fenced_zone(self.global_output_zone.as_deref(), &self.record_fence),
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
                        &self.block_id_read,
                        None,
                        &line,
                        self.broker_read.as_deref(),
                        &self.mstore_read,
                        &self.event_bus_read,
                    ) {
                        *flushed_failure_this_tick = true;
                    }
                }
                other => {
                    tracing::warn!(
                        block_id = %self.block_id_read,
                        effect = ?other,
                        "unexpected ResumeEffect from a SessionCaptured event"
                    );
                }
            }
        }
    }

    /// Hold an error result back while a stale `--resume` retry could still be confirmed.
    /// Returns whether this line is held back.
    fn track_error_result(&mut self, line: &str, flushed_failure_this_tick: &mut bool) -> bool {
        // Issue #2368: this generation's resume tracking
        // (`persistent_resume::ResumeState`) decides whether
        // this line is still a retry candidate — if it has
        // already resolved (a session id was captured by `capture_session_id` or
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
        // unchanged fallthrough (the append in `handle_line`) — executing it here too
        // would double-persist it.
        let (effects, still_tracking) = {
            let mut inner = self.inner_read.lock().unwrap();
            let effects = inner.apply_resume_event(persistent_resume::ResumeEvent::ErrorResultLine {
                generation: self.my_generation_read,
                line: format!("{}\n", line),
            });
            let still_tracking = matches!(
                &inner.resume,
                persistent_resume::ResumeState::AwaitingOutcome { generation, .. }
                    if *generation == self.my_generation_read
            ) || matches!(
                &inner.resume,
                persistent_resume::ResumeState::ConfirmedRetry { generation, .. }
                    if *generation == self.my_generation_read
            );
            (effects, still_tracking)
        };
        if still_tracking {
            for effect in effects {
                match effect {
                    persistent_resume::ResumeEffect::PersistImmediately(old_line)
                    | persistent_resume::ResumeEffect::FlushErrorLine(old_line) => {
                        if let Some(ref broker) = self.broker_read {
                            super::super::shell::handle_append_block_file(
                                broker,
                                &self.block_id_read,
                                PERSISTENT_OUTPUT_SUBJECT,
                                old_line.as_bytes(),
                                self.filestore_read.as_ref(),
                                crate::backend::agent_admission::fenced_zone(self.global_output_zone.as_deref(), &self.record_fence),
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
                        // clear-on-success step in `surface_turn_outcome`.
                        if surface_error_line(
                            &self.block_id_read,
                            None,
                            &old_line,
                            self.broker_read.as_deref(),
                            &self.mstore_read,
                            &self.event_bus_read,
                        ) {
                            *flushed_failure_this_tick = true;
                        }
                    }
                    other => {
                        tracing::warn!(
                            block_id = %self.block_id_read,
                            effect = ?other,
                            "unexpected ResumeEffect from a still-tracking ErrorResultLine event"
                        );
                    }
                }
            }
        }
        still_tracking
    }

    /// Classify and surface a final error result, or clear a stale failure on a good one.
    #[allow(clippy::too_many_arguments)]
    fn surface_turn_outcome(
        &mut self,
        parsed: &serde_json::Value,
        is_result_frame: bool,
        is_error_result: bool,
        hold_back_for_resume_retry: bool,
        no_early_failure: bool,
        inband_error: &str,
        flushed_failure_this_tick: bool,
    ) {
        // Classify + surface a genuine, non-retried error result
        // (429/overloaded/auth/etc.) to the pane's failure-recovery
        // UI — SPEC_PERSISTENT_CONTROLLER_FAILURE_CLASSIFICATION.
        // Gated on `!hold_back_for_resume_retry`: when the stale-
        // `--resume` machinery (`track_error_result`) is still tracking this exact
        // error as a live retry candidate, it must stay invisible
        // to the user (per its own "must never reach the user"
        // invariant at `process_waiter.rs`'s ProcessExited/FireRetry arm) —
        // classify() only runs once this error is confirmed final.
        if is_error_result {
            // A failed turn ends without the hint, so a finished task stays no boundary.
            crate::backend::context_reading::clear_task_boundary(&self.block_id_read);
        }
        if is_error_result && !hold_back_for_resume_retry && no_early_failure {
            let failure = crate::agents::failure::classify(None, None, inband_error, Some(parsed));
            surface_failure(&self.block_id_read, &failure, self.broker_read.as_deref(), &self.mstore_read, &self.event_bus_read);
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
            // path. Skipped when `apply_capture_effects`
            // just persisted a freshly-flushed OLDER failure this
            // same tick — that state must survive, not be
            // immediately wiped by this frame's own success.
            core::persist_last_failure(&self.block_id_read, None, &self.mstore_read, &self.event_bus_read);
            // A completed turn is when the agent's running
            // summary may be due for an update. Background, and
            // a no-op unless enough happened since the last one
            // (SPEC_DURABLE_CONVERSATION_MEMORY_2026_09_23.md §4.4).
            crate::backend::continuity_state::after_successful_turn(
                self.mstore_read.clone(),
                self.block_id_read.clone(),
            );
            // A large context when the work may continue: one note
            // for the next turn (SPEC_AGENT_SELF_COMPACTION §3.5).
            if let Some(note) = crate::backend::context_reading::take_turn_end_hint(&self.block_id_read) {
                crate::backend::reactive::held_notes::hold(&self.block_id_read, crate::backend::context_reading::HINT_NOTE_KEY, &note);
            }
        }
    }
}
