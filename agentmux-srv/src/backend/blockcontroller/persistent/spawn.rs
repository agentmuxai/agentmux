// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `spawn_process`: builds the CLI argv/env, spawns the child, and starts the
//! per-session I/O tasks (stdin writer, stdout reader, stderr reader, waiter).
//! Moved here verbatim; extracting the task bodies is a separate change.

use super::*;

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
            }
        }

        // Append `--resume <sid>` when we have a session id and the provider
        // supports simple-flag resume — same construction as
        // SubprocessController::spawn_turn. This is what makes a model/effort
        // change (which respawns the persistent CLI with new flags) preserve the
        // conversation. cli_args carries the runtime flags (model/effort/perm)
        // already rebuilt by the frontend (useAgentCommands buildRuntimeArgs).
        let mut spawn_args = config.cli_args.clone();
        // Recorded so the stderr reader can tell "No conversation found" apart
        // from an unrelated CLI error, and so it knows exactly which id to
        // poison against the stdout reader's own capture (see below) — a
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
        // One live instance per agent, across every AgentMux instance on this
        // host (SPEC_AGENT_SINGLE_LIVE_INSTANCE_2026_09_24.md Phase 1). The
        // check above only sees this process; this lease is host-global.
        // Claimed here — the one point all four spawn paths pass — and held
        // for this process's lifetime (stored below once the child runs).
        // A refusal returns before anything is spawned; any early return
        // after this point drops the handle, which releases the lease.
        let agent_lease = self.acquire_agent_lease(&config, requested_sid.as_deref())?;
        {
            let inner = self.inner.lock().unwrap();
            if let Some(ref sid) = inner.session_id {
                if !config.resume_flag.is_empty() {
                    spawn_args.push(config.resume_flag.clone());
                    spawn_args.push(sid.clone());
                    attempted_resume_sid = Some(sid.clone());
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
        // Matches acp.rs / subprocess.rs; sibling of shell.rs's PTY path.
        #[cfg(windows)]
        {
            use agentmux_common::win32::CREATE_NO_WINDOW;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
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
            // The replacement process is here, so the quiesce window is over —
            // messages may take `DeliverDirect` against this new `stdin_tx`
            // again. Cleared unconditionally rather than only when set: any
            // spawn ends the window by definition, whatever opened it.
            inner.restart_pending = false;
            // Same for a stop request: it targeted the process this spawn
            // replaces.
            inner.stop_pending = false;
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
            continuation = self.continuation_packet();
            if let Some(ref packet) = continuation {
                tracing::info!(
                    block_id = %self.block_id,
                    packet_chars = packet.len(),
                    "continuity: carrying AgentMux's record of the conversation into the fresh session"
                );
            }
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

        // Assign the persistent CLI to this block's process tracker.
        // Matches `SubprocessController`'s identical path — both controller
        // types share the same swarm-pane visibility story.
        if pid != 0 {
            crate::backend::process_tracker::registry::track_spawned(&self.block_id, pid);
        }

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
        // call below. See the process-waiter's own comment for the two
        // failure modes this closes.
        let stderr_reader_handle: Option<tokio::task::JoinHandle<()>> = stderr.map(|stderr_pipe| {
            let block_id_stderr = self.block_id.clone();
            let inner_stderr = Arc::clone(&self.inner);
            let mstore_stderr = self.mstore.clone();
            let event_bus_stderr = self.event_bus.clone();
            let attempted_resume_sid = attempted_resume_sid.clone();
            let my_generation_stderr = my_generation;
            tokio::spawn(async move {
                let mut reader = BufReader::new(stderr_pipe).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    tracing::warn!(
                        block_id = %block_id_stderr,
                        line = %line,
                        "persistent stderr"
                    );
                    // Claude Code's own message when `--resume <sid>` targets a
                    // conversation its current CLAUDE_CONFIG_DIR can't see — e.g.
                    // after a relogin/reseed moves the agent onto a different
                    // config dir than the one the session was recorded under. Left
                    // uncleared, EVERY future respawn (one per message, since a
                    // dead persistent process auto-restarts on next send) keeps
                    // retrying the same unreachable --resume and immediately
                    // exits again — a permanent "Agent encountered an error" with
                    // no path to recovery. Clear it so the next respawn starts a
                    // fresh conversation instead.
                    if line.contains("No conversation found with session ID") {
                        if let Some(ref bad_sid) = attempted_resume_sid {
                            // See PersistentInner::poison_resume — also guards
                            // against the stdout reader's own capture (below)
                            // re-adopting this same dead id if it wins the race.
                            inner_stderr.lock().unwrap().poison_resume(bad_sid, my_generation_stderr);
                            tracing::warn!(
                                block_id = %block_id_stderr,
                                session_id = %bad_sid,
                                "stale --resume session id unreachable under the current config dir — \
                                 clearing so the next message starts a fresh conversation"
                            );
                            core::persist_session_id(&block_id_stderr, "", &mstore_stderr, &event_bus_stderr);
                            // Surface this to the user — previously silent
                            // (only the warn! above). See
                            // SPEC_PANE_CLOSE_REOPEN_CONTINUITY_GUARANTEE_2026_07_27.md
                            // §4.2: a resumed conversation silently starting
                            // fresh, with no indication anything happened, is
                            // exactly the failure mode this flag exists to close.
                            if let Some(ref store) = mstore_stderr {
                                crate::backend::blockcontroller::session_recovery::mark_resume_failed(
                                    store,
                                    &event_bus_stderr,
                                    &block_id_stderr,
                                );
                            }
                        }
                    }
                }
            })
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
                continuation.is_some(),
                agent_lease.as_ref().map(|l| l.epoch()),
            )
        });
        *self.current_segment.lock().unwrap() = segment.as_ref().map(|(_, id)| id.clone());
        let segment_read = segment.clone();
        let segment_wait = segment;
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
        // its own copy to flush a held-back `pending_error_result_line`.
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
        // read and stashed the doomed attempt's terminal error-result line
        // in `pending_error_result_line` — without this wait, the waiter
        // could clear `pending_resume_retry` and launch the retry first,
        // after which this (now-lagging) reader would find
        // `pending_resume_retry` already `None` and append the error line
        // immediately, reproducing the exact bubble this PR exists to
        // suppress.
        let stdout_reader_handle: tokio::task::JoinHandle<()> = tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            let mut stats = super::super::session_stats::SessionStatsAccumulator::new(block_id_read.clone());

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
                // this exact attempt — see
                // `PersistentInner::pending_error_result_line`. `false` for
                // every other line, matching today's behavior exactly.
                let mut hold_back_for_resume_retry = false;
                // A deferred message released at this line's turn boundary.
                // Its stdin write happens at the boundary (atomically with the
                // idle decision), but its blockfile append waits until THIS
                // line — the `result` that ended the previous turn — has been
                // appended, so the transcript and live consumers see
                // `result(N)` before `user(N+1)`, never the reverse (codex P1
                // on #3562).
                let mut released_deferred_line: Option<String> = None;

                // Parse JSON for control-frame handling, turn-active tracking,
                // and session ID capture
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&line) {
                    // Control-protocol frames (can_use_tool / AskUserQuestion) are
                    // NOT conversation output — handle them and skip the blockfile
                    // so the frontend stream never sees them.
                    // Spec: docs/specs/SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md.
                    if let Some(kind) = parsed.get("type").and_then(|v| v.as_str()) {
                        if kind == "control_request" || kind == "control_response" {
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
                        // so stop the process: the next message respawns it,
                        // and `input.rs` rebuilds `cli_args` from block meta
                        // at that point, so the new flags take effect then.
                        // Not `poison_resume` and not a user Stop — the
                        // session id is retained, so the respawn `--resume`s
                        // the same conversation.
                        // `set_active_turn(false)` happens under `inner` so it
                        // serializes with `request_restart_when_idle`'s own
                        // check — see that method for the interleaving this
                        // closes. `restart_pending` is committed in the SAME
                        // acquisition so no send can slip into `DeliverDirect`
                        // between the decision and the kill.
                        // The turn boundary is also the flush point for
                        // messages deferred by
                        // `SPEC_NO_MIDTURN_DELIVERY_2026_09_23.md` §4.4.
                        //
                        // Exactly ONE message is released per boundary, never
                        // the whole queue: writing to stdin starts a new turn,
                        // so a burst drain would write message #2 while #1's
                        // turn was already running — the precise mid-turn write
                        // this exists to prevent. The next queued message goes
                        // out on *that* new turn's own `result` frame.
                        //
                        // Which outcomes end the turn, and when the deferred
                        // restart may apply: see `turn_boundary_locked`.
                        let boundary = PersistentSubprocessController::turn_boundary_locked(
                            &mut inner_read.lock().unwrap(),
                            &health_read,
                            &block_id_read,
                            my_generation_read,
                        );
                        // `None`: this reader's process has been replaced. Its
                        // `result` ends nothing the current process is doing,
                        // so it must not flush into, idle, or restart that
                        // process (codex P1 on #3562).
                        let boundary_is_current = boundary.is_some();
                        let turn_still_active = boundary.as_ref().is_some_and(|b| b.turn_still_active());
                        let (flushed, deferred_restart) = match boundary {
                            Some(b) => (b.flushed, b.apply_deferred_restart),
                            None => (DeferredFlush::Empty, false),
                        };
                        match flushed {
                            DeferredFlush::Released(line) => {
                                tracing::info!(
                                    block_id = %block_id_read,
                                    "turn ended — released one deferred message"
                                );
                                released_deferred_line = Some(line);
                            }
                            DeferredFlush::Held | DeferredFlush::Failed => {
                                if let Some(ctrl) = self_ref_read.as_ref().and_then(|w| w.upgrade()) {
                                    ctrl.ensure_deferred_watchdog();
                                }
                            }
                            DeferredFlush::Empty => {}
                        }
                        if deferred_restart {
                            tracing::info!(
                                block_id = %block_id_read,
                                "turn ended — applying the deferred runtime-config restart"
                            );
                            if let Some(ctrl) = self_ref_read.as_ref().and_then(|w| w.upgrade()) {
                                let _ = ctrl.stop_process(false);
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
                                BlockControllerRuntimeStatus {
                                    blockid: block_id_read.clone(),
                                    version: locked.status_version,
                                    shellprocstatus: locked.proc_status.clone(),
                                    shellprocconnname: "local".to_string(),
                                    shellprocexitcode: locked.proc_exit_code,
                                    shellprocpid: None,
                                    shellprocname: String::new(),
                                    spawn_ts_ms: None,
                                    is_agent_pane: true,
                                    // NOT unconditionally false: if a deferred
                                    // message was just released, or an earlier
                                    // writer's prompt is still running (`Held`),
                                    // the turn is live and publishing "idle" here
                                    // would hand subscribers (the Swarm badge,
                                    // `trackTurnJustEnded`) a bogus end-of-turn
                                    // for a turn that is actively in flight.
                                    turn_active: turn_still_active,
                                }
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
                            // to (re-)adopt an id the stderr reader (above) already
                            // confirmed unreachable, whichever task wins the race.
                            let (should_capture, capture_effects) = inner_read.lock().unwrap().try_capture_session_id(
                                &sid_string,
                                my_generation_read,
                                is_confirmed_success,
                            );
                            if should_capture {
                                tracing::info!(
                                    block_id = %block_id_read,
                                    session_id = %sid_string,
                                    "persistent session ID captured"
                                );
                                core::persist_session_id(&block_id_read, &sid_string, &mstore_read, &event_bus_read);
                                super::segments::record_segment_session(&segment_read, &sid_string);
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
                                        if let Some(failure) = classify_exit_line(None, &line) {
                                            flushed_failure_this_tick = true;
                                            core::persist_last_failure(&block_id_read, Some(&failure), &mstore_read, &event_bus_read);
                                            if let Some(ref broker) = broker_read {
                                                broker.publish(mps::MuxEvent {
                                                    event: mps::EVENT_AGENT_FAILURE.to_string(),
                                                    scopes: vec![format!("block:{}", block_id_read)],
                                                    sender: String::new(),
                                                    persist: 1,
                                                    data: serde_json::to_value(&failure).ok(),
                                                });
                                            }
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
                                        if let Some(failure) = classify_exit_line(None, &old_line) {
                                            flushed_failure_this_tick = true;
                                            core::persist_last_failure(&block_id_read, Some(&failure), &mstore_read, &event_bus_read);
                                            if let Some(ref broker) = broker_read {
                                                broker.publish(mps::MuxEvent {
                                                    event: mps::EVENT_AGENT_FAILURE.to_string(),
                                                    scopes: vec![format!("block:{}", block_id_read)],
                                                    sender: String::new(),
                                                    persist: 1,
                                                    data: serde_json::to_value(&failure).ok(),
                                                });
                                            }
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
                    // invariant below at the ProcessExited/FireRetry arm) —
                    // classify() only runs once this error is confirmed final.
                    if is_error_result && !hold_back_for_resume_retry {
                        let failure = crate::agents::failure::classify(None, None, "", Some(&parsed));
                        core::persist_last_failure(&block_id_read, Some(&failure), &mstore_read, &event_bus_read);
                        if let Some(ref broker) = broker_read {
                            broker.publish(mps::MuxEvent {
                                event: mps::EVENT_AGENT_FAILURE.to_string(),
                                scopes: vec![format!("block:{}", block_id_read)],
                                sender: String::new(),
                                persist: 1,
                                data: serde_json::to_value(&failure).ok(),
                            });
                        }
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
                    // This `result` is held back, not persisted — but the
                    // message already went to stdin, so it must still render.
                    if let Some(released) = released_deferred_line.take() {
                        if let Some(ctrl) = self_ref_read.as_ref().and_then(|w| w.upgrade()) {
                            ctrl.append_delivered_message(&released);
                        }
                    }
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
                let line_with_newline = format!("{}\n", line);
                if let Some(ref broker) = broker_read {
                    super::super::shell::handle_append_block_file(
                        broker,
                        &block_id_read,
                        PERSISTENT_OUTPUT_SUBJECT,
                        line_with_newline.as_bytes(),
                        filestore_read.as_ref(),
                        crate::backend::agent_admission::fenced_zone(global_output_zone.as_deref(), &record_fence),
                    );
                } else {
                    tracing::warn!(block_id = %block_id_read, "persistent stdout: no broker available");
                }
                if let Some(released) = released_deferred_line.take() {
                    if let Some(ctrl) = self_ref_read.as_ref().and_then(|w| w.upgrade()) {
                        ctrl.append_delivered_message(&released);
                    }
                }
            }

            tracing::info!(block_id = %block_id_read, "persistent stdout reader finished");
        });

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
        // Needed only to flush a held-back `pending_error_result_line` when
        // the stale-resume retry is NOT confirmed (or is overridden by an
        // explicit stop) — see the exit-handler's own comment below.
        let filestore_wait = self.filestore.clone();
        // Captured so the waiter can deregister this agent from muxbus on exit.
        let agent_id_wait = agent_id_for_muxbus.clone();
        // See `set_self_ref` / `retry_after_resume_failure` — lets this
        // detached task call back into an instance method once the process
        // actually exits, to transparently retry a stale-`--resume` failure.
        let self_ref_wait = self.self_ref.lock().unwrap().clone().unwrap_or_default();
        // This exact spawn's identity — see `stop_requested_generation`'s
        // doc comment for why the retry decision below needs it.
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

        tokio::spawn(async move {
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
                    // could clear `pending_resume_retry` below before the
                    // stderr reader ever promotes it, permanently losing
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
                    if let Some(handle) = stderr_reader_handle {
                        let abort_handle = handle.abort_handle();
                        if tokio::time::timeout(std::time::Duration::from_millis(500), handle).await.is_err() {
                            tracing::warn!(
                                block_id = %block_id_wait,
                                "stderr reader did not finish within 500ms of process exit — aborting it"
                            );
                            abort_handle.abort();
                        }
                    }

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
                    let abort_handle = stdout_reader_handle.abort_handle();
                    if tokio::time::timeout(std::time::Duration::from_secs(10), stdout_reader_handle)
                        .await
                        .is_err()
                    {
                        tracing::warn!(
                            block_id = %block_id_wait,
                            "stdout reader did not finish within 10s of process exit \
                             (SQLite contention, or a descendant process holding stdout open?) \
                             — aborting it"
                        );
                        abort_handle.abort();
                    }

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
                    drop(inner);

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
                            if registration_was_ours {
                                if let Some(sub) = crate::muxbus::cloud_subscriber::get_global_subscriber() {
                                    sub.remove_agent(agent_id);
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
                                // above — this arm reaches `Fresh` today, but
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
                                if let Some(failure) = classify_exit_line(Some(exit_code), &line) {
                                    core::persist_last_failure(&block_id_wait, Some(&failure), &mstore_wait, &event_bus_wait);
                                    if let Some(ref broker) = broker_wait {
                                        broker.publish(mps::MuxEvent {
                                            event: mps::EVENT_AGENT_FAILURE.to_string(),
                                            scopes: vec![format!("block:{}", block_id_wait)],
                                            sender: String::new(),
                                            persist: 1,
                                            data: serde_json::to_value(&failure).ok(),
                                        });
                                    }
                                }
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
                                    let status = BlockControllerRuntimeStatus {
                                        blockid: block_id_wait.clone(),
                                        version: 0,
                                        shellprocstatus: STATUS_DONE.to_string(),
                                        shellprocconnname: "local".to_string(),
                                        shellprocexitcode: exit_code,
                                        shellprocpid: None,
                                        shellprocname: String::new(),
                                        spawn_ts_ms: None,
                                        is_agent_pane: true,
                                        turn_active: false,
                                    };
                                    super::super::publish_controller_status(broker, &status);
                                }
                            }
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

                    // reagentx P1 on PR #2371: mirror the child.wait() arm's
                    // bounded await+abort of both reader tasks (above,
                    // codex P1) before taking `pending_error_result_line`
                    // below. Without this, a stop racing the doomed
                    // attempt's in-flight terminal error line could take
                    // `None` here while the stdout reader is still about to
                    // stash it — silently losing a genuine error the stop
                    // itself interrupted, and leaving a stale stash for a
                    // LATER, unrelated exit on a reused controller instance
                    // to wrongly pick up.
                    if let Some(handle) = stderr_reader_handle {
                        let abort_handle = handle.abort_handle();
                        if tokio::time::timeout(std::time::Duration::from_millis(500), handle).await.is_err() {
                            tracing::warn!(
                                block_id = %block_id_wait,
                                "stderr reader did not finish within 500ms of kill — aborting it"
                            );
                            abort_handle.abort();
                        }
                    }
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
                    let abort_handle = stdout_reader_handle.abort_handle();
                    if tokio::time::timeout(std::time::Duration::from_secs(10), stdout_reader_handle)
                        .await
                        .is_err()
                    {
                        tracing::warn!(
                            block_id = %block_id_wait,
                            "stdout reader did not finish within 10s of kill \
                             (SQLite contention, or a descendant process holding stdout open?) \
                             — aborting it"
                        );
                        abort_handle.abort();
                    }

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
                    drop(inner);

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
                            if registration_was_ours {
                                if let Some(sub) = crate::muxbus::cloud_subscriber::get_global_subscriber() {
                                    sub.remove_agent(agent_id);
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
                }
            }
        });

        Ok(())
    }
}
