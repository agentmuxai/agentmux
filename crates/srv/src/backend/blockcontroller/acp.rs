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

#[cfg(windows)]
use agentmux_common::win32::NoWindow;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use super::{
    agent_runtime_status, BlockControllerRuntimeStatus, BlockInputUnion, Controller, STATUS_DONE,
    STATUS_INIT, STATUS_RUNNING,
};
use super::core;
use super::meta_string_map;
use super::health::TurnActivityTracker;
use crate::backend::eventbus::EventBus;
use crate::backend::storage::filestore::FileStore;
use crate::backend::storage::store::Store;
use crate::backend::mps;

/// MPS file subject name for ACP output.
pub const ACP_OUTPUT_SUBJECT: &str = "output";

pub const BLOCK_CONTROLLER_ACP: &str = "acp";

// ---- ACP v1 messages (SPEC_ACP_CLIENT_CONFORMANCE_2026_10_07.md) ----
// Checked against @agentclientprotocol/sdk 0.26.0 (`PROTOCOL_VERSION = 1`).

/// `initialize`: the protocol version and what this client can do. AgentMux
/// offers no file system or terminal to the agent; it runs its own tools.
fn initialize_params() -> serde_json::Value {
    serde_json::json!({
        "protocolVersion": 1,
        "clientCapabilities": {
            "fs": { "readTextFile": false, "writeTextFile": false },
            "terminal": false,
        },
        "clientInfo": { "name": "AgentMux", "version": env!("CARGO_PKG_VERSION") },
    })
}

/// `session/new`, or `session/load` of `resume` when the agent can load one.
fn session_request(resume: Option<&str>, load_session: bool, cwd: &str) -> (&'static str, serde_json::Value) {
    match resume.filter(|_| load_session) {
        Some(sid) => (
            "session/load",
            serde_json::json!({ "sessionId": sid, "cwd": cwd, "mcpServers": [] }),
        ),
        None => ("session/new", serde_json::json!({ "cwd": cwd, "mcpServers": [] })),
    }
}

/// `session/prompt` params: the prompt is an array of content blocks.
fn prompt_params(session_id: &str, text: &str) -> serde_json::Value {
    serde_json::json!({
        "sessionId": session_id,
        "prompt": [{ "type": "text", "text": text }],
    })
}

/// The reply to a request the agent sends us, if `json` is one (it has both
/// `method` and `id`). AgentMux approves tool use by default, as it does for
/// every harness: the first `allow_always` option, else `allow_once`, else
/// cancelled. Anything else (`fs/*`, `terminal/*`, which this client does not
/// offer) gets "method not found", so the agent never waits on us.
fn reply_to_agent_request(json: &serde_json::Value) -> Option<serde_json::Value> {
    let method = json.get("method")?.as_str()?;
    let id = json.get("id")?.clone();
    if id.is_null() {
        return None;
    }
    if method == "session/request_permission" {
        let options = json
            .pointer("/params/options")
            .and_then(|o| o.as_array())
            .cloned()
            .unwrap_or_default();
        let pick = |kind: &str| {
            options
                .iter()
                .find(|o| o.get("kind").and_then(|k| k.as_str()) == Some(kind))
                .and_then(|o| o.get("optionId").cloned())
        };
        let outcome = match pick("allow_always").or_else(|| pick("allow_once")) {
            Some(option_id) => serde_json::json!({ "outcome": "selected", "optionId": option_id }),
            None => serde_json::json!({ "outcome": "cancelled" }),
        };
        return Some(serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": { "outcome": outcome } }));
    }
    Some(serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": -32601, "message": format!("AgentMux does not offer {method}") },
    }))
}

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
    /// ACP v1 handshake: the `initialize` request, whose result decides
    /// between `session/new` and `session/load`.
    init_request_id: Option<u64>,
    /// The `session/new` or `session/load` request in flight.
    session_request_id: Option<u64>,
    /// The session being loaded: its result carries no id of its own.
    loading_session: Option<String>,
    /// The pane's previous session (`agent:sessionid`), offered to
    /// `session/load` when the agent supports it.
    resume_session_id: Option<String>,
    /// The session's working directory.
    session_cwd: String,
}

/// AcpController manages an ACP-speaking agent process.
pub struct AcpController {
    #[allow(dead_code)]
    tab_id: String,
    block_id: String,
    inner: Arc<Mutex<AcpInner>>,
    broker: Option<Arc<mps::Broker>>,
    event_bus: Option<Arc<EventBus>>,
    mstore: Option<Arc<Store>>,
    filestore: Option<Arc<FileStore>>,
    health_monitor: Arc<TurnActivityTracker>,
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
        broker: Option<Arc<mps::Broker>>,
        event_bus: Option<Arc<EventBus>>,
        mstore: Option<Arc<Store>>,
        filestore: Option<Arc<FileStore>>,
    ) -> Self {
        let health_monitor = Arc::new(TurnActivityTracker::new(block_id.clone()));
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
                init_request_id: None,
                session_request_id: None,
                loading_session: None,
                resume_session_id: None,
                session_cwd: String::new(),
            })),
            broker,
            event_bus,
            mstore,
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
        agent_runtime_status(
            &self.block_id,
            inner.status_version,
            &inner.proc_status,
            inner.proc_exit_code,
            self.health_monitor.is_active_turn(),
        )
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
        // Identity M4a: record what this process is actually given.
        crate::backend::identity_spawn::record_process_spawn(
            &self.block_id,
            crate::backend::identity_spawn::SpawnPath::Acp,
            &env_vars,
        );
        // On Windows: suppress console-window allocation. Without CREATE_NO_WINDOW,
        // node.exe spawned from a windowless sidecar may try to create/attach to a
        // console, causing stdout to go to that console rather than the pipe.
        #[cfg(windows)]
        {
            cmd.no_window();
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

        // Assign to this block's process tracker — same path SubprocessController
        // and PersistentSubprocessController already use. Closes the Process
        // Broker Phase B coverage gap for ACP-type agent panes (see
        // docs/specs/SPEC_PROCESS_BROKER_PHASE_B_SHELL_ACP_REGISTRATION_2026_07_31.md).
        if pid != 0 {
            crate::backend::process_tracker::registry::track_spawned_agent(&self.block_id, pid);
        }

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

        // Spawn stdout reader task — reads NDJSON lines and broadcasts via MPS
        let block_id_stdout = self.block_id.clone();
        let broker_clone = self.broker.clone();
        let filestore_clone = self.filestore.clone();
        let inner_clone = self.inner.clone();
        let health_clone = self.health_monitor.clone();
        let rpc_id_clone = self.next_rpc_id.clone();
        let outstanding_prompt_ids_clone = self.outstanding_prompt_ids.clone();
        let mstore_clone = self.mstore.clone();
        let event_bus_clone = self.event_bus.clone();
        // Resolve the agent's GLOBAL transcript zone once (see persistent/spawn.rs).
        let global_output_zone =
            super::shell::resolve_global_output_zone(&self.mstore, &self.block_id);
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
                // A failed `session/load` the handshake recovers from (it
                // opens a new session): not shown, or the pane reads it as
                // an error that ended a turn.
                let mut recovered_error = false;
                // Parse as JSON to check for session/update notifications
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&line) {
                    // A request from the agent (permission, fs, terminal):
                    // answer it, or the agent waits forever.
                    if let Some(reply) = reply_to_agent_request(&json) {
                        let inner = inner_clone.lock().unwrap();
                        if let Some(ref tx) = inner.stdin_tx {
                            if tx.try_send(reply.to_string()).is_err() {
                                tracing::warn!(block_id = %block_id_stdout, "[acp] reply to an agent request dropped — channel full or closed");
                            }
                        }
                    }
                    // Handshake: `initialize` answered → open or load the
                    // session; a failed `session/load` → open a new one. A
                    // loaded session's result carries no id: it is the one
                    // we asked for.
                    let msg_id = json.get("id").and_then(|v| v.as_u64());
                    let mut loaded: Option<String> = None;
                    {
                        let mut inner = inner_clone.lock().unwrap();
                        let is = |id: Option<u64>| msg_id.is_some() && msg_id == id;
                        let next = if is(inner.init_request_id) && json.get("result").is_some() {
                            inner.init_request_id = None;
                            let can_load = json
                                .pointer("/result/agentCapabilities/loadSession")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);
                            let resume = inner.resume_session_id.clone();
                            let loading = if can_load { resume.clone() } else { None };
                            Some((session_request(resume.as_deref(), can_load, &inner.session_cwd), loading))
                        } else if is(inner.session_request_id) && json.get("error").is_some() && inner.loading_session.is_some() {
                            inner.loading_session = None;
                            recovered_error = true;
                            Some((session_request(None, false, &inner.session_cwd), None))
                        } else {
                            None
                        };
                        if is(inner.session_request_id) && json.get("result").is_some() {
                            loaded = inner.loading_session.take();
                        }
                        if let Some(((method, params), loading)) = next {
                            let id = rpc_id_clone.fetch_add(1, Ordering::Relaxed);
                            inner.session_request_id = Some(id);
                            inner.loading_session = loading;
                            let req = serde_json::json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }).to_string();
                            if let Some(ref tx) = inner.stdin_tx {
                                if tx.try_send(req).is_err() {
                                    tracing::error!(block_id = %block_id_stdout, method, "[acp] session request dropped — channel full or closed; agent will not start");
                                }
                            }
                        }
                    }
                    let session = json
                        .pointer("/result/sessionId")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                        .or(loaded);
                    if let Some(sid_owned) = session {
                        {
                            let sid = sid_owned.as_str();
                            {
                                let mut inner = inner_clone.lock().unwrap();
                                inner.session_id = Some(sid_owned.clone());
                                // Flush pending prompt now that session is ready.
                                if let Some(prompt) = inner.pending_prompt.take() {
                                    let id = rpc_id_clone.fetch_add(1, Ordering::Relaxed);
                                    outstanding_prompt_ids_clone.lock().unwrap().insert(id);
                                    // Mark the turn active + publish, mirroring
                                    // send_input's identical pattern — without
                                    // this, a prompt flushed here (queued before
                                    // the controller finished starting) is
                                    // invisible to is_active_turn()/wasTurnActive:
                                    // a concurrently-deferred /login controller
                                    // refresh could then read
                                    // isBackendTurnConfirmedIdle() as true and
                                    // force-restart the controller while this
                                    // flushed prompt is genuinely in flight.
                                    // reagentx P2 on PR #2338 (thirty-first
                                    // re-review).
                                    health_clone.mark_turn_active_returning_was_active();
                                    if let Some(ref broker) = broker_clone {
                                        let status = agent_runtime_status(
                                            &block_id_stdout,
                                            inner.status_version,
                                            &inner.proc_status,
                                            inner.proc_exit_code,
                                            true,
                                        );
                                        super::publish_controller_status(broker, &status);
                                    }
                                    let req = serde_json::json!({
                                        "jsonrpc": "2.0",
                                        "id": id,
                                        "method": "session/prompt",
                                        "params": prompt_params(sid, &prompt),
                                    }).to_string();
                                    if let Some(ref tx) = inner.stdin_tx {
                                        if tx.try_send(req).is_err() {
                                            tracing::warn!(block_id = %block_id_stdout, "[acp] session/prompt send failed — channel full or closed");
                                            // Mirror send_input's enqueue-failure
                                            // rollback: this prompt was never
                                            // really sent, so it must not be
                                            // left occupying the outstanding set
                                            // or leave the turn marked active
                                            // when nothing else is genuinely
                                            // outstanding.
                                            let now_empty = {
                                                let mut outstanding = outstanding_prompt_ids_clone.lock().unwrap();
                                                outstanding.remove(&id);
                                                outstanding.is_empty()
                                            };
                                            if now_empty {
                                                health_clone.set_active_turn(false);
                                                if let Some(ref broker) = broker_clone {
                                                    let status = agent_runtime_status(
                                                        &block_id_stdout,
                                                        inner.status_version,
                                                        &inner.proc_status,
                                                        inner.proc_exit_code,
                                                        false,
                                                    );
                                                    super::publish_controller_status(broker, &status);
                                                }
                                            }
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
                            // this mirrors the careful path from persistent/stdout_reader.rs /
                            // subprocess/host_spawn.rs.
                            core::persist_session_id(&block_id_stdout, &sid_owned, &mstore_clone, &event_bus_clone);
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
                            // mirroring persistent/stdout_reader.rs's matching publish
                            // on its own normal (non-kill, non-exit)
                            // turn-end path. Without this, nothing
                            // publishes turn_active: false for an ACP
                            // controller (OpenClaw/Copilot/Pi in
                            // catalog.ts) until kill or process exit —
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
                            // status (#2338).
                            if let Some(ref broker) = broker_clone {
                                let status = {
                                    let locked = inner_clone.lock().unwrap();
                                    agent_runtime_status(
                                        &block_id_stdout,
                                        locked.status_version,
                                        &locked.proc_status,
                                        locked.proc_exit_code,
                                        false,
                                    )
                                };
                                super::publish_controller_status(broker, &status);
                            }
                        }
                    }
                }

                // Persist + broadcast via the shared helper (same as subprocess/host_spawn.rs)
                if recovered_error {
                    tracing::info!(block_id = %block_id_stdout, "[acp] session/load refused; opening a new session");
                } else if let Some(ref broker) = broker_clone {
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
        let outstanding_prompt_ids_wait = self.outstanding_prompt_ids.clone();
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
                    // resync_controller (mod.rs) can REUSE this exact
                    // AcpController instance across a kill+restart cycle
                    // (needs_replace false, status STATUS_DONE — the
                    // non-forced pane-reopen path, or after watchdog.rs's
                    // max-runtime/idle-timeout stop) — outstanding_prompt_ids
                    // is the SAME Arc across that reuse. Any prompt ids still
                    // outstanding at kill time will never get a response (no
                    // process left to answer), so without clearing here they
                    // would linger forever: the NEXT turn's own prompt id
                    // resolving would never empty the set, permanently
                    // stranding turn_active at true for the rest of this
                    // instance's lifetime. reagentx P1 on PR #2338
                    // (thirty-third re-review).
                    outstanding_prompt_ids_wait.lock().unwrap().clear();

                    if let Some(ref broker) = broker_wait {
                        let status = agent_runtime_status(&block_id_wait, 0, STATUS_DONE, -1, false);
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
                    // See the matching comment in the kill_rx branch above —
                    // same reasoning applies to a natural process exit.
                    outstanding_prompt_ids_wait.lock().unwrap().clear();

                    if let Some(ref broker) = broker_wait {
                        let status = agent_runtime_status(&block_id_wait, 0, STATUS_DONE, exit_code, false);
                        super::publish_controller_status(broker, &status);
                    }
                }
            }
        });

        // ACP v1 handshake: `initialize` only. Its result decides between
        // `session/new` and `session/load`; the stdout reader sends that.
        let init_id = self.next_id();
        let init_req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": init_id,
            "method": "initialize",
            "params": initialize_params(),
        })
        .to_string();
        let mut inner = self.inner.lock().unwrap();
        inner.init_request_id = Some(init_id);
        inner.session_cwd = working_dir.clone();
        if let Some(ref tx) = inner.stdin_tx {
            if tx.try_send(init_req).is_err() {
                tracing::error!(block_id = %self.block_id, "[acp] initialize dropped — channel full or closed; agent will not start");
            }
        }

        Ok(())
    }
}

/// A meta value that is a string array, or a JSON string encoding one.
fn meta_string_list(meta: &super::super::obj::MetaMapType, key: &str) -> Vec<String> {
    match meta.get(key) {
        Some(serde_json::Value::Array(values)) => values
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        Some(serde_json::Value::String(s)) => serde_json::from_str(s).unwrap_or_default(),
        _ => Vec::new(),
    }
}


impl AcpController {
    /// A user message from `agentinput` / `agent.send` (`run_agent_turn`):
    /// sent as `session/prompt`, then acknowledged the way the other
    /// controllers acknowledge theirs, so the pane stops showing it as pending.
    pub fn send_message(&self, message: String, message_id: Option<&str>) -> Result<(), String> {
        Controller::send_input(self, BlockInputUnion::data(message.into_bytes()), None)?;
        if let (Some(id), Some(broker)) = (message_id, self.broker.as_ref()) {
            super::publish_message_accepted(broker, &self.block_id, id);
        }
        Ok(())
    }

    /// The env this block's ACP process is spawned with: `cmd:env` (either
    /// shape), with the block's row UID + token carried onto it — identity
    /// M4b-2. Buys no attribution today (ACP agents are not given
    /// agentmux-mcp) but keeps the counters true.
    fn spawn_env(&self, block_meta: &super::super::obj::MetaMapType) -> HashMap<String, String> {
        let mut env_vars = meta_string_map(block_meta, super::META_KEY_CMD_ENV);
        if let Some(mstore) = &self.mstore {
            crate::server::agent_handlers::input::carry_block_identity_env(
                mstore,
                &self.block_id,
                &mut env_vars,
            );
        }
        // The workspace the "Your workspace" Operator Config entry points at.
        crate::server::agent_handlers::input::carry_agent_workdir_env(&mut env_vars, block_meta);
        crate::backend::gh_guard::apply_gh_guard(&mut env_vars);
        crate::backend::account_login_guard::strip_account_login(&mut env_vars);
        env_vars
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
        if cmd.is_empty() {
            return Err("ACP controller: no cmd specified in block meta".to_string());
        }

        // `cmd:args` / `cmd:env` as `agent.open` stores them (an array and an
        // object) or as a JSON string — before M4b-2 only the string form was
        // read, so an `agent.open` launch lost both (spec §6.5.8).
        let args = meta_string_list(&block_meta, super::META_KEY_CMD_ARGS);
        let resume = super::super::obj::meta_get_string(&block_meta, super::core::META_SESSION_ID, "");
        self.inner.lock().unwrap().resume_session_id = Some(resume).filter(|s| !s.is_empty());
        let mut env_vars = self.spawn_env(&block_meta);
        if let Some(pi) = crate::backend::providers::pi_beside_pi_acp(&cmd) {
            env_vars.entry("PI_ACP_PI_COMMAND".to_string()).or_insert(pi);
        }

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
                let mut inner = self.inner.lock().unwrap();
                match inner.session_id.clone() {
                    Some(sid) => sid,
                    None => {
                        // The handshake hasn't opened the session yet (the
                        // startup message is sent right after launch): queue
                        // it; the stdout reader sends it once the session
                        // exists. Sent with no session id, the agent would
                        // reject it and the message would be lost.
                        inner.pending_prompt = Some(match inner.pending_prompt.take() {
                            Some(earlier) => format!("{earlier}\n\n{message}"),
                            None => message,
                        });
                        return Ok(());
                    }
                }
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
                "params": prompt_params(&session_id, &message),
            }).to_string();
            // `mark_turn_active_returning_was_active` (not the plain
            // `set_active_turn(true)` this used to call) is atomic across
            // the read-and-write, so a mid-turn steering send racing this
            // one can't both observe "was idle" — see persistent/input.rs's
            // identical guard.
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
            self.health_monitor.mark_turn_active_returning_was_active();
            // Publish the turn_active flip, mirroring persistent/queue.rs's
            // send_message and persistent/input.rs's send_user_message
            // (which both call self.publish_status() right after the same
            // atomic turn-active mark). Without this,
            // wasTurnActive === false left over from an EARLIER turn's
            // end-of-turn publish (or the initial spawn publish) is
            // indistinguishable from genuine current idleness — a `/login`
            // that defers a controller refresh during THIS turn, followed
            // by a premature frontend Done transition, would see
            // isBackendTurnConfirmedIdle() read true from that stale
            // signal and force-restart the controller, killing the
            // actually-active turn (#2338).
            self.publish_status();

            let send_result = {
                let inner = self.inner.lock().unwrap();
                if let Some(ref tx) = inner.stdin_tx {
                    tx.try_send(req)
                        .map_err(|e| format!("ACP stdin send failed: {e}"))
                } else {
                    // stdin_tx can vanish between the is_running() check
                    // above and this re-lock — e.g. a concurrent stop() or
                    // controller resync clearing it in between. Treating
                    // this as success (the old behavior) silently lost the
                    // message (never actually enqueued anywhere) while
                    // still marking the turn active and leaving prompt_id
                    // permanently outstanding — nothing will ever resolve
                    // it. Handling it identically to a try_send failure
                    // reuses the existing rollback below. codex P1 on
                    // PR #2338 (thirty-fourth re-review).
                    Err("ACP process not running — stdin sender vanished after the initial check".to_string())
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
                // conditioned rollback, and a latest/previous-id
                // compare-and-swap that could still strand turn_active
                // under out-of-order responses (codex P2 on PR #2338,
                // twenty-seventh through thirtieth re-reviews).
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

    #[test]
    fn initialize_states_protocol_version_1_and_offers_no_fs_or_terminal() {
        let p = super::initialize_params();
        assert_eq!(p["protocolVersion"], 1);
        assert_eq!(p["clientCapabilities"]["fs"]["readTextFile"], false);
        assert_eq!(p["clientCapabilities"]["terminal"], false);
        assert_eq!(p["clientInfo"]["name"], "AgentMux");
    }

    #[test]
    fn a_session_is_loaded_only_when_the_agent_can_and_there_is_one() {
        let (m, p) = super::session_request(Some("s1"), true, "C:/w");
        assert_eq!(m, "session/load");
        assert_eq!(p, serde_json::json!({ "sessionId": "s1", "cwd": "C:/w", "mcpServers": [] }));
        assert_eq!(super::session_request(Some("s1"), false, "C:/w").0, "session/new");
        let (m, p) = super::session_request(None, true, "C:/w");
        assert_eq!(m, "session/new");
        assert_eq!(p, serde_json::json!({ "cwd": "C:/w", "mcpServers": [] }));
    }

    #[test]
    fn the_prompt_is_an_array_of_content_blocks() {
        assert_eq!(
            super::prompt_params("s1", "hi"),
            serde_json::json!({ "sessionId": "s1", "prompt": [{ "type": "text", "text": "hi" }] })
        );
    }

    #[test]
    fn permission_requests_are_approved_preferring_allow_always() {
        let req = |options: serde_json::Value| {
            serde_json::json!({ "jsonrpc": "2.0", "id": 7, "method": "session/request_permission",
                "params": { "sessionId": "s", "toolCall": { "toolCallId": "t" }, "options": options } })
        };
        let both = req(serde_json::json!([
            { "optionId": "once", "kind": "allow_once", "name": "Allow" },
            { "optionId": "always", "kind": "allow_always", "name": "Always" },
            { "optionId": "no", "kind": "reject_once", "name": "Reject" },
        ]));
        assert_eq!(
            super::reply_to_agent_request(&both).unwrap(),
            serde_json::json!({ "jsonrpc": "2.0", "id": 7, "result": { "outcome": { "outcome": "selected", "optionId": "always" } } })
        );
        let once = req(serde_json::json!([{ "optionId": "once", "kind": "allow_once", "name": "Allow" }]));
        assert_eq!(super::reply_to_agent_request(&once).unwrap()["result"]["outcome"]["optionId"], "once");
        let reject = req(serde_json::json!([{ "optionId": "no", "kind": "reject_once", "name": "Reject" }]));
        assert_eq!(
            super::reply_to_agent_request(&reject).unwrap()["result"]["outcome"],
            serde_json::json!({ "outcome": "cancelled" })
        );
    }

    #[test]
    fn other_agent_requests_get_method_not_found_and_notifications_get_nothing() {
        let fs = serde_json::json!({ "jsonrpc": "2.0", "id": "a", "method": "fs/read_text_file", "params": {} });
        let reply = super::reply_to_agent_request(&fs).unwrap();
        assert_eq!(reply["id"], "a");
        assert_eq!(reply["error"]["code"], -32601);
        let note = serde_json::json!({ "jsonrpc": "2.0", "method": "session/update", "params": {} });
        assert!(super::reply_to_agent_request(&note).is_none());
        let response = serde_json::json!({ "jsonrpc": "2.0", "id": 3, "result": {} });
        assert!(super::reply_to_agent_request(&response).is_none());
    }

    /// Identity M4b-2 (spec §6.5.8): `agent.open` stores `cmd:args` as an
    /// array and `cmd:env` as an object; ACP read only JSON strings, so
    /// such a launch lost both. Both shapes now read the same.
    /// Identity M4b-2 wiring (spec §6.5.8): an ACP controller with a store
    /// spawns a row-backed block with `agent.open`'s object `cmd:env` kept
    /// and the row's UID + token carried onto it.
    #[test]
    fn an_acp_spawn_env_keeps_cmd_env_and_carries_the_rows_identity() {
        use crate::backend::storage::agents::test_agent_def;
        let store =
            std::sync::Arc::new(crate::backend::storage::store::Store::open_in_memory().unwrap());
        let mut def = test_agent_def("uid-acp", "AcpAgent", "openclaw", "agent", 1, "");
        store.agent_def_insert(&mut def).unwrap();
        let mut meta = super::super::super::obj::MetaMapType::new();
        meta.insert("agentId".to_string(), serde_json::json!("uid-acp"));
        meta.insert("cmd:env".to_string(), serde_json::json!({"K": "v"}));
        let mut block = crate::backend::obj::Block {
            oid: "block-acp".to_string(),
            parentoref: String::new(),
            version: 0,
            runtimeopts: None,
            stickers: None,
            meta: meta.clone(),
            subblockids: None,
        };
        store.insert(&mut block).unwrap();
        let ctrl = AcpController::new(
            "tab".to_string(),
            "block-acp".to_string(),
            None,
            None,
            Some(store.clone()),
            None,
        );
        let env = ctrl.spawn_env(&meta);
        assert_eq!(env.get("K").map(String::as_str), Some("v"));
        assert_eq!(
            env.get("AGENTMUX_AGENT_UID").map(String::as_str),
            Some("uid-acp")
        );
        assert_eq!(
            env.get("AGENTMUX_AGENT_TOKEN"),
            store.agent_token_load("uid-acp").unwrap().as_ref()
        );
    }

    /// Plain-`gh` guard on the ACP spawn path: `GH_CONFIG_DIR` from `cmd:env`
    /// is replaced by the guard dir, not passed through.
    #[test]
    fn an_acp_spawn_env_overrides_gh_config_dir_with_the_guard_dir() {
        let mut meta = super::super::super::obj::MetaMapType::new();
        meta.insert(
            "cmd:env".to_string(),
            serde_json::json!({"GH_CONFIG_DIR": "/home/human/.config/gh", "AGENTMUX_AGENT_SLUG": "acpy"}),
        );
        let ctrl = AcpController::new("tab".to_string(), "block-acp-gh".to_string(), None, None, None, None);
        let env = ctrl.spawn_env(&meta);
        assert_eq!(
            std::path::PathBuf::from(&env["GH_CONFIG_DIR"]),
            crate::backend::gh_guard::guard_config_home().join("gh-acpy")
        );
    }

    /// A host ACP agent is told where its workspace is, like every other
    /// host spawn path (`carry_agent_workdir_env`).
    #[test]
    fn an_acp_spawn_env_carries_the_agent_workdir() {
        let ws = std::env::temp_dir().join("acpy-0930a");
        let mut meta = super::super::super::obj::MetaMapType::new();
        meta.insert("cmd:cwd".to_string(), serde_json::json!(ws.to_string_lossy()));
        meta.insert("cmd:env".to_string(), serde_json::json!({"AGENTMUX_AGENT_WORKDIR": "stale"}));
        let ctrl = AcpController::new("tab".to_string(), "block-acp-workdir".to_string(), None, None, None, None);
        assert_eq!(std::path::PathBuf::from(&ctrl.spawn_env(&meta)["AGENTMUX_AGENT_WORKDIR"]), ws);
    }

    /// No agent env carries the account's cloud login, even from a
    /// `cmd:env` persisted before `account_login_guard` existed.
    #[test]
    fn an_acp_spawn_env_never_carries_the_account_login() {
        let mut meta = super::super::super::obj::MetaMapType::new();
        meta.insert(
            "cmd:env".to_string(),
            serde_json::json!({"MUXBUS_TOKEN": "eyJ.stale", "MUXBUS_COGNITO_DOMAIN": "https://auth.example", "AGENTMUX_AGENT_SLUG": "acpy"}),
        );
        let ctrl = AcpController::new("tab".to_string(), "block-acp-login".to_string(), None, None, None, None);
        let env = ctrl.spawn_env(&meta);
        assert!(!env.contains_key("MUXBUS_TOKEN"), "{env:?}");
        assert!(!env.contains_key("MUXBUS_COGNITO_DOMAIN"), "{env:?}");
    }

    #[test]
    fn acp_reads_cmd_args_and_env_as_arrays_objects_or_json_strings() {
        let mut meta = super::super::super::obj::MetaMapType::new();
        meta.insert("cmd:args".to_string(), serde_json::json!(["--acp", "x"]));
        meta.insert("cmd:env".to_string(), serde_json::json!({"K": "v"}));
        assert_eq!(
            super::meta_string_list(&meta, "cmd:args"),
            vec!["--acp", "x"]
        );
        assert_eq!(
            super::meta_string_map(&meta, "cmd:env")
                .get("K")
                .map(String::as_str),
            Some("v")
        );

        meta.insert("cmd:args".to_string(), serde_json::json!("[\"--acp\"]"));
        meta.insert("cmd:env".to_string(), serde_json::json!("{\"K\":\"w\"}"));
        assert_eq!(super::meta_string_list(&meta, "cmd:args"), vec!["--acp"]);
        assert_eq!(
            super::meta_string_map(&meta, "cmd:env")
                .get("K")
                .map(String::as_str),
            Some("w")
        );

        assert!(super::meta_string_list(&meta, "absent").is_empty());
        assert!(super::meta_string_map(&meta, "absent").is_empty());
    }

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

    /// `send_input` marks the turn active via `mark_turn_active_returning_was_active`
    /// — pins the call-site contract that `turn_active` flips true once a
    /// message is actually sent to an already-running process.
    /// A message sent before the handshake has opened the session (the
    /// startup message, sent right after launch) is queued for the stdout
    /// reader to send once the session exists, never sent with an empty
    /// session id (#4447).
    #[tokio::test]
    async fn a_message_before_the_session_exists_is_queued_not_sent() {
        let c = controller();
        let (tx, mut rx) = mpsc::channel::<String>(8);
        c.inner.lock().unwrap().stdin_tx = Some(tx);
        assert!(c.send_input(BlockInputUnion::data(b"startup".to_vec()), None).is_ok());
        assert!(c.send_input(BlockInputUnion::data(b"second".to_vec()), None).is_ok());
        assert!(rx.try_recv().is_err(), "nothing goes to the agent before the session exists");
        assert_eq!(c.inner.lock().unwrap().pending_prompt.as_deref(), Some("startup\n\nsecond"));
        assert!(!c.health_monitor.is_active_turn());
    }

    #[tokio::test]
    async fn send_input_marks_the_turn_active() {
        let c = controller();
        // Simulate an already-running process — send_input only reaches the
        // turn-active logic when `is_running()` is true (otherwise it
        // stashes the message as `pending_prompt` for the next start()).
        let (tx, _rx) = mpsc::channel::<String>(8);
        {
            let mut inner = c.inner.lock().unwrap();
            inner.stdin_tx = Some(tx);
            inner.session_id = Some("s1".to_string());
        }

        assert!(!c.health_monitor.is_active_turn());
        let res = c.send_input(BlockInputUnion::data(b"hello".to_vec()), None);
        assert!(res.is_ok(), "send_input should succeed against a simulated running process, got {res:?}");
        assert!(c.health_monitor.is_active_turn(), "send_input must mark the turn active");
    }

    /// A second send while already active (mid-turn steering) must not
    /// error or panic — `mark_turn_active_returning_was_active`'s
    /// was-already-active branch is what gates re-arming; this at least
    /// confirms the call site doesn't choke on repeated calls the way a
    /// naive re-check-then-act would.
    #[tokio::test]
    async fn repeated_send_input_while_active_does_not_error() {
        let c = controller();
        let (tx, _rx) = mpsc::channel::<String>(8);
        {
            let mut inner = c.inner.lock().unwrap();
            inner.stdin_tx = Some(tx);
            inner.session_id = Some("s1".to_string());
        }

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
    /// actually mid-turn. Mirrors persistent/queue.rs's send_message and
    /// persistent/input.rs's send_user_message, which both call
    /// `publish_status()` right after the same atomic turn-active mark.
    #[tokio::test]
    async fn send_input_publishes_the_turn_active_flip() {
        let broker = Arc::new(mps::Broker::new());
        let c = AcpController::new(
            "tab".to_string(),
            "block-acp-publish".to_string(),
            Some(broker.clone()),
            None,
            None,
            None,
        );
        let (tx, _rx) = mpsc::channel::<String>(8);
        {
            let mut inner = c.inner.lock().unwrap();
            inner.stdin_tx = Some(tx);
            inner.session_id = Some("s1".to_string());
        }

        assert!(c.send_input(BlockInputUnion::data(b"hello".to_vec()), None).is_ok());

        let history = broker.read_event_history(
            mps::EVENT_CONTROLLER_STATUS,
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
        let broker = Arc::new(mps::Broker::new());
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
        {
            let mut inner = c.inner.lock().unwrap();
            inner.stdin_tx = Some(tx);
            inner.session_id = Some("s1".to_string());
        }

        let res = c.send_input(BlockInputUnion::data(b"hello".to_vec()), None);
        assert!(res.is_err(), "send_input should surface the enqueue failure, got {res:?}");
        assert!(!c.health_monitor.is_active_turn(), "must not leave the turn marked active when the enqueue itself failed");

        // The state is briefly marked active (established BEFORE the
        // enqueue attempt to close the twenty-sixth re-review's race), then
        // rolled back once the enqueue failure is discovered — so the
        // LATEST published status must reflect the rollback, not the
        // transient true a live subscriber may have also observed.
        let history = broker.read_event_history(
            mps::EVENT_CONTROLLER_STATUS,
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
        {
            let mut inner = c.inner.lock().unwrap();
            inner.stdin_tx = Some(tx);
            inner.session_id = Some("s1".to_string());
        }

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
