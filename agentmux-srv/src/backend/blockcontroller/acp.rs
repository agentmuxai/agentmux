// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! AcpController: manages agent CLIs that speak the Agent Client Protocol (ACP).
//!
//! ACP is a JSON-RPC 2.0 protocol over stdin/stdout — the "LSP for AI agents."
//! Instead of custom per-provider output parsing, ACP provides a standardized
//! protocol for session management, prompting, and streaming events.
//!
//! Lifecycle:
//!   1. Spawn the ACP agent process (e.g., `gemini --acp`, `openclaw acp`)
//!   2. Send `initialize` request, receive capabilities
//!   3. Send `initialized` notification
//!   4. Create a session via `session/create`
//!   5. For each user turn: send `session/prompt`, stream `session/update` notifications
//!   6. On close: send `shutdown` + `exit`
//!
//! I/O model (similar to PersistentSubprocessController):
//!   - stdin_writer: sends JSON-RPC requests/notifications to agent
//!   - stdout_reader: reads JSON-RPC responses/notifications, persists + broadcasts
//!   - process_waiter: monitors process lifecycle
//!
//! See: https://github.com/agentclientprotocol/agent-client-protocol

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use super::{
    BlockControllerRuntimeStatus, BlockInputUnion, Controller, STATUS_DONE, STATUS_INIT,
    STATUS_RUNNING,
};
use super::core;
use super::health::HealthMonitor;
use crate::backend::eventbus::EventBus;
use crate::backend::storage::filestore::FileStore;
use crate::backend::storage::store::Store;
use crate::backend::wps;

/// WPS file subject name for ACP output.
pub const ACP_OUTPUT_SUBJECT: &str = "output";

pub const BLOCK_CONTROLLER_ACP: &str = "acp";

/// A `session/prompt` response only signals genuine turn-end if it (a) has
/// a `stopReason` AND (b) its echoed `id` matches the id of the most
/// recently SENT prompt. Extracted as a pure function so the staleness
/// logic itself is directly unit-testable without spawning a real process
/// — see `latest_prompt_id`'s doc comment for why (b) is necessary: the
/// stdin-writer and stdout-reader tasks run independently with no ordering
/// guarantee, so an EARLIER prompt's response can arrive after a
/// steering/new prompt was already sent. codex P2 on PR #2338
/// (twenty-seventh re-review).
fn is_latest_prompt_completion(response_id: Option<u64>, has_stop_reason: bool, latest_prompt_id: u64) -> bool {
    has_stop_reason && response_id == Some(latest_prompt_id)
}

/// Roll `latest` back to `previous_id`, but ONLY if `latest` still equals
/// `rejected_id` — i.e. nothing newer has been sent in the meantime.
/// Shared by both places a prompt can be discovered to have never
/// actually completed: `send_input`'s synchronous enqueue-failure
/// rollback, and the stdout-reader's async JSON-RPC error-response
/// handling. compare_exchange (not an unconditional store) so a
/// legitimately newer prompt sent since is never clobbered by restoring a
/// stale "previous" value. Returns whether the rollback was applied.
/// codex P2 on PR #2338 (twenty-ninth re-review).
fn rollback_latest_prompt_id_if_unchanged(latest: &AtomicU64, rejected_id: u64, previous_id: u64) -> bool {
    latest.compare_exchange(rejected_id, previous_id, Ordering::Relaxed, Ordering::Relaxed).is_ok()
}

/// Inner state protected by mutex.
struct AcpInner {
    proc_status: String,
    proc_exit_code: i32,
    status_version: i32,
    session_id: Option<String>,
    current_pid: Option<u32>,
    stdin_tx: Option<mpsc::Sender<String>>,
    kill_tx: Option<tokio::sync::oneshot::Sender<bool>>,
    /// First user prompt, deferred until session/create completes.
    pending_prompt: Option<String>,
}

/// AcpController manages an ACP-speaking agent process.
pub struct AcpController {
    #[allow(dead_code)]
    tab_id: String,
    block_id: String,
    inner: Arc<Mutex<AcpInner>>,
    broker: Option<Arc<wps::Broker>>,
    event_bus: Option<Arc<EventBus>>,
    wstore: Option<Arc<Store>>,
    filestore: Option<Arc<FileStore>>,
    health_monitor: Arc<HealthMonitor>,
    /// Monotonically increasing JSON-RPC request ID.
    next_rpc_id: Arc<AtomicU64>,
    /// The `id` of the most recently SENT `session/prompt` request. 0 is a
    /// safe "none sent yet" sentinel — `next_rpc_id` starts at 1, so no real
    /// request ever gets id 0. Used to detect a stale, out-of-order
    /// `stopReason` completion: when a steering/new prompt is sent while an
    /// EARLIER prompt is still in flight, the earlier prompt's response can
    /// arrive AFTER the new one is already sent (no ordering guarantee
    /// between a separate stdin-writer task and a separate stdout-reader
    /// task) — without this, that stale completion's hardcoded
    /// `turn_active: false` would overwrite the genuinely-active NEW turn's
    /// `true`, which flushPendingControllerRefresh (useAgentCommands.ts)
    /// could then read as backend-confirmed idle and force-restart the
    /// controller mid-turn. codex P2 on PR #2338 (twenty-seventh re-review).
    latest_prompt_id: Arc<AtomicU64>,
    /// The `id` latest_prompt_id held immediately BEFORE its most recent
    /// update — i.e. the id of the prompt that was in flight right before
    /// the current "latest" one was sent. Lets a steering send's own
    /// rejection roll `latest_prompt_id` back to the still-genuinely-in-
    /// flight EARLIER prompt (mirrors send_input's synchronous
    /// enqueue-failure rollback, but for an ASYNC agent-level rejection
    /// arriving later in the stdout-reader task — see the JSON-RPC `error`
    /// handling below). One level of history only, not a full stack of
    /// every outstanding prompt: a second, deeper rejection while this
    /// rollback is already pending isn't corrected — codex P2 on PR #2338
    /// (twenty-ninth re-review) offered this as the proportionate fix
    /// (vs. tracking every outstanding prompt) for what it characterized
    /// as an agent-dependent edge case (mid-turn prompt rejection).
    previous_prompt_id: Arc<AtomicU64>,
}

impl AcpController {
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
            wstore.clone(),
            event_bus.clone(),
        ));
        Self {
            tab_id,
            block_id,
            inner: Arc::new(Mutex::new(AcpInner {
                proc_status: STATUS_INIT.to_string(),
                proc_exit_code: 0,
                status_version: 0,
                session_id: None,
                current_pid: None,
                stdin_tx: None,
                kill_tx: None,
                pending_prompt: None,
            })),
            broker,
            event_bus,
            wstore,
            filestore,
            health_monitor,
            next_rpc_id: Arc::new(AtomicU64::new(1)),
            latest_prompt_id: Arc::new(AtomicU64::new(0)),
            previous_prompt_id: Arc::new(AtomicU64::new(0)),
        }
    }

    fn next_id(&self) -> u64 {
        self.next_rpc_id.fetch_add(1, Ordering::Relaxed)
    }

    fn set_status(inner: &mut AcpInner, status: &str) {
        inner.proc_status = status.to_string();
        inner.status_version += 1;
    }

    fn get_status_snapshot(&self) -> BlockControllerRuntimeStatus {
        let inner = self.inner.lock().unwrap();
        BlockControllerRuntimeStatus {
            blockid: self.block_id.clone(),
            version: inner.status_version,
            shellprocstatus: inner.proc_status.clone(),
            shellprocconnname: "local".to_string(),
            shellprocexitcode: inner.proc_exit_code,
            spawn_ts_ms: None,
            is_agent_pane: true,
            turn_active: self.health_monitor.is_active_turn(),
        }
    }

    fn publish_status(&self) {
        if let Some(ref broker) = self.broker {
            let status = self.get_status_snapshot();
            super::publish_controller_status(broker, &status);
        }
    }

    fn is_running(&self) -> bool {
        let inner = self.inner.lock().unwrap();
        inner.stdin_tx.is_some()
    }

    /// Build a JSON-RPC 2.0 request.
    fn make_request(&self, method: &str, params: serde_json::Value) -> String {
        let id = self.next_id();
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }).to_string()
    }

    /// Build a JSON-RPC 2.0 notification (no id field).
    fn make_notification(&self, method: &str, params: serde_json::Value) -> String {
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        }).to_string()
    }

    /// Spawn the ACP agent process and perform the initialize handshake.
    fn spawn_process(&self, cli_command: String, cli_args: Vec<String>, working_dir: String, env_vars: HashMap<String, String>) -> Result<(), String> {
        let mut cmd = crate::server::cli_handlers::make_cli_cmd(&cli_command);
        cmd.args(&cli_args);

        core::apply_working_dir(&mut cmd, &self.block_id, &working_dir, &env_vars);
        // On Windows: suppress console-window allocation. Without CREATE_NO_WINDOW,
        // node.exe spawned from a windowless sidecar may try to create/attach to a
        // console, causing stdout to go to that console rather than the pipe.
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            tracing::error!(block_id = %self.block_id, error = %e, "ACP process spawn failed");
            format!("failed to spawn ACP process: {e}")
        })?;

        let pid = child.id().unwrap_or(0);

        tracing::info!(
            block_id = %self.block_id,
            pid = pid,
            cmd = %cli_command,
            args = ?cli_args,
            "ACP agent process spawned"
        );

        let (kill_tx, kill_rx) = tokio::sync::oneshot::channel::<bool>();
        let stdin = child.stdin.take()
            .ok_or_else(|| format!("[acp] stdin not captured for block {}", self.block_id))?;
        let stdout = child.stdout.take()
            .ok_or_else(|| format!("[acp] stdout not captured for block {}", self.block_id))?;
        let stderr = child.stderr.take();

        // Drain stderr with explicit error/EOF logging
        if let Some(stderr_pipe) = stderr {
            let block_id_stderr = self.block_id.clone();
            tokio::spawn(async move {
                let mut reader = BufReader::new(stderr_pipe).lines();
                loop {
                    match reader.next_line().await {
                        Err(e) => {
                            tracing::warn!(block_id = %block_id_stderr, error = %e, "ACP stderr read error");
                            break;
                        }
                        Ok(None) => break,
                        Ok(Some(line)) => {
                            if !line.trim().is_empty() {
                                tracing::warn!(
                                    block_id = %block_id_stderr,
                                    line = %line,
                                    "ACP agent stderr"
                                );
                            }
                        }
                    }
                }
            });
        }

        // Stdin writer channel
        let (msg_tx, mut msg_rx) = mpsc::channel::<String>(32);

        {
            let mut inner = self.inner.lock().unwrap();
            inner.current_pid = Some(pid);
            inner.kill_tx = Some(kill_tx);
            inner.stdin_tx = Some(msg_tx.clone());
            Self::set_status(&mut inner, STATUS_RUNNING);
        }
        self.publish_status();

        // Spawn stdin writer task
        let block_id_stdin = self.block_id.clone();
        tokio::spawn(async move {
            let mut stdin = tokio::io::BufWriter::new(stdin);
            while let Some(line) = msg_rx.recv().await {
                if let Err(e) = stdin.write_all(line.as_bytes()).await {
                    tracing::error!(block_id = %block_id_stdin, error = %e, "ACP stdin write error");
                    break;
                }
                if let Err(e) = stdin.write_all(b"\n").await {
                    tracing::error!(block_id = %block_id_stdin, error = %e, "ACP stdin newline error");
                    break;
                }
                if let Err(e) = stdin.flush().await {
                    tracing::error!(block_id = %block_id_stdin, error = %e, "ACP stdin flush error");
                    break;
                }
            }
        });

        // Spawn stdout reader task — reads NDJSON lines and broadcasts via WPS
        let block_id_stdout = self.block_id.clone();
        let broker_clone = self.broker.clone();
        let filestore_clone = self.filestore.clone();
        let inner_clone = self.inner.clone();
        let health_clone = self.health_monitor.clone();
        let rpc_id_clone = self.next_rpc_id.clone();
        let latest_prompt_id_clone = self.latest_prompt_id.clone();
        let previous_prompt_id_clone = self.previous_prompt_id.clone();
        let wstore_clone = self.wstore.clone();
        let event_bus_clone = self.event_bus.clone();
        // Resolve the agent's GLOBAL transcript zone once (see persistent.rs).
        let global_output_zone =
            super::shell::resolve_global_output_zone(&self.wstore, &self.block_id);
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            tracing::info!(block_id = %block_id_stdout, "ACP stdout_reader started");

            loop {
                let line = match reader.next_line().await {
                    Err(e) => {
                        tracing::warn!(block_id = %block_id_stdout, error = %e, "ACP stdout read error");
                        break;
                    }
                    Ok(None) => {
                        tracing::info!(block_id = %block_id_stdout, "ACP stdout EOF");
                        break;
                    }
                    Ok(Some(l)) => l,
                };
                if line.is_empty() {
                    continue;
                }
                // reagent P1 (PR #2336): without this, `last_meaningful_ts`
                // never advances for ACP, so a genuinely wedged process
                // would silently never reach Dead via the silence branch
                // either (paired with the watchdog-spawn fix above — both
                // were needed for Restart-on-unresponsive to actually work
                // for this controller type). Any non-empty line counts as
                // meaningful — ACP doesn't have persistent.rs's finer-grained
                // classify_output_line integration; that's a reasonable
                // follow-up, not required to close this gap.
                health_clone.record_output(true);

                // Parse as JSON to check for session/update notifications
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&line) {
                    // Extract session ID from initialize result or session/create result
                    if let Some(result) = json.get("result") {
                        if let Some(sid) = result.get("sessionId").and_then(|v| v.as_str()) {
                            let sid_owned = sid.to_string();
                            {
                                let mut inner = inner_clone.lock().unwrap();
                                inner.session_id = Some(sid_owned.clone());
                                // Flush pending prompt now that session is ready.
                                if let Some(prompt) = inner.pending_prompt.take() {
                                    let id = rpc_id_clone.fetch_add(1, Ordering::Relaxed);
                                    latest_prompt_id_clone.store(id, Ordering::Relaxed);
                                    let req = serde_json::json!({
                                        "jsonrpc": "2.0",
                                        "id": id,
                                        "method": "session/prompt",
                                        "params": {
                                            "sessionId": sid,
                                            "prompt": { "type": "text", "text": prompt },
                                        }
                                    }).to_string();
                                    if let Some(ref tx) = inner.stdin_tx {
                                        if tx.try_send(req).is_err() {
                                            tracing::warn!(block_id = %block_id_stdout, "[acp] session/prompt send failed — channel full or closed");
                                        }
                                    }
                                }
                            }
                            tracing::info!(
                                block_id = %block_id_stdout,
                                session_id = %sid_owned,
                                "ACP session established"
                            );
                            // Persist to block metadata and broadcast so the frontend's
                            // "My Agents" reattach path can read agent:sessionid from
                            // block.meta. ACP previously captured the ID in memory only —
                            // this mirrors the careful path from persistent.rs / subprocess.rs.
                            core::persist_session_id(&block_id_stdout, &sid_owned, &wstore_clone, &event_bus_clone);
                        }
                    }

                    // Reset health monitor on prompt result (turn complete)
                    if json.get("id").is_some() && json.get("result").is_some() {
                        // This is a response to a request (e.g., session/prompt result)
                        if let Some(result) = json.get("result") {
                            // Only trust this completion if it's for the
                            // MOST RECENTLY SENT prompt — the stdin-writer
                            // task and this stdout-reader task run
                            // independently with no ordering guarantee
                            // between them, so a steering/new prompt sent
                            // while an EARLIER one is still in flight can
                            // have that earlier prompt's stopReason arrive
                            // AFTER the new one was already sent and marked
                            // active. Without this check, the stale
                            // completion's hardcoded turn_active: false
                            // below would overwrite the genuinely-active
                            // NEW turn's true. codex P2 on PR #2338
                            // (twenty-seventh re-review).
                            let is_latest_prompt = is_latest_prompt_completion(
                                json.get("id").and_then(|v| v.as_u64()),
                                result.get("stopReason").is_some(),
                                latest_prompt_id_clone.load(Ordering::Relaxed),
                            );
                            if is_latest_prompt {
                                health_clone.set_active_turn(false);
                                // Publish the flip so live controllerstatus
                                // subscribers see "turn ended" immediately,
                                // mirroring persistent.rs's matching publish
                                // on its own normal (non-kill, non-exit)
                                // turn-end path. Without this, the ONLY
                                // controllerstatus publishes for an ACP
                                // controller (Gemini/Codex/Kimi in
                                // catalog.ts) are on kill or process exit —
                                // a normal turn-end here left the frontend's
                                // `wasTurnActive` signal (fed only by live
                                // controllerstatus events, never local
                                // state) stuck at its last-seen value,
                                // stranding useAgentCommands.ts's
                                // flushPendingControllerRefresh (which
                                // requires isBackendTurnConfirmedIdle —
                                // wasTurnActive === false — before running a
                                // /login-deferred controller restart) until
                                // an unrelated event happened to republish
                                // status. reagentx P1 on PR #2338
                                // (twenty-second re-review).
                                if let Some(ref broker) = broker_clone {
                                    let status = {
                                        let locked = inner_clone.lock().unwrap();
                                        BlockControllerRuntimeStatus {
                                            blockid: block_id_stdout.clone(),
                                            version: locked.status_version,
                                            shellprocstatus: locked.proc_status.clone(),
                                            shellprocconnname: "local".to_string(),
                                            shellprocexitcode: locked.proc_exit_code,
                                            spawn_ts_ms: None,
                                            is_agent_pane: true,
                                            turn_active: false,
                                        }
                                    };
                                    super::publish_controller_status(broker, &status);
                                }
                            }
                        }
                    } else if json.get("id").is_some() && json.get("error").is_some() {
                        // A JSON-RPC error response for the LATEST prompt —
                        // mid-turn prompt acceptance is agent-dependent;
                        // some agents reject a steering prompt sent while
                        // an earlier one is still being processed. An
                        // error response has neither `result` nor
                        // `stopReason`, so it can never satisfy
                        // is_latest_prompt_completion above — without this,
                        // the rejected prompt's id would permanently
                        // occupy latest_prompt_id, and the EARLIER prompt's
                        // real eventual stopReason would be misclassified
                        // as stale (its id no longer matches), leaving
                        // turn_active stuck true for this pane. codex P2 on
                        // PR #2338 (twenty-ninth re-review).
                        if let Some(error_response_id) = json.get("id").and_then(|v| v.as_u64()) {
                            let _ = rollback_latest_prompt_id_if_unchanged(
                                &latest_prompt_id_clone,
                                error_response_id,
                                previous_prompt_id_clone.load(Ordering::Relaxed),
                            );
                        }
                    }
                }

                // Persist + broadcast via the shared helper (same as subprocess.rs)
                if let Some(ref broker) = broker_clone {
                    let line_with_newline = format!("{}\n", line);
                    super::shell::handle_append_block_file(
                        broker,
                        &block_id_stdout,
                        ACP_OUTPUT_SUBJECT,
                        line_with_newline.as_bytes(),
                        filestore_clone.as_ref(),
                        global_output_zone.as_deref(),
                    );
                }
            }
        });

        // Spawn process waiter task
        let block_id_wait = self.block_id.clone();
        let inner_wait = self.inner.clone();
        let broker_wait = self.broker.clone();
        let health_wait = self.health_monitor.clone();
        tokio::spawn(async move {
            tokio::select! {
                _ = kill_rx => {
                    let _ = child.kill().await;
                    tracing::info!(block_id = %block_id_wait, "ACP process killed");

                    let mut inner = inner_wait.lock().unwrap();
                    inner.stdin_tx = None;
                    inner.current_pid = None;
                    AcpController::set_status(&mut inner, STATUS_DONE);
                    drop(inner);

                    health_wait.set_active_turn(false);

                    if let Some(ref broker) = broker_wait {
                        let status = BlockControllerRuntimeStatus {
                            blockid: block_id_wait.clone(),
                            version: 0,
                            shellprocstatus: STATUS_DONE.to_string(),
                            shellprocconnname: "local".to_string(),
                            shellprocexitcode: -1,
                            spawn_ts_ms: None,
                            is_agent_pane: true,
                            turn_active: false,
                        };
                        super::publish_controller_status(broker, &status);
                    }
                }
                status = child.wait() => {
                    let exit_code = status.map(|s| s.code().unwrap_or(-1)).unwrap_or(-1);
                    tracing::info!(
                        block_id = %block_id_wait,
                        exit_code = exit_code,
                        "ACP process exited"
                    );
                    let mut inner = inner_wait.lock().unwrap();
                    inner.proc_exit_code = exit_code;
                    inner.stdin_tx = None;
                    AcpController::set_status(&mut inner, STATUS_DONE);
                    drop(inner);

                    health_wait.set_active_turn(false);

                    if let Some(ref broker) = broker_wait {
                        let status = BlockControllerRuntimeStatus {
                            blockid: block_id_wait.clone(),
                            version: 0,
                            shellprocstatus: STATUS_DONE.to_string(),
                            shellprocconnname: "local".to_string(),
                            shellprocexitcode: exit_code,
                            spawn_ts_ms: None,
                            is_agent_pane: true,
                            turn_active: false,
                        };
                        super::publish_controller_status(broker, &status);
                    }
                }
            }
        });

        // Send ACP initialize handshake
        let init_req = self.make_request("initialize", serde_json::json!({
            "clientInfo": {
                "name": "AgentMux",
                "version": env!("CARGO_PKG_VERSION"),
            },
            "capabilities": {
                "tools": true,
                "fileAccess": true,
            },
            "workspaceRoots": [working_dir],
        }));
        let init_notification = self.make_notification("initialized", serde_json::json!({}));
        let session_req = self.make_request("session/create", serde_json::json!({
            "cwd": working_dir,
        }));

        // Queue the handshake messages
        let inner = self.inner.lock().unwrap();
        if let Some(ref tx) = inner.stdin_tx {
            for (label, msg) in [("initialize", init_req), ("initialized", init_notification), ("session/create", session_req)] {
                if tx.try_send(msg).is_err() {
                    tracing::error!(block_id = %self.block_id, method = label, "[acp] handshake message dropped — channel full or closed; agent will not start");
                }
            }
        }

        Ok(())
    }
}

impl Controller for AcpController {
    fn start(
        &self,
        block_meta: super::super::obj::MetaMapType,
        _rt_opts: Option<serde_json::Value>,
        _force: bool,
    ) -> Result<(), String> {
        // Extract spawn config from block metadata
        let cmd = super::super::obj::meta_get_string(&block_meta, super::META_KEY_CMD, "");
        let cwd = super::super::obj::meta_get_string(&block_meta, super::META_KEY_CMD_CWD, "");
        let args_str = super::super::obj::meta_get_string(&block_meta, super::META_KEY_CMD_ARGS, "[]");
        let env_str = super::super::obj::meta_get_string(&block_meta, super::META_KEY_CMD_ENV, "{}");

        if cmd.is_empty() {
            return Err("ACP controller: no cmd specified in block meta".to_string());
        }

        let args: Vec<String> = serde_json::from_str(&args_str).unwrap_or_default();
        let env_vars: HashMap<String, String> = serde_json::from_str(&env_str).unwrap_or_default();

        self.spawn_process(cmd, args, cwd, env_vars)
    }

    fn stop(&self, _graceful: bool, _new_status: &str) -> Result<(), String> {
        // Send shutdown request before killing
        {
            let inner = self.inner.lock().unwrap();
            if let Some(ref tx) = inner.stdin_tx {
                let shutdown = self.make_request("shutdown", serde_json::json!({}));
                let exit = self.make_notification("exit", serde_json::json!({}));
                for (label, msg) in [("shutdown", shutdown), ("exit", exit)] {
                    if tx.try_send(msg).is_err() {
                        tracing::debug!(block_id = %self.block_id, method = label, "[acp] shutdown message dropped — process likely already gone");
                    }
                }
            }
        }

        // Kill the process
        let kill_tx = {
            let mut inner = self.inner.lock().unwrap();
            inner.stdin_tx = None;
            inner.kill_tx.take()
        };
        if let Some(tx) = kill_tx {
            let _ = tx.send(true);
        }

        {
            let mut inner = self.inner.lock().unwrap();
            Self::set_status(&mut inner, STATUS_DONE);
        }
        self.publish_status();
        Ok(())
    }

    fn get_runtime_status(&self) -> BlockControllerRuntimeStatus {
        self.get_status_snapshot()
    }

    fn send_input(&self, input: BlockInputUnion, _seq: Option<u64>) -> Result<(), String> {
        if let Some(data) = input.input_data {
            // Raw input from the frontend — treat as a user message.
            // The frontend sends the user prompt as UTF-8 bytes.
            let message = String::from_utf8_lossy(&data).to_string();
            if message.trim().is_empty() {
                return Ok(());
            }

            if !self.is_running() {
                // Process not running — stash as pending so start() picks it up.
                let mut inner = self.inner.lock().unwrap();
                inner.pending_prompt = Some(message);
                return Err("ACP process not running — message queued for next start()".to_string());
            }

            let session_id = {
                let inner = self.inner.lock().unwrap();
                inner.session_id.clone().unwrap_or_default()
            };
            // Not built via make_request: the id must be captured so it can
            // be recorded as the latest prompt (see latest_prompt_id's doc
            // comment) — make_request only returns the serialized string.
            let prompt_id = self.next_id();
            // swap (not store) so the PREVIOUS value is available to
            // restore if this send's own enqueue fails below — see that
            // rollback's comment for why. Also persisted into the shared
            // previous_prompt_id field so the stdout-reader task can
            // perform the SAME rollback later, asynchronously, if the
            // agent itself rejects this prompt with a JSON-RPC error
            // instead of a channel-level enqueue failure — see that
            // handling's own comment.
            let previous_prompt_id = self.latest_prompt_id.swap(prompt_id, Ordering::Relaxed);
            self.previous_prompt_id.store(previous_prompt_id, Ordering::Relaxed);
            let req = serde_json::json!({
                "jsonrpc": "2.0",
                "id": prompt_id,
                "method": "session/prompt",
                "params": {
                    "sessionId": session_id,
                    "prompt": {
                        "type": "text",
                        "text": message,
                    }
                }
            }).to_string();
            // reagent P1 (PR #2336): this controller wired wstore/event_bus
            // into HealthMonitor for the Unresponsive-failure/Restart
            // feature but never spawned the periodic watchdog — Dead (the
            // silence-based branch) can only ever be detected while
            // `spawn_health_watchdog`'s 5s loop is actually ticking, so
            // without this a genuinely wedged ACP process would hang
            // forever with no signal at all, same bug this whole feature
            // exists to close. `mark_turn_active_returning_was_active`
            // (not the plain `set_active_turn(true)` this used to call) so
            // a mid-turn steering send doesn't spawn a second, leaked
            // watchdog on top of an already-running one — see
            // `core::spawn_health_watchdog`'s own doc comment and
            // persistent.rs's identical guard.
            //
            // Established BEFORE the stdin enqueue below (not after) — a
            // fast-responding ACP agent can have the stdout reader task
            // observe and publish the resulting `stopReason` (turn_active:
            // false) on a DIFFERENT tokio worker before this thread would
            // otherwise reach this point, if the enqueue ran first. Only
            // ordering it this way guarantees this mark happens-before the
            // request can possibly be enqueued, sent, and answered. codex
            // P2 on PR #2338 (twenty-sixth re-review) — superseded the
            // prior (twenty-fifth re-review) ordering, which fixed a
            // different bug (see the rollback below) by moving this after
            // the enqueue, reintroducing this race.
            let was_active = self.health_monitor.mark_turn_active_returning_was_active();
            if !was_active {
                core::spawn_health_watchdog(&self.health_monitor);
            }
            // Publish the turn_active flip, mirroring persistent.rs's
            // send_message/send_user_message (which both call
            // self.publish_status() right after this same
            // mark_turn_active_returning_was_active() call). Without this,
            // wasTurnActive === false left over from an EARLIER turn's
            // end-of-turn publish (or the initial spawn publish) is
            // indistinguishable from genuine current idleness — a `/login`
            // that defers a controller refresh during THIS turn, followed
            // by a premature frontend Done transition, would see
            // isBackendTurnConfirmedIdle() read true from that stale
            // signal and force-restart the controller, killing the
            // actually-active turn. codex P1 on PR #2338 (twenty-third
            // re-review).
            self.publish_status();

            let send_result = {
                let inner = self.inner.lock().unwrap();
                if let Some(ref tx) = inner.stdin_tx {
                    tx.try_send(req)
                        .map_err(|e| format!("ACP stdin send failed: {e}"))
                } else {
                    Ok(())
                }
            };
            if let Err(e) = send_result {
                // try_send can fail (channel full, or the stdin writer task
                // already exited because the process died) — no turn
                // actually starts. Roll back the state just established
                // above, but ONLY if THIS call is what transitioned
                // idle->active (!was_active): if a turn was ALREADY active
                // (mid-turn steering), a genuinely active turn from an
                // EARLIER successful send must not be clobbered by this
                // failed one. The freshly-spawned watchdog (if any) exits
                // on its own next tick once is_active_turn() reads false
                // again — see spawn_health_watchdog's doc comment. codex P2
                // on PR #2338 (twenty-fifth re-review).
                if !was_active {
                    self.health_monitor.set_active_turn(false);
                    self.publish_status();
                }
                // Roll back latest_prompt_id too — UNCONDITIONALLY,
                // regardless of was_active (unlike the health-monitor
                // rollback above): a prompt that was never actually sent
                // must never be the id everything else waits to match
                // against. Without this, a mid-turn steering send whose
                // OWN enqueue fails leaves latest_prompt_id pointing at
                // this failed, never-sent request — the EARLIER prompt
                // that's still genuinely in flight then gets its real
                // stopReason response misclassified as stale by
                // is_latest_prompt_completion (its id no longer matches),
                // so turn_active never flips back to false for that pane,
                // permanently stranding flushPendingControllerRefresh.
                // compare_exchange (not an unconditional store) so a
                // legitimately newer prompt sent by another overlapping
                // call in the meantime is never clobbered by restoring
                // this stale "previous" value. reagent P1 on PR #2338
                // (twenty-eighth re-review).
                let _ = rollback_latest_prompt_id_if_unchanged(&self.latest_prompt_id, prompt_id, previous_prompt_id);
                return Err(e);
            }
        }

        if let Some(sig) = input.sig_name {
            if sig == "SIGTERM" || sig == "SIGINT" {
                return self.stop(true, STATUS_DONE);
            }
        }

        Ok(())
    }

    fn controller_type(&self) -> &str {
        BLOCK_CONTROLLER_ACP
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

    fn controller() -> AcpController {
        AcpController::new("tab".to_string(), "block".to_string(), None, None, None, None)
    }

    /// Regression for codex P2 on PR #2338 (twenty-seventh re-review): the
    /// staleness check that gates turn-end must reject any completion that
    /// doesn't echo the id of the most recently sent prompt, even if it
    /// carries a stopReason — this is exactly the out-of-order case an
    /// earlier prompt's response arriving after a steering/new prompt was
    /// already sent produces.
    #[test]
    fn is_latest_prompt_completion_requires_both_stop_reason_and_matching_id() {
        assert!(is_latest_prompt_completion(Some(3), true, 3), "matching id + stopReason must be trusted");
        assert!(!is_latest_prompt_completion(Some(2), true, 3), "an OLDER prompt's stopReason must not be trusted once a newer one has been sent");
        assert!(!is_latest_prompt_completion(Some(3), false, 3), "no stopReason at all is not turn-end, regardless of id");
        assert!(!is_latest_prompt_completion(None, true, 3), "a response with no id (malformed) must not be trusted");
    }

    /// Regression for codex P2 on PR #2338 (twenty-ninth re-review): the
    /// shared rollback helper used by both send_input's enqueue-failure
    /// path and the stdout-reader's JSON-RPC error-response handling.
    #[test]
    fn rollback_latest_prompt_id_if_unchanged_only_rolls_back_when_still_pointing_at_the_rejected_id() {
        let latest = AtomicU64::new(2);
        assert!(
            rollback_latest_prompt_id_if_unchanged(&latest, 2, 1),
            "must roll back when latest still points at the rejected id"
        );
        assert_eq!(latest.load(Ordering::Relaxed), 1, "must restore the previous id");

        // A newer prompt was sent in the meantime (latest is now 3, not 2) —
        // restoring the stale "previous" value must not clobber it.
        let latest = AtomicU64::new(3);
        assert!(
            !rollback_latest_prompt_id_if_unchanged(&latest, 2, 1),
            "must NOT roll back once a newer prompt has already been sent"
        );
        assert_eq!(latest.load(Ordering::Relaxed), 3, "the newer id must be left untouched");
    }

    /// Regression for reagent P1 on PR #2336: `send_input` used to call the
    /// plain `set_active_turn(true)`, which never spawns the health
    /// watchdog — combined with the stdout reader never calling
    /// `record_output` either, `evaluate_and_transition` could never
    /// observe silence, so the Restart-on-unresponsive feature could never
    /// trigger for ACP-backed panes at all. Confirms the swap to
    /// `mark_turn_active_returning_was_active` didn't change the
    /// observable turn-active behavior (full watchdog-tick coverage isn't
    /// practical here without spawning a real process — this pins the
    /// call-site contract; health.rs's own tests cover the watchdog
    /// mechanics once armed).
    // #[tokio::test], not #[test]: send_input now spawns the health
    // watchdog (core::spawn_health_watchdog does tokio::spawn), which
    // requires a running Tokio runtime — the exact watchdog-arming this
    // test exists to pin.
    #[tokio::test]
    async fn send_input_marks_the_turn_active() {
        let c = controller();
        // Simulate an already-running process — send_input only reaches the
        // turn-active/watchdog logic when `is_running()` is true (otherwise
        // it stashes the message as `pending_prompt` for the next start()).
        let (tx, _rx) = mpsc::channel::<String>(8);
        c.inner.lock().unwrap().stdin_tx = Some(tx);

        assert!(!c.health_monitor.is_active_turn());
        let res = c.send_input(BlockInputUnion::data(b"hello".to_vec()), None);
        assert!(res.is_ok(), "send_input should succeed against a simulated running process, got {res:?}");
        assert!(c.health_monitor.is_active_turn(), "send_input must mark the turn active");
    }

    /// A second send while already active (mid-turn steering) must not
    /// error or panic — `mark_turn_active_returning_was_active`'s
    /// was-already-active branch is what gates against spawning a second,
    /// leaked watchdog task; this at least confirms the call site doesn't
    /// choke on repeated calls the way a naive re-check-then-act would.
    #[tokio::test]
    async fn repeated_send_input_while_active_does_not_error() {
        let c = controller();
        let (tx, _rx) = mpsc::channel::<String>(8);
        c.inner.lock().unwrap().stdin_tx = Some(tx);

        assert!(c.send_input(BlockInputUnion::data(b"first".to_vec()), None).is_ok());
        assert!(c.send_input(BlockInputUnion::data(b"second".to_vec()), None).is_ok());
    }

    /// Regression for codex P1 on PR #2338 (twenty-third re-review):
    /// `send_input` marked the turn active via `health_monitor` but never
    /// published the flip — the only controllerstatus publishes for ACP
    /// were on spawn, kill, or process exit. A `wasTurnActive === false`
    /// left over from an EARLIER turn's end-of-turn publish (or the initial
    /// spawn publish) was therefore indistinguishable from genuine current
    /// idleness: useAgentCommands.ts's `isBackendTurnConfirmedIdle()` (fed
    /// only by live controllerstatus events) would read stale-true and let
    /// `flushPendingControllerRefresh` force-restart a controller that is
    /// actually mid-turn. Mirrors persistent.rs's send_message/
    /// send_user_message, which both call `publish_status()` right after
    /// the same `mark_turn_active_returning_was_active()` call.
    #[tokio::test]
    async fn send_input_publishes_the_turn_active_flip() {
        let broker = Arc::new(wps::Broker::new());
        let c = AcpController::new(
            "tab".to_string(),
            "block-acp-publish".to_string(),
            Some(broker.clone()),
            None,
            None,
            None,
        );
        let (tx, _rx) = mpsc::channel::<String>(8);
        c.inner.lock().unwrap().stdin_tx = Some(tx);

        assert!(c.send_input(BlockInputUnion::data(b"hello".to_vec()), None).is_ok());

        let history = broker.read_event_history(
            wps::EVENT_CONTROLLER_STATUS,
            "block:block-acp-publish",
            1,
        );
        assert_eq!(history.len(), 1, "send_input must publish a controllerstatus event");
        let published: BlockControllerRuntimeStatus =
            serde_json::from_value(history[0].data.clone().unwrap()).unwrap();
        assert!(published.turn_active, "published status must reflect the just-started turn as active");
        assert!(c.health_monitor.is_active_turn());
    }

    /// Regression for codex P2 on PR #2338 (twenty-fifth re-review), as
    /// refined by codex P2 on PR #2338 (twenty-sixth re-review):
    /// `send_input` marks the turn active and publishes BEFORE attempting
    /// `tx.try_send(req)` (established ordering, not after — see the
    /// twenty-sixth re-review's race fix below) — if the enqueue itself
    /// fails (channel full, or the stdin writer task already exited
    /// because the process died), no turn actually started, so the mark
    /// and publish must be ROLLED BACK rather than left standing. A
    /// rejected prompt left standing as "active" could re-promote the
    /// frontend to a working state and clear auth-recovery UI via
    /// notifyControllerHealthy, and the health watchdog would be armed for
    /// work that never happened.
    #[tokio::test]
    async fn send_input_rolls_back_turn_active_when_enqueue_fails() {
        let broker = Arc::new(wps::Broker::new());
        let c = AcpController::new(
            "tab".to_string(),
            "block-acp-enqueue-fail".to_string(),
            Some(broker.clone()),
            None,
            None,
            None,
        );
        let (tx, rx) = mpsc::channel::<String>(8);
        drop(rx); // Receiver gone — try_send fails with TrySendError::Closed.
        c.inner.lock().unwrap().stdin_tx = Some(tx);

        let res = c.send_input(BlockInputUnion::data(b"hello".to_vec()), None);
        assert!(res.is_err(), "send_input should surface the enqueue failure, got {res:?}");
        assert!(!c.health_monitor.is_active_turn(), "must not leave the turn marked active when the enqueue itself failed");

        // The state is briefly marked active (established BEFORE the
        // enqueue attempt to close the twenty-sixth re-review's race), then
        // rolled back once the enqueue failure is discovered — so the
        // LATEST published status must reflect the rollback, not the
        // transient true a live subscriber may have also observed.
        let history = broker.read_event_history(
            wps::EVENT_CONTROLLER_STATUS,
            "block:block-acp-enqueue-fail",
            1,
        );
        assert_eq!(history.len(), 1, "send_input must publish the rollback so live subscribers see the corrected state");
        let published: BlockControllerRuntimeStatus =
            serde_json::from_value(history[0].data.clone().unwrap()).unwrap();
        assert!(!published.turn_active, "the latest published status must reflect the rollback (turn_active: false)");
    }

    /// Regression for codex P2 on PR #2338 (twenty-sixth re-review): the
    /// rollback on enqueue failure must be conditioned on `!was_active` —
    /// a mid-turn steering send (turn already genuinely active from an
    /// EARLIER successful send) whose OWN enqueue then fails must not roll
    /// back and clobber that still-active turn.
    #[tokio::test]
    async fn send_input_does_not_roll_back_an_already_active_turn_on_a_failed_steering_send() {
        let c = controller();
        let (tx, _rx) = mpsc::channel::<String>(8);
        c.inner.lock().unwrap().stdin_tx = Some(tx);

        // First send succeeds — turn is now genuinely active.
        assert!(c.send_input(BlockInputUnion::data(b"first".to_vec()), None).is_ok());
        assert!(c.health_monitor.is_active_turn());
        let first_prompt_id = c.latest_prompt_id.load(Ordering::Relaxed);

        // Swap in a closed channel so the SECOND (steering) send's own
        // enqueue fails.
        let (tx2, rx2) = mpsc::channel::<String>(8);
        drop(rx2);
        c.inner.lock().unwrap().stdin_tx = Some(tx2);

        let res = c.send_input(BlockInputUnion::data(b"second".to_vec()), None);
        assert!(res.is_err(), "the steering send's own enqueue should fail, got {res:?}");
        assert!(c.health_monitor.is_active_turn(), "the turn genuinely active from the FIRST send must not be rolled back by the second send's own failure");

        // Regression for reagent P1 on PR #2338 (twenty-eighth re-review):
        // latest_prompt_id must be rolled back to the FIRST (still
        // genuinely in-flight) prompt's id, not left pointing at the
        // second, never-sent request's id — otherwise the first prompt's
        // real stopReason response is misclassified as stale by
        // is_latest_prompt_completion (since its id no longer matches),
        // and turn_active never flips back to false for this pane.
        assert_eq!(
            c.latest_prompt_id.load(Ordering::Relaxed),
            first_prompt_id,
            "latest_prompt_id must roll back to the still-in-flight FIRST prompt's id, not the failed second one"
        );
    }
}
