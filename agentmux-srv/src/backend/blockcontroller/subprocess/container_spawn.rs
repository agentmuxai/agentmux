// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Container-exec turn spawn (`spawn_container_turn`) for [`SubprocessController`].
//!
//! Docker-exec analog of `host_spawn::spawn_turn`, used for container agents.
//! Drives the exec via `bollard`'s Docker socket API instead of a host
//! `std::process` child (P1a: no secrets in argv — env vars travel through
//! `CreateExecOptions.env`, never process argv / `/proc/<pid>/cmdline`).
//!
//! `spawn_container_turn` is one continuous, non-trivially-ordered state
//! machine and is moved here WHOLE rather than decomposed further.
//! `publish_line` is its private output-line helper (used only here).

use std::sync::atomic::Ordering;
use std::sync::Arc;

use futures_util::StreamExt as _;
use tokio::io::AsyncWriteExt;

use crate::backend::blockcontroller::{
    core, publish_controller_status, session_stats, shell, STATUS_DONE, STATUS_RUNNING,
};
use crate::backend::wps;

use super::{argv::build_turn_argv, SubprocessController, SubprocessControllerInner, SubprocessSpawnConfig, SUBPROCESS_OUTPUT_SUBJECT};

impl SubprocessController {
    /// Spawn a container agent turn via Docker socket (P1a: no secrets in argv).
    ///
    /// This is the secure alternative to `spawn_turn` for container agents. Instead
    /// of running `docker exec -e KEY=VALUE ...` as a CLI subprocess (which exposes
    /// secrets in process argv / `/proc/<pid>/cmdline`, CWE-214), this method calls
    /// `ContainerManager::exec` directly, passing env vars through
    /// `CreateExecOptions.env` (Docker socket). The exec I/O (stdin write + stdout
    /// NDJSON stream) drives the same state machine as `spawn_turn`:
    ///   • appends `--resume <sid>` if a prior session_id is known
    ///   • writes the JSON message to exec stdin
    ///   • reads NDJSON from the output stream, publishing WPS blockfile events
    ///   • captures session_id from the provider's init event
    ///   • transitions status running → done
    ///   • drains the pending-message queue when the exec exits
    ///
    /// `base_cmd` is `[cli_command] + cli_args` WITHOUT resume — this method appends
    /// `--resume <sid>` internally before starting the exec.
    /// The exec env is derived from THIS message's `config.env_vars` (denylist
    /// applied here, per-turn) — not carried across queue drains — so a queued
    /// message runs with its own freshly-resolved auth/env, matching `spawn_turn`.
    ///
    /// Takes `cm` and `container_name` by value (not reference) so the returned
    /// future is `'static` — required for `tokio::spawn` in the queue-drain path.
    pub fn spawn_container_turn(
        &self,
        cm: crate::backend::container::ContainerManager,
        container_name: String,
        base_cmd: Vec<String>,
        config: SubprocessSpawnConfig,
    ) -> Result<(), String> {
        if !self.try_lock_run() {
            let mut inner = self.inner.lock().unwrap();
            tracing::info!(
                block_id = %self.block_id,
                queue_depth = inner.pending_messages.len() + 1,
                "container exec busy — message queued"
            );
            inner.pending_messages.push_back(config);
            return Ok(());
        }

        self.hydrate_session_id_from_config(config.session_id.as_deref());

        let session_id_hint = self.inner.lock().unwrap().session_id.clone();
        let cmd = match build_turn_argv(
            &base_cmd,
            &config.resume_strategy,
            &config.resume_flag,
            session_id_hint.as_deref(),
        ) {
            Ok(cmd) => cmd,
            Err(error) => {
                self.unlock_run();
                return Err(error);
            }
        };
        self.emit_message_accepted(&config);

        // Derive the exec env from THIS message's own env_vars (apply the
        // container denylist here, per-turn) rather than carrying a pre-filtered
        // list across drains — so a message queued behind a running turn uses its
        // own freshly-resolved auth/env, not the prior turn's stale values.
        let container_env: Vec<(String, String)> = config.env_vars.iter()
            .filter(|(k, _)| !crate::backend::container::CONTAINER_ENV_DENYLIST.contains(&k.as_str()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        // Snapshot container params for the queue-drain path before base_cmd is consumed.
        let cm_for_drain = cm.clone();
        let container_name_for_drain = container_name.clone();
        let base_cmd_for_drain = base_cmd.clone();

        // Command name to pkill if the turn is interrupted (see the kill path in
        // the reader select below). base_cmd[0] is the container-local CLI (e.g.
        // `claude`); -f matches its full cmdline inside the container.
        let kill_pattern = base_cmd.first().cloned().unwrap_or_else(|| "claude".to_string());

        // Clone all self fields needed by the inner tokio::spawn so we don't
        // borrow `self` across the async boundary (which would make the future
        // non-'static and break tokio::spawn).
        let inner_arc = Arc::clone(&self.inner);
        let run_lock = Arc::clone(&self.run_lock);
        let broker = self.broker.clone();
        let event_bus = self.event_bus.clone();
        let wstore = self.wstore.clone();
        let filestore = self.filestore.clone();
        let health_monitor = Arc::clone(&self.health_monitor);
        let block_id = self.block_id.clone();
        let self_ref_done = self.self_ref.lock().unwrap().clone().unwrap_or_default();

        // Spawn all async work (exec + I/O) into a background task so this
        // function returns synchronously. This is required so the queue-drain
        // path inside the reader task can call `spawn_container_turn` without
        // needing the returned future to be `'static`.
        tokio::spawn(async move {
            use bollard::container::LogOutput;

            // Start the exec via Docker socket — env vars travel through
            // CreateExecOptions.env (Docker API), never in process argv.
            let exec_result = cm
                .exec(&container_name, &cmd, None, &container_env)
                .await;
            let exec_session = match exec_result {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(block_id = %block_id, error = %e, "container exec failed");
                    // Surface the error in the agent pane so the user sees what went wrong.
                    if let Some(ref b) = broker {
                        let error_frame = serde_json::json!({
                            "type": "result",
                            "is_error": true,
                            "subtype": "error_during_execution",
                            "error": {"message": format!("[AgentMux] container exec failed: {e}")}
                        }).to_string();
                        shell::handle_append_block_file(
                            b, &block_id, SUBPROCESS_OUTPUT_SUBJECT,
                            format!("{error_frame}\n").as_bytes(),
                            filestore.as_ref(), None,
                        );
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
                        Self::set_status(&mut inner, STATUS_DONE);
                        inner.current_pid = None;
                        inner.kill_tx = None;
                    }
                    health_monitor.set_exited(1);
                    if let Some(ref b) = broker {
                        let status = {
                            let inner = inner_arc.lock().unwrap();
                            SubprocessController::build_status_snapshot(&inner, &block_id, false)
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
                                cm_for_drain,
                                container_name_for_drain,
                                base_cmd_for_drain,
                                cfg,
                            ) {
                                tracing::warn!(error = %e, "failed to spawn queued container turn");
                                Self::publish_queued_spawn_error(
                                    &block_id,
                                    &e,
                                    &broker,
                                    &filestore,
                                );
                            }
                        }
                    }
                    return;
                }
            };

            // Install a kill channel so stop_subprocess can interrupt this
            // in-flight exec. docker exec has no kill API, so the reader below
            // selects on kill_rx and pkills the in-container process. Stored only
            // after a successful exec start (the early-return failure path above
            // leaves kill_tx None — nothing to interrupt). Mirrors spawn_turn.
            let (kill_tx, mut kill_rx) = tokio::sync::oneshot::channel::<bool>();

            // Update status to running
            {
                let mut inner = inner_arc.lock().unwrap();
                inner.kill_tx = Some(kill_tx);
                Self::set_status(&mut inner, STATUS_RUNNING);
            }
            if let Some(ref b) = broker {
                let status = {
                    let inner = inner_arc.lock().unwrap();
                    // Published just before set_active_turn(true) below —
                    // accurate at the moment this snapshot is built.
                    SubprocessController::build_status_snapshot(&inner, &block_id, false)
                };
                publish_controller_status(b, &status);
            }
            health_monitor.set_active_turn(true);

            let crate::backend::container::ExecSession { exec_id, mut input, output } = exec_session;

            // Write the turn message to container stdin INLINE — not via a
            // detached `tokio::spawn`, which may not be scheduled for seconds
            // under runtime load and would trip the in-container CLI's "no
            // stdin data received in 3s" abort (the host path uses a dedicated
            // OS thread for the same reason — see spawn_turn). Awaiting here in
            // the already-running exec task guarantees the bytes hit the Docker
            // attach stream immediately. The CLI drains stdin to EOF before it
            // emits output, so this write cannot deadlock the read loop below.
            {
                let payload = format!("{}\n", config.message);
                if let Err(e) = input.write_all(payload.as_bytes()).await {
                    tracing::warn!(block_id = %block_id, "container exec stdin write error: {}", e);
                } else if let Err(e) = input.flush().await {
                    tracing::warn!(block_id = %block_id, "container exec stdin flush error: {}", e);
                }
                drop(input); // EOF to the container process
            }

            // Read stdout — accumulate bytes into lines.
            let mut line_buf = String::new();
            let mut stats = session_stats::SessionStatsAccumulator::new(block_id.clone());
            let session_id_field = config.session_id_field.clone();
            // Tracks an aborted output stream (`Some(Err(_))`). The exec may have
            // exited cleanly with a non-zero code OR the attach stream itself
            // failed mid-turn; either way the turn did not complete normally, so
            // this forces a non-zero exit even if inspect_exec can't be reached.
            let mut stream_errored = false;

            // Capture a bounded tail of stderr so a non-zero exit can be classified
            // into a real cause, mirroring host_spawn.rs's `stderr_tail`. No
            // Arc<Mutex<>> needed here (unlike host_spawn.rs) — this whole turn
            // runs in a single task, so a plain owned Vec captured by this async
            // block is sufficient.
            let mut stderr_tail: Vec<String> = Vec::new();
            // Retain the terminal `result` frame and any in-band API error
            // (e.g. a synthetic assistant message carrying a 401/auth failure
            // with exit 0 / is_error:false) so a completion-time classifier has
            // the same inputs host_spawn.rs's process_waiter does — this
            // controller has no HealthMonitor-driven route to AgentFailure now
            // that the silence watchdog is gone, so this is its only path.
            let mut last_result_frame: Option<serde_json::Value> = None;
            let mut last_inband_error: Option<serde_json::Value> = None;

            // Resolve the agent's GLOBAL transcript zone once (see persistent.rs)
            // so every container-exec `output` line is also mirrored to the
            // cross-channel store. `None` for non-agent blocks.
            let global_output_zone =
                shell::resolve_global_output_zone(&wstore, &block_id);

            tracing::info!(block_id = %block_id, "container exec output reader started");

            let mut pinned = std::pin::pin!(output);
            // Set when the turn is interrupted via stop_subprocess (Esc / agent.stop)
            // — drives a non-zero exit so an interrupted turn isn't reported as Idle.
            let mut killed = false;
            loop {
                tokio::select! {
                    // Prioritise the kill signal so Esc is responsive even under a
                    // steady output stream.
                    biased;
                    kill = &mut kill_rx => {
                        let force = kill.unwrap_or(false);
                        tracing::info!(block_id = %block_id, force, "container turn interrupt — pkill in container");
                        // Best-effort: actually terminate the in-container process.
                        // Even if this fails (e.g. no procps on an old image), we
                        // still break + finalize so AgentMux honours the stop.
                        if let Err(e) = cm.signal_exec_process(&container_name, &kill_pattern, force).await {
                            tracing::warn!(block_id = %block_id, error = %e, "container interrupt pkill failed");
                        }
                        killed = true;
                        break;
                    }
                    item = pinned.next() => {
                        match item {
                            None => {
                                // Stream ended — flush any remaining partial line.
                                if !line_buf.trim().is_empty() {
                                    Self::publish_line(&line_buf, &block_id, &session_id_field, &inner_arc, &wstore, &event_bus, &broker, &filestore, &mut stats, global_output_zone.as_deref(), &mut last_result_frame, &mut last_inband_error);
                                }
                                tracing::info!(block_id = %block_id, "container exec output EOF");
                                break;
                            }
                            Some(Err(e)) => {
                                tracing::warn!(block_id = %block_id, error = %e, "container exec output read error");
                                stream_errored = true;
                                break;
                            }
                            Some(Ok(log_output)) => {
                                let bytes = match log_output {
                                    LogOutput::StdOut { message } => message,
                                    LogOutput::StdErr { message } => {
                                        // Log stderr but don't publish as blockfile output.
                                        let s = String::from_utf8_lossy(&message);
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
                                        continue;
                                    }
                                    _ => continue,
                                };
                                let chunk = String::from_utf8_lossy(&bytes);
                                for ch in chunk.chars() {
                                    if ch == '\n' {
                                        if !line_buf.trim().is_empty() {
                                            Self::publish_line(&line_buf, &block_id, &session_id_field, &inner_arc, &wstore, &event_bus, &broker, &filestore, &mut stats, global_output_zone.as_deref(), &mut last_result_frame, &mut last_inband_error);
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
                match cm.inspect_exec(&exec_id).await {
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
                    let inband_text = last_inband_error.as_ref().map(|f| {
                        let err_str = f.get("error").and_then(|v| v.as_str()).unwrap_or("");
                        let content_text = f.pointer("/message/content/0/text")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        format!("{err_str} {content_text}")
                    }).unwrap_or_default();
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
                health_monitor.set_exited(inner.proc_exit_code);
            }

            if let Some(ref b) = broker {
                let status = {
                    let inner = inner_arc.lock().unwrap();
                    SubprocessController::build_status_snapshot(&inner, &block_id, false)
                };
                publish_controller_status(b, &status);
            }

            // Persist or clear agent:last_failure in block meta so the recovery
            // banner survives tab switches and page reloads, and a previously
            // persisted failure is cleared after a later successful turn — this
            // call handles both cases regardless of whether run_failure is
            // Some or None. Mirrors host_spawn.rs's process_waiter.
            core::persist_last_failure(&block_id, run_failure.as_ref(), &wstore, &event_bus);

            // Surface the classified failure cause to the pane. persist:1 so
            // reconnecting subscribers also receive the last failure without a
            // separate meta read (belt-and-suspenders with the meta write above).
            if let (Some(failure), Some(ref b)) = (run_failure.as_ref(), broker.as_ref()) {
                b.publish(wps::WaveEvent {
                    event: wps::EVENT_AGENT_FAILURE.to_string(),
                    scopes: vec![format!("block:{}", block_id)],
                    sender: String::new(),
                    persist: 1,
                    data: serde_json::to_value(failure).ok(),
                });
            }

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
                        cm_for_drain,
                        container_name_for_drain,
                        base_cmd_for_drain,
                        cfg,
                    ) {
                        tracing::warn!(error = %e, "failed to spawn queued container turn");
                        Self::publish_queued_spawn_error(
                            &block_id,
                            &e,
                            &broker,
                            &filestore,
                        );
                    }
                }
            }
        });

        Ok(())
    }

    /// A queued message has already been removed from `pending_messages` when
    /// its next spawn is attempted. If provider argv validation rejects that
    /// spawn, persist a transcript error so the message does not disappear
    /// with only a server log. Mirrors the host queue-drain failure path.
    fn publish_queued_spawn_error(
        block_id: &str,
        error: &str,
        broker: &Option<Arc<crate::backend::wps::Broker>>,
        filestore: &Option<Arc<crate::backend::storage::filestore::FileStore>>,
    ) {
        let Some(broker) = broker else {
            return;
        };
        let error_frame = serde_json::json!({
            "type": "result",
            "is_error": true,
            "subtype": "error_during_execution",
            "error": {"message": format!("[AgentMux] queued message could not be sent: {error}")}
        }).to_string();
        shell::handle_append_block_file(
            broker,
            block_id,
            SUBPROCESS_OUTPUT_SUBJECT,
            format!("{error_frame}\n").as_bytes(),
            filestore.as_ref(),
            None,
        );
    }

    /// Publish a single NDJSON line from container exec output: session-id
    /// capture, WPS blockfile event, and FileStore write-through. Used by
    /// `spawn_container_turn`'s output reader task.
    fn publish_line(
        line: &str,
        block_id: &str,
        session_id_field: &str,
        inner: &std::sync::Mutex<SubprocessControllerInner>,
        wstore: &Option<Arc<crate::backend::storage::store::Store>>,
        event_bus: &Option<Arc<crate::backend::eventbus::EventBus>>,
        broker: &Option<Arc<crate::backend::wps::Broker>>,
        filestore: &Option<Arc<crate::backend::storage::filestore::FileStore>>,
        stats: &mut session_stats::SessionStatsAccumulator,
        global_output_zone: Option<&str>,
        last_result_frame: &mut Option<serde_json::Value>,
        last_inband_error: &mut Option<serde_json::Value>,
    ) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return;
        }
        stats.record_line(trimmed.len(), wstore);

        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(trimmed) {
            // Capture session_id from provider init event.
            if let Some(sid) = parsed.get(session_id_field).and_then(|v| v.as_str()) {
                let changed = SubprocessController::record_captured_session_id_inner(inner, sid);
                if changed {
                    tracing::info!(block_id = %block_id, session_id = %sid, "container exec: captured session id");
                    core::persist_session_id(block_id, sid, &wstore, &event_bus);
                }
            }

            // Retain the terminal `result` frame for completion-time failure
            // classification (mirrors host_spawn.rs's stdout_reader).
            if parsed.get("type").and_then(|v| v.as_str()) == Some("result") {
                *last_result_frame = Some(parsed);
            } else if parsed.get("type").and_then(|v| v.as_str()) == Some("assistant")
                && (parsed.get("isApiErrorMessage").and_then(|v| v.as_bool()).unwrap_or(false)
                    || parsed.get("error").is_some())
            {
                // In-band API error: 401 / auth failures arrive as a synthetic
                // assistant message (exit 0, is_error:false on result frame) —
                // capture it so the completion-time classifier below can trip
                // the failure gate.
                *last_inband_error = Some(parsed);
            }
        }

        if let Some(ref broker) = broker {
            let line_with_newline = format!("{}\n", trimmed);
            shell::handle_append_block_file(
                broker,
                block_id,
                SUBPROCESS_OUTPUT_SUBJECT,
                line_with_newline.as_bytes(),
                filestore.as_ref(),
                global_output_zone,
            );
        }
    }
}
