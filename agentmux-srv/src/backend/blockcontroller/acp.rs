// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! AcpController: manages agent CLIs that speak the Agent Client Protocol (ACP).
//!
//! ACP is a JSON-RPC 2.0 protocol over stdin/stdout — the "LSP for AI agents."
//! Instead of custom per-provider output parsing, ACP provides a standardized
//! protocol for session management, prompting, and streaming events.
//!
//! Lifecycle:
//!   1. Spawn the ACP agent process (e.g., `gemini --acp`, `acpx --agent openclaw`)
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
use super::health::HealthMonitor;
use crate::backend::eventbus::EventBus;
use crate::backend::storage::filestore::FileStore;
use crate::backend::storage::wstore::WaveStore;
use crate::backend::wps;

/// WPS file subject name for ACP output.
pub const ACP_OUTPUT_SUBJECT: &str = "output";

/// Controller type constant.
pub const BLOCK_CONTROLLER_ACP: &str = "acp";

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
    wstore: Option<Arc<WaveStore>>,
    filestore: Option<Arc<FileStore>>,
    health_monitor: Arc<HealthMonitor>,
    /// Monotonically increasing JSON-RPC request ID.
    next_rpc_id: Arc<AtomicU64>,
}

impl AcpController {
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

    /// Send a user message via ACP session/prompt.
    /// If the process isn't spawned yet, spawns it first (the first prompt is
    /// deferred until the session is established — see `pending_prompt`).
    pub fn send_message(&self, message: String, cli_command: String, cli_args: Vec<String>, working_dir: String, env_vars: HashMap<String, String>) -> Result<(), String> {
        if !self.is_running() {
            // First turn: spawn and stash the prompt — the stdout reader will
            // send it once the session/create response arrives.
            {
                let mut inner = self.inner.lock().unwrap();
                inner.pending_prompt = Some(message.clone());
            }
            self.health_monitor.set_active_turn(true);
            return self.spawn_process(cli_command, cli_args, working_dir, env_vars);
        }

        // Subsequent turns: session_id is already populated.
        let session_id = {
            let inner = self.inner.lock().unwrap();
            inner.session_id.clone().unwrap_or_default()
        };

        let req = self.make_request("session/prompt", serde_json::json!({
            "sessionId": session_id,
            "prompt": {
                "type": "text",
                "text": message,
            }
        }));

        self.health_monitor.set_active_turn(true);

        let inner = self.inner.lock().unwrap();
        let tx = inner.stdin_tx.as_ref()
            .ok_or("ACP process not running after spawn")?;
        tx.try_send(req)
            .map_err(|e| format!("ACP stdin send failed: {e}"))
    }

    /// Spawn the ACP agent process and perform the initialize handshake.
    fn spawn_process(&self, cli_command: String, cli_args: Vec<String>, working_dir: String, env_vars: HashMap<String, String>) -> Result<(), String> {
        let mut cmd = crate::server::cli_handlers::make_cli_cmd(&cli_command);
        cmd.args(&cli_args);

        // Working directory
        if !working_dir.is_empty() {
            let expanded_dir = if working_dir.starts_with("~/") || working_dir == "~" {
                if let Some(home) = dirs::home_dir() {
                    home.join(working_dir.trim_start_matches("~/")).to_string_lossy().to_string()
                } else {
                    working_dir.clone()
                }
            } else {
                working_dir.clone()
            };
            let dir_path = std::path::Path::new(&expanded_dir);
            if !dir_path.exists() {
                if let Err(e) = std::fs::create_dir_all(dir_path) {
                    tracing::warn!(
                        block_id = %self.block_id,
                        dir = %expanded_dir,
                        error = %e,
                        "failed to create working directory for ACP agent"
                    );
                }
            }
            if dir_path.exists() {
                cmd.current_dir(&expanded_dir);
            }
        }

        // Environment variables
        for (k, v) in &env_vars {
            let expanded = crate::backend::base::expand_home_dir_safe(v);
            cmd.env(k, expanded.to_string_lossy().as_ref());
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
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take();

        // Drain stderr
        if let Some(stderr_pipe) = stderr {
            let block_id_stderr = self.block_id.clone();
            tokio::spawn(async move {
                let mut reader = BufReader::new(stderr_pipe).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    tracing::warn!(
                        block_id = %block_id_stderr,
                        line = %line,
                        "ACP agent stderr"
                    );
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
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if line.is_empty() {
                    continue;
                }

                // Parse as JSON to check for session/update notifications
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&line) {
                    // Extract session ID from initialize result or session/create result
                    if let Some(result) = json.get("result") {
                        if let Some(sid) = result.get("sessionId").and_then(|v| v.as_str()) {
                            let mut inner = inner_clone.lock().unwrap();
                            inner.session_id = Some(sid.to_string());
                            tracing::info!(
                                block_id = %block_id_stdout,
                                session_id = %sid,
                                "ACP session established"
                            );

                            // Flush pending prompt now that session is ready.
                            if let Some(prompt) = inner.pending_prompt.take() {
                                let id = rpc_id_clone.fetch_add(1, Ordering::Relaxed);
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
                                    let _ = tx.try_send(req);
                                }
                            }
                        }
                    }

                    // Reset health monitor on prompt result (turn complete)
                    if json.get("id").is_some() && json.get("result").is_some() {
                        // This is a response to a request (e.g., session/prompt result)
                        if let Some(result) = json.get("result") {
                            if result.get("stopReason").is_some() {
                                health_clone.set_active_turn(false);
                            }
                        }
                    }
                }

                // Persist to .jsonl file
                if let Some(ref fstore) = filestore_clone {
                    let _ = fstore.append_line(&block_id_stdout, ACP_OUTPUT_SUBJECT, &line);
                }

                // Broadcast via WPS so the frontend receives the event
                if let Some(ref broker) = broker_clone {
                    let event = wps::WaveEvent {
                        event: wps::EVENT_BLOCKFILE.to_string(),
                        scopes: vec![format!("block:{}", block_id_stdout)],
                        sender: String::new(),
                        persist: 0,
                        data: Some(serde_json::json!({
                            "zoneid": "",
                            "blockid": block_id_stdout,
                            "name": ACP_OUTPUT_SUBJECT,
                            "data": line,
                        })),
                    };
                    broker.publish(event);
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
            let _ = tx.try_send(init_req);
            let _ = tx.try_send(init_notification);
            let _ = tx.try_send(session_req);
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
                let _ = tx.try_send(shutdown);
                let _ = tx.try_send(exit);
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

    fn send_input(&self, input: BlockInputUnion) -> Result<(), String> {
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
            let req = self.make_request("session/prompt", serde_json::json!({
                "sessionId": session_id,
                "prompt": {
                    "type": "text",
                    "text": message,
                }
            }));
            self.health_monitor.set_active_turn(true);
            let inner = self.inner.lock().unwrap();
            if let Some(ref tx) = inner.stdin_tx {
                tx.try_send(req)
                    .map_err(|e| format!("ACP stdin send failed: {e}"))?;
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
