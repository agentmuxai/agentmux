// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The steps of one container turn (`spawn_container_turn`): ready the container, upload the prompt, start the
//! exec, read its output, and settle the turn from the exit code, then hand the container to the next message.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use futures_util::StreamExt as _;

use crate::backend::blockcontroller::{core, publish_controller_status, session_stats, shell, STATUS_DONE, STATUS_RUNNING};
use crate::backend::container::{ContainerError, ContainerManager, ExecSession};

use super::*;

/// One container turn: what its task owns from the moment it starts to the moment it hands the container on.
pub(super) struct Turn {
    cm: ContainerManager,
    container_name: String,
    /// The same container, name and base command, for the message queued behind this turn.
    cm_for_drain: ContainerManager,
    container_name_for_drain: String,
    base_cmd_for_drain: Vec<String>,
    /// What `pkill -f` matches to interrupt the CLI.
    kill_pattern: String,
    /// The CLI's argv (taken when the exec opens).
    cmd: Vec<String>,
    container_env: Vec<(String, String)>,
    inner_arc: Arc<std::sync::Mutex<SubprocessControllerInner>>,
    run_lock: Arc<std::sync::atomic::AtomicBool>,
    broker: Option<Arc<super::super::mps::Broker>>,
    event_bus: Option<Arc<super::super::EventBus>>,
    mstore: Option<Arc<super::super::Store>>,
    filestore: Option<Arc<super::super::FileStore>>,
    health_monitor: Arc<super::super::TurnActivityTracker>,
    block_id: String,
    turn_id: String,
    turn_message: String,
    startup: Option<super::super::StartupStep>,
    user_record: super::super::UserRecordSink,
    self_ref_done: std::sync::Weak<SubprocessController>,
    session_id_field: String,
}

/// What reading the exec's output left behind.
#[derive(Default)]
pub(super) struct Read {
    /// Set when the turn is interrupted via stop_subprocess (Esc / agent.stop)
    /// — drives a non-zero exit so an interrupted turn isn't reported as Idle.
    killed: bool,
    /// Tracks an aborted output stream (`Some(Err(_))`). The exec may have
    /// exited cleanly with a non-zero code OR the attach stream itself
    /// failed mid-turn; either way the turn did not complete normally, so
    /// this forces a non-zero exit even if inspect_exec can't be reached.
    stream_errored: bool,
    /// Capture a bounded tail of stderr so a non-zero exit can be classified
    /// into a real cause, mirroring host_spawn.rs's `stderr_tail`. No
    /// Arc<Mutex<>> needed here (unlike host_spawn.rs) — this whole turn
    /// runs in a single task, so a plain owned Vec captured by this async
    /// block is sufficient.
    stderr_tail: Vec<String>,
    /// Retain the terminal `result` frame and any in-band API error
    /// (e.g. a synthetic assistant message carrying a 401/auth failure
    /// with exit 0 / is_error:false) so a completion-time classifier has
    /// the same inputs host_spawn.rs's process_waiter does — this
    /// controller has no HealthMonitor-driven route to AgentFailure now
    /// that the silence watchdog is gone, so this is its only path.
    last_result_frame: Option<serde_json::Value>,
    last_inband_error: Option<serde_json::Value>,
}

/// Run one container turn to its end.
pub(super) async fn run_turn(mut turn: Turn, update_guard: tokio::sync::OwnedRwLockReadGuard<()>) {
    let _update_guard = update_guard;

    let mut kill_rx = turn.begin();

    // Start the exec via Docker socket — env vars travel through
    // CreateExecOptions.env (Docker API), never in process argv.
    // Set once the upload succeeds; the interrupt path uses it to remove
    // a prompt file whose wrapper was killed before it could.
    let mut prompt_path_for_kill = String::new();
    let startup_failed = turn.ready_container(&mut kill_rx).await;
    // Starting the container is what binds the listener on a first turn.
    crate::backend::container::listener::point_env_at_listener(&mut turn.container_env);
    // What the kernel has killed for memory in this container so far: a rise by the end of the turn says why
    // something in it died (SPEC_CONTAINER_AGENT_RESOURCE_LIMITS_2026_10_29.md §5).
    let oom_before = if startup_failed { None } else { turn.cm.oom_kill_count(&turn.container_name).await };
    let exec_session = match turn.open_exec(startup_failed, &mut prompt_path_for_kill).await {
        Ok(s) => s,
        Err(e) => {
            turn.fail_exec(e).await;
            return;
        }
    };

    // `input` is unused by design — the CLI reads its prompt from the
    // uploaded file, and only that file's PATH travels in argv (see
    // `container_turn_exec`). Dropping it immediately keeps no handle
    // on a write half that can never be closed anyway.
    let crate::backend::container::ExecSession { exec_id, input, output } = exec_session;
    drop(input);
    let read = turn.read_output(output, &mut kill_rx, &prompt_path_for_kill).await;
    turn.finish(exec_id, read, oom_before).await;
}

/// Name a failure from the exit code and what the CLI said, on stdout or stderr.
fn classify_failure(exit_code: i32, read: &Read) -> Option<crate::agents::failure::AgentFailure> {
    let Read { stderr_tail, last_result_frame, last_inband_error, .. } = read;
    // Classify a genuine non-zero exit OR a failure reported in-band as
    // an error `result` frame / synthetic assistant error (auth /
    // rate-limit / usage — the container CLI may even exit 0), mirroring
    // host_spawn.rs's own independent completion-time classifier. This
    // controller has no HealthMonitor-driven route to AgentFailure now
    // that the silence watchdog is gone — this is its only path to a
    // persisted/published failure for container-backed agents.
    let mut run_failure: Option<crate::agents::failure::AgentFailure> = None;
    {
        let frame_is_error = last_result_frame
            .as_ref()
            .and_then(|f| f.get("is_error"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let inband_is_api_error = last_inband_error.is_some();
        if exit_code != 0 || frame_is_error || inband_is_api_error {
            let tail = stderr_tail.join("\n");
            // Merge the in-band error text so classify() sees the
            // "authentication_failed" / "401" string even though it
            // arrived on stdout, not stderr.
            let inband_text = last_inband_error.as_ref().map(crate::agents::limit_reset::inband_error_text).unwrap_or_default();
            let combined_tail = if inband_text.trim().is_empty() {
                tail
            } else {
                format!("{tail}\n{inband_text}")
            };
            run_failure = Some(crate::agents::failure::classify(
                Some(exit_code),
                None, // container exec has no OS process signal
                &combined_tail,
                last_result_frame.as_ref(),
            ));
        }
    }
    run_failure
}

impl SubprocessController {
    /// Everything the container turn's task needs, taken from this controller and the request.
    pub(super) fn begin_container_turn(
        &self,
        cm: ContainerManager,
        container_name: String,
        base_cmd: Vec<String>,
        cmd: Vec<String>,
        config: &SubprocessSpawnConfig,
    ) -> Turn {
        // Derive the exec env from THIS message's own env_vars (leave out
        // host-only names, `agent_env_scope`, here, per-turn) rather than carrying a pre-filtered
        // list across drains — so a message queued behind a running turn uses its
        // own freshly-resolved auth/env, not the prior turn's stale values.
        // The instance key is swapped for this block's container token, which
        // srv honours only on agent routes (backend::container::credential).
        let container_env: Vec<(String, String)> =
            crate::backend::runtime_adapter::RuntimeKind::Container.adapter().credentials(&self.block_id, &config.env_vars);
        // Identity M4a: record what this turn's `docker exec` is actually given.
        crate::backend::identity_spawn::record_process_spawn(
            &self.block_id,
            crate::backend::identity_spawn::SpawnPath::Container,
            &container_env.iter().cloned().collect(),
        );
        // A container's CLI reaches its API from the container's network (a Docker-only name, `host.docker.internal`),
        // which a probe from srv can't judge, so a container pane has no address of its own in the status: this
        // clears any recorded by an earlier run on the host.
        core::attach_model_api(&self.block_id, &std::collections::HashMap::new(), &self.mstore, &self.event_bus);

        // Snapshot container params for the queue-drain path before base_cmd is consumed.
        let cm_for_drain = cm.clone();
        let container_name_for_drain = container_name.clone();
        let base_cmd_for_drain = base_cmd.clone();

        // Command name to pkill if the turn is interrupted (see
        // `interrupt`). base_cmd[0] is the container-local CLI (e.g.
        // `claude`); -f matches its full cmdline inside the container.
        let kill_pattern = base_cmd.first().cloned().unwrap_or_else(|| "claude".to_string());

        // Clone all self fields needed by the inner tokio::spawn so we don't
        // borrow `self` across the async boundary (which would make the future
        // non-'static and break tokio::spawn).
        let inner_arc = Arc::clone(&self.inner);
        let run_lock = Arc::clone(&self.run_lock);
        let broker = self.broker.clone();
        let event_bus = self.event_bus.clone();
        let mstore = self.mstore.clone();
        let filestore = self.filestore.clone();
        let health_monitor = Arc::clone(&self.health_monitor);
        let block_id = self.block_id.clone();
        // The prompt is written into the container as a file per turn (see
        // `container_turn_exec`). A fresh id per turn keeps two turns — this
        // block's next one, or another block sharing the container — from
        // racing on the same path, including the `rm -f` that cleans it up.
        let turn_id = uuid::Uuid::new_v4().to_string();
        let turn_message = config.message.clone();
        let startup = self.container_startup.lock().unwrap().clone();
        let user_record = self.user_record_sink(config.message_id.is_some(), config.record.clone());
        let self_ref_done = self.self_ref.lock().unwrap().clone().unwrap_or_default();

        Turn {
            cm,
            container_name,
            cm_for_drain,
            container_name_for_drain,
            base_cmd_for_drain,
            kill_pattern,
            cmd,
            container_env,
            inner_arc,
            run_lock,
            broker,
            event_bus,
            mstore,
            filestore,
            health_monitor,
            block_id,
            turn_id,
            turn_message,
            startup,
            user_record,
            self_ref_done,
            session_id_field: config.session_id_field.clone(),
        }
    }
}

impl Turn {
    /// Install the kill channel, mark the turn running and tell the pane. Returns the receiving end.
    fn begin(&self) -> tokio::sync::oneshot::Receiver<bool> {
        let inner_arc = &self.inner_arc;
        let broker = &self.broker;
        let health_monitor = &self.health_monitor;
        let block_id = &self.block_id;
        // Install the kill channel BEFORE any await, so a stop issued
        // while this turn is still starting is recorded rather than
        // dropped.
        //
        // `stop_subprocess` reports success if `inner.kill_tx` is None,
        // treating "nothing to interrupt" as "already stopped". While
        // kill_tx was installed only after the exec had started, every
        // await before that point was a hole: a stop landing in it was
        // silently discarded and the turn then ran on, flipping the status
        // back to running (codex P2, PR #2883). That hole always existed
        // for the exec round-trip, and this change widened it materially —
        // the identity probe plus a prompt upload that can be hundreds of
        // KB now sit inside it.
        //
        // A stop that arrives during startup is not lost: `kill_rx` holds
        // the value, and the reader loop in `read_output` selects on it `biased`, so
        // it is observed on the first poll and interrupted immediately —
        // including the prompt-file cleanup. The failure paths in `fail_exec` and the closing steps
        // clear `kill_tx` again, so nothing is left dangling.
        let (kill_tx, kill_rx) = tokio::sync::oneshot::channel::<bool>();
        {
            let mut inner = inner_arc.lock().unwrap();
            inner.kill_tx = Some(kill_tx);
            SubprocessController::set_status(&mut inner, STATUS_RUNNING);
        }
        // turn_active=true on the start publish — the start edge the
        // notification Router needs (same as host_spawn).
        health_monitor.set_active_turn(true);
        // Each turn runs its own exec: the watchdog's runtime counts from here.
        health_monitor.note_spawned();
        if let Some(ref b) = broker {
            let status = {
                let inner = inner_arc.lock().unwrap();
                SubprocessController::build_status_snapshot(&inner, block_id, true)
            };
            publish_controller_status(b, &status);
        }
        kill_rx
    }

    /// Get the container ready; `true` when the start-up step failed or a stop ended the wait.
    async fn ready_container(&self, kill_rx: &mut tokio::sync::oneshot::Receiver<bool>) -> bool {
        let block_id = &self.block_id;
        let startup = &self.startup;
        // Write the prompt into the container FIRST, then run the CLI with
        // its stdin redirected from that file. See `container_turn_exec`
        // and `ContainerManager::upload_turn_prompt` for why neither the
        // exec's own stdin, nor argv, nor an env var can carry it.
        //
        // Before that, get the container ready (first message: download the
        // image, create the container, install the CLI). That is minutes of
        // work, so it happens here and not in the handler that took the
        // message. Its failure has already been written to the pane.
        //
        // A stop during that wait ends the turn here: dropping the step
        // cancels the download at the daemon, and the exec never starts.
        let startup_failed = match startup {
            Some(step) => tokio::select! {
                done = step() => done.is_err(),
                _ = &mut *kill_rx => {
                    tracing::info!(block_id = %block_id, "container turn stopped while its container was starting");
                    true
                }
            },
            None => false,
        };
        startup_failed
    }

    /// Upload the prompt and start the exec. `Err(None)`: the start-up step failed and already told the pane.
    async fn open_exec(
        &mut self,
        startup_failed: bool,
        prompt_path_for_kill: &mut String,
    ) -> Result<ExecSession, Option<ContainerError>> {
        let cmd = std::mem::take(&mut self.cmd);
        let (cm, container_name, turn_id, turn_message) = (&self.cm, &self.container_name, &self.turn_id, &self.turn_message);
        let (container_env, user_record) = (&self.container_env, &self.user_record);
        if startup_failed {
            Err(None)
        } else {
            match cm
                .upload_turn_prompt(container_name, turn_id, turn_message)
                .await
            {
                Ok(prompt_path) => {
                    // Retained for `read_output`'s interrupt path, which has to do
                    // the cleanup the killed wrapper can't.
                    *prompt_path_for_kill = prompt_path.clone();
                    let wrapped = container_turn_exec(&prompt_path, cmd);
                    // attach_stdin: false — the CLI's stdin is the prompt file.
                    match cm.exec(container_name, &wrapped, None, container_env, false).await {
                        Ok(session) => {
                            // The exec started with the prompt as its stdin:
                            // now the transcript may say so (Codex review).
                            user_record.write(turn_message);
                            Ok(session)
                        }
                        Err(e) => {
                            // The wrapper never ran, so neither did its
                            // `rm -f "$F"` — clean up here or the prompt is
                            // orphaned in the container (reagent P2, PR #2883).
                            cm.remove_turn_prompt(container_name, &prompt_path).await;
                            Err(Some(e))
                        }
                    }
                }
                Err(e) => Err(Some(e)),
            }
        }
    }

    /// The exec never started: tell the pane, end the turn as failed and hand the container to the next message.
    async fn fail_exec(&self, e: Option<ContainerError>) {
        let inner_arc = &self.inner_arc;
        let run_lock = &self.run_lock;
        let broker = &self.broker;
        let mstore = &self.mstore;
        let filestore = &self.filestore;
        let health_monitor = &self.health_monitor;
        let block_id = &self.block_id;
        let self_ref_done = &self.self_ref_done;
        let cm_for_drain = &self.cm_for_drain;
        let container_name_for_drain = &self.container_name_for_drain;
        let base_cmd_for_drain = &self.base_cmd_for_drain;
        // `None`: the start-up step failed and already told the pane.
        if let Some(e) = &e {
            tracing::warn!(block_id = %block_id, error = %e, "container exec failed");
            // Surface the error in the agent pane so the user sees what went wrong.
            if let Some(ref b) = broker {
                let error_frame = serde_json::json!({
                    "type": "result",
                    "is_error": true,
                    "subtype": "error_during_execution",
                    "error": {"message": format!("[AgentMux] container exec failed: {e}")}
                }).to_string();
                // Into the agent's global zone too, like every other
                // record: that is what the pane reads on reload
                // (Phase 5a-3c).
                let global_zone = crate::backend::transcript::append::resolve_global_output_zone(mstore, block_id);
                shell::handle_append_block_file(
                    b, block_id, SUBPROCESS_OUTPUT_SUBJECT,
                    format!("{error_frame}\n").as_bytes(),
                    filestore.as_ref(), global_zone.as_deref(),
                );
            }
        }
        // A failed exec must still run the SAME completion + queue
        // drain as the normal-exit path below: publish a terminal
        // status so the client sees the turn end (exit 1), mark the
        // health monitor exited, release run_lock, AND drain any
        // queued message — otherwise the run_lock is freed but
        // pending_messages is never popped, stranding the queue.
        {
            let mut inner = inner_arc.lock().unwrap();
            inner.proc_exit_code = 1;
            SubprocessController::set_status(&mut inner, STATUS_DONE);
            inner.current_pid = None;
            inner.kill_tx = None;
        }
        // Clears active_turn as well as recording the exit code —
        // which is why the `set_active_turn(true)` in `begin`, which
        // runs above the upload, needs no explicit undo here.
        health_monitor.set_exited(1);
        if let Some(ref b) = broker {
            let status = {
                let inner = inner_arc.lock().unwrap();
                SubprocessController::build_status_snapshot(&inner, block_id, false)
            };
            publish_controller_status(b, &status);
        }
        run_lock.store(false, Ordering::SeqCst);
        let next_config = {
            let mut inner = inner_arc.lock().unwrap();
            inner.pending_messages.pop_front()
        };
        if let Some(cfg) = next_config {
            if let Some(ctrl) = self_ref_done.upgrade() {
                tracing::info!(block_id = %block_id, "draining queued container message after exec failure");
                if let Err(e) = ctrl.spawn_container_turn(
                    cm_for_drain.clone(),
                    container_name_for_drain.clone(),
                    base_cmd_for_drain.clone(),
                    cfg,
                ) {
                    tracing::warn!(error = %e, "failed to spawn queued container turn");
                    SubprocessController::publish_queued_spawn_error(
                        block_id,
                        &e,
                        broker,
                        filestore,
                        mstore,
                    );
                }
            }
        }
    }

    /// Read the exec's output until it ends, errors or is interrupted, publishing each line.
    async fn read_output(
        &self,
        output: std::pin::Pin<Box<dyn futures_util::Stream<Item = Result<bollard::container::LogOutput, bollard::errors::Error>> + Send>>,
        mut kill_rx: &mut tokio::sync::oneshot::Receiver<bool>,
        prompt_path_for_kill: &str,
    ) -> Read {
        use bollard::container::LogOutput;
        let inner_arc = &self.inner_arc;
        let broker = &self.broker;
        let event_bus = &self.event_bus;
        let mstore = &self.mstore;
        let filestore = &self.filestore;
        let health_monitor = &self.health_monitor;
        let block_id = &self.block_id;
        // Read stdout — accumulate bytes into lines.
        let mut line_buf = String::new();
        let mut stats = session_stats::SessionStatsAccumulator::new(block_id.clone());
        let session_id_field = self.session_id_field.clone();
        let mut read = Read::default();

        // A compaction boundary was seen and its summary hasn't followed yet: the memory card parked for it
        // is written after the summary, as on the host (`persistent/stdout_reader.rs`).
        let mut parked_notices_due = false;

        // Resolve the agent's GLOBAL transcript zone once (see persistent/spawn.rs)
        // so every container-exec `output` line is also mirrored to the
        // cross-channel store. `None` for non-agent blocks.
        let global_output_zone =
            crate::backend::transcript::append::resolve_global_output_zone(mstore, block_id);

        tracing::info!(block_id = %block_id, "container exec output reader started");

        let mut pinned = std::pin::pin!(output);
        loop {
            tokio::select! {
                // Prioritise the kill signal so Esc is responsive even under a
                // steady output stream.
                biased;
                kill = &mut kill_rx => {
                    let force = kill.unwrap_or(false);
                    self.interrupt(force, prompt_path_for_kill).await;
                    read.killed = true;
                    break;
                }
                item = pinned.next() => {
                    match item {
                        None => {
                            // Stream ended — flush any remaining partial line.
                            if !line_buf.trim().is_empty() {
                                SubprocessController::publish_line(&line_buf, block_id, &session_id_field, inner_arc, mstore, event_bus, broker, filestore, &mut stats, global_output_zone.as_deref(), &mut read.last_result_frame, &mut read.last_inband_error, &mut parked_notices_due);
                            }
                            tracing::info!(block_id = %block_id, "container exec output EOF");
                            break;
                        }
                        Some(Err(e)) => {
                            tracing::warn!(block_id = %block_id, error = %e, "container exec output read error");
                            read.stream_errored = true;
                            break;
                        }
                        Some(Ok(log_output)) => {
                            let bytes = match log_output {
                                LogOutput::StdOut { message } => {
                                    health_monitor.note_output();
                                    message
                                }
                                LogOutput::StdErr { message } => {
                                    self.note_stderr(&message, &mut read.stderr_tail);
                                    continue;
                                }
                                _ => continue,
                            };
                            let chunk = String::from_utf8_lossy(&bytes);
                            for ch in chunk.chars() {
                                if ch == '\n' {
                                    if !line_buf.trim().is_empty() {
                                        SubprocessController::publish_line(&line_buf, block_id, &session_id_field, inner_arc, mstore, event_bus, broker, filestore, &mut stats, global_output_zone.as_deref(), &mut read.last_result_frame, &mut read.last_inband_error, &mut parked_notices_due);
                                    }
                                    line_buf.clear();
                                } else {
                                    line_buf.push(ch);
                                }
                            }
                        }
                    }
                }
            }
        }

        tracing::info!(block_id = %block_id, "container exec output reader exiting");

        read
    }

    /// A stop arrived mid-turn: interrupt the CLI inside the container and remove its prompt.
    async fn interrupt(&self, force: bool, prompt_path_for_kill: &str) {
        let block_id = &self.block_id;
        let cm = &self.cm;
        let container_name = &self.container_name;
        let kill_pattern = &self.kill_pattern;
        tracing::info!(block_id = %block_id, force, "container turn interrupt — pkill in container");
        // Best-effort: actually terminate the in-container process.
        // Even if this fails (e.g. no procps on an old image), we
        // still break + finalize so AgentMux honours the stop.
        if let Err(e) = cm.signal_exec_process(container_name, kill_pattern, force).await {
            tracing::warn!(block_id = %block_id, error = %e, "container interrupt pkill failed");
        }
        // The wrapper carries the `rm -f`, and `pkill -f
        // <cli>` matches the wrapper too -- its argv contains
        // the CLI name as an argument -- so the shell dies
        // alongside the CLI and its cleanup never runs. An
        // interrupted turn would otherwise leave a mode-0600
        // prompt, possibly holding a pasted secret, in a
        // container that outlives the turn (codex P2,
        // PR #2883). Clean up from out here, where nothing was
        // killed.
        cm.remove_turn_prompt(container_name, prompt_path_for_kill).await;
    }

    /// Log the CLI's stderr and keep its last lines for classifying a failure.
    fn note_stderr(&self, message: &[u8], stderr_tail: &mut Vec<String>) {
        let block_id = &self.block_id;
        // Log stderr but don't publish as blockfile output.
        let s = String::from_utf8_lossy(message);
        for line in s.lines() {
            if !line.trim().is_empty() {
                tracing::info!(block_id = %block_id, stderr = %line, "container exec stderr");
                // Retain the last ~40 non-empty lines for classification
                // (mirrors host_spawn.rs's stderr_tail cap).
                stderr_tail.push(line.to_string());
                let overflow = stderr_tail.len().saturating_sub(40);
                if overflow > 0 {
                    stderr_tail.drain(0..overflow);
                }
            }
        }
    }

    /// The exec has ended: settle the turn from its exit code and what the CLI said.
    async fn finish(&self, exec_id: String, read: Read, oom_before: Option<u64>) {
        let exit_code = self.exit_code_of(&exec_id, &read).await;
        let mut run_failure = classify_failure(exit_code, &read);
        self.note_memory_kill(read.killed, oom_before, &mut run_failure).await;
        self.mark_done(exit_code, &run_failure);
        self.record_outcome(&run_failure);
        self.release_and_drain();
    }

    /// The turn's real exit code: the output ending is not the process ending, so ask the daemon.
    async fn exit_code_of(&self, exec_id: &str, read: &Read) -> i32 {
        let (killed, stream_errored) = (read.killed, read.stream_errored);
        let block_id = &self.block_id;
        let cm = &self.cm;
        // Determine the real turn exit code. The output stream ending is NOT
        // the process exit status (unlike the host path's `child.wait()`), so
        // inspect the exec over the Docker socket. A mid-turn stream error, an
        // unavailable code, or a failed inspect is treated as a failure so a
        // crashed / non-zero in-container CLI is never misreported to the
        // client and to the health monitor as a successful (Idle) turn.
        let exit_code: i32 = if killed {
            // Interrupted by stop_subprocess — report non-zero (matches the
            // host spawn_turn kill path) so health treats it as not-Idle.
            -1
        } else if stream_errored {
            1
        } else {
            match cm.inspect_exec(exec_id).await {
                Ok(Some(code)) => code as i32,
                Ok(None) => {
                    tracing::warn!(block_id = %block_id, "inspect_exec returned no exit code; treating turn as failed");
                    1
                }
                Err(e) => {
                    tracing::warn!(block_id = %block_id, error = %e, "inspect_exec failed; treating turn as failed");
                    1
                }
            }
        };
        exit_code
    }

    /// A memory kill in the container is named in the failure and written to the pane.
    async fn note_memory_kill(&self, killed: bool, oom_before: Option<u64>, run_failure: &mut Option<crate::agents::failure::AgentFailure>) {
        let broker = &self.broker;
        let mstore = &self.mstore;
        let filestore = &self.filestore;
        let block_id = &self.block_id;
        let cm = &self.cm;
        let container_name = &self.container_name;
        // A memory kill in the container is often a child of the CLI (the `cargo` of a Bash tool), so the turn may
        // have ended cleanly: the pane says so either way, and a turn that died of it says why.
        if !killed {
            if let Some(event) = cm.oom_since(container_name, oom_before).await {
                crate::backend::container::oom::note_in_pane(&event, block_id, broker, filestore, mstore);
                if let Some(failure) = run_failure.as_mut() {
                    crate::backend::container::oom::explain_failure(failure, &event);
                }
            }
        }
    }

    /// Record the exit, mark the turn done and publish the status.
    fn mark_done(&self, exit_code: i32, run_failure: &Option<crate::agents::failure::AgentFailure>) {
        let inner_arc = &self.inner_arc;
        let broker = &self.broker;
        let health_monitor = &self.health_monitor;
        let block_id = &self.block_id;
        // Mark done
        {
            let mut inner = inner_arc.lock().unwrap();
            inner.proc_exit_code = exit_code;
            SubprocessController::set_status(&mut inner, STATUS_DONE);
            inner.current_pid = None;
            inner.kill_tx = None;
        }

        {
            let inner = inner_arc.lock().unwrap();
            health_monitor.set_turn_exited(inner.proc_exit_code, inner.proc_exit_code == 0 && run_failure.is_none());
        }

        if let Some(ref b) = broker {
            let status = {
                let inner = inner_arc.lock().unwrap();
                SubprocessController::build_status_snapshot(&inner, block_id, false)
            };
            publish_controller_status(b, &status);
        }
    }

    /// Persist or clear the last failure, report the turn, and tell the pane what failed.
    fn record_outcome(&self, run_failure: &Option<crate::agents::failure::AgentFailure>) {
        let broker = &self.broker;
        let event_bus = &self.event_bus;
        let mstore = &self.mstore;
        let block_id = &self.block_id;
        // Persist or clear agent:last_failure in block meta so the recovery
        // banner survives tab switches and page reloads, and a previously
        // persisted failure is cleared after a later successful turn — this
        // call handles both cases regardless of whether run_failure is
        // Some or None. Mirrors host_spawn.rs's process_waiter.
        core::persist_last_failure(block_id, run_failure.as_ref(), mstore, event_bus);
        core::report_provider_turn(block_id, run_failure.as_ref(), mstore);
        // A finished turn is when the agent's running summary may be due for an update (a host agent's
        // persistent reader does this; without it a container agent never got one, so a compaction
        // delivered its memory but no summary).
        if run_failure.is_none() {
            crate::backend::continuity_state::after_successful_turn(mstore.clone(), block_id.clone());
        }

        // Surface the classified failure cause to the pane. persist:1 so
        // reconnecting subscribers also receive the last failure without a
        // separate meta read (belt-and-suspenders with the meta write above).
        if let (Some(failure), Some(b)) = (run_failure.as_ref(), broker.as_ref()) {
            b.publish_event::<crate::backend::mps_events::AgentFailed>(failure, vec![format!("block:{}", block_id)], 1);
        }
    }

    /// Free the run lock and start the message queued behind this turn, if any.
    fn release_and_drain(&self) {
        let inner_arc = &self.inner_arc;
        let run_lock = &self.run_lock;
        let broker = &self.broker;
        let mstore = &self.mstore;
        let filestore = &self.filestore;
        let block_id = &self.block_id;
        let self_ref_done = &self.self_ref_done;
        let cm_for_drain = &self.cm_for_drain;
        let container_name_for_drain = &self.container_name_for_drain;
        let base_cmd_for_drain = &self.base_cmd_for_drain;
        run_lock.store(false, std::sync::atomic::Ordering::SeqCst);

        // Drain queued messages via spawn_container_turn so the container
        // context (cm, container_name, base_cmd, container_env) is preserved.
        // spawn_turn has no container awareness and would spawn an empty command
        // on the host, silently losing the queued message.
        let next_config = {
            let mut inner = inner_arc.lock().unwrap();
            inner.pending_messages.pop_front()
        };
        if let Some(cfg) = next_config {
            if let Some(ctrl) = self_ref_done.upgrade() {
                tracing::info!(block_id = %block_id, "draining queued container message via spawn_container_turn");
                if let Err(e) = ctrl.spawn_container_turn(
                    cm_for_drain.clone(),
                    container_name_for_drain.clone(),
                    base_cmd_for_drain.clone(),
                    cfg,
                ) {
                    tracing::warn!(error = %e, "failed to spawn queued container turn");
                    SubprocessController::publish_queued_spawn_error(
                        block_id,
                        &e,
                        broker,
                        filestore,
                        mstore,
                    );
                }
            }
        }
    }
}
