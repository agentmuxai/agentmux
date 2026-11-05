// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `spawn_process`: builds the CLI argv/env, spawns the child, and starts the
//! per-session I/O tasks (stdin writer, stdout reader, stderr reader, waiter).
//! The stdin writer runs inline; the other three bodies live in
//! `stderr_reader.rs`, `stdout_reader.rs` and `process_waiter.rs`.

use super::*;

/// Whether a spawn resumes with `--fork-session`: the resume gate placed a
/// copy to fork from, the session grew outside AgentMux (`fork_next`), or the
/// launch itself asks to (a quick fork from the picker carries the flag in
/// its own args). One answer for the duplicate-session guard and for the
/// fork's bookkeeping, so the two can't disagree.
pub(super) fn spawn_forks(fork_copy: bool, fork_next: bool, cli_args: &[String]) -> bool {
    fork_copy || fork_next || cli_args.iter().any(|a| a == "--fork-session")
}

impl PersistentSubprocessController {
    /// Spawn the persistent CLI process. Called only while the caller
    /// holds the exclusive spawn claim (`spawning_in_progress`, see
    /// `decide_send_action`) — never directly.
    pub(super) fn spawn_process(
        &self,
        config: PersistentSpawnConfig,
        resume_retry_payload: Option<persistent_resume::QueuedRetryEntry>,
    ) -> Result<(), String> {
        let mut launch = self.start_child(&config, resume_retry_payload)?;
        let pipes = self.take_pipes(&config, &mut launch)?;
        let msg_rx = self.mark_running(&mut launch, pipes.kill_tx);
        let registered = self.register_spawn(&config, &launch);
        let sent_prompts = self.start_stdin_writer(&mut launch, pipes.stdin, msg_rx);
        let stdout_task = self.spawn_stdout_reader(pipes.stdout, &config, &launch, &registered, sent_prompts);
        self.spawn_status_heartbeat();
        self.spawn_waiter(launch, pipes.kill_rx, pipes.stderr_reader_handle, stdout_task, &registered);

        Ok(())
    }
}

impl PersistentSubprocessController {
    /// Hydrate the captured session id from the config when we don't have one yet (fresh controller after a
    /// forced restart, e.g. a /model change, or the picker reattach path). Mirrors
    /// `SubprocessController::hydrate_session_id_from_config` so the respawn resumes the same conversation
    /// instead of starting blank.
    pub(super) fn hydrate_session_id_from_config(&self, config: &PersistentSpawnConfig) {
        if config.session_id.is_empty() {
            return;
        }
        let mut inner = self.inner.lock().unwrap();
        // Not a retired one (`Agents fresh`): a config prepared before the transition still names it.
        if inner.session_id.is_none() && inner.resume_poisoned.as_deref() != Some(config.session_id.as_str()) {
            inner.session_id = Some(config.session_id.clone());
        }
    }
}

impl PersistentSubprocessController {
    /// A first spawn with no session id onto a pane that renders prior history continues that history's
    /// session instead of pairing it with a blank model (SPEC_DURABLE_CONVERSATION_MEMORY_2026_09_23.md
    /// §1.1, §5 P0a). Returns the session the history belongs to when `--resume` can't reach it here, for
    /// the resume gate to relocate. `inherited` is the parent session a fork inherited.
    pub(super) fn continue_first_spawn_history(&self, config: &PersistentSpawnConfig, inherited: Option<&str>) -> Option<String> {
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
        // A fork's history is a copy of its parent's, ending in the parent's
        // session; that session is continued as a fork (below), so its parent
        // being live is no reason to start blank.
        if first_spawn_without_sid {
            if let Some(sid) = self.find_continuation_session_id(config) {
                if inherited == Some(sid.as_str()) || self.session_held_elsewhere(&sid).is_none() {
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
        unreachable_history_session
    }

    /// The CLI's argv for this spawn, and the claims that go with it: `--resume <sid>` when there is a
    /// session and the provider supports it, `--fork-session` when the spawn forks, the agent's memory
    /// reconciled into its folder, and the per-session spawn lock held until the caller drops
    /// [`ResumeSpawn::claim`] (once its `current_pid` is set). Refused when another live process holds the
    /// session.
    pub(super) fn resume_spawn(&self, config: &PersistentSpawnConfig) -> Result<ResumeSpawn, String> {
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
        // (picker reattach, a new agent's launch, MCP), not just `agent.open`
        // (SPEC_AGENT_PANE_CLOSE_GRACEFUL_SHUTDOWN_2026_09_18.md §4.7).
        // A spawn that forks reads the session and writes a new one, so a
        // live process on it is no conflict: a fork started while its parent
        // runs (the fork-isolation report in #4744, F5). `spawn_forks` says
        // when; a quick fork from the picker was refused here while its
        // parent ran, because only the gate's own marks counted.
        let requested_sid = if config.resume_flag.is_empty() {
            None
        } else {
            let inner = self.inner.lock().unwrap();
            if spawn_forks(inner.fork_copy.is_some(), inner.fork_next, &config.cli_args) {
                None
            } else {
                inner.session_id.clone()
            }
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
            self.reconcile_memory_before_spawn(config);
        }
        let resume_claim = requested_sid.as_deref().map(lock_resume_spawn);
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
                    if spawn_forks(fork_copy.is_some(), fork_next, &config.cli_args) {
                        // The launch's own args may carry it already (a fork
                        // launched from the picker); once is enough.
                        if !spawn_args.iter().any(|a| a == "--fork-session") {
                            spawn_args.push("--fork-session".to_string());
                        }
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
        Ok(ResumeSpawn { spawn_args, attempted_resume_sid, relocated_copy, forked, claim: resume_claim })
    }

    /// Install the new process as the controller's current one: bumps `spawn_generation`, clears the
    /// per-spawn flags the previous process left, and feeds the resume state machine. Returns the new
    /// generation and the effects of superseding a prior one, with the child handed back. Refused (and the
    /// child killed) when a Stop cancelled a config restart's replacement while it was starting.
    ///
    /// The tentative resume-retry payload is stashed here, synchronously, before any background task
    /// exists: a process that dies fast enough would otherwise let the process waiter take it while it is
    /// still `None`, losing the retry for the very case it exists to catch (#2360). It is keyed on the exact
    /// sid this spawn attempted, so `poison_resume`'s later confirmation is unambiguous.
    pub(super) fn install_new_process(
        &self,
        mut child: tokio::process::Child,
        config: &PersistentSpawnConfig,
        attempted_resume_sid: &Option<String>,
        resume_retry_payload: Option<persistent_resume::QueuedRetryEntry>,
    ) -> Result<(tokio::process::Child, u64, Vec<persistent_resume::ResumeEffect>), String> {
        let mut inner = self.inner.lock().unwrap();
        // A config restart's replacement is installed only if no Stop has
        // cancelled it, decided in this acquisition: before it, a Stop has
        // no `kill_tx` to reach the child through (codex P1 on #3990).
        if let Err(e) = Self::commit_restart_spawn_locked(&mut inner) {
            drop(inner);
            let _ = agentmux_common::process::start_kill_child_and_group(&mut child);
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
        crate::backend::mcp_rewrite::remember_workdir(&self.block_id, &config.working_dir);
        // Requests parked for the user belong to the process that asked: a new process is not
        // waiting on them. Left in place, an abandoned question would read the new turns as
        // `waiting` (`Controller::awaiting_user_reply`).
        super::input::forget_parked_requests(&mut inner);
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
        Ok((child, generation, effects))
    }

    /// This fresh spawn can supersede a prior generation still awaiting its outcome with a held error line
    /// (`resolve_superseded_generation`, reachable via `respawn_once_for_leftover_queue`): flush it now,
    /// outside the lock, like every other `ResumeEffect` call site, or the error disappears (#2368).
    pub(super) fn flush_superseded_effects(&self, effects: Vec<persistent_resume::ResumeEffect>) {
        for effect in effects {
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
    }

    /// Say, in the transcript and before any of this process's own output lands, that it starts with none
    /// of the conversation the pane is about to display (`fresh_start_needs_disclosure` decides when).
    /// Returns the continuation packet that rides on its first message when AgentMux's own record is
    /// carried across.
    pub(super) fn disclose_fresh_start(&self, attempted_resume_sid: &Option<String>, my_generation: u64, had_retry_payload: bool) -> Option<String> {
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
        // An `Agents fresh` start carries the agent's handoff alone: the packet
        // tells the model to continue as if uninterrupted, the opposite of what
        // it asked for. The old session stays searchable.
        let by_choice = crate::backend::context_reading::take_fresh_by_choice(&self.block_id);
        if fresh_onto_history && !by_choice {
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
        // Also for a start the agent asked for (generation > 1): the pane's
        // marker, its meter reset and the memory fallback follow from it.
        if fresh_onto_history && (by_choice || fresh_start_needs_disclosure(attempted_resume_sid.as_deref(), my_generation)) {
            tracing::info!(
                block_id = %self.block_id,
                continued = continuation.is_some(),
                "spawned with no --resume while prior history exists — disclosing a fresh start"
            );
            // Idle when no message flows with this spawn (the same test as the
            // turn-active check below).
            // A start the agent asked for always has its handoff in flight.
            let idle_start = !by_choice && !had_retry_payload && self.inner.lock().unwrap().pending_send_messages.is_empty();
            self.emit_fresh_outcome_now(String::new(), continuation.is_some(), idle_start);
        }
        continuation
    }

    /// Auto-register with the muxbus reactive handler so inter-agent messages reach this persistent
    /// (no-PTY) agent (#1470); delivery is routed through the controller-aware MessageSender. Returns the
    /// nonce this spawn's exit handler cleans up by: its own, unless registration was skipped, in which case
    /// whichever nonce is on record (#3084).
    pub(super) fn register_with_muxbus(&self, agent_id_for_muxbus: &Option<String>, agent_uid_for_registry: &Option<String>, my_registration_nonce: u64) -> u64 {
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
                    mirror_registration_to_files(agent_id, &self.block_id, my_registration_nonce);
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
                    if let Some(current) = nonce_on_record(&self.block_id) {
                        exit_cleanup_nonce = current;
                    }
                }
            }
        }
        exit_cleanup_nonce
    }

    /// The task that writes messages to the CLI's stdin, one JSON line each. Only the first user message
    /// carries the continuation packet; the pane's record keeps it as typed. A closed channel or a write
    /// error ends the task, dropping stdin so the process gets EOF.
    pub(super) fn spawn_stdin_writer(
        stdin: tokio::process::ChildStdin,
        mut msg_rx: mpsc::Receiver<String>,
        continuation: Option<String>,
        sent_prompts_write: crate::backend::cli_native::sent::SentPrompts,
    ) {
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
                let msg = sent_prompts_write.tag(&msg);
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
    }
}

/// What `resume_spawn` decides for one spawn.
pub(super) struct ResumeSpawn {
    pub(super) spawn_args: Vec<String>,
    /// The session this spawn resumes (or forks), if any; the stderr and stdout readers poison it when the
    /// CLI rejects it.
    pub(super) attempted_resume_sid: Option<String>,
    /// A relocated resume's copy, removed once the fork reports its id.
    pub(super) relocated_copy: Option<crate::backend::continuity_relocate::Relocated>,
    /// This spawn resumes with `--fork-session`.
    pub(super) forked: bool,
    /// The per-session spawn lock: held until this spawn's `current_pid` is set, so two reopens of one
    /// session can't both pass the duplicate-session check.
    pub(super) claim: Option<std::sync::MutexGuard<'static, ()>>,
}

/// The spawn lock for one session. A stripe of locks chosen by hashing the session id: every reopen of one
/// session takes the same lock, unrelated agents' resume respawns almost never share one, and memory stays
/// bounded (a per-session map would grow forever). Only resume spawns take one; `spawn_process` is
/// synchronous, and no caller holds an `inner` lock across it.
fn lock_resume_spawn(sid: &str) -> std::sync::MutexGuard<'static, ()> {
    const STRIPES: usize = 64;
    static RESUME_SPAWN_LOCKS: [Mutex<()>; STRIPES] = [const { Mutex::new(()) }; STRIPES];
    let stripe = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        sid.hash(&mut h);
        (h.finish() as usize) % STRIPES
    };
    RESUME_SPAWN_LOCKS[stripe].lock().unwrap_or_else(|e| e.into_inner())
}

/// The registration nonce on record for a block, `None` when there is none or the handler is busy.
/// `try_get_agent_by_block`, NOT the plain `get_agent_by_block`: this runs when registration was skipped,
/// which is exactly when we might be on the same thread as an in-flight `inject_message`, and the blocking
/// version reproduced INCIDENT_2026_09_07's deadlock (#3084). In that reentrant case it returns `None` after
/// its own retry budget instead of hanging.
fn nonce_on_record(block_id: &str) -> Option<u64> {
    crate::backend::reactive::get_global_handler().try_get_agent_by_block(block_id).map(|current| current.registration_nonce)
}

/// Write the registration to the cross-instance (Tier-2) file registry and its host-global sibling (Tier
/// 2b, #1916): the auto-register path bypasses the HTTP register handler, which does this for the others.
fn mirror_registration_to_files(agent_id: &str, block_id: &str, my_registration_nonce: u64) {
    let Ok(local_url) = std::env::var("AGENTMUX_LOCAL_URL") else { return };
    let data_dir = crate::backend::base::get_mux_data_dir();
    crate::backend::reactive::registry::write_with_nonce(&data_dir, agent_id, &local_url, block_id, my_registration_nonce);
    crate::backend::reactive::registry::write_shared_from_env_with_nonce(agent_id, &local_url, block_id, my_registration_nonce);
}
