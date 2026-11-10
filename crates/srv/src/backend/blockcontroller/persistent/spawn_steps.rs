// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The steps of [`spawn_process`](PersistentSubprocessController::spawn_process), one function
//! each, in the order they ran: launch the child, take its pipes, mark the process running,
//! register the spawn, start the stdin writer, the stdout reader and the process waiter. Moved
//! out of `spawn_process` without a change in behaviour.

use super::process_waiter::ProcessWaiterCtx;
use super::spawn::ResumeSpawn;
use super::stderr_reader::StderrReaderCtx;
use super::stdout_reader::StdoutReaderCtx;
use super::*;
#[cfg(windows)]
use agentmux_common::win32::NoWindow;

/// What one spawn learned while launching its child, for the steps that follow.
pub(super) struct Launch {
    child: tokio::process::Child,
    pid: u32,
    spawn_args: Vec<String>,
    attempted_resume_sid: Option<String>,
    relocated_copy: Option<crate::backend::continuity_relocate::Relocated>,
    forked: bool,
    resume_claim: Option<std::sync::MutexGuard<'static, ()>>,
    agent_lease: Option<Arc<crate::backend::agent_admission::HeldAgentLease>>,
    my_generation: u64,
    my_registration_nonce: u64,
    continuation: Option<String>,
    had_retry_payload: bool,
}

/// The child's pipes, the kill channel and the stderr reader's handle.
pub(super) struct Pipes {
    pub(super) stdin: tokio::process::ChildStdin,
    pub(super) stdout: tokio::process::ChildStdout,
    pub(super) kill_tx: tokio::sync::oneshot::Sender<KillRequest>,
    pub(super) kill_rx: tokio::sync::oneshot::Receiver<KillRequest>,
    pub(super) stderr_reader_handle: Option<tokio::task::JoinHandle<()>>,
}

/// What registering the spawn produced, for the readers and the waiter.
pub(super) struct Registered {
    agent_id_for_muxbus: Option<String>,
    segment_read: super::segments::SegmentRef,
    segment_wait: super::segments::SegmentRef,
    relocated_copy_read: Option<crate::backend::continuity_relocate::Relocated>,
    forked_from_read: Option<String>,
    exit_cleanup_nonce: u64,
}

/// The stdout reader's handle, and what the waiter needs from the same record.
pub(super) struct StdoutTask {
    handle: tokio::task::JoinHandle<()>,
    global_output_zone_wait: Option<String>,
    record_fence_wait: Option<crate::backend::agent_admission::RecordFence>,
}

impl PersistentSubprocessController {
    /// Builds the command, takes the agent's lease, decides the resume and starts the child.
    pub(super) fn start_child(
        &self,
        config: &PersistentSpawnConfig,
        resume_retry_payload: Option<persistent_resume::QueuedRetryEntry>,
    ) -> Result<Launch, String> {
        // Captured before `resume_retry_payload` is moved further down (the
        // resume-bookkeeping match), for the turn-active decision in
        // `take_pipes` — see that check's own comment. Every
        // pre-existing caller either passes `Some(payload)` (a message is
        // about to be delivered directly) or `None` because a message is
        // already sitting in `pending_send_messages` waiting to be drained
        // right after this spawn succeeds (`respawn_once_for_leftover_
        // queue`) — this flag alone can't tell those two `None` cases
        // apart, which is exactly why the check in `take_pipes` also looks at
        // the queue.
        let had_retry_payload = resume_retry_payload.is_some();

        // Build command — use make_cli_cmd to resolve .cmd wrappers to node on Windows
        let mut cmd = crate::backend::pane_env::make_cli_cmd(&config.cli_command);

        self.hydrate_session_id_from_config(config);

        let inherited = self.inherited_fork_session();
        let unreachable_history_session =
            self.continue_first_spawn_history(config, inherited.as_deref());
        // Claim the agent's single-live-instance lease, then — under it — let
        // the resume gate decide which session is resumed. Any early return
        // after this point drops the handle, which releases the lease.
        let gfs = crate::backend::agent_session::global_transcript_store();
        let agent_lease =
            self.lease_then_gate(config, gfs.map(|g| &**g), unreachable_history_session)?;
        // The parent session a fork inherited is resumed as a fork, never in
        // place, however its id got here: the launch, the pane's history, the
        // gate (`agent_session::fork_history`).
        if let Some(parent_sid) = inherited.as_deref() {
            let mut inner = self.inner.lock().unwrap();
            if inner.session_id.as_deref() == Some(parent_sid) {
                inner.fork_next = true;
            }
        }

        let ResumeSpawn {
            spawn_args,
            attempted_resume_sid,
            relocated_copy,
            forked,
            claim: resume_claim,
        } = self.resume_spawn(config)?;
        cmd.args(&spawn_args);

        self.prepare_command(&mut cmd, config);

        // Don't even start a config restart's replacement a Stop has already
        // cancelled (see `restart_spawn_still_permitted_locked`); the decision
        // itself is made where the child is installed, below.
        Self::restart_spawn_still_permitted_locked(&mut self.inner.lock().unwrap())?;

        // In the block's tracker before it runs (`spawn_tracked`).
        let child = crate::backend::process_tracker::registry::spawn_tracked(&self.block_id, &mut cmd, crate::backend::process_tracker::registry::Join::Agent).map_err(|e| {
            tracing::error!(block_id = %self.block_id, error = %e, "persistent process spawn failed");
            format!("failed to spawn persistent process: {e}")
        })?;

        let (child, my_generation, superseded_effects) =
            self.install_new_process(child, config, &attempted_resume_sid, resume_retry_payload)?;
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
        self.flush_superseded_effects(superseded_effects);

        let continuation =
            self.disclose_fresh_start(&attempted_resume_sid, my_generation, had_retry_payload);
        let pid = child.id().unwrap_or(0);
        Ok(Launch {
            child,
            pid,
            spawn_args,
            attempted_resume_sid,
            relocated_copy,
            forked,
            resume_claim,
            agent_lease,
            my_generation,
            my_registration_nonce,
            continuation,
            had_retry_payload,
        })
    }

    /// Sets the working directory and environment, and pipes the child's stdio.
    fn prepare_command(&self, cmd: &mut tokio::process::Command, config: &PersistentSpawnConfig) {
        core::apply_working_dir(cmd, &self.block_id, &config.working_dir, &config.env_vars);
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
    }

    /// Takes the child's pipes, marks the turn active when one is under way and starts the stderr reader.
    pub(super) fn take_pipes(
        &self,
        config: &PersistentSpawnConfig,
        launch: &mut Launch,
    ) -> Result<Pipes, String> {
        let pid = launch.pid;
        let spawn_args = &launch.spawn_args;
        let attempted_resume_sid = &launch.attempted_resume_sid;
        let my_generation = launch.my_generation;
        let had_retry_payload = launch.had_retry_payload;
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
        let stdin = launch.child.stdin.take().ok_or_else(|| {
            format!(
                "[persistent] stdin not captured for block {}",
                self.block_id
            )
        })?;
        let stdout = launch.child.stdout.take().ok_or_else(|| {
            format!(
                "[persistent] stdout not captured for block {}",
                self.block_id
            )
        })?;
        let stderr = launch.child.stderr.take();

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
        Ok(Pipes {
            stdin,
            stdout,
            kill_tx,
            kill_rx,
            stderr_reader_handle,
        })
    }

    /// Records the running process in the controller's state; returns the stdin writer's channel.
    pub(super) fn mark_running(
        &self,
        launch: &mut Launch,
        kill_tx: tokio::sync::oneshot::Sender<KillRequest>,
    ) -> mpsc::Receiver<String> {
        let pid = launch.pid;
        let spawn_args = &launch.spawn_args;
        let agent_lease = &launch.agent_lease;
        // Create stdin writer channel
        let (msg_tx, msg_rx) = mpsc::channel::<String>(32);

        {
            let mut inner = self.inner.lock().unwrap();
            inner.current_pid = Some(pid);
            crate::backend::cli_native::pids::set(&self.block_id, pid);
            // Its first turn may now take a while: whoever waits on this identity's refresh waits for it.
            crate::identity::token_warmup::started(&self.block_id);
            inner.kill_tx = Some(kill_tx);
            inner.stdin_tx = Some(msg_tx);
            // The same `Arc` when an earlier generation's lease was reused
            // (see `acquire_agent_lease`), so no release happens here.
            inner.agent_lease = agent_lease.clone();
            Self::set_status(&mut inner, STATUS_RUNNING);
            inner.spawn_runtime = Some(crate::backend::agent_runtime::spawn_runtime_from_args(
                spawn_args,
            ));
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
                    let _ = tx.try_send(
                        crate::backend::agent_context_usage::context_usage_request_line(),
                    );
                }
            }
        }
        // Now visible to `session_held_elsewhere` — a concurrent resume of
        // the same session may run its check.
        drop(launch.resume_claim.take());
        self.health_monitor.note_spawned();
        self.publish_status();
        msg_rx
    }

    /// Registers the spawn with the muxbus and records its segment of the conversation.
    pub(super) fn register_spawn(
        &self,
        config: &PersistentSpawnConfig,
        launch: &Launch,
    ) -> Registered {
        let attempted_resume_sid = &launch.attempted_resume_sid;
        let forked = launch.forked;
        let relocated_copy = &launch.relocated_copy;
        let continuation = &launch.continuation;
        let agent_lease = &launch.agent_lease;
        let my_registration_nonce = launch.my_registration_nonce;
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
        let segment: super::segments::SegmentRef =
            agent_uid_for_registry.as_deref().and_then(|uid| {
                self.record_segment_start(
                    uid,
                    config,
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
        let exit_cleanup_nonce = self.register_with_muxbus(
            &agent_id_for_muxbus,
            &agent_uid_for_registry,
            my_registration_nonce,
        );
        Registered {
            agent_id_for_muxbus,
            segment_read,
            segment_wait,
            relocated_copy_read,
            forked_from_read,
            exit_cleanup_nonce,
        }
    }

    /// Records the pid for crash recovery and starts the stdin writer; returns the prompts it will send.
    pub(super) fn start_stdin_writer(
        &self,
        launch: &mut Launch,
        stdin: tokio::process::ChildStdin,
        msg_rx: mpsc::Receiver<String>,
    ) -> crate::backend::cli_native::sent::SentPrompts {
        let pid = launch.pid;
        // Record active pid for crash recovery (Phase 4.2). If the server
        // dies while this subprocess is running, scan_orphans() will find
        // the stale pid on next boot and flag the session as interrupted.
        if let Some(ref mstore) = self.mstore {
            super::super::session_recovery::mark_active_pid(mstore, &self.block_id, pid);
        }

        // The user turns the stdin writer sends, by uuid: the CLI echoes them back (`cli_native::sent`).
        let sent_prompts = crate::backend::cli_native::sent::SentPrompts::default();
        Self::spawn_stdin_writer(
            stdin,
            msg_rx,
            launch.continuation.take(),
            sent_prompts.clone(),
        );
        sent_prompts
    }

    /// A new process starts its own provider-reported spend total, as the
    /// harness's own `total_cost_usd` starts again with it (#30143). Called
    /// before the reader task exists, so a response settling first cannot land
    /// in the previous process's total.
    fn reset_route_cost_session(&self) {
        if let Some(mstore) = self.mstore.as_deref() {
            if let Some(agent) = crate::identity::resolver::block_agent_id(mstore, &self.block_id) {
                crate::backend::model_gateway::spend::reset_session(&agent);
            }
        }
    }

    /// Starts the stdout reader task.
    pub(super) fn spawn_stdout_reader(
        &self,
        stdout: tokio::process::ChildStdout,
        config: &PersistentSpawnConfig,
        launch: &Launch,
        registered: &Registered,
        sent_prompts: crate::backend::cli_native::sent::SentPrompts,
    ) -> StdoutTask {
        let my_generation = launch.my_generation;
        let my_registration_nonce = launch.my_registration_nonce;
        let agent_lease = &launch.agent_lease;
        let segment_read = registered.segment_read.clone();
        let relocated_copy_read = registered.relocated_copy_read.clone();
        let forked_from_read = registered.forked_from_read.clone();
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
        let global_output_zone = crate::backend::transcript::append::resolve_global_output_zone(
            &self.mstore,
            &self.block_id,
        );
        // Cloned before `global_output_zone` moves into the stdout-reader
        // task below — the process-waiter task (spawned in `spawn_waiter`) needs
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
        // (started in `take_pipes`). `child.wait()` resolving is NOT proof this task has already
        // read and held back the doomed attempt's terminal error-result line
        // (`ResumeState`'s `held_error_line`) — without this wait, the
        // waiter could resolve the resume state (`ProcessExited`) and launch
        // the retry first, after which this (now-lagging) reader would find
        // the state already `NotTracking` and persist the error line
        // immediately, reproducing the exact bubble this PR exists to
        // suppress.
        let model_api = core::attach_model_api(
            &self.block_id,
            &config.env_vars,
            &self.mstore,
            &self.event_bus,
        )
        .unwrap_or(crate::connectivity::Service::ModelApi(
            &crate::connectivity::ANTHROPIC,
        ));
        self.reset_route_cost_session();
        let stdout_reader_handle: tokio::task::JoinHandle<()> =
            tokio::spawn(Self::run_stdout_reader(StdoutReaderCtx {
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
                mcp_owner_read: my_registration_nonce,
                global_output_zone,
                record_fence,
                segment_read,
                relocated_copy_read,
                forked_from_read,
                sent_prompts,
                model_api,
            }));
        StdoutTask {
            handle: stdout_reader_handle,
            global_output_zone_wait,
            record_fence_wait,
        }
    }

    /// Starts the process waiter task, which owns the child from here on.
    pub(super) fn spawn_waiter(
        &self,
        launch: Launch,
        kill_rx: tokio::sync::oneshot::Receiver<KillRequest>,
        stderr_reader_handle: Option<tokio::task::JoinHandle<()>>,
        stdout: StdoutTask,
        registered: &Registered,
    ) {
        let Launch {
            child,
            pid,
            my_generation,
            my_registration_nonce,
            ..
        } = launch;
        let stdout_reader_handle = stdout.handle;
        let global_output_zone_wait = stdout.global_output_zone_wait;
        let record_fence_wait = stdout.record_fence_wait;
        let segment_wait = registered.segment_wait.clone();
        let agent_id_for_muxbus = &registered.agent_id_for_muxbus;
        let exit_cleanup_nonce = registered.exit_cleanup_nonce;
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
        // same value UNLESS registration was skipped (`register_with_muxbus`), in which case
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
            mcp_owner_wait: my_registration_nonce,
        }));
    }
}
