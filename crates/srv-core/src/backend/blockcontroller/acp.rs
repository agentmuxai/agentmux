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

use crate::backend::message_origin::RecordOrigin;
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

/// Written to the output after an error that ended the turn (no prompt left
/// running, or the session couldn't open). The translator ends a turn on a
/// `stopReason`; it can't tell from an error alone, which may answer only a
/// mid-turn steering prompt while the first one still runs.
const TURN_ENDED_BY_ERROR: &str = r#"{"jsonrpc":"2.0","result":{"stopReason":"error"}}"#;

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

/// How an agent takes an earlier session back, from its `initialize` answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SessionRestore {
    /// Neither: every start is a new session.
    No,
    /// `session/load` (`agentCapabilities.loadSession`): the agent replays the conversation first.
    Load,
    /// `session/resume` (`agentCapabilities.sessionCapabilities.resume`, dsh): the conversation comes back
    /// with no replay.
    Resume,
}

impl SessionRestore {
    /// The way an agent that said `can_load` and/or `can_resume` takes a session back. A replay is
    /// preferred when both are offered, as before `session/resume` was supported.
    pub(super) fn of(can_load: bool, can_resume: bool) -> Self {
        if can_load {
            Self::Load
        } else if can_resume {
            Self::Resume
        } else {
            Self::No
        }
    }
}

/// `session/new`, or `session/load` / `session/resume` of `resume` when the agent can take one back,
/// with the agent's MCP servers (`acp_mcp`).
fn session_request(
    resume: Option<&str>,
    restore: SessionRestore,
    cwd: &str,
    mcp_servers: &[serde_json::Value],
) -> (&'static str, serde_json::Value) {
    let method = match restore {
        SessionRestore::Load => "session/load",
        SessionRestore::Resume => "session/resume",
        SessionRestore::No => "session/new",
    };
    match resume.filter(|_| restore != SessionRestore::No) {
        Some(sid) => (method, serde_json::json!({ "sessionId": sid, "cwd": cwd, "mcpServers": mcp_servers })),
        None => ("session/new", serde_json::json!({ "cwd": cwd, "mcpServers": mcp_servers })),
    }
}

/// Whether a line from the agent is part of a `session/load` replay: the
/// agent streams the earlier conversation as `session/update`s before it
/// answers the load, and the load's own answer is not one of them.
fn is_session_replay(loading: bool, is_session_response: bool) -> bool {
    loading && !is_session_response
}

/// The absolute directory ACP's `session/new` and `session/load` take as
/// `cwd`: the pane's working directory (`~` expanded; a relative one resolved
/// against the server's, as `Command::current_dir` resolves it), else the
/// directory the agent process itself starts in.
pub(super) fn session_cwd(working_dir: &str) -> String {
    let here = std::env::current_dir().unwrap_or_default();
    let dir = if working_dir.is_empty() {
        here.clone()
    } else {
        let expanded = std::path::PathBuf::from(super::core::expand_home_dir(working_dir));
        if expanded.is_absolute() { expanded } else { here.join(expanded) }
    };
    dir.to_string_lossy().into_owned()
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
    // A request carries an `id`, which ACP lets be null and requires us to
    // echo; a notification has none.
    let id = json.get("id")?.clone();
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
    /// `session/new` was refused (pi: "Authentication required"): no session
    /// will open until the agent restarts.
    session_failed: bool,
    /// Counts spawns. A process's waiter cleans up the shared state only
    /// while its generation is current, so one that finishes after a restart
    /// can't disconnect the new process.
    generation: u64,
    /// The pane's previous session (`agent:sessionid`), offered to
    /// `session/load` when the agent supports it.
    resume_session_id: Option<String>,
    /// The session's working directory.
    session_cwd: String,
    /// The `.mcp.json` the launch wrote in that directory, read at start.
    mcp_json: Option<String>,
    /// Its servers in ACP's shape, for what `initialize` said the agent takes.
    mcp_servers: Vec<serde_json::Value>,
    /// The agent's `cmd:env`, which `.mcp.json`'s `${VAR}` placeholders expand from
    /// (then srv's own environment, which the agent inherits).
    mcp_env: HashMap<String, String>,
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
    /// Serializes `ensure_started`, so two deliveries that find the agent
    /// stopped can't both start it.
    start_lock: Mutex<()>,
    /// srv's auth key, for the agent's `agentmux-mcp` (`with_auth_key`):
    /// srv removes it from its own environment at startup, so nothing else
    /// would pass it on. Empty in tests.
    auth_key: String,
}

impl AcpController {
    /// The server auth key `agentmux-mcp` needs to reach srv (the same key
    /// `build_persistent_spawn_env` gives Claude's).
    pub fn with_auth_key(mut self, auth_key: String) -> Self {
        self.auth_key = auth_key;
        self
    }

    pub fn new(
        tab_id: String,
        block_id: String,
        broker: Option<Arc<mps::Broker>>,
        event_bus: Option<Arc<EventBus>>,
        mstore: Option<Arc<Store>>,
        filestore: Option<Arc<FileStore>>,
    ) -> Self {
        let health_monitor = Arc::new(TurnActivityTracker::for_block(block_id.clone(), broker.as_ref()));
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
                session_failed: false,
                generation: 0,
                resume_session_id: None,
                session_cwd: String::new(),
                mcp_json: None,
                mcp_servers: Vec::new(),
                mcp_env: HashMap::new(),
            })),
            broker,
            event_bus,
            mstore,
            filestore,
            health_monitor,
            next_rpc_id: Arc::new(AtomicU64::new(1)),
            outstanding_prompt_ids: Arc::new(Mutex::new(HashSet::new())),
            start_lock: Mutex::new(()),
            auth_key: String::new(),
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
        .spawned_at(self.health_monitor.spawned_at_ms())
    }

    fn publish_status(&self) {
        if let Some(ref broker) = self.broker {
            let status = self.get_status_snapshot();
            super::publish_controller_status(broker, &status);
        }
    }

    pub(crate) fn is_running(&self) -> bool {
        let inner = self.inner.lock().unwrap();
        inner.stdin_tx.is_some()
    }

    /// Ends the agent's session so the next message opens a new one (`Agents fresh`): the process is stopped and the
    /// pane's saved session id forgotten, so the launch that follows sends `session/new` and not `session/resume`.
    /// Only between turns. Returns the old session id.
    pub fn start_fresh_session(&self) -> Result<Option<String>, &'static str> {
        let old = {
            let mut inner = self.inner.lock().unwrap();
            if self.health_monitor.is_active_turn() {
                return Err("a turn is running");
            }
            inner.resume_session_id = None;
            inner.session_id.take()
        };
        core::persist_session_id(&self.block_id, "", &self.mstore, &self.event_bus);
        self.stop(true, STATUS_DONE).map_err(|_| "the agent's process could not be stopped")?;
        Ok(old)
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
        let (mut child, pid) = self.launch_child(&cli_command, &cli_args, &working_dir, &env_vars)?;

        let (kill_tx, kill_rx) = tokio::sync::oneshot::channel::<bool>();
        let stdin = child.stdin.take()
            .ok_or_else(|| format!("[acp] stdin not captured for block {}", self.block_id))?;
        let stdout = child.stdout.take()
            .ok_or_else(|| format!("[acp] stdout not captured for block {}", self.block_id))?;
        let stderr = child.stderr.take();

        if let Some(stderr_pipe) = stderr {
            self.spawn_stderr_drain(stderr_pipe);
        }
        let msg_rx = self.register_running(pid, kill_tx);
        self.spawn_stdin_writer(stdin, msg_rx);
        self.spawn_stdout_reader(stdout);
        self.spawn_process_waiter(child, kill_rx);
        self.send_initialize(&working_dir);

        Ok(())
    }
}

/// A meta value that is a string array, or a JSON string encoding one.
pub(super) fn meta_string_list(meta: &super::super::obj::MetaMapType, key: &str) -> Vec<String> {
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
    /// A new process takes over the controller: the generation moves on, and
    /// the prompts the previous process never answered are forgotten. Its
    /// waiter, which sees the new generation, no longer clears them; left
    /// there they would keep the turn active for good.
    fn begin_process(&self) -> u64 {
        self.outstanding_prompt_ids.lock().unwrap().clear();
        self.health_monitor.set_active_turn(false);
        let mut inner = self.inner.lock().unwrap();
        inner.generation += 1;
        inner.generation
    }

    /// Start the agent again if its process has exited, from the pane's
    /// meta; a no-op when it is running. One start at a time: a second
    /// delivery waits here and then finds it running.
    pub fn ensure_started(&self, block_meta: super::super::obj::MetaMapType) -> Result<(), String> {
        let _one_at_a_time = self.start_lock.lock().unwrap();
        if self.is_running() {
            return Ok(());
        }
        Controller::start(self, block_meta, None, false)
    }

    /// A user message from `agentinput` / `agent.send` (`run_agent_turn`):
    /// sent as `session/prompt`, then acknowledged the way the other
    /// controllers acknowledge theirs, so the pane stops showing it as pending.
    pub fn send_message(&self, message: String, message_id: Option<&str>) -> Result<(), String> {
        self.send_message_from(message, message_id, None)
    }

    /// [`send_message`](Self::send_message), saying where the message came from. ACP puts the user's
    /// words in no frame of its own, so srv stores them as a user record with that origin
    /// (`prompt_record`) once the agent has them; with no `record` nothing is stored.
    pub fn send_message_from(&self, message: String, message_id: Option<&str>, record: Option<&RecordOrigin>) -> Result<(), String> {
        let words = record.map(|_| message.clone());
        Controller::send_input(self, BlockInputUnion::data(message.into_bytes()), None)?;
        if let (Some(id), Some(broker)) = (message_id, self.broker.as_ref()) {
            super::publish_message_accepted(broker, &self.block_id, id);
        }
        if let Some(words) = words {
            let records = super::prompt_record::PromptRecords { broker: self.broker.clone(), filestore: self.filestore.clone(), mstore: self.mstore.clone() };
            records.write(&self.block_id, &words, message_id.is_some(), record);
        }
        Ok(())
    }

    /// The env this block's ACP process is spawned with: `cmd:env` (either
    /// shape), with the block's row UID + token carried onto it — identity
    /// M4b-2. agentmux-mcp, which ACP agents get in `session/new`
    /// (`acp_mcp`), runs as the agent's child and so carries it.
    fn spawn_env(&self, block_meta: &super::super::obj::MetaMapType) -> HashMap<String, String> {
        session_agent_env(block_meta, &self.block_id, &self.mstore, &self.auth_key)
    }
}

/// The env a session agent's process (ACP or MSP) is spawned with: see
/// `AcpController::spawn_env`.
pub(super) fn session_agent_env(
    block_meta: &super::super::obj::MetaMapType,
    block_id: &str,
    mstore: &Option<Arc<Store>>,
    auth_key: &str,
) -> HashMap<String, String> {
    let mut env_vars = meta_string_map(block_meta, super::META_KEY_CMD_ENV);
    if let Some(mstore) = mstore {
        crate::backend::spawn_env::block_identity::carry_block_identity_env(mstore, block_id, &mut env_vars);
    }
    // The workspace the "Your workspace" Operator Config entry points at.
    crate::backend::spawn_env::block_identity::carry_agent_workdir_env(&mut env_vars, block_meta);
    // The agent's `agentmux-mcp` reaches srv with these, as on the
    // persistent path: reserved values, so they replace any in `cmd:env`.
    if auth_key.is_empty() {
        env_vars.remove("AGENTMUX_AUTH_KEY");
    } else {
        env_vars.insert("AGENTMUX_AUTH_KEY".to_string(), auth_key.to_string());
    }
    env_vars.insert("AGENTMUX_BLOCKID".to_string(), block_id.to_string());
    crate::backend::spawn_env::workspaces::record_launch_workspace(block_id, &env_vars);
    // The tool dirs ahead of the system PATH, as on the persistent path: the build-slot shims
    // live there, and an ACP agent's `cargo test` must reach them.
    crate::backend::tool_store::prepend_agent_tools_path(&mut env_vars);
    crate::backend::gh_guard::apply_gh_guard(&mut env_vars);
    crate::backend::account_login_guard::strip_account_login(&mut env_vars);
    env_vars
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
        {
            // A restart (the agent exited) starts a new handshake: a prompt
            // must wait for the new session, never go to the old one.
            let mut inner = self.inner.lock().unwrap();
            inner.resume_session_id = Some(resume).filter(|s| !s.is_empty());
            inner.session_id = None;
            inner.init_request_id = None;
            inner.session_request_id = None;
            inner.loading_session = None;
            inner.session_failed = false;
        }
        // AgentMux's MCP servers: the config the launch wrote for this agent, which is
        // outside the working directory when that is the user's own project (#3680).
        // None when there is none.
        let mcp_json = crate::backend::mcp_config_file::read_for_turn(&session_cwd(&cwd));
        let mut env_vars = self.spawn_env(&block_meta);
        if let Some(pi) = crate::backend::providers::pi_beside_pi_acp(&cmd) {
            env_vars.entry("PI_ACP_PI_COMMAND".to_string()).or_insert(pi);
        }
        let provider = super::super::obj::meta_get_string(&block_meta, "agentProvider", "");
        super::acp_prelaunch::prepare_env(&provider, &mut env_vars);
        super::acp_prelaunch::prepare(&provider, &env_vars, &session_cwd(&cwd));
        super::acp_prelaunch::check_runtime(&provider, &env_vars)?;
        {
            let mut inner = self.inner.lock().unwrap();
            inner.mcp_json = mcp_json;
            inner.mcp_env = env_vars.clone();
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

            // The turn is reserved in the same lock hold that reads the session id, so `start_fresh_session`, which
            // checks for an idle turn under this lock, either sees the turn as active and refuses, or runs first and
            // leaves no session here, so the message below waits for the next launch (#47458).
            let (session_id, prompt_id) = {
                let mut inner = self.inner.lock().unwrap();
                match inner.session_id.clone() {
                    Some(sid) => {
                        // Not built via make_request: the id must be captured so it can be tracked as outstanding
                        // (see outstanding_prompt_ids's doc comment). Inserted before the turn is marked active
                        // and before the enqueue below, so the prompt's answer always finds both.
                        let prompt_id = self.next_id();
                        self.outstanding_prompt_ids.lock().unwrap().insert(prompt_id);
                        // `mark_turn_active_returning_was_active` (not the plain `set_active_turn(true)`) is atomic
                        // across the read-and-write, so a mid-turn steering send racing this one can't both observe
                        // "was idle" (see persistent/input.rs's identical guard). Established before the stdin
                        // enqueue, not after: a fast-responding agent can have the stdout reader observe and
                        // publish the resulting `stopReason` on another worker before this thread would otherwise
                        // reach the mark (#2338).
                        self.health_monitor.mark_turn_active_returning_was_active();
                        (sid, prompt_id)
                    }
                    None if inner.session_failed => {
                        return Err("The agent couldn't open a session (see the error above). Fix that, then restart the agent.".to_string());
                    }
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
            let req = serde_json::json!({
                "jsonrpc": "2.0",
                "id": prompt_id,
                "method": "session/prompt",
                "params": prompt_params(&session_id, &message),
            }).to_string();
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

    fn turn_tracker(&self) -> Option<Arc<TurnActivityTracker>> {
        Some(Arc::clone(&self.health_monitor))
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

#[path = "acp_spawn.rs"]
pub mod spawn_steps;

#[cfg(test)]
#[path = "acp_tests.rs"]
mod tests;
