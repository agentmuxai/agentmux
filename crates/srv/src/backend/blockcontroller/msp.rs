// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! MspController: Muse Code over the Muse Session Protocol (MSP), `muse
//! serve`'s newline-delimited JSON-RPC 2.0 on stdio. Shaped like the ACP
//! controller (acp.rs): one long-lived process, one session in it, each user
//! message a turn, every line the host writes kept in the pane's transcript
//! for the frontend's `msp-translator.ts`. The protocol state lives in
//! `msp_session.rs`; this file spawns the process and moves lines.
//!
//! SPEC_NINE_NEW_HARNESSES_2026_10_14.md §4, §7.7.

#[cfg(windows)]
use agentmux_common::win32::NoWindow;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use super::health::TurnActivityTracker;
use super::msp_session::{MspSession, PromptOutcome};
use super::{
    agent_runtime_status, core, BlockControllerRuntimeStatus, BlockInputUnion, Controller, BLOCK_CONTROLLER_MSP,
    STATUS_DONE, STATUS_INIT, STATUS_RUNNING,
};
use crate::backend::eventbus::EventBus;
use crate::backend::mps;
use crate::backend::storage::filestore::FileStore;
use crate::backend::storage::store::Store;

/// MPS file subject the transcript is written to (the ACP controller's too).
const MSP_OUTPUT_SUBJECT: &str = "output";

/// How many of the host's last stderr lines are kept, for the error shown
/// when it exits mid-turn.
const STDERR_TAIL_LINES: usize = 12;

struct MspInner {
    proc_status: String,
    proc_exit_code: i32,
    status_version: i32,
    current_pid: Option<u32>,
    stdin_tx: Option<mpsc::Sender<String>>,
    kill_tx: Option<tokio::sync::oneshot::Sender<bool>>,
    /// Counts spawns, so a finished process's waiter leaves a newer one alone.
    generation: u64,
    session: MspSession,
    stderr_tail: VecDeque<String>,
}

/// Drives one `muse serve` process for an agent pane.
pub struct MspController {
    block_id: String,
    inner: Arc<Mutex<MspInner>>,
    broker: Option<Arc<mps::Broker>>,
    event_bus: Option<Arc<EventBus>>,
    mstore: Option<Arc<Store>>,
    filestore: Option<Arc<FileStore>>,
    health_monitor: Arc<TurnActivityTracker>,
    /// Serializes `ensure_started`, so two deliveries can't both start it.
    start_lock: Mutex<()>,
    /// srv's auth key, for the agent's `agentmux-mcp` (see the ACP controller).
    auth_key: String,
}

/// What the reader, waiter and senders share: enough to publish the turn's
/// state and write lines to the transcript.
#[derive(Clone)]
struct Outlet {
    block_id: String,
    inner: Arc<Mutex<MspInner>>,
    broker: Option<Arc<mps::Broker>>,
    filestore: Option<Arc<FileStore>>,
    health: Arc<TurnActivityTracker>,
    zone: Option<String>,
}

impl Outlet {
    fn write(&self, line: &str) {
        if let Some(ref broker) = self.broker {
            let data = format!("{line}\n");
            super::shell::handle_append_block_file(
                broker,
                &self.block_id,
                MSP_OUTPUT_SUBJECT,
                data.as_bytes(),
                self.filestore.as_ref(),
                self.zone.as_deref(),
            );
        }
    }

    fn set_turn(&self, inner: &MspInner, active: bool) {
        publish_turn(&self.block_id, self.broker.as_ref(), &self.health, inner, active);
    }
}

/// Set the turn's state and publish it, so the pane and the fleet see a turn
/// start and end as it happens.
fn publish_turn(block_id: &str, broker: Option<&Arc<mps::Broker>>, health: &TurnActivityTracker, inner: &MspInner, active: bool) {
    if active {
        health.mark_turn_active_returning_was_active();
    } else {
        health.set_active_turn(false);
    }
    if let Some(broker) = broker {
        let status = agent_runtime_status(block_id, inner.status_version, &inner.proc_status, inner.proc_exit_code, active);
        super::publish_controller_status(broker, &status);
    }
}

impl MspController {
    pub fn new(
        block_id: String,
        broker: Option<Arc<mps::Broker>>,
        event_bus: Option<Arc<EventBus>>,
        mstore: Option<Arc<Store>>,
        filestore: Option<Arc<FileStore>>,
    ) -> Self {
        let health_monitor = Arc::new(TurnActivityTracker::for_block(block_id.clone(), broker.as_ref()));
        Self {
            block_id,
            inner: Arc::new(Mutex::new(MspInner {
                proc_status: STATUS_INIT.to_string(),
                proc_exit_code: 0,
                status_version: 0,
                current_pid: None,
                stdin_tx: None,
                kill_tx: None,
                generation: 0,
                session: MspSession::new(),
                stderr_tail: VecDeque::new(),
            })),
            broker,
            event_bus,
            mstore,
            filestore,
            health_monitor,
            start_lock: Mutex::new(()),
            auth_key: String::new(),
        }
    }

    /// The server auth key `agentmux-mcp` needs to reach srv.
    pub fn with_auth_key(mut self, auth_key: String) -> Self {
        self.auth_key = auth_key;
        self
    }

    fn outlet(&self) -> Outlet {
        Outlet {
            block_id: self.block_id.clone(),
            inner: self.inner.clone(),
            broker: self.broker.clone(),
            filestore: self.filestore.clone(),
            health: self.health_monitor.clone(),
            zone: super::shell::resolve_global_output_zone(&self.mstore, &self.block_id),
        }
    }

    fn set_turn(&self, inner: &MspInner, active: bool) {
        publish_turn(&self.block_id, self.broker.as_ref(), &self.health_monitor, inner, active);
    }

    fn set_status(inner: &mut MspInner, status: &str) {
        inner.proc_status = status.to_string();
        inner.status_version += 1;
    }

    fn publish_status(&self) {
        if let Some(ref broker) = self.broker {
            super::publish_controller_status(broker, &self.get_status_snapshot());
        }
    }

    fn get_status_snapshot(&self) -> BlockControllerRuntimeStatus {
        let inner = self.inner.lock().unwrap();
        agent_runtime_status(&self.block_id, inner.status_version, &inner.proc_status, inner.proc_exit_code, self.health_monitor.is_active_turn())
    }

    pub(crate) fn is_running(&self) -> bool {
        self.inner.lock().unwrap().stdin_tx.is_some()
    }

    /// Start the agent again if its process has exited; a no-op while it runs.
    pub fn ensure_started(&self, block_meta: super::super::obj::MetaMapType) -> Result<(), String> {
        let _one_at_a_time = self.start_lock.lock().unwrap();
        if self.is_running() {
            return Ok(());
        }
        Controller::start(self, block_meta, None, false)
    }

    /// A user message from `agentinput` / `agent.send`, acknowledged as the
    /// other controllers acknowledge theirs.
    pub fn send_message(&self, message: String, message_id: Option<&str>) -> Result<(), String> {
        Controller::send_input(self, BlockInputUnion::data(message.into_bytes()), None)?;
        if let (Some(id), Some(broker)) = (message_id, self.broker.as_ref()) {
            super::publish_message_accepted(broker, &self.block_id, id);
        }
        Ok(())
    }

    /// Spawn `muse serve` and send `initialize`; the stdout reader carries
    /// the handshake on (`MspSession`).
    fn spawn_process(&self, cli_command: String, cli_args: Vec<String>, launch: Launch) -> Result<(), String> {
        let Launch { working_dir, env_vars, mcp_servers, resume } = launch;
        let mut cmd = crate::server::cli_handlers::make_cli_cmd(&cli_command);
        cmd.args(&cli_args);
        core::apply_working_dir(&mut cmd, &self.block_id, &working_dir, &env_vars);
        crate::backend::identity_spawn::record_process_spawn(&self.block_id, crate::backend::identity_spawn::SpawnPath::Acp, &env_vars);
        #[cfg(windows)]
        {
            cmd.no_window();
        }
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let mut child = crate::backend::process_tracker::registry::spawn_tracked(&self.block_id, &mut cmd, crate::backend::process_tracker::registry::Join::Agent)
            .map_err(|e| format!("failed to spawn Muse Code: {e}"))?;
        let pid = child.id().unwrap_or(0);
        tracing::info!(block_id = %self.block_id, pid, cmd = %cli_command, args = ?cli_args, "[msp] muse serve spawned");

        let stdin = child.stdin.take().ok_or("[msp] stdin not captured")?;
        let stdout = child.stdout.take().ok_or("[msp] stdout not captured")?;
        let stderr = child.stderr.take();
        let (kill_tx, kill_rx) = tokio::sync::oneshot::channel::<bool>();
        let (msg_tx, mut msg_rx) = mpsc::channel::<String>(64);
        let (generation, init) = {
            let mut inner = self.inner.lock().unwrap();
            inner.generation += 1;
            inner.current_pid = Some(pid);
            inner.kill_tx = Some(kill_tx);
            inner.stdin_tx = Some(msg_tx.clone());
            inner.stderr_tail.clear();
            Self::set_status(&mut inner, STATUS_RUNNING);
            let init = inner.session.begin(super::acp::session_cwd(&working_dir), mcp_servers, resume);
            (inner.generation, init)
        };
        self.health_monitor.set_active_turn(false);
        self.publish_status();

        if let Some(stderr) = stderr {
            let (block_id, inner) = (self.block_id.clone(), self.inner.clone());
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if line.trim().is_empty() {
                        continue;
                    }
                    tracing::warn!(block_id = %block_id, line = %line, "[msp] muse serve stderr");
                    let mut inner = inner.lock().unwrap();
                    if inner.stderr_tail.len() == STDERR_TAIL_LINES {
                        inner.stderr_tail.pop_front();
                    }
                    inner.stderr_tail.push_back(line);
                }
            });
        }

        let block_id_stdin = self.block_id.clone();
        tokio::spawn(async move {
            let mut stdin = tokio::io::BufWriter::new(stdin);
            while let Some(line) = msg_rx.recv().await {
                let written = async {
                    stdin.write_all(line.as_bytes()).await?;
                    stdin.write_all(b"\n").await?;
                    stdin.flush().await
                };
                if let Err(e) = written.await {
                    tracing::error!(block_id = %block_id_stdin, error = %e, "[msp] stdin write error");
                    break;
                }
            }
        });

        let outlet = self.outlet();
        let (mstore, event_bus) = (self.mstore.clone(), self.event_bus.clone());
        let reader = tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if line.trim().is_empty() {
                    continue;
                }
                let Ok(json) = serde_json::from_str::<serde_json::Value>(&line) else {
                    outlet.write(&line);
                    continue;
                };
                let fx = {
                    let mut inner = outlet.inner.lock().unwrap();
                    let fx = inner.session.on_message(&json);
                    if let Some(ref tx) = inner.stdin_tx {
                        for msg in &fx.send {
                            if tx.try_send(msg.clone()).is_err() {
                                tracing::warn!(block_id = %outlet.block_id, "[msp] a message to muse serve was dropped — channel full or closed");
                            }
                        }
                    }
                    if let Some(active) = fx.turn_active {
                        outlet.set_turn(&inner, active);
                    }
                    fx
                };
                if let Some(ref sid) = fx.session {
                    tracing::info!(block_id = %outlet.block_id, session_id = %sid, "[msp] session open");
                    core::persist_session_id(&outlet.block_id, sid, &mstore, &event_bus);
                }
                if !fx.hide {
                    outlet.write(&line);
                }
                for extra in &fx.append {
                    outlet.write(extra);
                }
            }
            tracing::info!(block_id = %outlet.block_id, "[msp] stdout closed");
        });

        let outlet = self.outlet();
        tokio::spawn(async move {
            // The exit code when the process ended by itself; `None` when
            // `stop` killed it.
            let exited = tokio::select! {
                _ = kill_rx => {
                    let _ = child.kill().await;
                    None
                }
                status = child.wait() => Some(status.map(|s| s.code().unwrap_or(-1)).unwrap_or(-1)),
            };
            if exited.is_some() {
                // Its last lines (a final `turn/completed`) go first.
                let _ = tokio::time::timeout(std::time::Duration::from_secs(2), reader).await;
            }
            let mut inner = outlet.inner.lock().unwrap();
            if inner.generation != generation {
                return;
            }
            inner.stdin_tx = None;
            inner.current_pid = None;
            let exit_code = exited.unwrap_or(-1);
            if exited.is_some() {
                inner.proc_exit_code = exit_code;
            }
            Self::set_status(&mut inner, STATUS_DONE);
            let had_work = inner.session.process_ended();
            let tail: Vec<String> = inner.stderr_tail.iter().cloned().collect();
            outlet.set_turn(&inner, false);
            drop(inner);
            tracing::info!(block_id = %outlet.block_id, exit_code, killed = exited.is_none(), "[msp] muse serve ended");
            // Exited by itself with work in hand: say why, and end the turn.
            if exited.is_some() && had_work {
                outlet.write(&super::msp_protocol::process_exit_error(exit_code, &tail));
                outlet.write(super::msp_protocol::TURN_ENDED_BY_SRV);
            }
        });

        if msg_tx.try_send(init).is_err() {
            tracing::error!(block_id = %self.block_id, "[msp] initialize dropped; the agent will not start");
        }
        Ok(())
    }
}

/// Where and how a process is started, read from the pane's meta.
struct Launch {
    working_dir: String,
    env_vars: HashMap<String, String>,
    mcp_servers: serde_json::Map<String, serde_json::Value>,
    /// The pane's previous session (`agent:sessionid`).
    resume: Option<String>,
}

impl Controller for MspController {
    fn start(&self, block_meta: super::super::obj::MetaMapType, _rt_opts: Option<serde_json::Value>, _force: bool) -> Result<(), String> {
        use super::super::obj::meta_get_string;
        let cmd = meta_get_string(&block_meta, super::META_KEY_CMD, "");
        if cmd.is_empty() {
            return Err("MSP controller: no cmd specified in block meta".to_string());
        }
        let working_dir = meta_get_string(&block_meta, super::META_KEY_CMD_CWD, "");
        let args = super::acp::meta_string_list(&block_meta, super::META_KEY_CMD_ARGS);
        let resume = Some(meta_get_string(&block_meta, core::META_SESSION_ID, "")).filter(|s| !s.is_empty());
        let env_vars = super::acp::session_agent_env(&block_meta, &self.block_id, &self.mstore, &self.auth_key);
        // AgentMux's MCP servers: the `.mcp.json` the launch wrote where the
        // session runs, passed in `session/start`.
        let mcp_path = std::path::Path::new(&super::acp::session_cwd(&working_dir)).join(".mcp.json");
        let mcp_servers = std::fs::read_to_string(mcp_path).map_or_else(
            |_| serde_json::Map::new(),
            |raw| {
                let path = env_vars.get("PATH").map(String::as_str);
                let lookup = |k: &str| env_vars.get(k).cloned().or_else(|| std::env::var(k).ok());
                super::msp_protocol::msp_mcp_servers(&raw, |c| super::acp_mcp::resolve_command(c, path), lookup)
            },
        );
        self.spawn_process(cmd, args, Launch { working_dir, env_vars, mcp_servers, resume })
    }

    fn stop(&self, _graceful: bool, _new_status: &str) -> Result<(), String> {
        // Closing stdin ends `muse serve`'s sessions in order; the kill makes
        // sure it is gone.
        let kill_tx = {
            let mut inner = self.inner.lock().unwrap();
            inner.stdin_tx = None;
            Self::set_status(&mut inner, STATUS_DONE);
            inner.kill_tx.take()
        };
        if let Some(tx) = kill_tx {
            let _ = tx.send(true);
        }
        self.health_monitor.set_active_turn(false);
        self.publish_status();
        Ok(())
    }

    fn get_runtime_status(&self) -> BlockControllerRuntimeStatus {
        self.get_status_snapshot()
    }

    fn send_input(&self, input: BlockInputUnion, _seq: Option<u64>) -> Result<(), String> {
        if let Some(data) = input.input_data {
            let message = String::from_utf8_lossy(&data).to_string();
            if message.trim().is_empty() {
                return Ok(());
            }
            let mut inner = self.inner.lock().unwrap();
            let Some(tx) = inner.stdin_tx.clone() else {
                // Not running: kept for the next start's session.
                let _ = inner.session.prompt(&message);
                return Err("Muse Code is not running — message queued for next start()".to_string());
            };
            match inner.session.prompt(&message)? {
                PromptOutcome::Send { id, line } => {
                    // Marked before the send: the lock is held until it is
                    // queued, so the reader can't report the turn's end first.
                    self.set_turn(&inner, true);
                    if let Err(e) = tx.try_send(line) {
                        if inner.session.prompt_unsent(id) {
                            self.set_turn(&inner, false);
                        }
                        return Err(format!("Muse Code stdin send failed: {e}"));
                    }
                }
                PromptOutcome::Queued { reopen } => {
                    self.set_turn(&inner, true);
                    if let Some(line) = reopen {
                        let _ = tx.try_send(line);
                    }
                }
            }
        }
        if let Some(sig) = input.sig_name {
            // Stop the running turn when there is one; otherwise, as for an
            // ACP agent, stop the agent.
            if sig == "SIGINT" {
                let mut inner = self.inner.lock().unwrap();
                if let (Some(line), Some(tx)) = (inner.session.interrupt(), inner.stdin_tx.clone()) {
                    return tx.try_send(line).map_err(|e| format!("Muse Code stdin send failed: {e}"));
                }
            }
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
        BLOCK_CONTROLLER_MSP
    }

    fn block_id(&self) -> &str {
        &self.block_id
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
#[path = "msp_tests.rs"]
mod tests;
