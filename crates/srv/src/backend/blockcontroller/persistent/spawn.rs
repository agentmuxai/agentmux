// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `spawn_process`: builds the CLI argv/env, spawns the child, and starts the
//! per-session I/O tasks (stdin writer, stdout reader, stderr reader, waiter).
//! The stdin writer runs inline; the other three bodies live in
//! `stderr_reader.rs`, `stdout_reader.rs` and `process_waiter.rs`.

#[cfg(windows)]
use agentmux_common::win32::NoWindow;
use super::*;
use super::process_waiter::ProcessWaiterCtx;
use super::stderr_reader::StderrReaderCtx;
use super::stdout_reader::StdoutReaderCtx;

impl PersistentSubprocessController {
    /// Spawn the persistent CLI process. Called only while the caller
    /// holds the exclusive spawn claim (`spawning_in_progress`, see
    /// `decide_send_action`) — never directly.
    pub(super) fn spawn_process(
        &self,
        config: PersistentSpawnConfig,
        resume_retry_payload: Option<persistent_resume::QueuedRetryEntry>,
    ) -> Result<(), String> {
        // Captured before `resume_retry_payload` is moved further down (the
        // resume-bookkeeping match), for the turn-active decision near the
        // end of this function — see that check's own comment. Every
        // pre-existing caller either passes `Some(payload)` (a message is
        // about to be delivered directly) or `None` because a message is
        // already sitting in `pending_send_messages` waiting to be drained
        // right after this spawn succeeds (`respawn_once_for_leftover_
        // queue`) — this flag alone can't tell those two `None` cases
        // apart, which is exactly why the check below also looks at the
        // queue.
        let had_retry_payload = resume_retry_payload.is_some();

        // Build command — use make_cli_cmd to resolve .cmd wrappers to node on Windows
        let mut cmd = crate::server::cli_handlers::make_cli_cmd(&config.cli_command);

        // Hydrate the captured session id from the config when we don't have one
        // yet (fresh controller after a forced restart — e.g. a /model change —
        // or the picker reattach path). Mirrors SubprocessController::
        // hydrate_session_id_from_config so the respawn resumes the same
        // conversation instead of starting blank.
        if !config.session_id.is_empty() {
            let mut inner = self.inner.lock().unwrap();
            if inner.session_id.is_none() {
                inner.session_id = Some(config.session_id.clone());
            }
        }

        // First spawn with no id, onto a pane that renders prior history:
        // continue that history's session instead of pairing it with a blank
        // model (SPEC_DURABLE_CONVERSATION_MEMORY_2026_09_23.md §1.1, §5 P0a).
        // A session live in another pane is left alone — adopting it would
        // turn a plain open into the held-elsewhere refusal below.
        let first_spawn_without_sid = {
            let inner = self.inner.lock().unwrap();
            inner.session_id.is_none() && inner.spawn_generation == 0
        };
        // The session the pane's history belongs to when `--resume` can't
        // reach it here: the resume gate may relocate it (spec §4.3).
        let mut unreachable_history_session: Option<String> = None;
        if first_spawn_without_sid {
            if let Some(sid) = self.find_continuation_session_id(&config) {
                if self.session_held_elsewhere(&sid).is_none() {
                    tracing::info!(
                        block_id = %self.block_id,
                        session_id = %sid,
                        "continuity: first spawn has no session id; continuing the session this pane's history belongs to"
                    );
                    self.inner.lock().unwrap().session_id = Some(sid);
                } else {
                    tracing::info!(
                        block_id = %self.block_id,
                        session_id = %sid,
                        "continuity: prior session is live in another pane; starting fresh"
                    );
                }
            } else if !config.resume_flag.is_empty() {
                unreachable_history_session = pane_history_session_id(
                    self.filestore.as_deref(),
                    self.mstore.as_deref(),
                    &self.block_id,
                    &config.session_id_field,
                )
                .filter(|sid| self.session_held_elsewhere(sid).is_none());
            }
        }
        // Claim the agent's single-live-instance lease, then — under it — let
        // the resume gate decide which session is resumed. Any early return
        // after this point drops the handle, which releases the lease.
        let gfs = crate::backend::agent_session::global_transcript_store();
        let agent_lease = self.lease_then_gate(&config, gfs.map(|g| &**g), unreachable_history_session)?;

        // Append `--resume <sid>` when we have a session id and the provider
        // supports simple-flag resume — same construction as
        // SubprocessController::spawn_turn. This is what makes a model/effort
        // change (which respawns the persistent CLI with new flags) preserve the
        // conversation. cli_args carries the runtime flags (model/effort/perm)
        // already rebuilt by the frontend (useAgentCommands buildRuntimeArgs).
        let mut spawn_args = config.cli_args.clone();
        // Recorded so the stderr reader can tell "No conversation found" apart
        // from an unrelated CLI error, and so it knows exactly which id to
        // poison against the stdout reader's own capture (`stdout_reader.rs`) — a
        // provider that echoes back whatever --resume it was given as its
        // first stdout line, even when that id turns out to be unreachable.
        let mut attempted_resume_sid: Option<String> = None;
        // One session, one process. Resuming a session another live process
        // is still running on — the agent's other pane, or one that is
        // closing and hasn't exited yet — puts two CLIs on one transcript.
        // Refuse instead: the user closes the other one (or waits the few
        // seconds a close takes) and tries again. Covers every reopen path
        // (picker reattach, launch modal, MCP), not just `agent.open`
        // (SPEC_AGENT_PANE_CLOSE_GRACEFUL_SHUTDOWN_2026_09_18.md §4.7).
        let requested_sid = if config.resume_flag.is_empty() {
            None
        } else {
            self.inner.lock().unwrap().session_id.clone()
        };
        // Check and claim must be one step: the check reads other
        // controllers' `current_pid`, which a concurrent resume of the same
        // session only sets further down this function. Held from here until
        // this spawn's `current_pid` is set (or it returns early), so two
        // reopens of one session — two picker clicks, picker + MCP — can't
        // both pass. Keyed by session (reagent P1 on #3421): a fixed stripe
        // of locks chosen by hashing the session id — every reopen of one
        // session takes the same lock, unrelated agents' resume respawns
        // almost never share one, and memory stays bounded (a per-session map
        // would grow forever — reagent P2). Only resume spawns take one;
        // `spawn_process` is synchronous, and no caller holds an `inner` lock
        // across it.
        // The agent's memory is in its folder before the provider reads it.
        // Runs before the resume claim below, not under it: a pass can take
        // up to its one-second budget, and that lock is meant to be held only
        // briefly (reagent P1 on #3721). Not for a session already live in
        // another pane — that spawn is about to be refused, and its pass
        // would run beside the live provider writing the same folder. (A
        // second pass for the agent at the same moment skips on its lease.)
        if requested_sid.as_deref().is_none_or(|sid| self.session_held_elsewhere(sid).is_none()) {
            self.reconcile_memory_before_spawn(&config);
        }
        let resume_claim = requested_sid.as_deref().map(|sid| {
            const STRIPES: usize = 64;
            static RESUME_SPAWN_LOCKS: [Mutex<()>; STRIPES] = [const { Mutex::new(()) }; STRIPES];
            let stripe = {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                sid.hash(&mut h);
                (h.finish() as usize) % STRIPES
            };
            RESUME_SPAWN_LOCKS[stripe].lock().unwrap_or_else(|e| e.into_inner())
        });
        if let Some(sid) = requested_sid.as_deref() {
            if let Some((other, closing)) = self.session_held_elsewhere(sid) {
                return Err(held_elsewhere_error(&other, closing));
            }
        }
        // Set when the resume gate relocated the session this spawn resumes
        // (spec §4.3): the copy it placed, which the fork reads and never
        // writes.
        let mut relocated_copy: Option<crate::backend::continuity_relocate::Relocated> = None;
        // This spawn resumes with `--fork-session` (relocated, or the session
        // grew outside AgentMux, spec §4.4).
        let mut forked = false;
        {
            let mut inner = self.inner.lock().unwrap();
            let fork_copy = inner.fork_copy.take();
            let fork_next = std::mem::take(&mut inner.fork_next);
            if let Some(sid) = inner.session_id.clone() {
                if !config.resume_flag.is_empty() {
                    spawn_args.push(config.resume_flag.clone());
                    spawn_args.push(sid.clone());
                    if fork_copy.is_some() || fork_next {
                        spawn_args.push("--fork-session".to_string());
                        relocated_copy = fork_copy.clone();
                        forked = true;
                    }
                    attempted_resume_sid = Some(sid);
                }
            }
            if relocated_copy.is_none() {
                if let Some(copy) = fork_copy {
                    crate::backend::continuity_relocate::remove(&copy);
                }
            }
        }
        cmd.args(&spawn_args);

        core::apply_working_dir(&mut cmd, &self.block_id, &config.working_dir, &config.env_vars);
        // Identity M4a: record what this process is actually given.
        crate::backend::identity_spawn::record_process_spawn(
            &self.block_id,
            crate::backend::identity_spawn::SpawnPath::Persistent,
            &config.env_vars,
        );

        // On Windows: suppress console-window allocation. The srv runs without a
        // console of its own, so spawning the agent CLI without CREATE_NO_WINDOW
        // makes Windows allocate a fresh console — which Windows 11's default-
        // terminal handler renders as a NEW Windows Terminal window. One leaks per
        // agent start / resume / respawn; a flapping or restart-heavy session
        // accumulates dozens. stdio is piped here, so the console is never needed.
        // See docs/retro/retro-windows-terminal-window-leak-2026-06-21.md.
        // Matches acp.rs / subprocess/host_spawn.rs; sibling of shell/pty.rs's
        // PTY path.
        #[cfg(windows)]
        {
            cmd.no_window();
        }

        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        // Don't even start a config restart's replacement a Stop has already
        // cancelled (see `restart_spawn_still_permitted_locked`); the decision
        // itself is made where the child is installed, below.
        Self::restart_spawn_still_permitted_locked(&mut self.inner.lock().unwrap())?;

        // In the block's tracker before it runs (`spawn_tracked`).
        let mut child = crate::backend::process_tracker::registry::spawn_tracked(&self.block_id, &mut cmd, crate::backend::process_tracker::registry::Join::Agent).map_err(|e| {
            tracing::error!(block_id = %self.block_id, error = %e, "persistent process spawn failed");
            format!("failed to spawn persistent process: {e}")
        })?;

        // Stash the TENTATIVE resume-retry payload synchronously, right
        // here — before any background task (stdin writer, stdout/stderr
        // readers, process-waiter) is created below — reagentx P1 on PR
        // #2360: doing this later, back in send_message after this
        // function returned, left a window where a process that dies fast
        // enough (the exact case this exists to catch) lets the
        // process-waiter task — already racing on another thread once
        // it's spawned — observe the exit and take() this payload while
        // it's still `None`, silently losing the retry for the very case
        // it's meant to catch. Keyed on the EXACT sid this spawn attempted
        // (not `config.session_id`, which can differ from what's actually
        // held in `inner.session_id` once an earlier call has already
        // hydrated it) so `poison_resume`'s later confirmation check is
        // unambiguous.
        // Bumped in this SAME lock acquisition — see `spawn_generation`'s
        // own doc comment for what this identifies and why.
        let (my_generation, superseded_effects) = {
            let mut inner = self.inner.lock().unwrap();
            // A config restart's replacement is installed only if no Stop has
            // cancelled it, decided in this acquisition: before it, a Stop has
            // no `kill_tx` to reach the child through (codex P1 on #3990).
            if let Err(e) = Self::commit_restart_spawn_locked(&mut inner) {
                drop(inner);
                let _ = child.start_kill();
                tokio::spawn(async move {
                    let _ = child.wait().await;
                });
                tracing::info!(
                    block_id = %self.block_id,
                    "config restart: a stop landed while the replacement was starting — killed it"
                );
                return Err(e);
            }
            // The replacement process is here, so the quiesce window is over —
            // messages may take `DeliverDirect` against this new `stdin_tx`
            // again. Cleared unconditionally rather than only when set: any
            // spawn ends the window by definition, whatever opened it.
            inner.restart_pending = false;
            // Same for a stop request: it targeted the process this spawn
            // replaces.
            inner.stop_pending = false;
            // …and for a config restart's pending replacement: this spawn is
            // it (or supersedes it), so no exit handler may start another.
            inner.config_restart_generation = None;
            inner.restart_spawn_for = None;
            // …and any deferred restart is moot now, for the same reason: this
            // spawn read `cmd:args` fresh from block meta, so the new config is
            // already applied and there is nothing left to restart FOR.
            //
            // This is what stops a leak (reagent P1 on PR #2858):
            // `restart_when_idle` is consumed at the `is_result_frame` turn
            // end, but a generation that dies abnormally instead — a user
            // Stop/SIGINT, a crash, a permanently-failed turn resolving via
            // `ProcessExited` + `PublishDone` — never reaches that branch. The
            // stale `true` would then ride into an unrelated later generation
            // and kill a healthy process at the end of some future turn.
            // Clearing per-spawn scopes the flag to the generation that
            // requested it, which is the only one it ever meant anything for.
            inner.restart_when_idle = false;
            inner.spawn_generation += 1;
            // Any spawn consumes or invalidates an adopted leftover
            // candidate — it only ever meant "for the very next spawn".
            inner.leftover_resume_candidate = None;
            let generation = inner.spawn_generation;
            let effects = match (attempted_resume_sid.clone(), resume_retry_payload) {
                (Some(sid), Some(retry_json)) => {
                    inner.apply_resume_event(persistent_resume::ResumeEvent::SpawnedWithResume {
                        generation,
                        attempted_sid: sid,
                        retry: persistent_resume::RetryPayload { config: config.clone(), messages: vec![retry_json] },
                    })
                }
                // `--resume <sid>` WAS attempted, just with no message of
                // our own to seed the retry batch with — the eager-resume
                // path (issue #3463), which revives a session with nothing
                // queued rather than in response to a message. codex P1 on
                // PR #3513: routing this to `SpawnedFresh` below (the
                // catch-all's original behavior) discarded `attempted_sid`
                // entirely, leaving `NotTracking` in place for a spawn that
                // in fact has an unconfirmed `--resume` in flight. If that
                // resume turns out to be stale and a direct message arrives
                // before the failure is detected, `MessageAppendedToRetryBatch`
                // below has nothing to append it to and no retry ever fires
                // when the doomed process exits — the message is silently
                // lost. An empty `messages` starts the SAME `AwaitingOutcome`
                // tracking a seeded resume gets; `MessageAppendedToRetryBatch`
                // (`DeliverDirect`, below) is what fills it in as messages
                // actually arrive. Never reachable before eager resume
                // existed — every other caller either omits `--resume`
                // entirely (`respawn_once_for_leftover_queue` clears
                // `session_id` first) or always has a real payload
                // (`BecomeSpawner`, `retry_after_resume_failure`).
                (Some(sid), None) => {
                    inner.apply_resume_event(persistent_resume::ResumeEvent::SpawnedWithResume {
                        generation,
                        attempted_sid: sid,
                        retry: persistent_resume::RetryPayload { config: config.clone(), messages: vec![] },
                    })
                }
                (None, _) => inner.apply_resume_event(persistent_resume::ResumeEvent::SpawnedFresh { generation }),
            };
            (generation, effects)
        };
        // Identity for this spawn's muxbus/registry registrations
        // (`registration_nonce` on `AgentRegistration`/`AgentEntry`).
        // Deliberately NOT `my_generation`: the generation is
        // controller-LOCAL (starts at 0 per controller instance), so a
        // replacement controller for this same block
        // (`resync_controller` stops the old one asynchronously and
        // constructs a new one immediately) restarts at generation 1 —
        // the old and new processes could then share a generation, and
        // the old exit-handler's compare-and-remove would accept the
        // replacement's fresh registration as its own and delete it
        // (codex P1 on PR #2500). A process-wide counter can never
        // collide across controller instances.
        let my_registration_nonce = next_registration_nonce();
        // reagentx P1 (round 7 on this PR): this fresh spawn can supersede
        // a PRIOR generation that was still AwaitingOutcome/ConfirmedRetry
        // with a held error line (see `resolve_superseded_generation`'s
        // own doc comment for the exact race — reachable via
        // `respawn_once_for_leftover_queue`) — flush it now, outside the
        // lock, same as every other `ResumeEffect` call site in this
        // module. Discarding this return value silently lost the exact
        // "error disappears" bug class (#2368) this PR exists to fix.
        for effect in superseded_effects {
            match effect {
                persistent_resume::ResumeEffect::FlushErrorLine(line)
                | persistent_resume::ResumeEffect::PersistImmediately(line) => {
                    self.flush_error_line_now(line);
                }
                other => {
                    tracing::warn!(
                        block_id = %self.block_id,
                        effect = ?other,
                        "unexpected ResumeEffect from a fresh spawn superseding a prior generation"
                    );
                }
            }
        }

        // This process starts with none of the conversation the pane is about
        // to display — say so, in the transcript, before any of its own output
        // lands. See `fresh_start_needs_disclosure` for why
        // `persistent_resume`'s `SpawnedFresh` can't decide this itself.
        //
        // `attempted_sid` is empty: there was no id to attempt, which is the
        // whole point. The frontend renders that as "—" rather than a blank
        // (`DocumentRow.tsx`'s session-outcome body).
        let mut continuation: Option<String> = None;
        let fresh_onto_history = attempted_resume_sid.is_none() && self.has_prior_transcript();
        if fresh_onto_history {
            // The provider can't give this process the conversation the pane
            // shows, so AgentMux's own record rides on its first message
            // (SPEC_DURABLE_CONVERSATION_MEMORY_2026_09_23.md §4.4). On any
            // generation, not just the first: a resume the CLI rejected falls
            // through to here within the same launch (§4.2, "fall through,
            // don't fail"), and its retry needs the record as much as a
            // first spawn does. The disclosure above stays first-spawn-only;
            // the retry path emits its own.
            continuation = self.carry_continuation();
        }
        // After the packet is built, so the disclosure can say whether this
        // fresh session was given the record ("continued") or not.
        if fresh_onto_history && fresh_start_needs_disclosure(attempted_resume_sid.as_deref(), my_generation) {
            tracing::info!(
                block_id = %self.block_id,
                continued = continuation.is_some(),
                "spawned with no --resume while prior history exists — disclosing a fresh start"
            );
            self.emit_fresh_outcome_now(String::new(), continuation.is_some());
        }

        let pid = child.id().unwrap_or(0);

        // Notify the turn-activity tracker that a turn is starting — but
        // only if one actually is. Every pre-existing caller of this method
        // always has a message about to flow through, one way or the other
        // (see `had_retry_payload`'s own comment), so this was previously
        // unconditionally correct. `PersistentSubprocessController::
        // start()`'s eager-resume path (issue #3463) is the first caller
        // that genuinely has nothing queued — it revives a session so it is
        // ready for the NEXT message, the same as `ShellController::
        // start()` reviving a shell leaves it idle rather than mid-command.
        // Marking a turn active here with no message ever sent meant a
        // freshly eager-resumed pane was reported as perpetually WORKING —
        // to the pane UI, Swarm view, subagent watcher, and shutdown logic,
        // all of which read this flag — until a human happened to notice
        // and send it something, since nothing would ever produce the
        // `result` frame that normally clears it (codex P1 on PR #3513).
        if had_retry_payload || !self.inner.lock().unwrap().pending_send_messages.is_empty() {
            self.health_monitor.set_active_turn(true);
        }

        tracing::info!(
            block_id = %self.block_id,
            pid = pid,
            cmd = %config.cli_command,
            args = ?spawn_args,
            working_dir = %config.working_dir,
            "persistent process spawned"
        );

        let (kill_tx, kill_rx) = tokio::sync::oneshot::channel::<KillRequest>();
        let stdin = child.stdin.take()
            .ok_or_else(|| format!("[persistent] stdin not captured for block {}", self.block_id))?;
        let stdout = child.stdout.take()
            .ok_or_else(|| format!("[persistent] stdout not captured for block {}", self.block_id))?;
        let stderr = child.stderr.take();

        // Drain stderr in background — log lines for debugging. The
        // JoinHandle is kept (not discarded) so the process-waiter task
        // can await this task's full completion before deciding whether a
        // stale-resume retry was confirmed — codex P1/P2 on PR #2360
        // (second review pass): `child.wait()` resolving is NOT proof this
        // task has already seen and reacted to a "No conversation found"
        // line, or finished its OWN subsequent `persist_session_id("")`
        // call (`stderr_reader.rs`). See the process-waiter's own comment
        // (`process_waiter.rs`) for the two
        // failure modes this closes.
        let stderr_reader_handle: Option<tokio::task::JoinHandle<()>> = stderr.map(|stderr_pipe| {
            let block_id_stderr = self.block_id.clone();
            let inner_stderr = Arc::clone(&self.inner);
            let mstore_stderr = self.mstore.clone();
            let event_bus_stderr = self.event_bus.clone();
            let attempted_resume_sid = attempted_resume_sid.clone();
            let my_generation_stderr = my_generation;
            tokio::spawn(Self::run_stderr_reader(StderrReaderCtx {
                stderr_pipe,
                block_id_stderr,
                inner_stderr,
                mstore_stderr,
                event_bus_stderr,
                attempted_resume_sid,
                my_generation_stderr,
            }))
        });

        // Create stdin writer channel
        let (msg_tx, mut msg_rx) = mpsc::channel::<String>(32);

        {
            let mut inner = self.inner.lock().unwrap();
            inner.current_pid = Some(pid);
            inner.kill_tx = Some(kill_tx);
            inner.stdin_tx = Some(msg_tx);
            // The same `Arc` when an earlier generation's lease was reused
            // (see `acquire_agent_lease`), so no release happens here.
            inner.agent_lease = agent_lease.clone();
            Self::set_status(&mut inner, STATUS_RUNNING);
            inner.spawn_runtime = Some(crate::backend::agent_runtime::spawn_runtime_from_args(&spawn_args));
            inner.effective_runtime = None;
            // Ask the CLI what it is really using as soon as it is up: the argv
            // carries an alias, the CLI resolves it. Only a control-protocol
            // process reads such requests from stdin. An answer is folded into
            // the `agentruntime` event (the stdout reader, `stdout_reader.rs`).
            inner.settings_readback = spawn_args.iter().any(|a| a == "--permission-prompt-tool");
            inner.context_usage = None;
            if inner.settings_readback {
                if let Some(tx) = inner.stdin_tx.as_ref() {
                    let _ = tx.try_send(crate::backend::agent_runtime::settings_request_line());
                    // And where it will auto-compact (agent_context_usage.rs),
                    // answered before the first message too.
                    let _ = tx.try_send(crate::backend::agent_context_usage::context_usage_request_line());
                }
            }
        }
        // Now visible to `session_held_elsewhere` — a concurrent resume of
        // the same session may run its check.
        drop(resume_claim);
        self.publish_status();

        // Auto-register with the muxbus reactive handler so inter-agent
        // messages reach this persistent (no-PTY) agent. The PTY shell
        // controller (shell.rs) was the only prior auto-register path, so
        // stream-json agents were in the directory but absent from the
        // delivery registry — `inject_message` returned "agent not found"
        // (issue #1470). Tier-1 delivery is routed through the controller-
        // aware MessageSender (→ send_user_message), not PTY keystrokes.
        // See SPEC_MUXBUS_AGENT_DISCOVERY_AND_PERSISTENT_DELIVERY_2026_06_16.
        let agent_id_for_muxbus = muxbus_agent_id_from_env(&config.env_vars);
        *self.agent_id.lock().unwrap() = agent_id_for_muxbus.clone();
        // Write-once: unlike `agent_id` above (refreshed every turn by
        // `input.rs`'s Register-tail), `stable_agent_id` is only ever set
        // here, at spawn, and never touched again — see its field doc
        // comment. `spawn_process` can in principle run again for the same
        // controller on a respawn; re-writing the SAME env-derived value
        // each time is harmless (idempotent), and a respawn with genuinely
        // different env (rare, config change) correctly updates the alias
        // to match, same as the primary registration already does.
        *self.stable_agent_id.lock().unwrap() = agent_id_for_muxbus.clone();
        // Identity M2: the UID carried in the spawn env (M1a). Captured with
        // the same write-once discipline as `stable_agent_id`; `None` when
        // the env carried none (no `db_agents` row at spawn — a continuation
        // launch eager-resumes before the frontend creates the row, spec
        // §4.4.4 Q1 — or a quick-launch pane), in which case this spawn
        // registers name-only and the first turn's Register-tail upgrades
        // the block in place.
        let agent_uid_for_registry = config
            .env_vars
            .get("AGENTMUX_AGENT_UID")
            .map(|u| u.trim().to_string())
            .filter(|u| !u.is_empty());
        *self.stable_agent_uid.lock().unwrap() = agent_uid_for_registry.clone();
        // This process is one segment of the agent's conversation
        // (SPEC_DURABLE_CONVERSATION_MEMORY_2026_09_23.md §4.1, P1). The
        // stdout reader records its provider session id, the waiter its end.
        let segment: super::segments::SegmentRef = agent_uid_for_registry.as_deref().and_then(|uid| {
            self.record_segment_start(
                uid,
                &config,
                attempted_resume_sid.as_deref(),
                attempted_resume_sid.as_deref().filter(|_| forked),
                relocated_copy.is_some(),
                continuation.is_some(),
                agent_lease.as_ref().map(|l| l.epoch()),
            )
        });
        *self.current_segment.lock().unwrap() = segment.as_ref().map(|(_, id)| id.clone());
        let segment_read = segment.clone();
        let segment_wait = segment;
        // A relocated resume's copy, removed once the fork reports its id.
        let relocated_copy_read = relocated_copy.clone();
        let forked_from_read = attempted_resume_sid.clone().filter(|_| forked);
        // Defaults to this spawn's own nonce; overridden below if
        // registration is skipped (reagent P1 on PR #3084 — see the `Err`
        // arm just below for why `my_registration_nonce` alone is wrong
        // once a skip is possible).
        let mut exit_cleanup_nonce = my_registration_nonce;
        if let Some(ref agent_id) = agent_id_for_muxbus {
            // `_with_nonce` variants record this spawn's process-wide
            // registration nonce so this exact spawn's exit-handler can
            // compare-and-remove its own registrations instead of
            // blindly wiping a fallback respawn's (or replacement
            // controller's) fresh ones (issue #2363; codex P1 on PR
            // #2500 for why not the controller-local generation).
            // `try_register_agent_with_nonce`, not the plain
            // `register_agent_with_nonce` — this spawn can be running on the
            // same thread as an in-flight `inject_message` (the
            // reactive-delivery fallback's synchronous respawn), and that
            // call already holds this same handler's lock. The plain
            // version would re-lock it on that thread and deadlock the
            // reactive handler process-wide. See
            // `ReactiveHandler::try_register_agent_with_nonce`'s doc comment
            // and `docs/incident/INCIDENT_2026_09_07_BACKEND_UPTIME_TIMER_FROZEN.md`.
            match crate::backend::reactive::get_global_handler()
                .try_register_agent_full(
                    agent_id,
                    &self.block_id,
                    Some(&self.tab_id),
                    my_registration_nonce,
                    // Also park `agent_id` (== AGENTMUX_AGENT_ID here) as the
                    // block's STABLE name binding, kept when `input.rs`'s
                    // Register-tail replaces the display binding with the
                    // live display name on this agent's very first turn —
                    // otherwise nothing would be left to answer a jekt
                    // tagged with the stable ID. See
                    // `INCIDENT_2026_09_09_JEKT_STABLE_ID_ALIAS.md`.
                    Some(agent_id.as_str()),
                    // Identity M2: the key. Carried from the env, never
                    // looked up here — nothing new is read under the
                    // handler's lock (spec §4.4.4 Q5).
                    agent_uid_for_registry.as_deref(),
                    "registration.no_uid.spawn",
                )
            {
                Ok(()) => {
                    tracing::info!(
                        block_id = %self.block_id,
                        agent_id = %agent_id,
                        "muxbus: auto-registered persistent agent"
                    );
                    // Also write the cross-instance (Tier-2) file registry,
                    // and its host-global sibling (Tier 2b, issue #1916) —
                    // this auto-register path bypasses the HTTP register
                    // handler entirely, so it needs its own mirror call
                    // exactly like that handler does.
                    if let Ok(local_url) = std::env::var("AGENTMUX_LOCAL_URL") {
                        let data_dir = crate::backend::base::get_mux_data_dir();
                        crate::backend::reactive::registry::write_with_nonce(
                            &data_dir,
                            agent_id,
                            &local_url,
                            &self.block_id,
                            my_registration_nonce,
                        );
                        crate::backend::reactive::registry::write_shared_from_env_with_nonce(
                            agent_id,
                            &local_url,
                            &self.block_id,
                            my_registration_nonce,
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        block_id = %self.block_id,
                        agent_id = %agent_id,
                        error = %e,
                        "muxbus: persistent auto-register failed"
                    );
                    // reagent P1 on PR #3084: registration was skipped, so
                    // `my_registration_nonce` was never written to the
                    // handler. Using it as this spawn's exit-time cleanup
                    // key would always mismatch whatever nonce IS on
                    // record, making the exit-handler wrongly conclude "a
                    // newer respawn already took over" and skip BOTH the
                    // reactive-handler unregister and the cloud_subscriber
                    // deregister — leaking both on every skip. Target
                    // whichever nonce is actually on record instead, so
                    // this spawn's own eventual exit can still correctly
                    // clean it up. `registration_nonce == 0` (no nonce on
                    // record at all) is handled safely by
                    // `unregister_block_if_nonce` itself — it never
                    // matches, so this degrades to today's no-cleanup
                    // behavior rather than a wrong one.
                    //
                    // `try_get_agent_by_block`, NOT the plain
                    // `get_agent_by_block` — this Err arm is reached
                    // precisely when we might be on the same thread as an
                    // in-flight `inject_message` (reagent P0 on PR #3084,
                    // caught after the first attempt used the blocking
                    // version here and reproduced INCIDENT_2026_09_07's
                    // exact deadlock). In that reentrant case this
                    // correctly returns `None` after its own retry budget
                    // instead of hanging forever; `exit_cleanup_nonce`
                    // then simply stays at its default, same safe
                    // no-cleanup degradation as before this fix existed.
                    if let Some(current) = crate::backend::reactive::get_global_handler()
                        .try_get_agent_by_block(&self.block_id)
                    {
                        exit_cleanup_nonce = current.registration_nonce;
                    }
                }
            }
        }

        // Record active pid for crash recovery (Phase 4.2). If the server
        // dies while this subprocess is running, scan_orphans() will find
        // the stale pid on next boot and flag the session as interrupted.
        if let Some(ref mstore) = self.mstore {
            super::super::session_recovery::mark_active_pid(mstore, &self.block_id, pid);
        }

        // Spawn stdin writer task
        tokio::spawn(async move {
            let mut stdin = stdin;
            let mut continuation = continuation;
            while let Some(msg) = msg_rx.recv().await {
                // Only this write carries the packet; the pane's record keeps
                // the message as typed. Control responses pass through.
                let msg = match continuation
                    .as_deref()
                    .and_then(|packet| crate::backend::continuity::prefix_user_message(&msg, packet))
                {
                    Some(with_packet) => {
                        continuation = None;
                        with_packet
                    }
                    None => msg,
                };
                if let Err(e) = stdin.write_all(msg.as_bytes()).await {
                    tracing::warn!("persistent stdin write error: {}", e);
                    break;
                }
                if let Err(e) = stdin.write_all(b"\n").await {
                    tracing::warn!("persistent stdin newline error: {}", e);
                    break;
                }
                if let Err(e) = stdin.flush().await {
                    tracing::warn!("persistent stdin flush error: {}", e);
                    break;
                }
            }
            // Channel closed or write error → stdin drops → process gets EOF
            drop(stdin);
        });

        // Spawn stdout reader task
        let block_id_read = self.block_id.clone();
        let broker_read = self.broker.clone();
        let inner_read = Arc::clone(&self.inner);
        let mstore_read = self.mstore.clone();
        let event_bus_read = self.event_bus.clone();
        let filestore_read = self.filestore.clone();
        let health_read = Arc::clone(&self.health_monitor);
        // Weak, like every other self-reference here: the reader task must not
        // keep the controller alive. Used only for the deferred
        // runtime-config restart at turn end (`request_restart_when_idle`).
        let self_ref_read = self.self_ref.lock().unwrap().clone();
        let stdout_seq_read = Arc::clone(&self.stdout_seq);
        let session_id_field = config.session_id_field.clone();
        let my_generation_read = my_generation;
        // Resolve the agent's GLOBAL transcript zone (`agent:<defId>:current`)
        // once, from the block's `agentId` meta, so every `output` line is also
        // mirrored to the cross-channel store. `None` for non-agent blocks.
        let global_output_zone =
            super::super::shell::resolve_global_output_zone(&self.mstore, &self.block_id);
        // Cloned before `global_output_zone` moves into the stdout-reader
        // task below — the process-waiter task (spawned further down) needs
        // its own copy for the segment-end record and to append the resume
        // state machine's exit-time lines (a held-back error-result line, a
        // session-outcome event).
        let global_output_zone_wait = global_output_zone.clone();
        // Writer-side fence on that shared record
        // (SPEC_AGENT_SINGLE_LIVE_INSTANCE_2026_09_24 Phase 3): once this
        // process has lost the agent's lease, its output still reaches this
        // pane's own log but no longer the agent's record. `None` when no
        // lease is held (no store, or no UID) — nothing to fence.
        let record_fence = agent_lease.as_ref().map(|l| l.record_fence());
        let record_fence_wait = record_fence.clone();

        // codex P1 on PR #2371: the JoinHandle is kept (not discarded) so the
        // process-waiter task can await this task's full completion before
        // resolving the retry decision, mirroring `stderr_reader_handle`
        // below. `child.wait()` resolving is NOT proof this task has already
        // read and held back the doomed attempt's terminal error-result line
        // (`ResumeState`'s `held_error_line`) — without this wait, the
        // waiter could resolve the resume state (`ProcessExited`) and launch
        // the retry first, after which this (now-lagging) reader would find
        // the state already `NotTracking` and persist the error line
        // immediately, reproducing the exact bubble this PR exists to
        // suppress.
        let stdout_reader_handle: tokio::task::JoinHandle<()> = tokio::spawn(Self::run_stdout_reader(StdoutReaderCtx {
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
            relocated_copy_read,
            forked_from_read,
        }));

        self.spawn_status_heartbeat();

        // Spawn process waiter task
        let block_id_wait = self.block_id.clone();
        let inner_wait = Arc::clone(&self.inner);
        let broker_wait = self.broker.clone();
        let mstore_wait = self.mstore.clone();
        // Needed to persist a classified failure (rate-limit/overloaded/etc.)
        // into block meta alongside the MPS publish below — mirrors
        // `event_bus_read`'s equivalent clone for the stdout-reader task.
        let event_bus_wait = self.event_bus.clone();
        let health_wait = Arc::clone(&self.health_monitor);
        // Needed to append the resume state machine's exit-time lines: a
        // held-back error-result line when the stale-resume retry is NOT
        // confirmed (or is overridden by an explicit stop), and the
        // session-outcome event — see the exit-handler's own comment in
        // `process_waiter.rs`.
        let filestore_wait = self.filestore.clone();
        // Captured so the waiter can deregister this agent from muxbus on exit.
        let agent_id_wait = agent_id_for_muxbus.clone();
        // See `set_self_ref` / `retry_after_resume_failure` — lets this
        // detached task call back into an instance method once the process
        // actually exits, to transparently retry a stale-`--resume` failure.
        let self_ref_wait = self.self_ref.lock().unwrap().clone().unwrap_or_default();
        // This exact spawn's identity — every resume event below carries it,
        // and `persistent_resume::update()` ignores one whose generation
        // isn't the tracked one (see that module's doc comment).
        let my_generation_wait = my_generation;
        // This exact spawn's OS pid, for the compare-and-clear below — the
        // unconditional clear could wipe a fallback respawn's fresh
        // registration (issue #2363, see clear_active_pid_if_pid).
        let pid_wait = pid;
        // This spawn's registration identity, for the guarded
        // muxbus/registry removals below (issue #2363 / codex P1 on PR
        // #2500 — see `my_registration_nonce`'s own doc comment). NOT
        // `my_registration_nonce` directly — `exit_cleanup_nonce` is that
        // same value UNLESS registration was skipped above, in which case
        // it's whatever nonce actually ended up on record instead (reagent
        // P1 on PR #3084; see the skip arm's own comment for why using our
        // own never-written nonce here would leak the registration).
        let nonce_wait = exit_cleanup_nonce;

        tokio::spawn(Self::run_process_waiter(ProcessWaiterCtx {
            child,
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
        }));

        Ok(())
    }
}
