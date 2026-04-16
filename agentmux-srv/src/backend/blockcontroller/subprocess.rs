// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! SubprocessController: manages agent CLI as stateless per-turn subprocess invocations.
//!
//! Architecture:
//!   Each user message spawns a fresh `claude -p` process.
//!   Multi-turn continuity uses `--resume <session-id>`.
//!   The process reads one JSON message from stdin, runs the agentic loop,
//!   streams NDJSON on stdout, then exits.
//!
//! State machine:
//!   INIT ─(spawn)─> RUNNING ─(process exits)─> DONE
//!   DONE ─(new message)─> RUNNING (re-spawn with --resume)
//!
//! I/O model (2 async tasks per turn):
//! 1. stdout_reader: piped stdout → .jsonl persistence + WPS blockfile events on "output" subject
//! 2. process_waiter: wait for exit, update status, publish lifecycle event


use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};


use super::{
    BlockControllerRuntimeStatus, BlockInputUnion, Controller, STATUS_DONE, STATUS_INIT,
    STATUS_RUNNING,
};
use super::health::{classify_output_line, HealthMonitor};
use crate::backend::eventbus::EventBus;
use crate::backend::storage::filestore::FileStore;
use crate::backend::storage::wstore::WaveStore;
use crate::backend::wps;

/// WPS file subject name for subprocess output (replaces "term" from PTY).
pub const SUBPROCESS_OUTPUT_SUBJECT: &str = "output";

/// Controller type constant.
pub const BLOCK_CONTROLLER_SUBPROCESS: &str = "subprocess";

/// Configuration for spawning a subprocess turn.
#[derive(Debug, Clone)]
pub struct SubprocessSpawnConfig {
    /// CLI executable (e.g., "claude").
    pub cli_command: String,
    /// CLI arguments (e.g., ["-p", "--output-format", "stream-json", ...]).
    pub cli_args: Vec<String>,
    /// Working directory for the subprocess.
    pub working_dir: String,
    /// Environment variables to set.
    pub env_vars: HashMap<String, String>,
    /// The user's JSON message to write to stdin.
    pub message: String,
    /// Flag used to resume a previous session, e.g. "--resume" (Claude), "-r" (Gemini).
    /// Empty string means this provider does not support simple-flag resume.
    pub resume_flag: String,
    /// JSON field name in the CLI's init event that contains the session/thread ID.
    /// e.g. "session_id" (Claude/Gemini) or "thread_id" (Codex).
    pub session_id_field: String,
}

/// Inner state protected by mutex.
struct SubprocessControllerInner {
    /// Current process status.
    proc_status: String,
    /// Process exit code from the most recent turn.
    proc_exit_code: i32,
    /// Status version counter (incremented on each change).
    status_version: i32,
    /// Session ID captured from the first `system/init` message.
    session_id: Option<String>,
    /// PID of the currently running subprocess (None if idle).
    current_pid: Option<u32>,
    /// Handle to kill the current subprocess.
    kill_tx: Option<tokio::sync::oneshot::Sender<bool>>,
    /// Messages queued while a turn is in progress.
    /// Drained sequentially after the current turn exits.
    pending_messages: VecDeque<SubprocessSpawnConfig>,
}

/// SubprocessController manages per-turn subprocess lifecycle for agent blocks.
///
/// Unlike `ShellController` which maintains a long-running PTY process,
/// `SubprocessController` spawns a fresh process for each user turn.
/// Multi-turn continuity comes from `--resume <session-id>`.
pub struct SubprocessController {
    /// Parent tab UUID.
    #[allow(dead_code)]
    tab_id: String,
    /// Block UUID.
    block_id: String,
    /// Prevents concurrent spawns.
    run_lock: Arc<AtomicBool>,
    /// Protected inner state.
    inner: Arc<Mutex<SubprocessControllerInner>>,
    /// WPS broker for publishing events (blockfile, controllerstatus).
    broker: Option<Arc<wps::Broker>>,
    /// Event bus for obj:update broadcasts.
    event_bus: Option<Arc<EventBus>>,
    /// Wave object store for block metadata persistence.
    wstore: Option<Arc<WaveStore>>,
    /// FileStore for write-through persistence of output lines (Phase 1.3).
    filestore: Option<Arc<FileStore>>,
    /// Agent health monitor (output activity + error tracking).
    health_monitor: Arc<HealthMonitor>,
    /// Weak self-reference for queue drain. Set by `set_self_ref` after
    /// the controller is wrapped in Arc.
    self_ref: Mutex<Option<std::sync::Weak<Self>>>,
}

impl SubprocessController {
    /// Create a new SubprocessController.
    pub fn new(
        tab_id: String,
        block_id: String,
        broker: Option<Arc<wps::Broker>>,
        event_bus: Option<Arc<EventBus>>,
        wstore: Option<Arc<WaveStore>>,
        filestore: Option<Arc<FileStore>>,
    ) -> Self {
        let health_monitor = Arc::new(HealthMonitor::new(
            block_id.clone(),
            broker.clone(),
        ));
        Self {
            tab_id,
            block_id,
            run_lock: Arc::new(AtomicBool::new(false)),
            inner: Arc::new(Mutex::new(SubprocessControllerInner {
                proc_status: STATUS_INIT.to_string(),
                proc_exit_code: 0,
                status_version: 0,
                session_id: None,
                current_pid: None,
                kill_tx: None,
                pending_messages: VecDeque::new(),
            })),
            broker,
            event_bus,
            wstore,
            filestore,
            health_monitor,
            self_ref: Mutex::new(None),
        }
    }

    /// Store a weak self-reference so the process_waiter can drain queued
    /// messages by calling spawn_turn after the current turn exits.
    /// Must be called after wrapping in Arc.
    pub fn set_self_ref(self: &Arc<Self>) {
        *self.self_ref.lock().unwrap() = Some(Arc::downgrade(self));
    }

    /// Try to acquire the run lock. Returns false if a turn is already in progress.
    fn try_lock_run(&self) -> bool {
        self.run_lock
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    /// Release the run lock.
    fn unlock_run(&self) {
        self.run_lock.store(false, Ordering::SeqCst);
    }

    /// Update process status and increment version (must hold inner lock).
    fn set_status(inner: &mut SubprocessControllerInner, status: &str) {
        inner.proc_status = status.to_string();
        inner.status_version += 1;
    }

    /// Get the runtime status (snapshot).
    fn get_status_snapshot(&self) -> BlockControllerRuntimeStatus {
        let inner = self.inner.lock().unwrap();
        BlockControllerRuntimeStatus {
            blockid: self.block_id.clone(),
            version: inner.status_version,
            shellprocstatus: inner.proc_status.clone(),
            shellprocconnname: "local".to_string(),
            shellprocexitcode: inner.proc_exit_code,
            spawn_ts_ms: None,
            is_agent_pane: false,
        }
    }

    /// Publish current controller status via the WPS broker.
    fn publish_status(&self) {
        if let Some(ref broker) = self.broker {
            let status = self.get_status_snapshot();
            super::publish_controller_status(broker, &status);
        }
    }

    /// Get the stored session ID (if any).
    #[allow(dead_code)]
    pub fn session_id(&self) -> Option<String> {
        self.inner.lock().unwrap().session_id.clone()
    }

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

        // Build CLI args, appending resume flag + session_id if we have one and the provider supports it
        let mut args = config.cli_args.clone();
        {
            let inner = self.inner.lock().unwrap();
            if let Some(ref sid) = inner.session_id {
                if !config.resume_flag.is_empty() {
                    args.push(config.resume_flag.clone());
                    args.push(sid.clone());
                }
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
        if !config.working_dir.is_empty() {
            // Expand ~ to home directory (cross-platform)
            let expanded_dir = if config.working_dir.starts_with("~/") || config.working_dir == "~" {
                if let Some(home) = dirs::home_dir() {
                    home.join(config.working_dir.trim_start_matches("~/")).to_string_lossy().to_string()
                } else {
                    config.working_dir.clone()
                }
            } else {
                config.working_dir.clone()
            };
            // Create directory if it doesn't exist
            let dir_path = std::path::Path::new(&expanded_dir);
            if !dir_path.exists() {
                if let Err(e) = std::fs::create_dir_all(dir_path) {
                    tracing::warn!(
                        block_id = %self.block_id,
                        dir = %expanded_dir,
                        error = %e,
                        "failed to create working directory, using current dir"
                    );
                } else {
                    tracing::info!(
                        block_id = %self.block_id,
                        dir = %expanded_dir,
                        "created working directory"
                    );
                }
            }
            if dir_path.exists() {
                cmd.current_dir(&expanded_dir);
            }
        }
        for (k, v) in &config.env_vars {
            let expanded = crate::backend::base::expand_home_dir_safe(v);
            cmd.env(k, expanded.to_string_lossy().as_ref());
        }
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        // Spawn
        let mut child = cmd.spawn().map_err(|e| {
            let mut inner = self.inner.lock().unwrap();
            Self::set_status(&mut inner, STATUS_DONE);
            inner.proc_exit_code = -1;
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

        // Store PID
        let (kill_tx, kill_rx) = tokio::sync::oneshot::channel::<bool>();
        {
            let mut inner = self.inner.lock().unwrap();
            inner.current_pid = Some(pid);
            inner.kill_tx = Some(kill_tx);
        }

        // Take ownership of stdin/stdout
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

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
        tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            let mut stats = super::session_stats::SessionStatsAccumulator::new(block_id_read.clone());

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

                        // Classify output for health monitoring
                        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(trimmed) {
                            let (meaningful, error) = classify_output_line(&parsed);
                            health_read.record_output(meaningful);
                            if let Some((class, msg)) = error {
                                health_read.record_error(class, msg);
                            }
                        }

                        // Try to capture session/thread ID from the provider's init event.
                        // Claude: {"type":"system","subtype":"init","session_id":"..."}
                        // Gemini: {"type":"init","session_id":"..."}
                        // Codex:  {"type":"thread.started","thread_id":"..."}
                        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(trimmed) {
                            if let Some(sid) = parsed.get(&session_id_field).and_then(|v| v.as_str()) {
                                let sid_string = sid.to_string();
                                // Only capture once (first occurrence in the output)
                                let already_captured = inner_read.lock().unwrap().session_id.is_some();
                                if !already_captured {
                                    tracing::info!(
                                        block_id = %block_id_read,
                                        field = %session_id_field,
                                        session_id = %sid_string,
                                        "captured session id"
                                    );
                                    {
                                        let mut inner = inner_read.lock().unwrap();
                                        inner.session_id = Some(sid_string.clone());
                                    }

                                    // Persist session_id to block metadata
                                    if let Some(ref store) = wstore_read {
                                        let oref_str = format!("block:{}", block_id_read);
                                        let mut meta_update =
                                            crate::backend::obj::MetaMapType::new();
                                        meta_update.insert(
                                            "agent:sessionid".to_string(),
                                            serde_json::Value::String(sid_string),
                                        );
                                        if let Err(e) = crate::server::service::update_object_meta(
                                            store, &oref_str, &meta_update,
                                        ) {
                                            tracing::warn!(
                                                block_id = %block_id_read,
                                                error = %e,
                                                "failed to persist agent:sessionid"
                                            );
                                        } else if let Some(ref event_bus) = event_bus_read {
                                            // Broadcast metadata update to frontend
                                            if let Ok(updated_block) = store.must_get::<crate::backend::obj::Block>(&block_id_read) {
                                                let update_data = serde_json::to_value(
                                                    &crate::backend::obj::WaveObjUpdate {
                                                        updatetype: "update".into(),
                                                        otype: "block".into(),
                                                        oid: block_id_read.clone(),
                                                        obj: Some(crate::backend::obj::wave_obj_to_value(&updated_block)),
                                                    },
                                                )
                                                .ok();
                                                event_bus.broadcast_event(
                                                    &crate::backend::eventbus::WSEventType {
                                                        eventtype: "waveobj:update".to_string(),
                                                        oref: oref_str,
                                                        data: update_data,
                                                    },
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // Publish the NDJSON line as a WPS blockfile event on the "output" subject
                        // and write-through to FileStore for persistent history (Phase 1.3).
                        if let Some(ref broker) = broker_read {
                            tracing::info!(block_id = %block_id_read, line = %trimmed, "subprocess stdout → blockfile");
                            // Include the newline so the frontend line splitter works correctly
                            let line_with_newline = format!("{}\n", trimmed);
                            super::shell::handle_append_block_file(
                                broker,
                                &block_id_read,
                                SUBPROCESS_OUTPUT_SUBJECT,
                                line_with_newline.as_bytes(),
                                filestore_read.as_ref(),
                            );
                        }
                    }
                }
            }

            tracing::info!(block_id = %block_id_read, "stdout_reader exiting");
        });

        // Spawn stderr reader (log warnings, don't publish)
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
                        }
                    }
                }
            }
        });

        // Spawn health watchdog (checks every 5s while turn is active)
        let health_watchdog = Arc::clone(&self.health_monitor);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
            loop {
                interval.tick().await;
                if !health_watchdog.is_active_turn() {
                    break;
                }
                health_watchdog.check();
            }
        });

        // Spawn process_waiter task
        let inner_wait = Arc::clone(&self.inner);
        let block_id_wait = self.block_id.clone();
        let broker_wait = self.broker.clone();
        let run_lock = Arc::clone(&self.run_lock);
        let health_wait = Arc::clone(&self.health_monitor);
        let self_ref_wait = self.self_ref.lock().unwrap().clone().unwrap_or_default();
        tokio::spawn(async move {
            // Wait for either process exit or kill signal
            tokio::select! {
                exit_result = child.wait() => {
                    let exit_code = match exit_result {
                        Ok(status) => status.code().unwrap_or(-1),
                        Err(e) => {
                            tracing::warn!(
                                block_id = %block_id_wait,
                                error = %e,
                                "subprocess wait error"
                            );
                            -1
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
                                super::DEFAULT_GRACEFUL_KILL_WAIT_MS,
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

            // Update health monitor with exit status
            {
                let inner = inner_wait.lock().unwrap();
                health_wait.set_exited(inner.proc_exit_code);
            }

            // Publish done status
            if let Some(ref broker) = broker_wait {
                let status = {
                    let inner = inner_wait.lock().unwrap();
                    BlockControllerRuntimeStatus {
                        blockid: block_id_wait.clone(),
                        version: inner.status_version,
                        shellprocstatus: inner.proc_status.clone(),
                        shellprocconnname: "local".to_string(),
                        shellprocexitcode: inner.proc_exit_code,
                        spawn_ts_ms: None,
                        is_agent_pane: false,
                    }
                };
                super::publish_controller_status(broker, &status);
            }

            // Release run lock
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
                    }
                }
            }
        });

        Ok(())
    }

    /// Stop the currently running subprocess.
    pub fn stop_subprocess(&self, force: bool) -> Result<(), String> {
        let kill_tx = {
            let mut inner = self.inner.lock().unwrap();
            inner.kill_tx.take()
        };
        match kill_tx {
            Some(tx) => {
                let _ = tx.send(force);
                Ok(())
            }
            None => Ok(()), // No running process
        }
    }
}

impl Controller for SubprocessController {
    fn start(
        &self,
        _block_meta: super::super::obj::MetaMapType,
        _rt_opts: Option<serde_json::Value>,
        _force: bool,
    ) -> Result<(), String> {
        // SubprocessController doesn't auto-start on resync.
        // Turns are initiated by SubprocessSpawnCommand / AgentInputCommand.
        tracing::info!(
            block_id = %self.block_id,
            "subprocess controller registered (no auto-start)"
        );
        Ok(())
    }

    fn stop(&self, _graceful: bool, new_status: &str) -> Result<(), String> {
        // Stop any running subprocess
        self.stop_subprocess(true)?;

        let mut inner = self.inner.lock().unwrap();
        if inner.proc_status != new_status {
            Self::set_status(&mut inner, new_status);
        }

        Ok(())
    }

    fn get_runtime_status(&self) -> BlockControllerRuntimeStatus {
        self.get_status_snapshot()
    }

    fn send_input(&self, _input: BlockInputUnion) -> Result<(), String> {
        // SubprocessController doesn't accept raw PTY input.
        // User messages go through spawn_turn() (via AgentInputCommand RPC).
        Err("subprocess controller does not accept raw input; use AgentInputCommand".to_string())
    }

    fn controller_type(&self) -> &str {
        BLOCK_CONTROLLER_SUBPROCESS
    }

    fn block_id(&self) -> &str {
        &self.block_id
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_subprocess_controller_new() {
        let ctrl = SubprocessController::new(
            "tab-1".to_string(),
            "block-1".to_string(),
            None,
            None,
            None,
            None,
        );
        assert_eq!(ctrl.controller_type(), BLOCK_CONTROLLER_SUBPROCESS);
        assert_eq!(ctrl.block_id(), "block-1");

        let status = ctrl.get_runtime_status();
        assert_eq!(status.shellprocstatus, STATUS_INIT);
        assert_eq!(status.blockid, "block-1");
    }

    #[test]
    fn test_subprocess_controller_rejects_raw_input() {
        let ctrl = SubprocessController::new(
            "tab-1".to_string(),
            "block-1".to_string(),
            None,
            None,
            None,
            None,
        );
        let result = ctrl.send_input(BlockInputUnion::data(b"hello".to_vec()));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("AgentInputCommand"));
    }

    #[test]
    fn test_subprocess_controller_start_is_noop() {
        let ctrl = SubprocessController::new(
            "tab-1".to_string(),
            "block-1".to_string(),
            None,
            None,
            None,
            None,
        );
        let result = ctrl.start(HashMap::new(), None, false);
        assert!(result.is_ok());

        // Still in init state — no auto-start
        let status = ctrl.get_runtime_status();
        assert_eq!(status.shellprocstatus, STATUS_INIT);
    }

    #[test]
    fn test_subprocess_controller_stop_when_idle() {
        let ctrl = SubprocessController::new(
            "tab-1".to_string(),
            "block-1".to_string(),
            None,
            None,
            None,
            None,
        );
        let result = ctrl.stop(true, STATUS_DONE);
        assert!(result.is_ok());

        let status = ctrl.get_runtime_status();
        assert_eq!(status.shellprocstatus, STATUS_DONE);
    }

    #[test]
    fn test_subprocess_controller_session_id_initially_none() {
        let ctrl = SubprocessController::new(
            "tab-1".to_string(),
            "block-1".to_string(),
            None,
            None,
            None,
            None,
        );
        assert!(ctrl.session_id().is_none());
    }

    #[test]
    fn test_subprocess_controller_concurrent_spawn_blocked() {
        let ctrl = SubprocessController::new(
            "tab-1".to_string(),
            "block-1".to_string(),
            None,
            None,
            None,
            None,
        );

        // Manually acquire run lock
        ctrl.run_lock.store(true, Ordering::SeqCst);

        let config = SubprocessSpawnConfig {
            cli_command: "echo".to_string(),
            cli_args: vec![],
            working_dir: String::new(),
            env_vars: HashMap::new(),
            message: "test".to_string(),
            resume_flag: String::new(),
            session_id_field: "session_id".to_string(),
        };

        let result = ctrl.spawn_turn(config);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("already running"));

        // Release lock
        ctrl.run_lock.store(false, Ordering::SeqCst);
    }
}
