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

use crate::backend::blockcontroller::{
    core, publish_controller_status, session_stats, shell, STATUS_DONE, STATUS_RUNNING,
};
use crate::backend::wps;

use super::{argv::build_turn_argv, SubprocessController, SubprocessControllerInner, SubprocessSpawnConfig, SUBPROCESS_OUTPUT_SUBJECT};

/// Wrap the CLI argv so it reads the turn prompt from `prompt_path`, a file
/// already written into the container by
/// [`ContainerManager::upload_turn_prompt`].
///
/// Redirecting from a file is the only transport that satisfies all three
/// constraints at once (see `upload_turn_prompt`'s doc for the measurements):
/// the CLI gets a real stdin that really reaches EOF, the message never touches
/// argv or env — so `docker top` / host `ps` / `/proc/<pid>/cmdline` can't see a
/// secret pasted into a chat message — and there is no `MAX_ARG_STRLEN` ceiling.
///
/// The CLI argv is passed through as `"$@"` rather than interpolated into the
/// script, so there is no shell quoting to get wrong, and it is passed
/// UNCHANGED — including a trailing `-` (codex) or a `-p ""` placeholder
/// (gemini/qwen/kimi/antigravity), both of which mean "read the prompt from
/// stdin" and are correct again now that stdin genuinely works (codex P1,
/// PR #2883).
///
/// The prompt file is removed after the CLI exits, and the CLI's own exit
/// status is preserved for `inspect_exec` to classify the turn.
fn container_turn_exec(prompt_path: &str, cmd: Vec<String>) -> Vec<String> {
    let mut out = vec![
        "sh".to_string(),
        "-c".to_string(),
        // $1 = prompt path, "$@" (after shift) = the CLI and its args.
        r#"F="$1"; shift; "$@" < "$F"; rc=$?; rm -f "$F"; exit $rc"#.to_string(),
        // $0 for the script -- a label only.
        "agentmux-turn".to_string(),
        prompt_path.to_string(),
    ];
    out.extend(cmd);
    out
}

#[cfg(test)]
mod turn_exec_tests {
    use super::container_turn_exec;

    fn v(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| s.to_string()).collect()
    }

    /// The security property this shape exists for (reagent P1, PR #2883): the
    /// message must never reach argv, where `docker top` / `ps` /
    /// `/proc/<pid>/cmdline` would expose a pasted secret. It can't — the argv
    /// is built from the prompt's PATH and never sees the message at all.
    #[test]
    fn the_argv_carries_only_a_path_never_the_message() {
        let argv = container_turn_exec("/tmp/agentmux-turn-abc", v(&["claude", "-p"]));
        assert!(argv.iter().any(|a| a == "/tmp/agentmux-turn-abc"), "the path is there");
        assert!(
            argv.iter().all(|a| !a.contains("ghp_") && !a.contains("secret")),
            "nothing message-shaped is: {argv:?}",
        );
    }

    /// The CLI must actually read the file — this redirect IS the fix.
    #[test]
    fn redirects_the_clis_stdin_from_the_prompt_file() {
        let argv = container_turn_exec("/tmp/p", v(&["claude", "-p"]));
        assert!(argv[2].contains(r#""$@" < "$F""#), "must redirect: {}", argv[2]);
    }

    /// The prompt file is transient state; leaving it behind would accumulate
    /// one file per turn for the life of the container.
    #[test]
    fn removes_the_prompt_file_afterwards() {
        let argv = container_turn_exec("/tmp/p", v(&["claude", "-p"]));
        assert!(argv[2].contains(r#"rm -f "$F""#), "must clean up: {}", argv[2]);
    }

    /// `inspect_exec` classifies the turn from the exit code, so the CLI's own
    /// status must survive the cleanup that runs after it.
    #[test]
    fn preserves_the_clis_exit_status_across_cleanup() {
        let argv = container_turn_exec("/tmp/p", v(&["claude", "-p"]));
        assert!(argv[2].contains("rc=$?"), "captures status before rm");
        assert!(argv[2].contains("exit $rc"), "and re-raises it: {}", argv[2]);
    }

    /// Passed as "$@", so the CLI argv survives verbatim and in order.
    #[test]
    fn passes_the_cli_argv_through_unchanged() {
        let argv = container_turn_exec("/tmp/p", v(&["claude", "-p", "--model", "opus"]));
        assert_eq!(&argv[0], "sh");
        assert_eq!(&argv[1], "-c");
        // argv[3] is $0, argv[4] is the prompt path; the CLI argv follows.
        assert_eq!(&argv[5..], &v(&["claude", "-p", "--model", "opus"])[..]);
    }

    /// codex's trailing `-` means "read the prompt from stdin" and must NOT be
    /// stripped: stdin is a real, EOF-terminating file now. An earlier cut of
    /// this fix removed it, which was only necessary while the message was
    /// being smuggled through argv.
    #[test]
    fn keeps_codexs_trailing_stdin_positional() {
        let argv = container_turn_exec("/tmp/p", v(&["codex", "exec", "--json", "-"]));
        assert_eq!(argv.last().unwrap(), "-");
    }

    /// Same for the `-p ""` placeholder gemini/qwen/kimi/antigravity use to
    /// mean "prompt comes from stdin" (codex P1, PR #2883).
    #[test]
    fn keeps_an_empty_prompt_placeholder() {
        let argv = container_turn_exec("/tmp/p", v(&["gemini", "-p", "", "--json"]));
        assert_eq!(&argv[5..], &v(&["gemini", "-p", "", "--json"])[..]);
    }
}

impl SubprocessController {
    /// Spawn a container agent turn via Docker socket (P1a: no secrets in argv).
    ///
    /// This is the secure alternative to `spawn_turn` for container agents. Instead
    /// of running `docker exec -e KEY=VALUE ...` as a CLI subprocess (which exposes
    /// secrets in process argv / `/proc/<pid>/cmdline`, CWE-214), this method calls
    /// `ContainerManager::exec` directly, passing env vars through
    /// `CreateExecOptions.env` (Docker socket). The exec I/O (file-backed prompt
    /// + stdout NDJSON stream) drives the same state machine as `spawn_turn`:
    ///   • appends `--resume <sid>` if a prior session_id is known
    ///   • uploads the turn message into the container as a file and redirects
    ///     the CLI's stdin from it — see `container_turn_exec` for why the
    ///     exec's own stdin, argv, and env are all unusable for this
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
        // The prompt is written into the container as a file per turn (see
        // `container_turn_exec`). A fresh id per turn keeps two turns — this
        // block's next one, or another block sharing the container — from
        // racing on the same path, including the `rm -f` that cleans it up.
        let turn_id = uuid::Uuid::new_v4().to_string();
        let turn_message = config.message.clone();
        let self_ref_done = self.self_ref.lock().unwrap().clone().unwrap_or_default();

        // Spawn all async work (exec + I/O) into a background task so this
        // function returns synchronously. This is required so the queue-drain
        // path inside the reader task can call `spawn_container_turn` without
        // needing the returned future to be `'static`.
        tokio::spawn(async move {
            use bollard::container::LogOutput;

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
            // the value, and the reader loop below selects on it `biased`, so
            // it is observed on the first poll and interrupted immediately —
            // including the prompt-file cleanup. The failure paths above/below
            // clear `kill_tx` again, so nothing is left dangling.
            let (kill_tx, mut kill_rx) = tokio::sync::oneshot::channel::<bool>();
            {
                let mut inner = inner_arc.lock().unwrap();
                inner.kill_tx = Some(kill_tx);
                Self::set_status(&mut inner, STATUS_RUNNING);
            }
            if let Some(ref b) = broker {
                let status = {
                    let inner = inner_arc.lock().unwrap();
                    SubprocessController::build_status_snapshot(&inner, &block_id, false)
                };
                publish_controller_status(b, &status);
            }
            health_monitor.set_active_turn(true);

            // Start the exec via Docker socket — env vars travel through
            // CreateExecOptions.env (Docker API), never in process argv.
            // Set once the upload succeeds; the interrupt path uses it to remove
            // a prompt file whose wrapper was killed before it could.
            let mut prompt_path_for_kill = String::new();
            // Write the prompt into the container FIRST, then run the CLI with
            // its stdin redirected from that file. See `container_turn_exec`
            // and `ContainerManager::upload_turn_prompt` for why neither the
            // exec's own stdin, nor argv, nor an env var can carry it.
            let exec_result = match cm
                .upload_turn_prompt(&container_name, &turn_id, &turn_message)
                .await
            {
                Ok(prompt_path) => {
                    // Retained for the interrupt path below, which has to do
                    // the cleanup the killed wrapper can't.
                    prompt_path_for_kill = prompt_path.clone();
                    let wrapped = container_turn_exec(&prompt_path, cmd);
                    // attach_stdin: false — the CLI's stdin is the prompt file.
                    match cm.exec(&container_name, &wrapped, None, &container_env, false).await {
                        Ok(session) => Ok(session),
                        Err(e) => {
                            // The wrapper never ran, so neither did its
                            // `rm -f "$F"` — clean up here or the prompt is
                            // orphaned in the container (reagent P2, PR #2883).
                            cm.remove_turn_prompt(&container_name, &prompt_path).await;
                            Err(e)
                        }
                    }
                }
                Err(e) => Err(e),
            };
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
                    // Clears active_turn as well as recording the exit code —
                    // which is why the earlier `set_active_turn(true)`, now
                    // hoisted above the upload, needs no explicit undo here.
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

            // `input` is unused by design — the turn message is in argv (see
            // above). Dropping it immediately keeps no handle on a write half
            // that can never be closed anyway.
            let crate::backend::container::ExecSession { exec_id, input, output } = exec_session;
            drop(input);

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
                        // The wrapper carries the `rm -f`, and `pkill -f
                        // <cli>` matches the wrapper too -- its argv contains
                        // the CLI name as an argument -- so the shell dies
                        // alongside the CLI and its cleanup never runs. An
                        // interrupted turn would otherwise leave a mode-0600
                        // prompt, possibly holding a pasted secret, in a
                        // container that outlives the turn (codex P2,
                        // PR #2883). Clean up from out here, where nothing was
                        // killed.
                        cm.remove_turn_prompt(&container_name, &prompt_path_for_kill).await;
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
