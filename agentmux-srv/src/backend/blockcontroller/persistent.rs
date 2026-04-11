// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! PersistentSubprocessController: manages agent CLI as a long-running process
//! with bidirectional NDJSON streaming via stdin/stdout.
//!
//! Architecture:
//!   A single CLI process is spawned on first message and kept alive for the
//!   entire session. User messages are written as NDJSON lines to stdin without
//!   closing it. This enables mid-turn input (redirecting the agent while it
//!   is still processing).
//!
//! State machine:
//!   INIT ─(first message)─> RUNNING ─(idle between turns)─> RUNNING
//!   RUNNING ─(kill/stop)─> DONE
//!   RUNNING ─(process crash)─> DONE (auto-restart possible via session_id)
//!
//! I/O model (3 async tasks per session):
//! 1. stdin_writer: mpsc channel → process stdin (NDJSON lines)
//! 2. stdout_reader: process stdout → .jsonl persistence + WPS blockfile events
//! 3. process_waiter: wait for exit, update status

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use super::{
    BlockControllerRuntimeStatus, BlockInputUnion, Controller, STATUS_DONE, STATUS_INIT,
    STATUS_RUNNING,
};
use super::health::{classify_output_line, HealthMonitor};
use crate::backend::eventbus::EventBus;
use crate::backend::storage::wstore::WaveStore;
use crate::backend::wps;

/// WPS file subject name for persistent subprocess output.
pub const PERSISTENT_OUTPUT_SUBJECT: &str = "output";

/// Controller type constant.
pub const BLOCK_CONTROLLER_PERSISTENT: &str = "persistent";

/// Configuration for spawning the persistent process.
#[derive(Debug, Clone)]
pub struct PersistentSpawnConfig {
    pub cli_command: String,
    pub cli_args: Vec<String>,
    pub working_dir: String,
    pub env_vars: HashMap<String, String>,
    pub session_id_field: String,
}

/// Inner state protected by mutex.
struct PersistentInner {
    proc_status: String,
    proc_exit_code: i32,
    status_version: i32,
    session_id: Option<String>,
    current_pid: Option<u32>,
    /// Channel to send messages to the stdin writer task.
    stdin_tx: Option<mpsc::Sender<String>>,
    /// Handle to kill the process.
    kill_tx: Option<tokio::sync::oneshot::Sender<bool>>,
}

/// PersistentSubprocessController keeps a long-running CLI process alive,
/// sending user messages as NDJSON lines on stdin.
pub struct PersistentSubprocessController {
    #[allow(dead_code)]
    tab_id: String,
    block_id: String,
    inner: Arc<Mutex<PersistentInner>>,
    broker: Option<Arc<wps::Broker>>,
    event_bus: Option<Arc<EventBus>>,
    wstore: Option<Arc<WaveStore>>,
    health_monitor: Arc<HealthMonitor>,
}

impl PersistentSubprocessController {
    pub fn new(
        tab_id: String,
        block_id: String,
        broker: Option<Arc<wps::Broker>>,
        event_bus: Option<Arc<EventBus>>,
        wstore: Option<Arc<WaveStore>>,
    ) -> Self {
        let health_monitor = Arc::new(HealthMonitor::new(
            block_id.clone(),
            broker.clone(),
        ));
        Self {
            tab_id,
            block_id,
            inner: Arc::new(Mutex::new(PersistentInner {
                proc_status: STATUS_INIT.to_string(),
                proc_exit_code: 0,
                status_version: 0,
                session_id: None,
                current_pid: None,
                stdin_tx: None,
                kill_tx: None,
            })),
            broker,
            event_bus,
            wstore,
            health_monitor,
        }
    }

    fn set_status(inner: &mut PersistentInner, status: &str) {
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

    /// Send a user message to the running CLI process.
    /// If the process isn't spawned yet, spawns it first.
    pub fn send_message(&self, message: String, config: PersistentSpawnConfig) -> Result<(), String> {
        // Spawn process if not running
        if !self.is_running() {
            self.spawn_process(config)?;
        }

        // Format as stream-json user message
        let json_msg = serde_json::json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": message
            }
        });

        let inner = self.inner.lock().unwrap();
        let tx = inner.stdin_tx.as_ref()
            .ok_or("persistent process not running after spawn")?;
        tx.try_send(json_msg.to_string())
            .map_err(|e| format!("stdin send failed: {e}"))
    }

    /// Spawn the persistent CLI process.
    fn spawn_process(&self, config: PersistentSpawnConfig) -> Result<(), String> {
        // Build command — use make_cli_cmd to resolve .cmd wrappers to node on Windows
        let mut cmd = crate::server::cli_handlers::make_cli_cmd(&config.cli_command);
        cmd.args(&config.cli_args);

        // Working directory
        if !config.working_dir.is_empty() {
            let expanded_dir = if config.working_dir.starts_with("~/") || config.working_dir == "~" {
                if let Some(home) = dirs::home_dir() {
                    home.join(config.working_dir.trim_start_matches("~/")).to_string_lossy().to_string()
                } else {
                    config.working_dir.clone()
                }
            } else {
                config.working_dir.clone()
            };
            let dir_path = std::path::Path::new(&expanded_dir);
            if !dir_path.exists() {
                if let Err(e) = std::fs::create_dir_all(dir_path) {
                    tracing::warn!(
                        block_id = %self.block_id,
                        dir = %expanded_dir,
                        error = %e,
                        "failed to create working directory"
                    );
                }
            }
            if dir_path.exists() {
                cmd.current_dir(&expanded_dir);
            }
        }

        // Environment variables (with tilde expansion)
        for (k, v) in &config.env_vars {
            let expanded = crate::backend::base::expand_home_dir_safe(v);
            cmd.env(k, expanded.to_string_lossy().as_ref());
        }

        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            tracing::error!(block_id = %self.block_id, error = %e, "persistent process spawn failed");
            format!("failed to spawn persistent process: {e}")
        })?;

        let pid = child.id().unwrap_or(0);
        tracing::info!(
            block_id = %self.block_id,
            pid = pid,
            cmd = %config.cli_command,
            args = ?config.cli_args,
            working_dir = %config.working_dir,
            "persistent process spawned"
        );

        let (kill_tx, kill_rx) = tokio::sync::oneshot::channel::<bool>();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take();

        // Drain stderr in background — log lines for debugging
        if let Some(stderr_pipe) = stderr {
            let block_id_stderr = self.block_id.clone();
            tokio::spawn(async move {
                let mut reader = BufReader::new(stderr_pipe).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    tracing::warn!(
                        block_id = %block_id_stderr,
                        line = %line,
                        "persistent stderr"
                    );
                }
            });
        }

        // Create stdin writer channel
        let (msg_tx, mut msg_rx) = mpsc::channel::<String>(32);

        {
            let mut inner = self.inner.lock().unwrap();
            inner.current_pid = Some(pid);
            inner.kill_tx = Some(kill_tx);
            inner.stdin_tx = Some(msg_tx);
            Self::set_status(&mut inner, STATUS_RUNNING);
        }
        self.publish_status();

        // Spawn stdin writer task
        tokio::spawn(async move {
            let mut stdin = stdin;
            while let Some(msg) = msg_rx.recv().await {
                if let Err(e) = stdin.write_all(msg.as_bytes()).await {
                    tracing::warn!("persistent stdin write error: {}", e);
                    break;
                }
                if let Err(e) = stdin.write_all(b"\n").await {
                    tracing::warn!("persistent stdin newline error: {}", e);
                    break;
                }
                if let Err(e) = stdin.flush().await {
                    tracing::warn!("persistent stdin flush error: {}", e);
                    break;
                }
            }
            // Channel closed or write error → stdin drops → process gets EOF
            drop(stdin);
        });

        // Spawn stdout reader task
        let block_id_read = self.block_id.clone();
        let broker_read = self.broker.clone();
        let inner_read = Arc::clone(&self.inner);
        let wstore_read = self.wstore.clone();
        let event_bus_read = self.event_bus.clone();
        let health_read = Arc::clone(&self.health_monitor);
        let session_id_field = config.session_id_field.clone();

        tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();

            while let Ok(Some(line)) = lines.next_line().await {
                if line.trim().is_empty() {
                    continue;
                }

                // Parse JSON for health monitoring and session ID capture
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&line) {
                    let (meaningful, _error) = classify_output_line(&parsed);
                    health_read.record_output(meaningful);
                    if let Some(sid) = parsed.get(&session_id_field).and_then(|v| v.as_str()) {
                        let sid_string = sid.to_string();
                        let already_captured = inner_read.lock().unwrap().session_id.is_some();
                        if !already_captured {
                            tracing::info!(
                                block_id = %block_id_read,
                                session_id = %sid_string,
                                "persistent session ID captured"
                            );
                            {
                                let mut inner = inner_read.lock().unwrap();
                                inner.session_id = Some(sid_string.clone());
                            }
                            // Persist to block metadata (same pattern as subprocess.rs)
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

                // Publish line as WPS blockfile event
                tracing::info!(
                    block_id = %block_id_read,
                    line_len = line.len(),
                    "persistent stdout → blockfile"
                );
                let line_with_newline = format!("{}\n", line);
                if let Some(ref broker) = broker_read {
                    super::shell::handle_append_block_file(
                        broker,
                        &block_id_read,
                        PERSISTENT_OUTPUT_SUBJECT,
                        line_with_newline.as_bytes(),
                    );
                } else {
                    tracing::warn!(block_id = %block_id_read, "persistent stdout: no broker available");
                }
            }

            tracing::info!(block_id = %block_id_read, "persistent stdout reader finished");
        });

        // Spawn process waiter task
        let block_id_wait = self.block_id.clone();
        let inner_wait = Arc::clone(&self.inner);
        let broker_wait = self.broker.clone();

        tokio::spawn(async move {
            tokio::select! {
                status = child.wait() => {
                    let exit_code = status.map(|s| s.code().unwrap_or(-1)).unwrap_or(-1);
                    tracing::info!(
                        block_id = %block_id_wait,
                        exit_code = exit_code,
                        "persistent process exited"
                    );

                    let mut inner = inner_wait.lock().unwrap();
                    inner.proc_exit_code = exit_code;
                    inner.current_pid = None;
                    inner.stdin_tx = None;
                    inner.kill_tx = None;
                    Self::set_status(&mut inner, STATUS_DONE);
                    drop(inner);

                    // Publish status
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
                Ok(force) = kill_rx => {
                    tracing::info!(
                        block_id = %block_id_wait,
                        force = force,
                        "persistent process kill requested"
                    );
                    if force {
                        let _ = child.kill().await;
                    } else {
                        // Graceful: drop stdin to send EOF, then wait briefly
                        {
                            let mut inner = inner_wait.lock().unwrap();
                            inner.stdin_tx = None; // drops the sender → stdin writer exits → stdin closes
                        }
                        tokio::select! {
                            _ = child.wait() => {}
                            _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => {
                                let _ = child.kill().await;
                            }
                        }
                    }

                    let mut inner = inner_wait.lock().unwrap();
                    inner.proc_exit_code = -1;
                    inner.current_pid = None;
                    inner.stdin_tx = None;
                    inner.kill_tx = None;
                    Self::set_status(&mut inner, STATUS_DONE);
                }
            }
        });

        Ok(())
    }

    pub fn stop_process(&self, force: bool) -> Result<(), String> {
        let kill_tx = {
            let mut inner = self.inner.lock().unwrap();
            inner.kill_tx.take()
        };
        match kill_tx {
            Some(tx) => {
                let _ = tx.send(force);
                Ok(())
            }
            None => Ok(()),
        }
    }

    pub fn session_id(&self) -> Option<String> {
        self.inner.lock().unwrap().session_id.clone()
    }
}

impl Controller for PersistentSubprocessController {
    fn start(
        &self,
        _block_meta: super::super::obj::MetaMapType,
        _rt_opts: Option<serde_json::Value>,
        _force: bool,
    ) -> Result<(), String> {
        tracing::info!(
            block_id = %self.block_id,
            "persistent controller registered (spawns on first message)"
        );
        Ok(())
    }

    fn stop(&self, _graceful: bool, new_status: &str) -> Result<(), String> {
        self.stop_process(true)?;
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
        Err("persistent controller does not accept raw input; use send_message()".to_string())
    }

    fn controller_type(&self) -> &str {
        BLOCK_CONTROLLER_PERSISTENT
    }

    fn block_id(&self) -> &str {
        &self.block_id
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
