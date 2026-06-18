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
use futures_util::StreamExt as _;


use super::{
    BlockControllerRuntimeStatus, BlockInputUnion, Controller, STATUS_DONE, STATUS_INIT,
    STATUS_RUNNING,
};
use super::health::{classify_output_line, HealthMonitor};
use crate::backend::eventbus::EventBus;
use crate::backend::storage::filestore::FileStore;
use crate::backend::storage::store::Store;
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
    /// Optional client-supplied message id. Echoed back via the
    /// `agent-message-accepted` event when this config transitions from
    /// queued → running so the frontend can pair the event with a
    /// pending `PendingMessage`. None means no feedback is emitted.
    pub message_id: Option<String>,
    /// Session id to hydrate `inner.session_id` with BEFORE the first
    /// turn — used by the picker "My Agents" reattach path. The
    /// caller reads this from `block.meta["agent:sessionid"]` which
    /// the frontend pre-populates from the prior block's session id
    /// when launching with `continueOfInstanceId`. Without this
    /// hydration `spawn_turn` would only see the captured session id
    /// AFTER the first turn — meaning the first turn always launches
    /// the CLI fresh (no `--resume <sid>`) and starts a new
    /// conversation that re-injects the startup context.
    ///
    /// Empty / `None` means "no prior session" (greenfield launch).
    ///
    /// Best-effort, not authoritative: if the hydrated id is stale,
    /// the CLI's stdout-emitted session id always overwrites it at
    /// capture time (see `spawn_turn`'s stdout-reader block). The
    /// reattach turn passes the (possibly stale) hydrated id via
    /// `--resume`; the CLI either accepts it or starts a new
    /// session and emits its own id, which then becomes the in-
    /// memory authority for every subsequent turn.
    pub session_id: Option<String>,
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
    wstore: Option<Arc<Store>>,
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
        wstore: Option<Arc<Store>>,
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

    /// Emit `agent-message-accepted` for the given config, if it carries
    /// a `message_id`. Called from both `spawn_turn` (direct path) and
    /// the `process_waiter` drain site (queue path). No-op if the config
    /// has no id, or the broker isn't configured.
    fn emit_message_accepted(&self, config: &SubprocessSpawnConfig) {
        let Some(id) = config.message_id.as_deref() else { return };
        let Some(ref broker) = self.broker else { return };
        let event = super::super::wps::WaveEvent {
            event: super::super::wps::EVENT_AGENT_MESSAGE_ACCEPTED.to_string(),
            scopes: vec![format!("block:{}", self.block_id)],
            sender: String::new(),
            persist: 0,
            data: Some(serde_json::json!({
                "block_id": self.block_id,
                "message_id": id,
            })),
        };
        broker.publish(event);
        tracing::info!(
            block_id = %self.block_id,
            message_id = %id,
            "emitted agent-message-accepted"
        );
    }

    /// Get the stored session ID (if any).
    #[allow(dead_code)]
    pub fn session_id(&self) -> Option<String> {
        self.inner.lock().unwrap().session_id.clone()
    }

    /// Record an authoritative session id captured from the CLI's
    /// stdout init/`thread.started` event. The CLI is the source of
    /// truth for which session is live, so this ALWAYS overwrites
    /// any prior value of `inner.session_id` — including values
    /// previously hydrated from config on a picker reattach (which
    /// may be stale by the time the CLI speaks).
    ///
    /// Free-function form (taking `&Arc<Mutex<…Inner>>` instead of
    /// `&self`) so the spawn_turn stdout-reader tokio task can call
    /// it without holding an `Arc<Self>` reference. The
    /// `&SubprocessController` method below just delegates.
    ///
    /// Returns `true` when the value changed (caller should
    /// broadcast the meta update + persist to block meta). Returns
    /// `false` when the new id matches the current one — common
    /// when the CLI emits the same `session_id` on every NDJSON
    /// frame within a single turn.
    pub(crate) fn record_captured_session_id_inner(
        inner: &Mutex<SubprocessControllerInner>,
        sid: &str,
    ) -> bool {
        if sid.is_empty() {
            return false;
        }
        let mut guard = inner.lock().unwrap();
        let differs = guard.session_id.as_deref() != Some(sid);
        if differs {
            guard.session_id = Some(sid.to_string());
        }
        differs
    }

    /// `&self` convenience wrapper around
    /// `record_captured_session_id_inner` — used by tests that
    /// already hold a `SubprocessController`.
    #[cfg(test)]
    pub(crate) fn record_captured_session_id(&self, sid: &str) -> bool {
        Self::record_captured_session_id_inner(&self.inner, sid)
    }

    /// Hydrate `inner.session_id` from a config-supplied id when the
    /// controller hasn't seen a value yet.
    ///
    /// Picker reattach path: a fresh `SubprocessController` is
    /// registered for the new block, so its `inner.session_id` is
    /// `None`. The frontend persisted the prior block's session id
    /// into `agent:sessionid` meta, the websocket / app_api caller
    /// read it into `SubprocessSpawnConfig::session_id`, and this
    /// method copies it to inner so the spawn_turn args-builder
    /// appends `--resume <sid>` on the FIRST turn.
    ///
    /// **Hydration is best-effort, not authoritative.** If
    /// `inner.session_id` is already `Some` we no-op (don't overwrite
    /// a value already in place — could be a captured-from-stdout
    /// id from an earlier turn, or a prior hydration on the same
    /// reattach). Critically, the **CLI's stdout-emitted session id
    /// is authoritative** and overwrites any prior value at capture
    /// time (see the stdout-reader block in `spawn_turn`). So if the
    /// hydrated value is stale, the FIRST turn passes the stale id
    /// via `--resume` (likely accepted as a no-op or rejected with a
    /// "no such session" error from the CLI), the CLI then emits its
    /// own session id in the init event, and `inner.session_id` is
    /// overwritten with that authoritative value for subsequent
    /// turns. Without the capture overwrite, a stale hydrated id
    /// would be re-used forever — that was the bug codex flagged on
    /// PR #1018 first cut.
    ///
    /// Empty `&str` is treated as "no value" so the caller can use
    /// it unconditionally without filtering.
    pub(crate) fn hydrate_session_id_from_config(&self, config_sid: Option<&str>) {
        let Some(sid) = config_sid.filter(|s| !s.is_empty()) else {
            return;
        };
        let mut inner = self.inner.lock().unwrap();
        if inner.session_id.is_some() {
            return;
        }
        tracing::info!(
            block_id = %self.block_id,
            session_id = %sid,
            "hydrated session_id from config (picker reattach)"
        );
        inner.session_id = Some(sid.to_string());
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

        // Direct-spawn path (queue was empty): emit the accepted event
        // now so the frontend can promote its pending entry. The
        // drain-from-queue path (in process_waiter) emits the same
        // event just before calling spawn_turn recursively.
        self.emit_message_accepted(&config);

        // Hydrate inner.session_id from the config-supplied id if the
        // controller hasn't captured one yet. See
        // `hydrate_session_id_from_config` for the full rationale.
        self.hydrate_session_id_from_config(config.session_id.as_deref());

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
            super::shell::resolve_global_output_zone(&self.wstore, &self.block_id);
        // Retain the terminal `result` frame so a failure reported on STDOUT
        // (auth / rate-limit / usage — the common case; claude may even exit 0)
        // can be classified, not just stderr-reported ones. Shared with the
        // process_waiter below.
        let last_result_frame: Arc<Mutex<Option<serde_json::Value>>> = Arc::new(Mutex::new(None));
        let last_result_frame_read = Arc::clone(&last_result_frame);
        let stdout_reader_handle = tokio::spawn(async move {
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
        let stderr_tail_wait = Arc::clone(&stderr_tail);
        let last_result_frame_wait = Arc::clone(&last_result_frame);
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
                if exit_code != 0 || frame_is_error {
                    let tail = stderr_tail_wait.lock().unwrap().join("\n");
                    run_failure = Some(crate::agents::failure::classify(
                        Some(exit_code),
                        exit_signal,
                        &tail,
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

            // Surface the classified failure cause to the pane (Phase 2 of
            // SPEC_AGENT_FAILURE_DIAGNOSTICS). A dedicated `agentfailure` event so
            // the pane shows the real reason — auth, rate-limit, OOM, etc. — and
            // the stderr tail, instead of just an opaque exit code.
            if let (Some(failure), Some(broker)) = (run_failure.as_ref(), broker_wait.as_ref()) {
                broker.publish(wps::WaveEvent {
                    event: wps::EVENT_AGENT_FAILURE.to_string(),
                    scopes: vec![format!("block:{}", block_id_wait)],
                    sender: String::new(),
                    persist: 0,
                    data: serde_json::to_value(failure).ok(),
                });
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

        self.emit_message_accepted(&config);
        self.hydrate_session_id_from_config(config.session_id.as_deref());

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

        // Build final command: append --resume <sid> if we have a prior session.
        let mut cmd = base_cmd;
        {
            let inner = self.inner.lock().unwrap();
            if let Some(ref sid) = inner.session_id {
                if !config.resume_flag.is_empty() {
                    cmd.push(config.resume_flag.clone());
                    cmd.push(sid.clone());
                }
            }
        }

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
                            super::BlockControllerRuntimeStatus {
                                blockid: block_id.clone(),
                                version: inner.status_version,
                                shellprocstatus: inner.proc_status.clone(),
                                shellprocconnname: "local".to_string(),
                                shellprocexitcode: inner.proc_exit_code,
                                spawn_ts_ms: None,
                                is_agent_pane: false,
                            }
                        };
                        super::publish_controller_status(b, &status);
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
                    super::BlockControllerRuntimeStatus {
                        blockid: block_id.clone(),
                        version: inner.status_version,
                        shellprocstatus: inner.proc_status.clone(),
                        shellprocconnname: "local".to_string(),
                        shellprocexitcode: inner.proc_exit_code,
                        spawn_ts_ms: None,
                        is_agent_pane: false,
                    }
                };
                super::publish_controller_status(b, &status);
            }
            health_monitor.set_active_turn(true);

            // Health watchdog: drive check() every 5s while the turn is active,
            // mirroring spawn_turn. set_active_turn(true) alone never calls
            // check(), so without this a container turn gets no Stalled/Dead
            // detection. Self-terminates when the turn ends — completion calls
            // health_monitor.set_exited(), which clears the active-turn flag.
            {
                let health_watchdog = Arc::clone(&health_monitor);
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
            }

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
            let mut stats = super::session_stats::SessionStatsAccumulator::new(block_id.clone());
            let session_id_field = config.session_id_field.clone();
            // Tracks an aborted output stream (`Some(Err(_))`). The exec may have
            // exited cleanly with a non-zero code OR the attach stream itself
            // failed mid-turn; either way the turn did not complete normally, so
            // this forces a non-zero exit even if inspect_exec can't be reached.
            let mut stream_errored = false;

            // Resolve the agent's GLOBAL transcript zone once (see persistent.rs)
            // so every container-exec `output` line is also mirrored to the
            // cross-channel store. `None` for non-agent blocks.
            let global_output_zone =
                super::shell::resolve_global_output_zone(&wstore, &block_id);

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
                                    Self::publish_line(&line_buf, &block_id, &session_id_field, &inner_arc, &wstore, &event_bus, &broker, &filestore, &health_monitor, &mut stats, global_output_zone.as_deref());
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
                                            Self::publish_line(&line_buf, &block_id, &session_id_field, &inner_arc, &wstore, &event_bus, &broker, &filestore, &health_monitor, &mut stats, global_output_zone.as_deref());
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
                    super::BlockControllerRuntimeStatus {
                        blockid: block_id.clone(),
                        version: inner.status_version,
                        shellprocstatus: inner.proc_status.clone(),
                        shellprocconnname: "local".to_string(),
                        shellprocexitcode: inner.proc_exit_code,
                        spawn_ts_ms: None,
                        is_agent_pane: false,
                    }
                };
                super::publish_controller_status(b, &status);
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
                    }
                }
            }
        });

        Ok(())
    }

    /// Publish a single NDJSON line from container exec output: session-id capture,
    /// health classification, WPS blockfile event, and FileStore write-through.
    /// Used by `spawn_container_turn`'s output reader task.
    fn publish_line(
        line: &str,
        block_id: &str,
        session_id_field: &str,
        inner: &std::sync::Mutex<SubprocessControllerInner>,
        wstore: &Option<Arc<crate::backend::storage::store::Store>>,
        event_bus: &Option<Arc<crate::backend::eventbus::EventBus>>,
        broker: &Option<Arc<crate::backend::wps::Broker>>,
        filestore: &Option<Arc<crate::backend::storage::filestore::FileStore>>,
        health: &Arc<super::health::HealthMonitor>,
        stats: &mut super::session_stats::SessionStatsAccumulator,
        global_output_zone: Option<&str>,
    ) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return;
        }
        stats.record_line(trimmed.len(), wstore);

        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(trimmed) {
            let (meaningful, error) = super::health::classify_output_line(&parsed);
            health.record_output(meaningful);
            if let Some((class, msg)) = error {
                health.record_error(class, msg);
            }

            // Capture session_id from provider init event.
            if let Some(sid) = parsed.get(session_id_field).and_then(|v| v.as_str()) {
                let changed = SubprocessController::record_captured_session_id_inner(inner, sid);
                if changed {
                    tracing::info!(block_id = %block_id, session_id = %sid, "container exec: captured session id");
                    if let Some(ref store) = wstore {
                        let oref_str = format!("block:{}", block_id);
                        let mut meta_update = crate::backend::obj::MetaMapType::new();
                        meta_update.insert("agent:sessionid".to_string(), serde_json::Value::String(sid.to_string()));
                        if let Err(e) = crate::server::service::update_object_meta(store, &oref_str, &meta_update) {
                            tracing::warn!(block_id = %block_id, error = %e, "failed to persist agent:sessionid");
                        } else if let Some(ref bus) = event_bus {
                            if let Ok(updated_block) = store.must_get::<crate::backend::obj::Block>(block_id) {
                                let update_data = serde_json::to_value(
                                    &crate::backend::obj::WaveObjUpdate {
                                        updatetype: "update".into(),
                                        otype: "block".into(),
                                        oid: block_id.to_string(),
                                        obj: Some(crate::backend::obj::wave_obj_to_value(&updated_block)),
                                    },
                                ).ok();
                                bus.broadcast_event(
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

        if let Some(ref broker) = broker {
            let line_with_newline = format!("{}\n", trimmed);
            super::shell::handle_append_block_file(
                broker,
                block_id,
                SUBPROCESS_OUTPUT_SUBJECT,
                line_with_newline.as_bytes(),
                filestore.as_ref(),
                global_output_zone,
            );
        }
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

    fn send_input(&self, input: BlockInputUnion, _seq: Option<u64>) -> Result<(), String> {
        // SubprocessController doesn't accept raw PTY input — user messages
        // go through spawn_turn() (via AgentInputCommand RPC).
        //
        // Signals ARE accepted though: the agent-pane composer's Esc
        // handler sends SIGINT via `ControllerInputCommand({signame:"SIGINT"})`
        // when the user wants to cancel an in-flight turn. Route that to
        // `stop_subprocess(force=true)` so the current subprocess is
        // killed via `kill_tx`. Without this, Esc was silently rejected
        // and the agent kept running.
        if let Some(sig) = input.sig_name.as_deref() {
            if sig == "SIGINT" || sig == "SIGTERM" {
                tracing::info!(
                    block_id = %self.block_id,
                    sig = %sig,
                    "subprocess controller: received signal, killing current turn"
                );
                return self.stop_subprocess(true);
            }
            return Err(format!(
                "subprocess controller: unsupported signal {sig} (only SIGINT/SIGTERM)"
            ));
        }
        if input.input_data.is_some() {
            return Err("subprocess controller does not accept raw input; use AgentInputCommand".to_string());
        }
        // Term resize / other input types: accepted-no-op.
        Ok(())
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
        let result = ctrl.send_input(BlockInputUnion::data(b"hello".to_vec()), None);
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
            message_id: None,
            session_id: None,
        };

        let result = ctrl.spawn_turn(config);
        // spawn_turn now queues instead of rejecting when busy
        assert!(result.is_ok());

        // Verify the message was queued
        let inner = ctrl.inner.lock().unwrap();
        assert_eq!(inner.pending_messages.len(), 1);
        assert_eq!(inner.pending_messages[0].message, "test");
        drop(inner);

        // Release lock
        ctrl.run_lock.store(false, Ordering::SeqCst);
    }

    #[test]
    fn hydrate_session_id_populates_inner_when_none() {
        // Regression test for the 2026-05-24 "clicking My Agents
        // re-inserts the startup context" report. A fresh
        // SubprocessController is created for the reattached block;
        // its inner.session_id starts as None. The picker reattach
        // flow persists the prior block's session id into
        // `agent:sessionid` meta, the caller plumbs it into
        // `SubprocessSpawnConfig::session_id`, and spawn_turn calls
        // `hydrate_session_id_from_config` before building args.
        // After hydration, the existing args-builder appends
        // `--resume <sid>` on this very first turn — no
        // re-injected startup context.
        let ctrl = SubprocessController::new(
            "tab-1".to_string(),
            "block-reattach".to_string(),
            None,
            None,
            None,
            None,
        );
        assert!(ctrl.inner.lock().unwrap().session_id.is_none());

        ctrl.hydrate_session_id_from_config(Some("prior-sid-from-meta"));
        assert_eq!(
            ctrl.inner.lock().unwrap().session_id.as_deref(),
            Some("prior-sid-from-meta")
        );
    }

    #[test]
    fn hydrate_session_id_is_noop_when_value_already_present() {
        // Hydration is best-effort, not authoritative — it only
        // sets `inner.session_id` when None. The reason isn't
        // captured-id-wins (that's enforced at CAPTURE time below);
        // it's just to avoid re-hydrating on every spawn_turn call
        // within a controller lifetime. A stale value here is fine
        // because the next CLI emit at `record_captured_session_id_inner`
        // will overwrite.
        let ctrl = SubprocessController::new(
            "tab-1".to_string(),
            "block-resume".to_string(),
            None,
            None,
            None,
            None,
        );
        ctrl.inner.lock().unwrap().session_id = Some("captured-sid".to_string());

        ctrl.hydrate_session_id_from_config(Some("different-config-sid"));
        assert_eq!(
            ctrl.inner.lock().unwrap().session_id.as_deref(),
            Some("captured-sid"),
            "hydration must not overwrite an existing value"
        );
    }

    #[test]
    fn record_captured_overwrites_hydrated_value() {
        // The CLI is authoritative for session id once it speaks.
        // Codex P1 on PR #1018 first cut: my original
        // `if !already_captured` guard in the stdout reader meant
        // that a hydrated (possibly stale) session id would lock
        // out every subsequent CLI-emitted value, so a wrong
        // `--resume <stale>` would be passed forever. The fix
        // (`record_captured_session_id_inner`) always overwrites
        // and returns whether the value changed.
        let ctrl = SubprocessController::new(
            "tab-1".to_string(),
            "block-overwrite".to_string(),
            None,
            None,
            None,
            None,
        );
        ctrl.hydrate_session_id_from_config(Some("stale-hydrated-sid"));
        assert_eq!(
            ctrl.session_id().as_deref(),
            Some("stale-hydrated-sid")
        );

        let changed = ctrl.record_captured_session_id("authoritative-sid");
        assert!(changed, "value differs from hydrated; must report changed");
        assert_eq!(
            ctrl.session_id().as_deref(),
            Some("authoritative-sid"),
            "CLI-emitted id must overwrite hydrated value"
        );
    }

    #[test]
    fn record_captured_dedups_same_value() {
        // Real CLI streams emit `session_id` on every NDJSON frame,
        // not just the first. The dedup is a perf knob (skips the
        // meta-update broadcast on repeats), not a correctness
        // gate — captured-id is still authoritative on first emit.
        let ctrl = SubprocessController::new(
            "tab-1".to_string(),
            "block-dedup".to_string(),
            None,
            None,
            None,
            None,
        );
        assert!(ctrl.record_captured_session_id("sid-1"));
        assert!(!ctrl.record_captured_session_id("sid-1"),
            "second call with same value must return false (no broadcast)");
        assert_eq!(ctrl.session_id().as_deref(), Some("sid-1"));
    }

    #[test]
    fn record_captured_ignores_empty() {
        // Defensive: empty string from a malformed CLI emit must
        // not clear a valid prior value.
        let ctrl = SubprocessController::new(
            "tab-1".to_string(),
            "block-empty".to_string(),
            None,
            None,
            None,
            None,
        );
        ctrl.record_captured_session_id("real-sid");
        assert!(!ctrl.record_captured_session_id(""),
            "empty CLI emit must be ignored");
        assert_eq!(ctrl.session_id().as_deref(), Some("real-sid"));
    }

    #[test]
    fn hydrate_session_id_ignores_empty_and_none() {
        // Greenfield launches pass `None` (or `Some("")` if the
        // caller didn't filter) — hydration must be a no-op in
        // either case so inner.session_id stays None until the CLI
        // captures its own.
        let ctrl = SubprocessController::new(
            "tab-1".to_string(),
            "block-greenfield".to_string(),
            None,
            None,
            None,
            None,
        );
        ctrl.hydrate_session_id_from_config(None);
        assert!(ctrl.inner.lock().unwrap().session_id.is_none());

        ctrl.hydrate_session_id_from_config(Some(""));
        assert!(ctrl.inner.lock().unwrap().session_id.is_none());
    }

    #[test]
    fn spawn_turn_preserves_session_id_in_queued_config() {
        // When the controller is busy, spawn_turn queues the config
        // for the drain-from-queue path. The hydration ONLY runs on
        // the direct-spawn path (after try_lock_run), so the queued
        // config must carry session_id through unchanged for the
        // drain path's recursive call to see it.
        let ctrl = SubprocessController::new(
            "tab-1".to_string(),
            "block-queued".to_string(),
            None,
            None,
            None,
            None,
        );
        ctrl.run_lock.store(true, Ordering::SeqCst);

        let config = SubprocessSpawnConfig {
            cli_command: "claude".to_string(),
            cli_args: vec!["-p".to_string()],
            working_dir: String::new(),
            env_vars: HashMap::new(),
            message: "hi".to_string(),
            resume_flag: "--resume".to_string(),
            session_id_field: "session_id".to_string(),
            message_id: None,
            session_id: Some("prior-sid".to_string()),
        };
        let _ = ctrl.spawn_turn(config);

        let inner = ctrl.inner.lock().unwrap();
        assert_eq!(inner.pending_messages.len(), 1);
        assert_eq!(
            inner.pending_messages[0].session_id.as_deref(),
            Some("prior-sid"),
        );
        // Hydration didn't run yet — direct-spawn path was bypassed
        // by the busy lock; the drain will hydrate when it dequeues.
        assert!(inner.session_id.is_none());
    }
}
