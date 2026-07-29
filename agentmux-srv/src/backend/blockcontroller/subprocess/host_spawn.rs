// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Host-subprocess turn spawn (`spawn_turn`) for [`SubprocessController`].
//!
//! Runs the agent CLI as a `std::process` child (via `tokio::process`),
//! writes the turn message to its stdin, and drives two background tasks:
//! a stdout/stderr reader pair and a process-waiter that finalizes status
//! and drains the queued-message backlog. See `container_spawn` for the
//! Docker-exec analog used by container agents.
//!
//! `spawn_turn` is one continuous, non-trivially-ordered state machine and
//! is moved here WHOLE rather than decomposed further.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncBufReadExt, BufReader};

use crate::backend::blockcontroller::{
    core, health::classify_output_line, publish_controller_status, session_stats, shell,
    DEFAULT_GRACEFUL_KILL_WAIT_MS, STATUS_DONE, STATUS_RUNNING,
};
use crate::backend::wps;

use super::{SubprocessController, SubprocessSpawnConfig, SUBPROCESS_OUTPUT_SUBJECT};

impl SubprocessController {
    /// Spawn a single turn of the agent CLI.
    ///
    /// This is the core method — it spawns `claude -p`, writes the user message to stdin,
    /// reads NDJSON from stdout (publishing WPS events), and waits for exit.
    ///
    /// If a session_id exists from a previous turn, `--resume <sid>` is appended to args.
    pub fn spawn_turn(&self, config: SubprocessSpawnConfig) -> Result<(), String> {
        if !self.try_lock_run() {
            // Turn in progress — queue the message for after it exits.
            let mut inner = self.inner.lock().unwrap();
            tracing::info!(
                block_id = %self.block_id,
                queue_depth = inner.pending_messages.len() + 1,
                "subprocess busy — message queued"
            );
            inner.pending_messages.push_back(config);
            return Ok(());
        }

        // Hydrate inner.session_id from the config-supplied id if the
        // controller hasn't captured one yet — MUST run before the
        // session_id_hint read just below. See
        // `hydrate_session_id_from_config` for the full rationale
        // (picker reattach: a fresh controller's inner.session_id is
        // None, but config.session_id carries the persisted id).
        // Reordering this after the read (reagent P0 on #2359, an
        // earlier revision of this same PR) silently dropped --resume
        // on every reattach's first turn — hydration ran too late for
        // the read below to see it.
        self.hydrate_session_id_from_config(config.session_id.as_deref());

        // Read the (now-hydrated) session id once, up front — used
        // both by the lease claim below and by the resume-flag args
        // built after it.
        let session_id_hint = self.inner.lock().unwrap().session_id.clone();

        // Claim the cross-process session-ownership lease BEFORE
        // `emit_message_accepted` / flipping status to running — a
        // refusal here must look like the turn never started at all,
        // not "frontend was told this message was accepted, then it
        // silently never ran" (reagent P1 on #2359: emitting accepted
        // before the claim meant a HeldByOther refusal left the
        // frontend believing the message was in flight). Empty
        // `instance_id` (container-mode branch in this PR) or no
        // `lease_store` (registry unavailable) both mean leasing is a
        // no-op for this spawn. See `registry::LeaseStore` and
        // `docs/retros/RETRO_DEV_BUILD_SHARED_AGENT_SESSION_COLLISION_2026_07_29.md`.
        let claimed_lease: Option<crate::registry::Lease> = if config.instance_id.is_empty() {
            None
        } else {
            match &self.lease_store {
                None => None,
                Some(store) => match store.claim(
                    &config.instance_id,
                    &self.boot_id,
                    &self.block_id,
                    session_id_hint.as_deref(),
                ) {
                    Ok(lease) => Some(lease),
                    Err(crate::registry::LeaseError::HeldByOther { owner_boot_id, age_ms, .. }) => {
                        self.unlock_run();
                        return Err(format!(
                            "session '{}' is already owned by another AgentMux process \
                             (boot {owner_boot_id}, renewed {age_ms}ms ago) — refusing to \
                             start a turn against the same session from two processes at once",
                            config.instance_id
                        ));
                    }
                    Err(crate::registry::LeaseError::Io(e)) => {
                        tracing::warn!(
                            block_id = %self.block_id,
                            instance_id = %config.instance_id,
                            error = %e,
                            "lease claim failed (io) — proceeding without a lease for this turn"
                        );
                        None
                    }
                },
            }
        };

        // Direct-spawn path (queue was empty): emit the accepted event
        // now so the frontend can promote its pending entry. The
        // drain-from-queue path (in process_waiter) emits the same
        // event just before calling spawn_turn recursively. Only
        // reached once the lease claim above has succeeded (or was a
        // no-op) — see that block's comment.
        self.emit_message_accepted(&config);

        // Build CLI args, appending resume flag + session_id if we have one and the provider supports it
        let mut args = config.cli_args.clone();
        if let Some(ref sid) = session_id_hint {
            if !config.resume_flag.is_empty() {
                args.push(config.resume_flag.clone());
                args.push(sid.clone());
            }
        }

        // Update status to running
        {
            let mut inner = self.inner.lock().unwrap();
            Self::set_status(&mut inner, STATUS_RUNNING);
        }
        self.publish_status();
        self.health_monitor.set_active_turn(true);

        // Build command — on Windows, .cmd batch wrappers can't be reliably spawned
        // via cmd.exe /C with piped stdio. Resolve to node <script> instead.
        let mut cmd = crate::server::cli_handlers::make_cli_cmd(&config.cli_command);
        cmd.args(&args);

        // On Windows: suppress console-window allocation. Without CREATE_NO_WINDOW,
        // node.exe spawned from a windowless sidecar may try to create/attach to a
        // console, causing stdout to go to that console rather than the pipe.
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        core::apply_working_dir(&mut cmd, &self.block_id, &config.working_dir, &config.env_vars);
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        // Spawn
        let mut child = cmd.spawn().map_err(|e| {
            let mut inner = self.inner.lock().unwrap();
            Self::set_status(&mut inner, STATUS_DONE);
            inner.proc_exit_code = -1;
            if let (Some(store), Some(lease)) = (&self.lease_store, &claimed_lease) {
                if let Err(release_err) = store.release(lease) {
                    tracing::warn!(
                        block_id = %self.block_id,
                        error = %release_err,
                        "lease release failed after spawn failure (self-heals via TTL expiry)"
                    );
                }
            }
            self.unlock_run();
            format!("failed to spawn subprocess: {e}")
        })?;

        let pid = child.id().unwrap_or(0);
        tracing::info!(
            block_id = %self.block_id,
            pid = pid,
            cmd = %config.cli_command,
            args = ?args,
            "subprocess spawned"
        );

        // Assign the child to this block's process tracker so every
        // descendant it spawns (bg bash, dev servers, watchers, etc.)
        // is caught by the per-platform tracking mechanism and surfaces
        // in the swarm activity panel. No-op if the tracker global
        // hasn't been initialized (tests) or on platforms without a
        // real tracker impl yet (stub handle accepts silently).
        // See `backend::process_tracker`.
        if pid != 0 {
            if let Some(registry) = crate::backend::process_tracker::registry::global() {
                let tracker = registry.ensure_tracker(&self.block_id);
                if let Err(e) = tracker.assign_process(pid) {
                    tracing::warn!(
                        block_id = %self.block_id,
                        pid = pid,
                        err = %e,
                        "[process-tracker] assign_process failed"
                    );
                }
            }
        }

        // Store PID
        let (kill_tx, kill_rx) = tokio::sync::oneshot::channel::<bool>();
        {
            let mut inner = self.inner.lock().unwrap();
            inner.current_pid = Some(pid);
            inner.kill_tx = Some(kill_tx);
        }

        // Take ownership of stdin/stdout (piped via Stdio::piped() in spawn config).
        let stdin = child.stdin.take()
            .ok_or_else(|| format!("[subprocess] stdin not captured for block {}", self.block_id))?;
        let stdout = child.stdout.take()
            .ok_or_else(|| format!("[subprocess] stdout not captured for block {}", self.block_id))?;
        let stderr = child.stderr.take()
            .ok_or_else(|| format!("[subprocess] stderr not captured for block {}", self.block_id))?;

        // Write user message to stdin, then close it.
        // CRITICAL: This must complete BEFORE the child's stdin timeout
        // (Claude CLI: 3s). Using std::thread + synchronous write to
        // bypass the Tokio task scheduler — a tokio::spawn'd task may
        // not run for seconds on a busy runtime, causing the child to
        // time out with "no stdin data received in 3s".
        let message = config.message;
        let block_id_stdin = self.block_id.clone();
        {
            // Convert Tokio's async ChildStdin to a raw OS handle, then
            // wrap in a std::fs::File for synchronous write. The pipe
            // buffer (4-64KB on Windows) easily fits our message, so
            // write_all returns instantly without blocking.
            #[cfg(unix)]
            let raw_handle = {
                use std::os::unix::io::{AsRawFd, FromRawFd};
                let fd = stdin.as_raw_fd();
                unsafe { std::fs::File::from_raw_fd(fd) }
            };
            #[cfg(windows)]
            let raw_handle = {
                use std::os::windows::io::{AsRawHandle, FromRawHandle};
                let handle = stdin.as_raw_handle();
                unsafe { std::fs::File::from_raw_handle(handle) }
            };

            // Spawn a real OS thread (not a Tokio task) for the write.
            // This ensures it runs immediately regardless of runtime load.
            // The raw handle is valid as long as `stdin` lives — we move
            // `stdin` into the thread via a guard to keep it alive.
            std::thread::spawn(move || {
                use std::io::Write;
                let _keep_alive = stdin; // prevent Tokio ChildStdin drop
                let mut pipe = raw_handle;
                let payload = format!("{}\n", message);
                if let Err(e) = pipe.write_all(payload.as_bytes()) {
                    tracing::warn!(block_id = %block_id_stdin, "subprocess stdin write error: {}", e);
                    std::mem::forget(pipe); // don't close the handle — _keep_alive owns it
                    return;
                }
                if let Err(e) = pipe.flush() {
                    tracing::warn!(block_id = %block_id_stdin, "subprocess stdin flush error: {}", e);
                }
                std::mem::forget(pipe); // don't double-close — _keep_alive owns the handle
                // _keep_alive (Tokio ChildStdin) drops here → EOF to the subprocess
            });
        }

        // Spawn stdout_reader task
        let block_id_read = self.block_id.clone();
        let broker_read = self.broker.clone();
        let inner_read = Arc::clone(&self.inner);
        let wstore_read = self.wstore.clone();
        let event_bus_read = self.event_bus.clone();
        let filestore_read = self.filestore.clone();
        let health_read = Arc::clone(&self.health_monitor);
        let session_id_field = config.session_id_field.clone();
        // Resolve the agent's GLOBAL transcript zone once (see persistent.rs).
        let global_output_zone =
            shell::resolve_global_output_zone(&self.wstore, &self.block_id);
        // Retain the terminal `result` frame so a failure reported on STDOUT
        // (auth / rate-limit / usage — the common case; claude may even exit 0)
        // can be classified, not just stderr-reported ones. Shared with the
        // process_waiter below.
        let last_result_frame: Arc<Mutex<Option<serde_json::Value>>> = Arc::new(Mutex::new(None));
        let last_result_frame_read = Arc::clone(&last_result_frame);
        // Track in-band API errors delivered as `assistant` frames (e.g. a
        // 401 auth failure that claude wraps in a synthetic assistant message
        // with `"error":"authentication_failed"` and exit 0 — bypasses the
        // `is_error` flag on the `result` frame entirely).
        let last_inband_error: Arc<Mutex<Option<serde_json::Value>>> = Arc::new(Mutex::new(None));
        let last_inband_error_read = Arc::clone(&last_inband_error);
        let stdout_reader_handle = tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            let mut stats = session_stats::SessionStatsAccumulator::new(block_id_read.clone());

            tracing::info!(block_id = %block_id_read, "stdout_reader started");

            loop {
                match lines.next_line().await {
                    Err(e) => {
                        tracing::warn!(block_id = %block_id_read, error = %e, "subprocess stdout read error");
                        break;
                    }
                    Ok(None) => {
                        tracing::info!(block_id = %block_id_read, "subprocess stdout EOF");
                        break;
                    }
                    Ok(Some(line)) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }

                        // Track session metadata (debounced 1 s).
                        // Use `line.len()` (not `trimmed.len()`) to match persistent.rs
                        // so token_estimate stays consistent across controller types.
                        stats.record_line(line.len(), &wstore_read);

                        // Classify output for health monitoring + retain the
                        // terminal `result` frame for failure classification.
                        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(trimmed) {
                            let (meaningful, error) = classify_output_line(&parsed);
                            health_read.record_output(meaningful);
                            if let Some((class, msg)) = error {
                                health_read.record_error(class, msg);
                            }
                            if parsed.get("type").and_then(|v| v.as_str()) == Some("result") {
                                *last_result_frame_read.lock().unwrap() = Some(parsed);
                            } else if parsed.get("type").and_then(|v| v.as_str()) == Some("assistant")
                                && (parsed.get("isApiErrorMessage").and_then(|v| v.as_bool()).unwrap_or(false)
                                    || parsed.get("error").is_some())
                            {
                                // In-band API error: 401 / auth failures arrive as a synthetic
                                // assistant message (exit 0, is_error:false on result frame) —
                                // capture it so the process_waiter can trip the failure gate.
                                *last_inband_error_read.lock().unwrap() = Some(parsed);
                            }
                        }

                        // Try to capture session/thread ID from the provider's init event.
                        // Claude: {"type":"system","subtype":"init","session_id":"..."}
                        // Gemini: {"type":"init","session_id":"..."}
                        // Codex:  {"type":"thread.started","thread_id":"..."}
                        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(trimmed) {
                            if let Some(sid) = parsed.get(&session_id_field).and_then(|v| v.as_str()) {
                                let sid_string = sid.to_string();
                                // Authoritative CLI capture —
                                // overwrites any prior value
                                // (including stale hydrated ids
                                // from picker reattach). De-dups
                                // when the same id repeats across
                                // turns. See
                                // `record_captured_session_id_inner`
                                // for the unit-tested form.
                                let changed = SubprocessController::record_captured_session_id_inner(
                                    &inner_read,
                                    &sid_string,
                                );
                                if changed {
                                    tracing::info!(
                                        block_id = %block_id_read,
                                        field = %session_id_field,
                                        session_id = %sid_string,
                                        "captured session id"
                                    );
                                    core::persist_session_id(&block_id_read, &sid_string, &wstore_read, &event_bus_read);
                                }
                            }
                        }

                        // Publish the NDJSON line as a WPS blockfile event on the "output" subject
                        // and write-through to FileStore for persistent history (Phase 1.3).
                        if let Some(ref broker) = broker_read {
                            // debug, not info: fires on every NDJSON line, and
                            // logs the FULL line content (not just length like
                            // persistent.rs's sibling) — a real contributor
                            // (~6%) to an unrotated 406 MB launcher-log mirror
                            // on a real machine (SPEC_WIN10_PAGEFILE_OOM_CRASH_
                            // 2026_06_29 P1). muxlog.mjs already treats this
                            // exact line as noise-to-drop-by-default when
                            // tailing, independent corroboration. Default
                            // production filter is info, so this is now
                            // suppressed unless RUST_LOG=debug is set.
                            tracing::debug!(block_id = %block_id_read, line = %trimmed, "subprocess stdout → blockfile");
                            // Include the newline so the frontend line splitter works correctly
                            let line_with_newline = format!("{}\n", trimmed);
                            shell::handle_append_block_file(
                                broker,
                                &block_id_read,
                                SUBPROCESS_OUTPUT_SUBJECT,
                                line_with_newline.as_bytes(),
                                filestore_read.as_ref(),
                                global_output_zone.as_deref(),
                            );
                        }
                    }
                }
            }

            tracing::info!(block_id = %block_id_read, "stdout_reader exiting");
        });

        // Capture a bounded tail of stderr so a non-zero exit can be classified
        // into a real cause (SPEC_AGENT_FAILURE_DIAGNOSTICS Phase 2) instead of a
        // bare "exit N". Shared with the process_waiter below.
        let stderr_tail: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let stderr_tail_reader = Arc::clone(&stderr_tail);
        // Spawn stderr reader (logs warnings + retains a tail for classification)
        let block_id_err = self.block_id.clone();
        let stderr_reader_handle = tokio::spawn(async move {
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            loop {
                match lines.next_line().await {
                    Err(e) => {
                        tracing::warn!(block_id = %block_id_err, error = %e, "subprocess stderr read error");
                        break;
                    }
                    Ok(None) => break,
                    Ok(Some(line)) => {
                        if !line.trim().is_empty() {
                            tracing::info!(
                                block_id = %block_id_err,
                                stderr = %line,
                                "subprocess stderr"
                            );
                            // Retain the last ~40 non-empty lines for classification.
                            let mut buf = stderr_tail_reader.lock().unwrap();
                            buf.push(line);
                            let overflow = buf.len().saturating_sub(40);
                            if overflow > 0 {
                                buf.drain(0..overflow);
                            }
                        }
                    }
                }
            }
        });

        core::spawn_health_watchdog(&self.health_monitor);

        // Renew the lease (if any) on the same cadence as the health
        // watchdog above, for as long as THIS turn is active. A
        // dedicated task rather than widening `spawn_health_watchdog`'s
        // signature — that helper has 7 call sites across every
        // controller type (ACP, persistent, container, host); only
        // host-mode claims a lease in this PR (see module + struct doc
        // comments), so touching the other 6 for an always-`None`
        // param isn't warranted.
        //
        // Exit condition is a fresh per-turn flag (`turn_done`), NOT
        // `health_monitor.is_active_turn()` — that flag is shared by
        // the whole controller, not scoped to this specific spawn.
        // Queued messages drain synchronously in process_waiter: the
        // next turn's `set_active_turn(true)` can land in the same
        // tick this turn's process_waiter releases its lease, with no
        // async gap. A stale renewal task watching the shared flag
        // would see it flip back to `true` before its next 5s tick and
        // loop forever, leaking one never-terminating renewal task
        // (blocking flock + file write every tick) per turn in a busy,
        // back-to-back conversation — reagent P1 on #2359.
        let turn_done = Arc::new(AtomicBool::new(false));
        if let (Some(store), Some(lease)) = (self.lease_store.clone(), claimed_lease.clone()) {
            let turn_done_renew = Arc::clone(&turn_done);
            let block_id_renew = self.block_id.clone();
            tokio::spawn(async move {
                let mut interval =
                    tokio::time::interval(std::time::Duration::from_millis(
                        crate::registry::RENEW_INTERVAL_MS,
                    ));
                loop {
                    interval.tick().await;
                    if turn_done_renew.load(Ordering::SeqCst) {
                        break;
                    }
                    if let Err(e) = store.renew(&lease) {
                        // Lost the lease to a TTL reclaim mid-turn — log and
                        // let the turn finish naturally rather than killing
                        // it. Force-killing on lost renewal is deliberately
                        // deferred (see the analysis doc's follow-ups); this
                        // is a real residual gap, not an oversight.
                        tracing::warn!(
                            block_id = %block_id_renew,
                            instance_id = %lease.instance_id(),
                            error = %e,
                            "lease renew failed — lease may have been reclaimed by another process"
                        );
                        break;
                    }
                }
            });
        }

        // Spawn process_waiter task
        let inner_wait = Arc::clone(&self.inner);
        let block_id_wait = self.block_id.clone();
        let broker_wait = self.broker.clone();
        let run_lock = Arc::clone(&self.run_lock);
        let health_wait = Arc::clone(&self.health_monitor);
        let turn_done_wait = Arc::clone(&turn_done);
        let self_ref_wait = self.self_ref.lock().unwrap().clone().unwrap_or_default();
        let stderr_tail_wait = Arc::clone(&stderr_tail);
        let last_result_frame_wait = Arc::clone(&last_result_frame);
        let last_inband_error_wait = Arc::clone(&last_inband_error);
        let wstore_wait = self.wstore.clone();
        let event_bus_wait = self.event_bus.clone();
        let lease_store_wait = self.lease_store.clone();
        let claimed_lease_wait = claimed_lease;
        tokio::spawn(async move {
            // Classified failure cause, surfaced to the pane after the readers drain.
            let mut run_failure: Option<crate::agents::failure::AgentFailure> = None;
            // Set on a clean (non-killed) exit so classification runs AFTER the
            // stdout/stderr readers are joined — otherwise the final error line can
            // race the buffer read and be lost (reagent P1).
            let mut clean_exit: Option<(i32, Option<i32>)> = None;
            // Wait for either process exit or kill signal
            tokio::select! {
                exit_result = child.wait() => {
                    let (exit_code, exit_signal) = match exit_result {
                        Ok(status) => {
                            let code = status.code().unwrap_or(-1);
                            #[cfg(unix)]
                            let sig = std::os::unix::process::ExitStatusExt::signal(&status);
                            #[cfg(not(unix))]
                            let sig: Option<i32> = None;
                            (code, sig)
                        }
                        Err(e) => {
                            tracing::warn!(
                                block_id = %block_id_wait,
                                error = %e,
                                "subprocess wait error"
                            );
                            (-1, None)
                        }
                    };

                    tracing::info!(
                        block_id = %block_id_wait,
                        exit_code = exit_code,
                        "subprocess exited"
                    );

                    // Update inner state
                    {
                        let mut inner = inner_wait.lock().unwrap();
                        inner.proc_exit_code = exit_code;
                        SubprocessController::set_status(&mut inner, STATUS_DONE);
                        inner.current_pid = None;
                        inner.kill_tx = None;
                    }

                    // Defer classification until after the readers are joined
                    // (below); a user-initiated stop (kill arm) stays unclassified.
                    clean_exit = Some((exit_code, exit_signal));
                }
                force = kill_rx => {
                    let force = force.unwrap_or(false);
                    tracing::info!(
                        block_id = %block_id_wait,
                        force = force,
                        "subprocess kill requested"
                    );

                    if force {
                        let _ = child.kill().await;
                    } else {
                        // On Unix, send SIGTERM. On Windows, kill() is the only option.
                        #[cfg(unix)]
                        {
                            if let Some(pid) = child.id() {
                                unsafe { libc::kill(pid as i32, libc::SIGTERM); }
                            }
                            // Give it a moment to exit gracefully
                            tokio::time::sleep(tokio::time::Duration::from_millis(
                                DEFAULT_GRACEFUL_KILL_WAIT_MS,
                            )).await;
                            let _ = child.kill().await;
                        }
                        #[cfg(not(unix))]
                        {
                            let _ = child.kill().await;
                        }
                    }

                    let _ = child.wait().await;

                    {
                        let mut inner = inner_wait.lock().unwrap();
                        inner.proc_exit_code = -1;
                        SubprocessController::set_status(&mut inner, STATUS_DONE);
                        inner.current_pid = None;
                        inner.kill_tx = None;
                    }
                }
            }

            // Classify a genuine non-zero exit OR a failure reported on stdout as
            // an error `result` frame (auth / rate-limit / usage — claude may even
            // exit 0). Join the stdout + stderr readers first (bounded) so their
            // final lines — the ones carrying the error text — are in the buffers
            // before we read them (reagent P1).
            if let Some((exit_code, exit_signal)) = clean_exit {
                let drain = std::time::Duration::from_secs(2);
                let _ = tokio::time::timeout(drain, stdout_reader_handle).await;
                let _ = tokio::time::timeout(drain, stderr_reader_handle).await;
                let result_frame = last_result_frame_wait.lock().unwrap().clone();
                let frame_is_error = result_frame
                    .as_ref()
                    .and_then(|f| f.get("is_error"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                // Also catch in-band API errors (e.g. auth 401) delivered as
                // synthetic assistant messages with exit 0 and is_error:false.
                let inband_error_frame = last_inband_error_wait.lock().unwrap().clone();
                let inband_is_api_error = inband_error_frame.is_some();
                if exit_code != 0 || frame_is_error || inband_is_api_error {
                    let tail = stderr_tail_wait.lock().unwrap().join("\n");
                    // Merge the in-band error text so classify() sees the
                    // "authentication_failed" / "401" string even though it
                    // arrived on stdout, not stderr.
                    let inband_text = inband_error_frame.as_ref().map(|f| {
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
                        exit_signal,
                        &combined_tail,
                        result_frame.as_ref(),
                    ));
                }
            }

            // Update health monitor with exit status
            {
                let inner = inner_wait.lock().unwrap();
                health_wait.set_exited(inner.proc_exit_code);
            }

            // Publish done status
            if let Some(ref broker) = broker_wait {
                let status = {
                    let inner = inner_wait.lock().unwrap();
                    SubprocessController::build_status_snapshot(&inner, &block_id_wait, false)
                };
                publish_controller_status(broker, &status);
            }

            // Persist or clear agent:last_failure in block meta so the recovery
            // banner survives tab switches and page reloads (P1.1 of
            // SPEC_AGENT_ERROR_FRAMEWORK_2026_06_20). Done before the WPS
            // publish so the durable state is written first; the event is then
            // a low-latency push to any active subscriber.
            core::persist_last_failure(
                &block_id_wait,
                run_failure.as_ref(),
                &wstore_wait,
                &event_bus_wait,
            );

            // Surface the classified failure cause to the pane (Phase 2 of
            // SPEC_AGENT_FAILURE_DIAGNOSTICS). persist:1 so reconnecting
            // subscribers also receive the last failure without needing a
            // separate meta read (belt-and-suspenders with the meta write above).
            if let (Some(failure), Some(broker)) = (run_failure.as_ref(), broker_wait.as_ref()) {
                broker.publish(wps::WaveEvent {
                    event: wps::EVENT_AGENT_FAILURE.to_string(),
                    scopes: vec![format!("block:{}", block_id_wait)],
                    sender: String::new(),
                    persist: 1,
                    data: serde_json::to_value(failure).ok(),
                });
            }

            // Signal the renewal task to stop BEFORE releasing the
            // lease/run_lock — a queued next turn (dequeued a few
            // lines below) may spawn immediately and claim its own
            // lease; this turn's renewal task must not still be
            // ticking when that happens (reagent P1 on #2359).
            turn_done_wait.store(true, Ordering::SeqCst);

            // Release the lease (if any), then the run lock — mirrors
            // the ordering in the spawn-failure closure above.
            if let (Some(store), Some(lease)) = (&lease_store_wait, &claimed_lease_wait) {
                if let Err(e) = store.release(lease) {
                    tracing::warn!(
                        block_id = %block_id_wait,
                        instance_id = %lease.instance_id(),
                        error = %e,
                        "lease release failed (self-heals via TTL expiry)"
                    );
                }
            }
            run_lock.store(false, Ordering::SeqCst);

            // Drain message queue: if messages were queued while this turn
            // was running, pop the next one and spawn it via the weak
            // self-reference.
            let next_config = {
                let mut inner = inner_wait.lock().unwrap();
                inner.pending_messages.pop_front()
            };
            if let Some(config) = next_config {
                if let Some(ctrl) = self_ref_wait.upgrade() {
                    tracing::info!(
                        block_id = %block_id_wait,
                        "draining queued message"
                    );
                    if let Err(e) = ctrl.spawn_turn(config) {
                        tracing::warn!(
                            block_id = %block_id_wait,
                            error = %e,
                            "failed to spawn queued turn"
                        );
                        // The message was already popped from the queue —
                        // a bare warn-log here means it vanishes with no
                        // trace the user can see (reagent P1 on #2359,
                        // most likely to fire on a lease refusal: another
                        // process claimed this session in the gap between
                        // this turn's release and the drain). Surface it
                        // the same way other pre-spawn failures in this
                        // module do (e.g. input.rs's container
                        // ensure_running failure): a visible error frame
                        // in the block, not just a log line.
                        let error_frame = serde_json::json!({
                            "type": "result",
                            "is_error": true,
                            "subtype": "error_during_execution",
                            "error": {"message": format!("[AgentMux] queued message could not be sent: {e}")}
                        }).to_string();
                        if let Some(ref broker) = broker_wait {
                            crate::backend::blockcontroller::shell::handle_append_block_file(
                                broker,
                                &block_id_wait,
                                SUBPROCESS_OUTPUT_SUBJECT,
                                format!("{error_frame}\n").as_bytes(),
                                None,
                                None,
                            );
                        }
                    }
                }
            }
        });

        Ok(())
    }
}
