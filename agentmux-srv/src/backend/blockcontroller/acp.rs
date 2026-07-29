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
                            if result.get("stopReason").is_some() {
                                health_clone.set_active_turn(false);
                            }
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
            let req = self.make_request("session/prompt", serde_json::json!({
                "sessionId": session_id,
                "prompt": {
                    "type": "text",
                    "text": message,
                }
            }));
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
            let was_active = self.health_monitor.mark_turn_active_returning_was_active();
            if !was_active {
                core::spawn_health_watchdog(&self.health_monitor);
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn controller() -> AcpController {
        AcpController::new("tab".to_string(), "block".to_string(), None, None, None, None)
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
        assert!(c.health_monitor.is_active_turn());
    }
}
