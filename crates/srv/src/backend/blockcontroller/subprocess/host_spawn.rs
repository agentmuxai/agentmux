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

#[cfg(windows)]
use agentmux_common::win32::NoWindow;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncBufReadExt, BufReader};

#[cfg(unix)]
use crate::backend::blockcontroller::DEFAULT_GRACEFUL_KILL_WAIT_MS;
use crate::backend::blockcontroller::{
    core, publish_controller_status,
    session_stats, shell, STATUS_DONE, STATUS_RUNNING,
};

use super::{
    argv::{append_prompt_arg, args_for_log, build_turn_argv},
    SubprocessController, SubprocessSpawnConfig, SUBPROCESS_OUTPUT_SUBJECT,
};

impl SubprocessController {
    /// Spawn a single turn of the agent CLI.
    ///
    /// This is the core method — it spawns `claude -p`, writes the user message to stdin,
    /// reads NDJSON from stdout (publishing MPS events), and waits for exit.
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

        // A Codex `/compact` compacts the thread through a short-lived App
        // Server instead of running as an `exec` turn (codex_compact.rs).
        if super::codex_compact::is_compact_request(&config) {
            return self.compact_codex_thread(config, session_id_hint);
        }

        // Build continuation argv before claiming the lease or acknowledging
        // the message. A malformed provider configuration must fail as an
        // unstarted turn, not strand the pane in an accepted/running state.
        let prompt_in_argv = !config.prompt_arg_flag.is_empty();
        let args = match build_turn_argv(
            &config.cli_args,
            &config.resume_strategy,
            &config.resume_flag,
            session_id_hint.as_deref(),
        )
        .and_then(|mut args| {
            if prompt_in_argv {
                append_prompt_arg(&mut args, &config.prompt_arg_flag, &config.message)?;
            }
            Ok(args)
        }) {
            Ok(args) => args,
            Err(error) => {
                self.unlock_run();
                return Err(error);
            }
        };

        // Claim the cross-process session-ownership lease BEFORE `emit_message_accepted` / flipping status
        // to running: a refusal must look like the turn never started at all (#2359).
        let claimed_lease = match self.claim_turn_lease(&config, session_id_hint.as_deref()) {
            Ok(lease) => lease,
            Err(error) => {
                self.unlock_run();
                return Err(error);
            }
        };

        // Direct-spawn path (queue was empty): emit the accepted event
        // now so the frontend can promote its pending entry. The
        // drain-from-queue path (in process_waiter) emits the same
        // event just before calling spawn_turn recursively. Only
        // reached once the lease claim above has succeeded (or was a
        // no-op) — see that block's comment.
        self.emit_message_accepted(&config);

        // Update status to running
        {
            let mut inner = self.inner.lock().unwrap();
            Self::set_status(&mut inner, STATUS_RUNNING);
        }
        // Mark the turn active BEFORE publishing, so this "running" status
        // carries turn_active=true: it is the start edge the notification
        // Router pairs with the process_waiter's turn_active=false to detect
        // "finished" (subprocess agents otherwise never showed a true→false).
        self.health_monitor.set_active_turn(true);
        // Each turn runs its own process: the watchdog's runtime counts from here.
        self.health_monitor.note_spawned();
        self.publish_status();

        // Build command — on Windows, .cmd batch wrappers can't be reliably spawned
        // via cmd.exe /C with piped stdio. Resolve to node <script> instead.
        let mut cmd = crate::server::cli_handlers::make_cli_cmd(&config.cli_command);
        cmd.args(&args);

        // On Windows: suppress console-window allocation. Without CREATE_NO_WINDOW,
        // node.exe spawned from a windowless sidecar may try to create/attach to a
        // console, causing stdout to go to that console rather than the pipe.
        #[cfg(windows)]
        {
            cmd.no_window();
        }
        core::apply_working_dir(&mut cmd, &self.block_id, &config.working_dir, &config.env_vars);
        // Identity M4a: record what this process is actually given.
        crate::backend::identity_spawn::record_process_spawn(
            &self.block_id,
            crate::backend::identity_spawn::SpawnPath::Subprocess,
            &config.env_vars,
        );
        // Which model API address this turn's CLI is given, for the connectivity status.
        core::attach_model_api(&self.block_id, &config.env_vars, &self.mstore, &self.event_bus);
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        // Spawn
        // In the block's tracker before it runs (`spawn_tracked`), so every
        // descendant it spawns (bg bash, dev servers, watchers) is caught.
        let spawned = crate::backend::process_tracker::registry::spawn_tracked(&self.block_id, &mut cmd, crate::backend::process_tracker::registry::Join::Agent);
        if spawned.is_err() {
            // The turn was marked active (and published) above; a spawn that
            // never started must not leave it "active" forever — roll back and
            // publish the terminal state (Codex P2 on #3705). Done before the
            // map_err below takes the inner lock that publish_status needs.
            self.health_monitor.set_active_turn(false);
        }
        let child = spawned.map_err(|e| {
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
        });
        if child.is_err() {
            // Inner lock released by now: publish DONE with turn_active=false.
            self.publish_status();
        }
        let mut child = child?;

        let pid = child.id().unwrap_or(0);
        tracing::info!(
            block_id = %self.block_id,
            pid = pid,
            cmd = %config.cli_command,
            args = ?args_for_log(&args, prompt_in_argv),
            "subprocess spawned"
        );

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

        let message = config.message;
        let user_record = self.user_record_sink(config.message_id.is_some(), config.record.clone());
        self.write_prompt_and_close_stdin(stdin, message, prompt_in_argv, user_record);

        // Retain the terminal `result` frame so a failure reported on STDOUT
        // (auth / rate-limit / usage — the common case; claude may even exit 0)
        // can be classified, not just stderr-reported ones. Shared with the
        // process_waiter below.
        let last_result_frame: Arc<Mutex<Option<serde_json::Value>>> = Arc::new(Mutex::new(None));
        // Track in-band API errors delivered as `assistant` frames (e.g. a
        // 401 auth failure that claude wraps in a synthetic assistant message
        // with `"error":"authentication_failed"` and exit 0 — bypasses the
        // `is_error` flag on the `result` frame entirely).
        let last_inband_error: Arc<Mutex<Option<serde_json::Value>>> = Arc::new(Mutex::new(None));
        let stdout_reader_handle = self.spawn_stdout_reader(
            stdout,
            config.session_id_field.clone(),
            Arc::clone(&last_result_frame),
            Arc::clone(&last_inband_error),
        );

        // Capture a bounded tail of stderr so a non-zero exit can be classified
        // into a real cause (SPEC_AGENT_FAILURE_DIAGNOSTICS Phase 2) instead of a
        // bare "exit N". Shared with the process_waiter below.
        let stderr_tail: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let stderr_reader_handle = self.spawn_stderr_reader(stderr, Arc::clone(&stderr_tail));

        let turn_done = self.spawn_lease_renewal(&claimed_lease);

        let codex_home = (config.resume_strategy == "codex-exec")
            .then(|| super::codex_context::codex_home(&config.env_vars))
            .flatten();
        self.spawn_process_waiter(TurnWait {
            child,
            kill_rx,
            stdout_reader_handle,
            stderr_reader_handle,
            turn_done,
            stderr_tail,
            last_result_frame,
            last_inband_error,
            claimed_lease,
            codex_home,
        });

        Ok(())
    }
}

impl SubprocessController {
    /// Claim the cross-process session-ownership lease for a turn (see `registry::LeaseStore` and
    /// `docs/retro/RETRO_DEV_BUILD_SHARED_AGENT_SESSION_COLLISION_2026_07_29.md`). An empty `instance_id` or
    /// no `lease_store` (registry unavailable) means leasing is a no-op: `Ok(None)`. `Err` carries the
    /// refusal for a session another AgentMux process owns; the caller releases the run lock.
    fn claim_turn_lease(
        &self,
        config: &SubprocessSpawnConfig,
        session_id_hint: Option<&str>,
    ) -> Result<Option<crate::registry::Lease>, String> {
        if config.instance_id.is_empty() {
            return Ok(None);
        }
        let Some(store) = &self.lease_store else { return Ok(None) };
        match store.claim(&config.instance_id, &self.boot_id, &self.block_id, session_id_hint) {
            Ok(lease) => Ok(Some(lease)),
            Err(crate::registry::LeaseError::HeldByOther { owner_boot_id, age_ms, .. }) => Err(format!(
                "session '{}' is already owned by another AgentMux process \
                 (boot {owner_boot_id}, renewed {age_ms}ms ago) — refusing to \
                 start a turn against the same session from two processes at once",
                config.instance_id
            )),
            Err(crate::registry::LeaseError::Io(e)) => {
                tracing::warn!(
                    block_id = %self.block_id,
                    instance_id = %config.instance_id,
                    error = %e,
                    "lease claim failed (io) — proceeding without a lease for this turn"
                );
                Ok(None)
            }
        }
    }

    /// Write the user message to the child's stdin, then close it.
    /// CRITICAL: this must complete BEFORE the child's stdin timeout (Claude CLI: 3s). It uses a real OS
    /// thread and a synchronous write to bypass the Tokio task scheduler: a `tokio::spawn`'d task may not
    /// run for seconds on a busy runtime, and the child would time out with "no stdin data received in 3s".
    fn write_prompt_and_close_stdin(
        &self,
        stdin: tokio::process::ChildStdin,
        message: String,
        prompt_in_argv: bool,
        user_record: super::UserRecordSink,
    ) {
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
                if prompt_in_argv {
                    // The prompt went on the command line; stdin just closes.
                    std::mem::forget(pipe);
                    user_record.write(&message);
                    return;
                }
                let payload = format!("{}\n", message);
                if let Err(e) = pipe.write_all(payload.as_bytes()) {
                    tracing::warn!(block_id = %block_id_stdin, "subprocess stdin write error: {}", e);
                    std::mem::forget(pipe); // don't close the handle — _keep_alive owns it
                    return;
                }
                if let Err(e) = pipe.flush() {
                    tracing::warn!(block_id = %block_id_stdin, "subprocess stdin flush error: {}", e);
                    std::mem::forget(pipe);
                    return;
                }
                std::mem::forget(pipe); // don't double-close — _keep_alive owns the handle
                // Delivered: now the transcript may say so (Codex review).
                user_record.write(&message);
                // _keep_alive (Tokio ChildStdin) drops here → EOF to the subprocess
            });
        }
    }

    /// Reads the turn's stdout: counts each line, retains the `result` frame and any in-band API error for
    /// failure classification, captures the provider's session id from its init event, and publishes every
    /// line as a blockfile event and a transcript line.
    fn spawn_stdout_reader(
        &self,
        stdout: tokio::process::ChildStdout,
        session_id_field: String,
        last_result_frame_read: FrameSlot,
        last_inband_error_read: FrameSlot,
    ) -> tokio::task::JoinHandle<()> {
        let block_id_read = self.block_id.clone();
        let broker_read = self.broker.clone();
        let inner_read = Arc::clone(&self.inner);
        let mstore_read = self.mstore.clone();
        let event_bus_read = self.event_bus.clone();
        let filestore_read = self.filestore.clone();
        let health_read = Arc::clone(&self.health_monitor);
        // Resolve the agent's GLOBAL transcript zone once (see persistent/spawn.rs).
        let global_output_zone =
            crate::backend::transcript::append::resolve_global_output_zone(&self.mstore, &self.block_id);
        tokio::spawn(async move {
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
                        // Use `line.len()` (not `trimmed.len()`) to match persistent/stdout_reader.rs
                        // so token_estimate stays consistent across controller types.
                        stats.record_line(line.len(), &mstore_read);
                        health_read.note_output();

                        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(trimmed) {
                            // Try to capture session/thread ID from the provider's init event.
                            // Claude: {"type":"system","subtype":"init","session_id":"..."}
                            // Gemini: {"type":"init","session_id":"..."}
                            // Codex:  {"type":"thread.started","thread_id":"..."}
                            // Read before the frame may move into a slot below.
                            let sid = parsed
                                .get(&session_id_field)
                                .and_then(|v| v.as_str())
                                .map(str::to_string);

                            retain_failure_frame(parsed, &last_result_frame_read, &last_inband_error_read);

                            if let Some(sid_string) = sid {
                                capture_session_id(&inner_read, &block_id_read, &session_id_field, &sid_string, &mstore_read, &event_bus_read);
                            }
                        }

                        // Publish the NDJSON line as a MPS blockfile event on the "output" subject
                        // and write-through to FileStore for persistent history (Phase 1.3).
                        if let Some(ref broker) = broker_read {
                            // debug, not info: fires on every NDJSON line, and
                            // logs the FULL line content (not just length like
                            // persistent/stdout_reader.rs's sibling) — a real contributor
                            // (~6%) to an unrotated 406 MB launcher-log mirror
                            // on a real machine (SPEC_WIN10_PAGEFILE_OOM_CRASH_
                            // 2026_06_29 P1). muxlog.mjs already treats this
                            // exact line as noise-to-drop-by-default when
                            // tailing, independent corroboration. Default
                            // production filter is info, so this is now
                            // suppressed unless RUST_LOG=debug is set.
                            tracing::debug!(block_id = %block_id_read, line = %trimmed, "subprocess stdout → blockfile");
                            crate::backend::transcript::append::append_output_line(
                                broker,
                                &block_id_read,
                                trimmed,
                                filestore_read.as_ref(),
                                global_output_zone.as_deref(),
                            );
                        }
                    }
                }
            }

            tracing::info!(block_id = %block_id_read, "stdout_reader exiting");
        })
    }

    /// Logs the turn's stderr and retains its last ~40 non-empty lines for failure classification.
    fn spawn_stderr_reader(
        &self,
        stderr: tokio::process::ChildStderr,
        stderr_tail_reader: Arc<Mutex<Vec<String>>>,
    ) -> tokio::task::JoinHandle<()> {
        let block_id_err = self.block_id.clone();
        tokio::spawn(async move {
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
        })
    }

    /// Renews the lease (if any) on its own interval, for as long as THIS turn is active; returns the
    /// per-turn flag the turn's process waiter sets to stop it. Only host turns claim a lease.
    ///
    /// The exit condition is a fresh per-turn flag, NOT `health_monitor.is_active_turn()`, which is shared
    /// by the whole controller: queued messages drain synchronously in the process waiter, so the next
    /// turn's `set_active_turn(true)` can land in the same tick this turn releases its lease. A renewal
    /// task watching the shared flag would see it flip back before its next tick and never end, leaking
    /// one task per turn in a busy conversation (#2359).
    fn spawn_lease_renewal(&self, claimed_lease: &Option<crate::registry::Lease>) -> Arc<AtomicBool> {
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
                    // Blocking OS file lock + fsync'd write: off the async
                    // worker (see `backend::blocking`).
                    let (s, l) = (Arc::clone(&store), lease.clone());
                    let renewed = tokio::task::spawn_blocking(move || s.renew(&l).map_err(|e| e.to_string()))
                        .await
                        .unwrap_or_else(|e| Err(format!("renew task failed: {e}")));
                    if let Err(e) = renewed {
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
        turn_done
    }

    /// Waits for the turn's process to exit or be killed, then closes the turn: joins the readers and
    /// classifies a failure, reports the end, releases the lease and the run lock, and drains the next
    /// queued message.
    fn spawn_process_waiter(&self, wait: TurnWait) {
        let TurnWait {
            mut child,
            kill_rx,
            stdout_reader_handle,
            stderr_reader_handle,
            turn_done,
            stderr_tail,
            last_result_frame,
            last_inband_error,
            claimed_lease,
            codex_home,
        } = wait;
        let end = TurnEnd {
            inner: Arc::clone(&self.inner),
            block_id: self.block_id.clone(),
            broker: self.broker.clone(),
            mstore: self.mstore.clone(),
            event_bus: self.event_bus.clone(),
            filestore: self.filestore.clone(),
            health: Arc::clone(&self.health_monitor),
            run_lock: Arc::clone(&self.run_lock),
            lease_store: self.lease_store.clone(),
            self_ref: self.self_ref.lock().unwrap().clone().unwrap_or_default(),
        };
        tokio::spawn(async move {
            let clean_exit = end.wait_for_exit(&mut child, kill_rx).await;
            let run_failure = match clean_exit {
                Some((exit_code, exit_signal)) => {
                    end.classify_clean_exit(
                        exit_code,
                        exit_signal,
                        (stdout_reader_handle, stderr_reader_handle),
                        &TurnFrames { stderr_tail, last_result_frame, last_inband_error },
                        codex_home,
                    )
                    .await
                }
                None => None,
            };
            end.report_end(clean_exit.is_some(), run_failure.as_ref());
            end.release_turn(&turn_done, &claimed_lease);
            end.drain_next_queued();
        });
    }
}

/// What a turn's stdout and stderr readers retained for classifying its failure.
struct TurnFrames {
    stderr_tail: Arc<Mutex<Vec<String>>>,
    last_result_frame: FrameSlot,
    last_inband_error: FrameSlot,
}

/// The handles a turn's process waiter closes the turn with (clones of the controller's own).
struct TurnEnd {
    inner: Arc<Mutex<super::SubprocessControllerInner>>,
    block_id: String,
    broker: Option<Arc<crate::backend::mps::Broker>>,
    mstore: Option<Arc<crate::backend::storage::store::Store>>,
    event_bus: Option<Arc<crate::backend::eventbus::EventBus>>,
    filestore: Option<Arc<crate::backend::storage::filestore::FileStore>>,
    health: Arc<crate::backend::blockcontroller::health::TurnActivityTracker>,
    run_lock: Arc<AtomicBool>,
    lease_store: Option<Arc<crate::registry::LeaseStore>>,
    self_ref: std::sync::Weak<SubprocessController>,
}

impl TurnEnd {
    /// Wait for either process exit or a kill signal and record the exit. `Some((code, signal))` is a
    /// process that exited on its own; a stop returns `None` and stays unclassified.
    async fn wait_for_exit(
        &self,
        child: &mut tokio::process::Child,
        kill_rx: tokio::sync::oneshot::Receiver<bool>,
    ) -> Option<(i32, Option<i32>)> {
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
                            block_id = %self.block_id,
                            error = %e,
                            "subprocess wait error"
                        );
                        (-1, None)
                    }
                };

                tracing::info!(
                    block_id = %self.block_id,
                    exit_code = exit_code,
                    "subprocess exited"
                );

                // Update inner state
                {
                    let mut inner = self.inner.lock().unwrap();
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
                    block_id = %self.block_id,
                    force = force,
                    "subprocess kill requested"
                );

                // The final kill takes the CLI's process group (its MCP
                // servers) with it; the SIGTERM stays the CLI's own.
                if force {
                    let _ = agentmux_common::process::kill_child_and_group(child).await;
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
                        let _ = agentmux_common::process::kill_child_and_group(child).await;
                    }
                    #[cfg(not(unix))]
                    {
                        let _ = child.kill().await;
                    }
                }

                let _ = child.wait().await;

                {
                    let mut inner = self.inner.lock().unwrap();
                    inner.proc_exit_code = -1;
                    SubprocessController::set_status(&mut inner, STATUS_DONE);
                    inner.current_pid = None;
                    inner.kill_tx = None;
                }
            }
        }
        clean_exit
    }

    /// Classify a genuine non-zero exit OR a failure reported on stdout as an error `result` frame (auth,
    /// rate-limit, usage; claude may even exit 0). The stdout and stderr readers are joined first
    /// (bounded) so their final lines, the ones carrying the error text, are in the buffers before they
    /// are read.
    async fn classify_clean_exit(
        &self,
        exit_code: i32,
        exit_signal: Option<i32>,
        readers: (tokio::task::JoinHandle<()>, tokio::task::JoinHandle<()>),
        frames: &TurnFrames,
        codex_home: Option<std::path::PathBuf>,
    ) -> Option<crate::agents::failure::AgentFailure> {
        let (stdout_reader_handle, stderr_reader_handle) = readers;
        let TurnFrames { stderr_tail, last_result_frame, last_inband_error } = frames;
        let mut run_failure: Option<crate::agents::failure::AgentFailure> = None;
        let drain = std::time::Duration::from_secs(2);
        let _ = tokio::time::timeout(drain, stdout_reader_handle).await;
        let _ = tokio::time::timeout(drain, stderr_reader_handle).await;
        let result_frame = last_result_frame.lock().unwrap().clone();
        let frame_is_error = result_frame
            .as_ref()
            .and_then(|f| f.get("is_error"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        // Also catch in-band API errors (e.g. auth 401) delivered as
        // synthetic assistant messages with exit 0 and is_error:false.
        let inband_error_frame = last_inband_error.lock().unwrap().clone();
        let inband_is_api_error = inband_error_frame.is_some();
        if exit_code != 0 || frame_is_error || inband_is_api_error {
            let tail = stderr_tail.lock().unwrap().join("\n");
            // Merge the in-band error text so classify() sees the
            // "authentication_failed" / "401" string even though it
            // arrived on stdout, not stderr.
            let inband_text = inband_error_frame.as_ref().map(crate::agents::limit_reset::inband_error_text).unwrap_or_default();
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
        self.publish_codex_context(codex_home).await;
        run_failure
    }

    /// A Codex turn's context, read from its session file once it exits (`codex_context.rs`): `codex
    /// exec` reports only the thread's totals.
    async fn publish_codex_context(&self, codex_home: Option<std::path::PathBuf>) {
        let thread_id = self.inner.lock().unwrap().session_id.clone();
        if let (Some(home), Some(thread_id), Some(broker)) = (codex_home, thread_id, self.broker.as_ref()) {
            let read = tokio::task::spawn_blocking(move || super::codex_context::read(&home, &thread_id)).await;
            if let Ok(Some(ctx)) = read {
                crate::backend::context_reading::observe_codex(&self.block_id, ctx.tokens, ctx.window, ctx.model.clone());
                let zone = crate::backend::transcript::append::resolve_global_output_zone(&self.mstore, &self.block_id);
                crate::backend::blockcontroller::shell::handle_append_block_file(
                    broker,
                    &self.block_id,
                    SUBPROCESS_OUTPUT_SUBJECT,
                    format!("{}\n", super::codex_context::frame(&ctx)).as_bytes(),
                    self.filestore.as_ref(),
                    zone.as_deref(),
                );
            }
        }
    }

    /// Record the exit for the health monitor, publish the done status, persist or clear the failure,
    /// and tell the pane about it.
    fn report_end(&self, exited_on_its_own: bool, run_failure: Option<&crate::agents::failure::AgentFailure>) {
                        // Update health monitor with exit status
        {
            let inner = self.inner.lock().unwrap();
            self.health.set_turn_exited(inner.proc_exit_code, inner.proc_exit_code == 0 && run_failure.is_none());
        }

        // Publish done status
        if let Some(ref broker) = self.broker {
            let status = {
                let inner = self.inner.lock().unwrap();
                SubprocessController::build_status_snapshot(&inner, &self.block_id, false)
            };
            publish_controller_status(broker, &status);
        }

        // Persist or clear agent:last_failure in block meta so the recovery
        // banner survives tab switches and page reloads (P1.1 of
        // SPEC_AGENT_ERROR_FRAMEWORK_2026_06_20). Done before the MPS
        // publish so the durable state is written first; the event is then
        // a low-latency push to any active subscriber.
        core::persist_last_failure(
            &self.block_id,
            run_failure,
            &self.mstore,
            &self.event_bus,
        );
        // Only a process that exited on its own says anything about the API; a stop leaves `clean_exit` unset.
        if exited_on_its_own {
            core::report_provider_turn(&self.block_id, run_failure, &self.mstore);
        }

        // Surface the classified failure cause to the pane (Phase 2 of
        // SPEC_AGENT_FAILURE_DIAGNOSTICS). persist:1 so reconnecting
        // subscribers also receive the last failure without needing a
        // separate meta read (belt-and-suspenders with the meta write above).
        if let (Some(failure), Some(broker)) = (run_failure, self.broker.as_ref()) {
            broker.publish_event::<crate::backend::mps_events::AgentFailed>(failure, vec![format!("block:{}", self.block_id)], 1);
        }
    }

    /// Stop the renewal task, release the lease (if any), then the run lock.
    fn release_turn(&self, turn_done: &AtomicBool, claimed_lease: &Option<crate::registry::Lease>) {
        // Signal the renewal task to stop BEFORE releasing the
        // lease/run_lock — a queued next turn (dequeued a few
        // lines below) may spawn immediately and claim its own
        // lease; this turn's renewal task must not still be
        // ticking when that happens (reagent P1 on #2359).
        turn_done.store(true, Ordering::SeqCst);

        // Release the lease (if any), then the run lock — mirrors
        // the ordering in the spawn-failure closure above.
        if let (Some(store), Some(lease)) = (&self.lease_store, &claimed_lease) {
            // Synchronous: the release must land before run_lock drops
            // below and a queued turn claims its own lease.
            if let Err(e) = crate::backend::blocking::off_async_worker(|| store.release(lease)) {
                tracing::warn!(
                    block_id = %self.block_id,
                    instance_id = %lease.instance_id(),
                    error = %e,
                    "lease release failed (self-heals via TTL expiry)"
                );
            }
        }
        self.run_lock.store(false, Ordering::SeqCst);
    }

    /// Run the next message queued while the turn was running.
    fn drain_next_queued(&self) {
        // Drain message queue: if messages were queued while this turn
        // was running, pop the next one and spawn it via the weak
        // self-reference.
        let next_config = {
            let mut inner = self.inner.lock().unwrap();
            inner.pending_messages.pop_front()
        };
        if let Some(config) = next_config {
            if let Some(ctrl) = self.self_ref.upgrade() {
                tracing::info!(
                    block_id = %self.block_id,
                    "draining queued message"
                );
                if let Err(e) = ctrl.spawn_turn(config) {
                    tracing::warn!(
                        block_id = %self.block_id,
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
                    if let Some(ref broker) = self.broker {
                        // Into the agent's global zone too: that is what
                        // the pane reads on reload (Phase 5a-3c).
                        let global_zone =
                            crate::backend::transcript::append::resolve_global_output_zone(&self.mstore, &self.block_id);
                        crate::backend::blockcontroller::shell::handle_append_block_file(
                            broker,
                            &self.block_id,
                            SUBPROCESS_OUTPUT_SUBJECT,
                            format!("{error_frame}\n").as_bytes(),
                            self.filestore.as_ref(),
                            global_zone.as_deref(),
                        );
                    }
                }
            }
        }
    }
}

/// The most recent frame of one kind a turn's stdout reader retains for its process waiter.
type FrameSlot = Arc<Mutex<Option<serde_json::Value>>>;

/// What a turn's process waiter takes over once the process is running.
struct TurnWait {
    child: tokio::process::Child,
    kill_rx: tokio::sync::oneshot::Receiver<bool>,
    stdout_reader_handle: tokio::task::JoinHandle<()>,
    stderr_reader_handle: tokio::task::JoinHandle<()>,
    turn_done: Arc<AtomicBool>,
    stderr_tail: Arc<Mutex<Vec<String>>>,
    last_result_frame: FrameSlot,
    last_inband_error: FrameSlot,
    claimed_lease: Option<crate::registry::Lease>,
    /// A Codex turn's `CODEX_HOME`: its context is read from its session file once it exits
    /// (`codex_context.rs`), since `codex exec` reports only the thread's totals.
    codex_home: Option<std::path::PathBuf>,
}

/// Keep the frames a failure is classified from: the terminal `result` frame, and an in-band API error
/// (a 401 arrives as a synthetic assistant message, or a Codex `error` / `turn.failed` frame, with exit 0).
/// A completed Codex turn clears the in-band error.
fn retain_failure_frame(parsed: serde_json::Value, last_result_frame: &FrameSlot, last_inband_error: &FrameSlot) {
    // Retain the terminal `result` frame for failure
    // classification.
    if parsed.get("type").and_then(|v| v.as_str()) == Some("result") {
        *last_result_frame.lock().unwrap() = Some(parsed);
    } else if parsed.get("type").and_then(|v| v.as_str()) == Some("assistant")
        && (parsed.get("isApiErrorMessage").and_then(|v| v.as_bool()).unwrap_or(false)
            || parsed.get("error").is_some())
    {
        // In-band API error: 401 / auth failures arrive as a synthetic
        // assistant message (exit 0, is_error:false on result frame) —
        // capture it so the process_waiter can trip the failure gate.
        *last_inband_error.lock().unwrap() = Some(parsed);
    } else if crate::agents::limit_reset::codex_failure_text(&parsed).is_some() {
        // Codex reports a failed turn (a usage limit
        // included) as `error` / `turn.failed` frames on
        // stdout, not on stderr.
        *last_inband_error.lock().unwrap() = Some(parsed);
    } else if parsed.get("type").and_then(|v| v.as_str()) == Some("turn.completed") {
        *last_inband_error.lock().unwrap() = None;
    }
}

/// The CLI's own session id is authoritative: it overwrites any earlier value (a stale hydrated id from a
/// picker reattach included) and is de-duplicated when the same id repeats across turns. See
/// `SubprocessController::record_captured_session_id_inner`.
fn capture_session_id(
    inner: &Arc<Mutex<super::SubprocessControllerInner>>,
    block_id: &str,
    session_id_field: &str,
    sid: &str,
    mstore: &Option<Arc<crate::backend::storage::store::Store>>,
    event_bus: &Option<Arc<crate::backend::eventbus::EventBus>>,
) {
    if SubprocessController::record_captured_session_id_inner(inner, sid) {
        tracing::info!(block_id = %block_id, field = %session_id_field, session_id = %sid, "captured session id");
        core::persist_session_id(block_id, sid, mstore, event_bus);
    }
}
