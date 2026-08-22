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
//!
//! ## Module layout
//!
//! This module was split from a single ~1770-line `subprocess.rs` file into a
//! directory so each piece of the (large, deliberately-not-decomposed-further)
//! turn state machines could live in its own file:
//!   - `mod.rs` (this file): spawn config, inner state, controller struct +
//!     bookkeeping (`new`/lock helpers/status snapshot/publish), and
//!     `impl Controller for SubprocessController`.
//!   - `session`: session-id capture/hydration state machine.
//!   - `host_spawn`: `spawn_turn` — host-subprocess turn via `std::process`.
//!   - `container_spawn`: `spawn_container_turn` — Docker-exec turn via
//!     `bollard`, plus its `publish_line` output helper.
//!   - `tests` (cfg(test)): unit tests, mostly session-id focused.
//!
//! `host_spawn` and `container_spawn` are `impl SubprocessController` blocks
//! in their own files — Rust allows multiple `impl <Type>` blocks across
//! files/modules for the same type. Each turn method is moved WHOLE into its
//! file rather than decomposed further: both are continuous, non-trivially-
//! ordered state machines where splitting the body would make the control
//! flow harder to follow, not easier.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use super::health::HealthMonitor;
use super::{BlockControllerRuntimeStatus, BlockInputUnion, Controller, STATUS_INIT};
use crate::backend::eventbus::EventBus;
use crate::backend::storage::filestore::FileStore;
use crate::backend::storage::store::Store;
use crate::backend::wps;

mod argv;
mod container_spawn;
mod host_spawn;
mod session;
#[cfg(test)]
mod tests;

/// WPS file subject name for subprocess output (replaces "term" from PTY).
pub const SUBPROCESS_OUTPUT_SUBJECT: &str = "output";

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
    /// Resume argv strategy: `none`, `flag`, or `codex-exec`.
    /// Empty preserves compatibility with blocks created before this metadata
    /// existed by inferring `flag` from a non-empty `resume_flag`.
    pub resume_strategy: String,
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
    /// The registry's `instance_id` (`block.meta["agentId"]`) for this
    /// agent — the cross-process session-lease key (see
    /// `registry::LeaseStore`; NOT `session_id` above, which can be
    /// `None` on a greenfield first turn). Empty string disables
    /// leasing for this spawn (e.g. the container-mode branch in this
    /// PR — see `host_spawn::spawn_turn`'s own doc comment).
    pub instance_id: String,
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
    #[allow(dead_code)]
    tab_id: String,
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
    /// Cross-process session-ownership lease store. `None` when the
    /// shared registry can't be resolved (CI / unusual envs) — leasing
    /// degrades to a no-op in that case, same convention as
    /// `shared_agent_registry()` elsewhere. See
    /// `docs/retro/RETRO_DEV_BUILD_SHARED_AGENT_SESSION_COLLISION_2026_07_29.md`.
    lease_store: Option<Arc<crate::registry::LeaseStore>>,
    /// This process's boot id — the lease owner id. See `AppState::boot_id`.
    boot_id: Arc<str>,
}

impl SubprocessController {
    /// Create a new SubprocessController.
    ///
    /// `registry`/`boot_id` back the cross-process session-ownership
    /// lease (see `lease_store` field doc). Passing `registry: None`
    /// (or a registry whose root can't be opened as a `LeaseStore`,
    /// which is logged and degrades the same way) disables leasing
    /// for every turn this controller spawns — used by test call
    /// sites that don't need a real registry.
    pub fn new(
        tab_id: String,
        block_id: String,
        broker: Option<Arc<wps::Broker>>,
        event_bus: Option<Arc<EventBus>>,
        wstore: Option<Arc<Store>>,
        filestore: Option<Arc<FileStore>>,
        registry: Option<Arc<crate::registry::Registry>>,
        boot_id: Arc<str>,
    ) -> Self {
        let health_monitor = Arc::new(HealthMonitor::new(
            block_id.clone(),
            broker.clone(),
            wstore.clone(),
            event_bus.clone(),
        ));
        let lease_store = registry.and_then(|r| {
            crate::registry::LeaseStore::open(r.root())
                .map(Arc::new)
                .map_err(|e| {
                    tracing::warn!(
                        block_id = %block_id,
                        error = %e,
                        "subprocess controller: failed to open lease store — leasing disabled for this controller"
                    );
                })
                .ok()
        });
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
            lease_store,
            boot_id,
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

    /// Build a `BlockControllerRuntimeStatus` snapshot from inner state.
    ///
    /// Factored out of 4+ verbatim-duplicated struct literals (one here, one
    /// in `host_spawn`'s process_waiter, two in `container_spawn`'s
    /// running/done/failure transitions) — an audit finding from the
    /// modularization pass. `turn_active` is passed explicitly because call
    /// sites disagree on its value: `get_status_snapshot` reads the live
    /// health-monitor flag, while every turn-completion / exec-failure site
    /// always publishes `false` (the turn is over by construction at that
    /// point).
    fn build_status_snapshot(
        inner: &SubprocessControllerInner,
        block_id: &str,
        turn_active: bool,
    ) -> BlockControllerRuntimeStatus {
        BlockControllerRuntimeStatus {
            blockid: block_id.to_string(),
            version: inner.status_version,
            shellprocstatus: inner.proc_status.clone(),
            shellprocconnname: "local".to_string(),
            shellprocexitcode: inner.proc_exit_code,
            spawn_ts_ms: None,
            is_agent_pane: false,
            turn_active,
        }
    }

    /// Get the runtime status (snapshot).
    fn get_status_snapshot(&self) -> BlockControllerRuntimeStatus {
        let inner = self.inner.lock().unwrap();
        Self::build_status_snapshot(&inner, &self.block_id, self.health_monitor.is_active_turn())
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

    fn health_monitor(&self) -> Option<Arc<HealthMonitor>> {
        Some(Arc::clone(&self.health_monitor))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
