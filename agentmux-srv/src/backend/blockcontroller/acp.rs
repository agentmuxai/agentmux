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

use std::collections::{HashMap, HashSet};
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

/// Remove `resolved_id` from the set of outstanding (sent, not yet
/// resolved) prompts and report whether the set is now EMPTY — i.e.
/// whether every prompt sent so far has now been resolved, by either a
/// real `stopReason` completion or a JSON-RPC `error` rejection, meaning
/// the turn has genuinely ended. Extracted as a pure function over a
/// plain `HashSet` so this exact logic is directly unit-testable; call
/// sites wrap it with the shared mutex around the real
/// `outstanding_prompt_ids`.
///
/// Tracking every outstanding prompt — not just "the latest" — is what
/// makes response ORDERING irrelevant: "is the turn over" only ever asks
/// "is anything still outstanding," never "was this the most recent
/// one." codex P2 on PR #2338 (thirtieth re-review) — superseded the
/// twenty-seventh through twenty-ninth re-reviews' single latest/
/// previous-id tracking (`is_latest_prompt_completion` +
/// `rollback_latest_prompt_id_if_unchanged`, both removed), which could
/// permanently strand `turn_active` at `true`: a steering prompt's
/// rejection arriving AFTER the earlier prompt's completion had already
/// been consumed (and discarded as "not the latest") rolled
/// `latest_prompt_id` back to a prompt whose own completion would never
/// arrive again — nothing left to ever satisfy the match.
fn resolve_prompt_and_check_idle(outstanding: &mut HashSet<u64>, resolved_id: u64) -> bool {
    outstanding.remove(&resolved_id);
    outstanding.is_empty()
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
    /// Every `session/prompt` request id that has been SENT but not yet
    /// RESOLVED (by either a real `stopReason` completion or a JSON-RPC
    /// `error` rejection). The turn is genuinely over only once this set
    /// is empty — see `resolve_prompt_and_check_idle`'s doc comment for
    /// why tracking the full set (not just "the latest" id) is necessary:
    /// steering can leave multiple prompts outstanding at once, and
    /// responses/rejections can arrive in any order relative to each
    /// other and relative to new sends (independent stdin-writer and
    /// stdout-reader tasks, no ordering guarantee between them). codex P2
    /// on PR #2338 (twenty-seventh through thirtieth re-reviews).
    outstanding_prompt_ids: Arc<Mutex<HashSet<u64>>>,
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
            outstanding_prompt_ids: Arc::new(Mutex::new(HashSet::new())),
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
        let outstanding_prompt_ids_clone = self.outstanding_prompt_ids.clone();
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
                                    outstanding_prompt_ids_clone.lock().unwrap().insert(id);
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

                    // A prompt is RESOLVED — one way or another — by either
                    // a real `stopReason` completion or a JSON-RPC `error`
                    // rejection (mid-turn prompt acceptance is agent-
                    // dependent; some agents reject a steering prompt sent
                    // while an earlier one is still being processed).
                    // Either way, remove it from the outstanding set and
                    // only declare the turn genuinely over once NOTHING is
                    // left outstanding — never based on "was this the most
                    // recently sent prompt," which breaks under steering:
                    // an earlier prompt's completion can arrive after a
                    // newer one was sent, or a newer prompt's rejection can
                    // arrive after an earlier one already completed. codex
                    // P2 on PR #2338 (twenty-seventh through thirtieth
                    // re-reviews).
                    let resolved_id = if json.get("id").is_some()
                        && json.get("result").and_then(|r| r.get("stopReason")).is_some()
                    {
                        json.get("id").and_then(|v| v.as_u64())
                    } else if json.get("id").is_some() && json.get("error").is_some() {
                        json.get("id").and_then(|v| v.as_u64())
                    } else {
                        None
                    };
                    if let Some(resolved_id) = resolved_id {
                        let turn_is_over = {
                            let mut outstanding = outstanding_prompt_ids_clone.lock().unwrap();
                            resolve_prompt_and_check_idle(&mut outstanding, resolved_id)
                        };
                        if turn_is_over {
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
            // be tracked as outstanding (see outstanding_prompt_ids's doc
            // comment) — make_request only returns the serialized string.
            // Inserted BEFORE the enqueue below (not after), for the same
            // happens-before reason mark_turn_active_returning_was_active
            // is called before the enqueue — see that call's own comment.
            let prompt_id = self.next_id();
            self.outstanding_prompt_ids.lock().unwrap().insert(prompt_id);
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
            // The return value (was a turn already active?) is used ONLY to
            // gate the watchdog spawn below, not to decide whether to roll
            // back on enqueue failure (see that rollback's own comment) —
            // outstanding_prompt_ids's emptiness is the authoritative signal
            // for that now.
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
                // actually starts for THIS prompt. Remove it from the
                // outstanding set (it was never really sent) and only
                // declare the turn over if NOTHING else is left outstanding
                // — a turn ALREADY active from an earlier successful send
                // (mid-turn steering) must not be clobbered by this failed
                // one. This single emptiness check replaces what used to be
                // two separate, easy-to-desync mechanisms: a `was_active`-
                // conditioned health-monitor rollback, and a
                // latest/previous-id compare-and-swap that could still
                // strand turn_active under out-of-order responses (codex
                // P2 on PR #2338, twenty-seventh through thirtieth
                // re-reviews). The freshly-spawned watchdog (if any) exits
                // on its own next tick once is_active_turn() reads false
                // again — see spawn_health_watchdog's doc comment.
                let now_empty = {
                    let mut outstanding = self.outstanding_prompt_ids.lock().unwrap();
                    outstanding.remove(&prompt_id);
                    outstanding.is_empty()
                };
                if now_empty {
                    self.health_monitor.set_active_turn(false);
                    self.publish_status();
                }
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

    /// Regression for codex P2 on PR #2338 (twenty-seventh/thirtieth
    /// re-reviews): the single-prompt case — one prompt sent, one
    /// response — must still correctly declare the turn over.
    #[test]
    fn resolve_prompt_and_check_idle_reports_empty_after_the_only_outstanding_prompt_resolves() {
        let mut outstanding = HashSet::from([3]);
        assert!(
            resolve_prompt_and_check_idle(&mut outstanding, 3),
            "resolving the only outstanding prompt must report the turn as over"
        );
        assert!(outstanding.is_empty());
    }

    /// Regression for codex P2 on PR #2338 (twenty-seventh re-review): an
    /// EARLIER prompt's response arriving after a steering/new prompt was
    /// already sent must not end the turn while the newer one is still
    /// outstanding.
    #[test]
    fn resolve_prompt_and_check_idle_stays_active_while_a_newer_steering_prompt_is_still_outstanding() {
        let mut outstanding = HashSet::from([1, 2]);
        assert!(
            !resolve_prompt_and_check_idle(&mut outstanding, 1),
            "the newer (2) prompt is still outstanding — the turn must not end yet"
        );
        assert_eq!(outstanding, HashSet::from([2]));
    }

    /// Regression for codex P2 on PR #2338 (thirtieth re-review) — the
    /// exact scenario the single latest/previous-id tracking (removed)
    /// could not handle: the EARLIER prompt completes and is resolved
    /// FIRST (while a steering prompt is still outstanding), and the
    /// steering prompt is REJECTED afterward. The turn must end once that
    /// rejection resolves the LAST remaining outstanding prompt — not get
    /// permanently stuck because the earlier prompt's completion was
    /// already "consumed."
    #[test]
    fn resolve_prompt_and_check_idle_ends_the_turn_when_a_later_rejection_resolves_the_last_outstanding_prompt() {
        let mut outstanding = HashSet::from([1, 2]);
        // The earlier prompt (1) completes first.
        assert!(!resolve_prompt_and_check_idle(&mut outstanding, 1), "prompt 2 is still outstanding");
        // The steering prompt (2) is REJECTED afterward — this must now
        // correctly end the turn, since nothing else is outstanding.
        assert!(
            resolve_prompt_and_check_idle(&mut outstanding, 2),
            "the last outstanding prompt resolving (even via rejection) must end the turn"
        );
        assert!(outstanding.is_empty());
    }

    /// A response for an id that was never tracked (or already resolved) —
    /// e.g. a duplicate delivery — must not panic; HashSet::remove is a
    /// harmless no-op for a non-member.
    #[test]
    fn resolve_prompt_and_check_idle_is_a_harmless_no_op_for_an_untracked_id() {
        let mut outstanding: HashSet<u64> = HashSet::new();
        assert!(resolve_prompt_and_check_idle(&mut outstanding, 999));
        assert!(outstanding.is_empty());
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

    /// Regression for codex P2 on PR #2338 (twenty-sixth re-review): a
    /// mid-turn steering send (turn already genuinely active from an
    /// EARLIER successful send) whose OWN enqueue then fails must not roll
    /// back and clobber that still-active turn — the FIRST prompt's id
    /// must remain outstanding.
    ///
    /// Also covers reagent P1 (twenty-eighth re-review) and codex P2
    /// (twenty-ninth/thirtieth re-reviews): the failed second prompt's id
    /// must NOT remain in the outstanding set either (it was never really
    /// sent) — outstanding_prompt_ids's set-based tracking (superseding
    /// the single latest/previous-id fields those rounds patched
    /// incrementally) makes both of these hold simultaneously without any
    /// special-cased rollback logic: removing the failed id and checking
    /// emptiness is the ONLY rule, and it's correct regardless of send/
    /// response ordering.
    #[tokio::test]
    async fn send_input_does_not_roll_back_an_already_active_turn_on_a_failed_steering_send() {
        let c = controller();
        let (tx, _rx) = mpsc::channel::<String>(8);
        c.inner.lock().unwrap().stdin_tx = Some(tx);

        // First send succeeds — turn is now genuinely active.
        assert!(c.send_input(BlockInputUnion::data(b"first".to_vec()), None).is_ok());
        assert!(c.health_monitor.is_active_turn());
        let outstanding_after_first: Vec<u64> = c.outstanding_prompt_ids.lock().unwrap().iter().copied().collect();
        assert_eq!(outstanding_after_first.len(), 1, "exactly the first prompt's id must be outstanding");
        let first_prompt_id = outstanding_after_first[0];

        // Swap in a closed channel so the SECOND (steering) send's own
        // enqueue fails.
        let (tx2, rx2) = mpsc::channel::<String>(8);
        drop(rx2);
        c.inner.lock().unwrap().stdin_tx = Some(tx2);

        let res = c.send_input(BlockInputUnion::data(b"second".to_vec()), None);
        assert!(res.is_err(), "the steering send's own enqueue should fail, got {res:?}");
        assert!(c.health_monitor.is_active_turn(), "the turn genuinely active from the FIRST send must not be rolled back by the second send's own failure");

        let outstanding_after_failure = c.outstanding_prompt_ids.lock().unwrap().clone();
        assert_eq!(
            outstanding_after_failure,
            HashSet::from([first_prompt_id]),
            "only the still-in-flight FIRST prompt's id must remain outstanding — the failed second one must not linger"
        );
    }
}
