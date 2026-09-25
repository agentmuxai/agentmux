// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Block controller: manages lifecycle of each block (terminal, command, web app).
//! Port of Go's pkg/blockcontroller/blockcontroller.go.

//!
//! Architecture:
//! - Global controller registry maps block_id → Controller
//! - Each controller manages the lifecycle of one block
//! - ShellController handles "shell" and "cmd" block types
//! - Controllers dispatch I/O between the user and the process/service

pub mod acp;
pub mod agent_lock;
pub mod app_server;
pub mod app_server_controller;
pub mod app_server_protocol;
pub mod core;
pub mod health;
pub mod persistent;
mod persistent_resume;
pub mod pidregistry;
pub mod process_tree;
pub mod session_recovery;
pub mod session_stats;
pub mod shell;
pub mod subprocess;
pub mod watchdog;

use std::any::Any;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};

use super::eventbus::EventBus;
use super::obj::{Block, MetaMapType, TermSize};
use super::storage::filestore::FileStore;
use super::storage::store::Store;
use super::mps::Broker;

// ---- Controller status constants (match Go) ----

pub const STATUS_INIT: &str = "init";
pub const STATUS_RUNNING: &str = "running";
pub const STATUS_DONE: &str = "done";

/// Sentinel error string `resync_controller` returns when it declines to
/// respawn a `STATUS_DONE` controller because its caller passed
/// `respawn_if_done: false` (see that parameter's own doc comment). Callers
/// that want to treat "already exited" differently from a genuine failure
/// (e.g. `try_attach_to_existing_shell` falling through to a fresh shell
/// instead of surfacing a 500) match on this exact string rather than
/// parsing every other possible error message `resync_controller` can
/// return.
pub const RESYNC_ERR_ALREADY_EXITED: &str = "resync_controller: controller already exited (respawn suppressed)";

// ---- Controller type constants (match Go) ----

pub const BLOCK_CONTROLLER_SHELL: &str = "shell";
pub const BLOCK_CONTROLLER_CMD: &str = "cmd";
pub const BLOCK_CONTROLLER_TSUNAMI: &str = "tsunami";
pub const BLOCK_CONTROLLER_SUBPROCESS: &str = "subprocess";
pub const BLOCK_CONTROLLER_PERSISTENT: &str = "persistent";
pub const BLOCK_CONTROLLER_ACP: &str = "acp";
pub const BLOCK_CONTROLLER_APP_SERVER: &str = "app-server";

// ---- Block metadata key constants (match Go) ----

pub const META_KEY_CONTROLLER: &str = "controller";
pub const META_KEY_CONNECTION: &str = "connection";
pub const META_KEY_CMD: &str = "cmd";
pub const META_KEY_CMD_CWD: &str = "cmd:cwd";
#[allow(dead_code)]
pub const META_KEY_CMD_SHELL: &str = "cmd:shell";
pub const META_KEY_CMD_ARGS: &str = "cmd:args";
pub const META_KEY_CMD_ENV: &str = "cmd:env";
#[allow(dead_code)]
pub const META_KEY_CMD_JWT: &str = "cmd:jwt";
pub const META_KEY_CMD_RUN_ON_START: &str = "cmd:runonstart";
pub const META_KEY_CMD_RUN_ONCE: &str = "cmd:runonce";
pub const META_KEY_CMD_CLEAR_ON_START: &str = "cmd:clearonstart";
pub const META_KEY_CMD_CLOSE_ON_EXIT: &str = "cmd:closeonexit";
pub const META_KEY_CMD_CLOSE_ON_EXIT_FORCE: &str = "cmd:closeonexitforce";
pub const META_KEY_CMD_CLOSE_ON_EXIT_DELAY: &str = "cmd:closeonexitdelay";
#[allow(dead_code)]
pub const META_KEY_CMD_INIT_SCRIPT: &str = "cmd:initscript";
#[allow(dead_code)]
pub const META_KEY_CMD_INIT_SCRIPT_BASH: &str = "cmd:initscript.bash";
#[allow(dead_code)]
pub const META_KEY_CMD_INIT_SCRIPT_ZSH: &str = "cmd:initscript.zsh";
#[allow(dead_code)]
pub const META_KEY_CMD_INIT_SCRIPT_FISH: &str = "cmd:initscript.fish";
#[allow(dead_code)]
pub const META_KEY_CMD_INIT_SCRIPT_PWSH: &str = "cmd:initscript.pwsh";
#[allow(dead_code)]
pub const META_KEY_TERM_LOCAL_SHELL_PATH: &str = "term:localshellpath";
#[allow(dead_code)]
pub const META_KEY_TERM_LOCAL_SHELL_OPTS: &str = "term:localshellopts";

// ---- Default timeouts ----

/// Default controller operation timeout in milliseconds.
#[allow(dead_code)]
pub const DEFAULT_TIMEOUT_MS: u64 = 2000;

/// Grace period before forceful kill in milliseconds.
#[allow(dead_code)]
pub const DEFAULT_GRACEFUL_KILL_WAIT_MS: u64 = 400;

// ---- Input union (matches Go's BlockInputUnion) ----

/// Input sent to a block controller.
/// Can be raw terminal data, a signal, or a resize event.
#[derive(Debug, Clone)]
pub struct BlockInputUnion {
    /// Raw terminal input bytes (base64 decoded from wire format).
    pub input_data: Option<Vec<u8>>,
    /// Signal name (e.g., "SIGTERM", "SIGINT").
    pub sig_name: Option<String>,
    /// Terminal resize event.
    pub term_size: Option<TermSize>,
}

impl BlockInputUnion {
    pub fn data(data: Vec<u8>) -> Self {
        Self {
            input_data: Some(data),
            sig_name: None,
            term_size: None,
        }
    }

    pub fn signal(name: &str) -> Self {
        Self {
            input_data: None,
            sig_name: Some(name.to_string()),
            term_size: None,
        }
    }

    pub fn resize(size: TermSize) -> Self {
        Self {
            input_data: None,
            sig_name: None,
            term_size: Some(size),
        }
    }
}

fn is_false(v: &bool) -> bool {
    !v
}

// ---- Runtime status (matches Go's BlockControllerRuntimeStatus) ----

/// Runtime status of a block controller, sent to the UI.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BlockControllerRuntimeStatus {
    pub blockid: String,
    #[serde(default)]
    pub version: i32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub shellprocstatus: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub shellprocconnname: String,
    #[serde(default)]
    pub shellprocexitcode: i32,
    /// Unix timestamp (ms) when the process was spawned; None until first spawn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawn_ts_ms: Option<i64>,
    /// PID of the controller's own child process. Set by the shell
    /// controller, which is the one the UI needs it for: the agent pane's
    /// Shell drawer identifies its shell by PID (see
    /// docs/specs/SPEC_AGENT_SHELL_DRAWER_INFO_PANEL_2026_09_19.md §4.1).
    /// None before the first spawn, and for controllers that don't set it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shellprocpid: Option<u32>,
    /// Program name of that child (`pwsh`, `bash`, …). The process tracker
    /// can't supply it — it omits each block's root process by design, and
    /// for a shell sub-block the root IS the shell.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub shellprocname: String,
    /// True if this pane is running an agent CLI (e.g. claude, codex, gemini, kimi, openclaw, pi).
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_agent_pane: bool,
    /// True if a turn is currently in flight (message sent, no terminating
    /// `"result"` event observed yet). For `PersistentSubprocessController`
    /// this is the only signal that distinguishes "actively generating a
    /// turn" from "process alive, idle between turns" — `shellprocstatus`
    /// stays `"running"` for the whole process lifetime either way. Backed
    /// by `TurnActivityTracker.active_turn` (see `blockcontroller/health.rs`).
    /// Frontend seeds `TurnPhase` from this at mount instead of always
    /// defaulting to `Idle` — see
    /// docs/specs/REPORT_AGENT_PANE_STATE_RECONCILIATION_2026_07_07.md
    /// Finding 1.
    #[serde(default, skip_serializing_if = "is_false")]
    pub turn_active: bool,
}

// ---- Controller trait ----

/// How a [`Controller::shutdown`] ended. Logged per closed agent
/// (spec §9.8) — a provider that is often `Killed` needs a better shutdown or
/// a longer grace period.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopOutcome {
    /// No process was running (lazy controller never spawned, or already gone).
    NotRunning,
    /// Stopped the immediate way (default impl) — no wait, no verdict.
    Stopped,
    /// The process exited on its own before the deadline.
    Exited,
    /// The process was still alive at the deadline and was killed.
    Killed,
}

impl StopOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NotRunning => "not_running",
            Self::Stopped => "stopped",
            Self::Exited => "exited",
            Self::Killed => "killed",
        }
    }
}

pub type ShutdownFuture = std::pin::Pin<Box<dyn std::future::Future<Output = StopOutcome> + Send>>;

/// Grace period for closing agents: one deadline per close, shared by every
/// agent it stops (spec §9.2). Matches the kill task's long-standing 5s.
pub const SHUTDOWN_GRACE: std::time::Duration = std::time::Duration::from_secs(5);

/// Trait for block controllers. Each block type has its own implementation.
/// Port of Go's `blockcontroller.Controller` interface.
pub trait Controller: Send + Sync {
    /// Start the controller. May spawn background tasks.
    /// `force` restarts even if already running.
    fn start(
        &self,
        block_meta: MetaMapType,
        rt_opts: Option<serde_json::Value>,
        force: bool,
    ) -> Result<(), String>;

    /// Stop the controller.
    /// `graceful` waits for process to exit; `new_status` is the target state.
    fn stop(&self, graceful: bool, new_status: &str) -> Result<(), String>;

    /// Stop this controller because it's being REPLACED by a new one for
    /// the same block (session restart / resync's `needs_replace` path),
    /// NOT because the block itself is being closed. Must terminate this
    /// controller's own CLI process so it doesn't linger, but — unlike
    /// `stop()` — must NOT reach any declared-background descendant the
    /// process may have spawned (e.g. a `task dev` instance launched via
    /// `run_in_background: true`); those must survive the replace. See
    /// docs/specs/SPEC_BACKGROUND_TASK_TEARDOWN_SURVIVAL_2026_08_20.md.
    ///
    /// Default delegates to `stop()` — correct for any controller type
    /// with no subprocess tree of its own to be careful about (nothing to
    /// preserve, so the two calls are equivalent). Only `ShellController`
    /// overrides this today.
    fn stop_for_replace(&self, new_status: &str) -> Result<(), String> {
        self.stop(true, new_status)
    }

    /// Stop gracefully because the block is being CLOSED: end any turn in
    /// progress, ask the process to exit, force-kill at `deadline`. The
    /// returned future resolves once the process has actually exited (or
    /// there was none). Does NOT drop the block's process tracker — the
    /// caller does that after this resolves, so nothing the agent started
    /// outlives it. See
    /// docs/specs/SPEC_AGENT_PANE_CLOSE_GRACEFUL_SHUTDOWN_2026_09_18.md §9.3.
    ///
    /// Default: today's immediate stop. Right for controllers with no agent
    /// session worth ending cleanly (shells, cmd, per-turn subprocess) and for
    /// providers whose exit behaviour hasn't been measured yet (ACP,
    /// app-server — §9.3 requires that before they get a graceful path).
    fn shutdown(&self, _deadline: std::time::Instant) -> ShutdownFuture {
        let _ = self.stop(true, STATUS_DONE);
        Box::pin(async { StopOutcome::Stopped })
    }

    /// Get the current runtime status.
    fn get_runtime_status(&self) -> BlockControllerRuntimeStatus;

    /// Send input (terminal data, signal, or resize) to the controller.
    /// `seq` is the per-TermViewModel monotonic counter; `None` means fire-and-forget (no ordering).
    fn send_input(&self, input: BlockInputUnion, seq: Option<u64>) -> Result<(), String>;

    /// Get the controller type (e.g., "shell", "cmd").
    fn controller_type(&self) -> &str;

    /// Get the block ID.
    #[allow(dead_code)]
    fn block_id(&self) -> &str;

    /// This block's own live, spawn-time-captured jekt/muxbus identity, if
    /// it has one — an independent source of truth for the recipient-
    /// identity check in `ReactiveHandler::inject_message_inner`, deliberately
    /// NOT derived from `ReactiveHandler`'s own `agent_to_block`/`agent_info`
    /// maps (checking a registry against itself would be a tautology and
    /// catch nothing). Default `None` for controller types that aren't
    /// jekt-addressable at all (e.g. plain terminals) — only `ShellController`
    /// and `PersistentSubprocessController` currently override this, mirroring
    /// the two paths that already resolve a jekt-registration identity at
    /// spawn time (`resolve_agent_id_for_jekt`, `muxbus_agent_id_from_env`).
    fn agent_id(&self) -> Option<String> {
        None
    }

    /// Refresh this block's own captured jekt/muxbus identity (see
    /// [`agent_id`](Controller::agent_id)'s doc comment). Called whenever
    /// `ReactiveHandler::register_agent`/`register_agent_with_nonce`
    /// (re-)registers THIS block's block_id under a (possibly different)
    /// agent_id, so the two independently-written copies never drift apart
    /// (reagentx P1 on #2697: `agent_id()` was captured once at spawn and
    /// never refreshed, while `agent_to_block` gets re-keyed on every
    /// `register_agent` call — e.g. `handle_reactive_register`'s
    /// frontend-initiated HTTP path — causing a legitimately renamed or
    /// reconfigured agent's own messages to be falsely rejected as an
    /// identity mismatch). Default no-op for controller types that don't
    /// override [`agent_id`](Controller::agent_id) either.
    fn set_agent_id(&self, _id: Option<String>) {}

    /// This block's STABLE jekt identity — `AGENTMUX_AGENT_ID` as captured
    /// once at spawn, never refreshed. Distinct from [`agent_id`](Controller::agent_id),
    /// which tracks the LIVE, renameable display name and is deliberately
    /// refreshed every turn (see that method's doc comment on #2697).
    /// AGENTMUX_AGENT_ID is embedded verbatim in PR-body tags specifically
    /// because it does NOT change if the agent is later renamed — but
    /// `ReactiveHandler`'s registry only tracks one key per block, and
    /// every turn's `agent_id()` refresh evicts whatever this block was
    /// previously registered under. `ReactiveHandler::inject_message_inner`'s
    /// recipient-identity check (#2695) uses this alongside `agent_id()` so
    /// a jekt addressed to the stable ID is validated correctly even after
    /// the primary registry key has moved on to the live display name. See
    /// `INCIDENT_2026_09_09_JEKT_STABLE_ID_ALIAS.md`. Default `None`,
    /// mirroring `agent_id`'s default — only `PersistentSubprocessController`
    /// currently overrides this.
    fn stable_agent_id(&self) -> Option<String> {
        None
    }
    /// Identity M2: this block's UID (`db_agents.id`) as carried in its
    /// spawn env (`AGENTMUX_AGENT_UID`, set by `build_persistent_spawn_env`
    /// since M1a), captured once at spawn like [`stable_agent_id`]. Backs the
    /// registry's UID confirmer: consulted only for targets that resolved by
    /// UID, and `None` — a registered-but-not-yet-spawned controller, or a
    /// block with no row — is "unverifiable", not a mismatch. Default
    /// `None`; only `PersistentSubprocessController` overrides this.
    fn stable_agent_uid(&self) -> Option<String> {
        None
    }

    /// Downcast support for concrete controller types.
    fn as_any(&self) -> &dyn Any;
}

// ---- Global controller registry ----

/// Thread-safe global controller registry.
/// Maps block_id → Arc<dyn Controller>.
static CONTROLLER_REGISTRY: std::sync::LazyLock<RwLock<HashMap<String, Arc<dyn Controller>>>> =
    std::sync::LazyLock::new(|| RwLock::new(HashMap::new()));

/// Get a controller by block ID.
pub fn get_controller(block_id: &str) -> Option<Arc<dyn Controller>> {
    CONTROLLER_REGISTRY.read().unwrap().get(block_id).cloned()
}

/// Register a controller, stopping any previous one for the same block.
pub fn register_controller(block_id: &str, controller: Arc<dyn Controller>) {
    let mut registry = CONTROLLER_REGISTRY.write().unwrap();
    if let Some(old) = registry.remove(block_id) {
        // Stop the old controller before replacing
        let _ = old.stop(true, STATUS_DONE);
    }
    registry.insert(block_id.to_string(), controller);
    drop(registry);
    notify_tracked_blocks_changed();
}

/// Announce a `CONTROLLER_REGISTRY` membership change to the Process Broker,
/// if one is running (never set in most unit tests). See
/// `ProcessBroker::emit_tracked_blocks_changed`'s doc comment for why this
/// exists: `agent.tracked-blocks` reads this registry directly, but nothing
/// used to tell a subscriber when a write here changed what that read would
/// return — Swarm's client-side list depended on two unrelated, independently-
/// timed events instead, and went stale whenever neither happened to fire for
/// a given block. See docs/reports/REPORT_SWARM_MOUNT_DEPENDENT_TRACKING_GAP_2026_09_15.md.
fn notify_tracked_blocks_changed() {
    if let Some(broker) = crate::broker::process::global() {
        broker.emit_tracked_blocks_changed();
    }
}

/// Remove a controller from `CONTROLLER_REGISTRY` only — does NOT touch the
/// process tracker or the Process Broker's cached status, unlike
/// `delete_controller` below. For `resync_controller`'s replace path
/// (session restart), where a NEW controller for the same block is about
/// to be registered right after this call: `AgentProcessRegistry::
/// ensure_tracker` is idempotent by design ("the job survives controller
/// re-creation" — its own doc comment), so leaving the tracker alone here
/// means any declared-background descendant (e.g. `task dev`) the old
/// controller's process spawned stays alive and gets reattached to the new
/// controller's own `track_spawned` call, instead of dying with the old
/// one. The caller is responsible for actually terminating the OLD
/// controller's own process first (via `stop_for_replace`, not `stop`) —
/// this function only ever touches the registry map. See
/// docs/specs/SPEC_BACKGROUND_TASK_TEARDOWN_SURVIVAL_2026_08_20.md.
fn remove_controller_entry_only(block_id: &str) {
    CONTROLLER_REGISTRY.write().unwrap().remove(block_id);
}

/// Blocks whose pane is being closed. Between stopping a block's
/// controller and deleting its record, a `resync_controller` (frontend
/// ControllerResync, `agent.open`, ...) would otherwise find the block
/// still present and spawn a fresh process for it — an orphan the close
/// never sees. See docs/specs/SPEC_AGENT_PANE_CLOSE_GRACEFUL_SHUTDOWN_2026_09_18.md
/// §4.2 step 1.
///
/// Each entry also carries a completion signal (`true` once the block's
/// process has exited), so a reopen racing the close can wait for the old
/// process instead of starting a second `--resume` on the same session
/// (spec §9.6).
static CLOSING_BLOCKS: std::sync::LazyLock<RwLock<HashMap<String, tokio::sync::watch::Sender<bool>>>> =
    std::sync::LazyLock::new(|| RwLock::new(HashMap::new()));

/// Refuse to (re)spawn a controller for `block_id` until [`unmark_closing`].
pub fn mark_closing(block_id: &str) {
    CLOSING_BLOCKS
        .write()
        .unwrap()
        .entry(block_id.to_string())
        .or_insert_with(|| tokio::sync::watch::channel(false).0);
}

/// Signal that a closing block's process has exited. The block stays marked
/// closing (no respawn) until [`unmark_closing`].
pub fn mark_closing_stopped(block_id: &str) {
    if let Some(tx) = CLOSING_BLOCKS.read().unwrap().get(block_id) {
        tx.send_replace(true);
    }
}

/// Lift [`mark_closing`]. Called once the block's record is gone (or the
/// close failed and the block stays). Wakes anyone still waiting.
pub fn unmark_closing(block_id: &str) {
    if let Some(tx) = CLOSING_BLOCKS.write().unwrap().remove(block_id) {
        tx.send_replace(true);
    }
}

pub fn is_closing(block_id: &str) -> bool {
    CLOSING_BLOCKS.read().unwrap().contains_key(block_id)
}

/// Blocks mid-close whose process has not exited yet. Their controllers have
/// already left `CONTROLLER_REGISTRY`, so a registry scan can't see them.
pub fn closing_blocks_still_running() -> Vec<String> {
    CLOSING_BLOCKS
        .read()
        .unwrap()
        .iter()
        .filter(|(_, stopped)| !*stopped.borrow())
        .map(|(id, _)| id.clone())
        .collect()
}

/// Wait until a closing block's process has exited, or `timeout`. Returns
/// immediately for a block that isn't closing.
pub async fn wait_closing_stopped(block_id: &str, timeout: std::time::Duration) {
    let rx = CLOSING_BLOCKS.read().unwrap().get(block_id).map(|tx| tx.subscribe());
    if let Some(mut rx) = rx {
        let _ = tokio::time::timeout(timeout, rx.wait_for(|stopped| *stopped)).await;
    }
}

/// Unregister (delete) a controller by block ID, stopping it first.
/// Removes from the registry before calling stop() so no new callers can reach it.
pub fn delete_controller(block_id: &str) {
    let ctrl = take_controller(block_id);
    if let Some(ctrl) = ctrl {
        let _ = ctrl.stop(true, STATUS_DONE);
    }
    release_block_processes(block_id);
}

/// Remove a controller from `CONTROLLER_REGISTRY` and hand it back, without
/// stopping it — for the graceful close, which awaits
/// [`Controller::shutdown`] and only then calls [`release_block_processes`].
/// Once removed, no new input reaches it.
pub fn take_controller(block_id: &str) -> Option<Arc<dyn Controller>> {
    CONTROLLER_REGISTRY.write().unwrap().remove(block_id)
}

/// Drop the block's process tracker (killing anything left in its tree) and
/// its Process Broker entry. The second half of [`delete_controller`]; a
/// graceful close calls it only after the process has exited.
pub fn release_block_processes(block_id: &str) {
    // Drop the process tracker for this block. On Windows the job
    // object's `KILL_ON_JOB_CLOSE` flag nukes the whole descendant
    // tree; on Linux/macOS the tracker's `Drop` does the same.
    // No-op if the tracker global isn't initialized.
    if let Some(registry) = crate::backend::process_tracker::registry::global() {
        registry.remove(block_id);
    }
    // Drop the Process Broker's cached status for this block, so a closed
    // pane's stale entry doesn't linger unreachable (get_all_controllers()
    // already won't list it — see ProcessBroker::forget's doc comment).
    if let Some(broker) = crate::broker::process::global() {
        broker.forget(block_id);
        broker.emit_tracked_blocks_changed();
    }
    // Identity M4a: the block's "spawned with a token" record goes with it.
    crate::backend::identity_spawn::forget_block(block_id);
}

// ---- Close-on-exit handler (SPEC_TERM_EXIT_RESPAWN_LOOP_2026_09_15.md §10) ----

/// `shell/lifecycle.rs`'s PTY wait/cleanup task needs to actually CLOSE a
/// pane (delete its block, prune it from the owning tab's layout tree, and
/// notify any connected frontend) when the shell inside it exits — but that
/// task lives deep in `backend::blockcontroller`, which has no `AppState`
/// handle (only the individual `Store`/`EventBus`/`Broker` pieces
/// `ShellController` was constructed with). The actual close logic
/// (`crate::sagas::delete_block::run`) needs the full `AppState` — its
/// reducer state, saga-id allocator, and saga log, not just those three
/// pieces. Rather than threading `AppState` through every
/// `ShellController::new`/`resync_controller` call site (a much larger,
/// architecture-inverting change), this mirrors the exact shape
/// `crate::broker::process::set_global`/`global` already use for the
/// identical class of problem (`broker::process`'s `AppState`-derived
/// `ProcessBroker` also needs to be reachable from deep in `blockcontroller`
/// without a direct dependency): a process-wide callback slot, set once at
/// boot from `bootstrap::install_close_on_exit_handler` (which has
/// `AppState` in scope) and read from `shell/lifecycle.rs`.
type CloseOnExitFn = Arc<dyn Fn(String, String) + Send + Sync>;

static CLOSE_ON_EXIT_HANDLER: std::sync::OnceLock<CloseOnExitFn> = std::sync::OnceLock::new();

/// Install the close-on-exit callback. Called exactly once, from
/// `bootstrap::install_close_on_exit_handler` right after `AppState` is
/// built in `main.rs` — see that function's doc comment for what the
/// callback actually does. A second call is a no-op (`OnceLock::set`
/// silently returns `Err` on an already-initialized cell, which this
/// discards — matching `broker::process::set_global`'s identical
/// fire-and-forget style; there is exactly one legitimate caller).
pub fn set_close_on_exit_handler(f: CloseOnExitFn) {
    let _ = CLOSE_ON_EXIT_HANDLER.set(f);
}

/// Trigger the installed close-on-exit handler for `(tab_id, block_id)`, if
/// one has been installed. Fire-and-forget: the handler owns spawning
/// whatever async work it needs (the saga call is `async`; this function
/// itself is not). A missing handler (astronomically early in boot, before
/// `main.rs` finishes wiring `AppState` — no real caller can reach this
/// before then, since nothing spawns a PTY that early) just skips closing;
/// the pane sits inert the same way it did before this feature existed, not
/// a hard failure.
pub(crate) fn close_on_exit(tab_id: String, block_id: String) {
    match CLOSE_ON_EXIT_HANDLER.get() {
        Some(f) => f(tab_id, block_id),
        None => {
            tracing::warn!(
                block_id = %block_id,
                "close_on_exit: handler not installed yet, skipping pane close"
            );
        }
    }
}

/// Get all controllers (snapshot).
pub fn get_all_controllers() -> HashMap<String, Arc<dyn Controller>> {
    CONTROLLER_REGISTRY.read().unwrap().clone()
}

/// Stop all running controllers gracefully.
#[allow(dead_code)]
pub fn stop_all_controllers() {
    let controllers = get_all_controllers();
    for (_, ctrl) in controllers {
        let _ = ctrl.stop(true, STATUS_DONE);
    }
}

// ---- Public API functions ----

/// Get the runtime status for a block's controller.
/// Returns None if no controller is registered.
pub fn get_block_controller_status(block_id: &str) -> Option<BlockControllerRuntimeStatus> {
    get_controller(block_id).map(|c| c.get_runtime_status())
}

/// Stop a block's controller gracefully.
#[allow(dead_code)]
pub fn stop_block_controller(block_id: &str) -> Result<(), String> {
    match get_controller(block_id) {
        Some(ctrl) => ctrl.stop(true, STATUS_DONE),
        None => Ok(()), // No controller = already stopped
    }
}

/// Send input to a block's controller.
pub fn send_input(block_id: &str, input: BlockInputUnion, seq: Option<u64>) -> Result<(), String> {
    match get_controller(block_id) {
        Some(ctrl) => ctrl.send_input(input, seq),
        None => Err(format!("no controller for block {block_id}")),
    }
}

/// When a message may be written to a running agent's live input.
///
/// The distinction exists because "deliver this to the agent" means two
/// different things depending on who is asking. A human typing into their own
/// agent's pane is deliberately interrupting it, and that is correct. An
/// automated sender — a GitHub/ReAgent notification, a CI result, another
/// agent's coordination ping — is not asking to interrupt anything, and
/// writing it mid-turn makes the agent abandon whatever it was explaining.
///
/// Spec: `docs/specs/SPEC_NO_MIDTURN_DELIVERY_2026_09_23.md` §4.1. This
/// replaces `InjectionRequest::wait_for_idle`, which named this distinction
/// but was never read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DeliverPolicy {
    /// Write to live stdin now, even mid-turn. Reserved for the human
    /// operator's own deliberate action.
    Immediate,
    /// Queue if a turn is in flight; deliver at the next turn boundary, one
    /// message per boundary. The default for every automated sender.
    #[default]
    NextIdle,
}

/// What a [`DeliverPolicy`] send actually did with one message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendOutcome {
    /// Written to the agent's input now.
    Sent,
    /// Queued behind the turn in flight (or older queued messages); released
    /// at a later turn boundary.
    Deferred,
}

/// How a controller-aware agent message was delivered.
#[derive(Debug)]
pub enum AgentDelivery {
    /// Accepted on the controller's structured input channel — a persistent
    /// stream-json stdin line or an ACP `session/prompt`. No PTY keystrokes are
    /// needed.
    ///
    /// "Accepted", not necessarily "already written": under
    /// [`DeliverPolicy::NextIdle`] a message arriving mid-turn is held and
    /// released at the next turn boundary. This variant means the controller
    /// has taken responsibility for it, not that the agent has seen it yet —
    /// [`AgentDelivery::StructuredDeferred`] says when it is still waiting.
    Structured,
    /// Accepted like [`AgentDelivery::Structured`], but held for the next
    /// turn boundary because the agent is mid-turn (`DeliverPolicy::NextIdle`).
    StructuredDeferred,
    /// The controller is PTY/terminal-based (shell/term) or otherwise has no
    /// structured input channel. The caller should fall back to keystroke
    /// injection.
    Pty,
}

/// Deliver an inter-agent / muxbus message to a running agent the way its controller
/// expects.
///
/// Callers of this function are automated senders by definition — muxbus, the
/// reactive/jekt handler, MCP `SendMessage`, the messaging bridges. Delivery is
/// therefore [`DeliverPolicy::NextIdle`]: if the agent is mid-turn the message
/// waits for the next turn boundary instead of cutting its explanation in half.
/// Spec: `docs/specs/SPEC_NO_MIDTURN_DELIVERY_2026_09_23.md`. The human
/// operator's own input does not come through here — it goes via the
/// `agentinput` RPC — so it is unaffected.
///
/// - **Persistent** (stream-json) agents have no PTY: the message is written as a
///   `{type:"user",…}` line on the live stdin, queued behind any in-flight turn.
/// - **ACP** agents receive the message as a `session/prompt` (the ACP controller's
///   `send_input` already wraps raw input that way).
/// - **App Server** (Codex) agents have no PTY either: the message is queued/dispatched
///   as a `turn/start` request via [`app_server_controller::AppServerController::send_message`].
/// - Everything else (shell/term PTY agents, one-shot subprocess agents) is reported
///   as [`AgentDelivery::Pty`] so the caller uses keystroke injection — preserving
///   today's behavior.
///
/// This is the controller-aware delivery primitive muxbus Tier-1 needs: PTY
/// keystrokes silently fail to reach a persistent stream-json agent (it rejects raw
/// input). Spec: docs/specs/SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md §6 (Phase 3).
pub fn deliver_agent_message(block_id: &str, message: &str) -> Result<AgentDelivery, String> {
    let ctrl =
        get_controller(block_id).ok_or_else(|| format!("no controller for block {block_id}"))?;

    if let Some(persistent_ctrl) = ctrl
        .as_any()
        .downcast_ref::<persistent::PersistentSubprocessController>()
    {
        return Ok(match persistent_ctrl.send_user_message_outcome(message.to_string())? {
            SendOutcome::Sent => AgentDelivery::Structured,
            SendOutcome::Deferred => AgentDelivery::StructuredDeferred,
        });
    }

    if ctrl.controller_type() == BLOCK_CONTROLLER_ACP {
        ctrl.send_input(BlockInputUnion::data(message.as_bytes().to_vec()), None)?;
        return Ok(AgentDelivery::Structured);
    }

    if let Some(app_server_ctrl) = ctrl
        .as_any()
        .downcast_ref::<app_server_controller::AppServerController>()
    {
        app_server_ctrl.send_message(message.to_string())?;
        return Ok(AgentDelivery::Structured);
    }

    Ok(AgentDelivery::Pty)
}

/// Resync a block's controller — the main entry point for starting/restarting blocks.
/// Port of Go's `ResyncController`.
///
/// Logic:
/// 1. Load block from database
/// 2. Determine controller type from meta["controller"]
/// 3. If existing controller needs replacing (type changed, conn changed, force), stop it
/// 4. Create new controller if needed
/// 5. Start if status is init or done
/// Is this forced replace merely a runtime-config change (model / effort /
/// permission) on a controller that stays persistent — i.e. the one case that
/// may be deferred to the end of an in-flight turn rather than killing it?
///
/// Requires the type to be persistent on BOTH sides. Checking only the
/// existing side was a real bug (reagent P2 on PR #2858): the container-agent
/// override in `resync_controller` rewrites the target persistent ->
/// subprocess, so a forced resync landing mid-turn on such a block would defer
/// and silently keep the persistent controller — precisely the
/// incompatibility that override exists to correct.
///
/// A genuine type change must replace immediately, mid-turn or not; losing an
/// in-flight turn is the lesser harm against running a container agent on a
/// controller that cannot do per-turn `docker exec` at all.
fn is_runtime_config_only_replace(existing_type: &str, target_type: &str, force: bool) -> bool {
    force
        && existing_type == BLOCK_CONTROLLER_PERSISTENT
        && target_type == BLOCK_CONTROLLER_PERSISTENT
}

/// `respawn_if_done`: when the existing controller's status is
/// `STATUS_DONE` (its process already exited), should this call revive it
/// via `ctrl.start()` (the historical, unconditional behavior — pass
/// `true`), or leave it dead and return `Err(RESYNC_ERR_ALREADY_EXITED)`
/// instead (pass `false`)? `true` is correct for crash/backend-restart
/// recovery (the WS `ControllerResyncCommand` path, `TermResyncHandler` —
/// see its own doc comment for why a dead PTY must come back transparently
/// there). `false` is for a caller that needs to tell "this exited on
/// purpose" apart from "this needs reviving" using the SAME status read
/// `resync_controller` itself makes, rather than checking status in the
/// caller first and racing a StatusDone transition landing between that
/// check and this call — `try_attach_to_existing_shell`
/// (`server/mod.rs`, PtyShellCreate's pane-shell-reuse path) is the one
/// caller that needs this; see
/// `docs/specs/SPEC_TERM_EXIT_RESPAWN_LOOP_2026_09_15.md` §7.
pub fn resync_controller(
    block: &Block,
    tab_id: &str,
    rt_opts: Option<serde_json::Value>,
    force: bool,
    respawn_if_done: bool,
    broker: Option<Arc<Broker>>,
    event_bus: Option<Arc<EventBus>>,
    mstore: Option<Arc<Store>>,
    filestore: Option<Arc<FileStore>>,
    // `id_store`/`identity_store`: the Layer 3 identity/credential spawn
    // gate's dependencies (`identity::resolver::inject_identity_env`, per
    // `SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4_2026_07_14.md`). Only consulted
    // by `PersistentSubprocessController::start()`'s eager-resume path
    // (`SPEC_PERSISTENT_CONTROLLER_EAGER_RESUME_ON_RECONNECT_2026_09_20.md`)
    // — every other controller type ignores them. `None` here does not
    // disable the gate for a LIVE message send (that path, `agent_handlers::
    // input`, has its own direct access to these stores) — it only means a
    // controller CONSTRUCTED via this call can't eager-resume at all, and
    // falls back to today's lazy "wait for the first message" behavior even
    // when `agent:sessionid` is present. That fallback is the deliberately
    // safe default: spawning an eager resume WITHOUT the gate would spawn a
    // possibly-deauthed agent on whatever ambient credential is lying
    // around, silently reintroducing the exact vulnerability class that gate
    // was built to close. See `PersistentSubprocessController::start()`'s
    // own doc comment.
    id_store: Option<Arc<Store>>,
    identity_store: Option<Arc<Store>>,
    registry: Option<Arc<crate::registry::Registry>>,
    boot_id: Arc<str>,
    auth_key: &str,
    // The live, file-watched config (`AppState::config_watcher`). Only the
    // shell/cmd controller reads it, for the global `cmd:env` defaults its
    // interactive-shell spawn applies. `None` (tests) means no global
    // defaults.
    config: Option<Arc<crate::backend::wconfig::ConfigState>>,
) -> Result<(), String> {
    let block_id = &block.oid;
    let block_meta = &block.meta;

    // Get controller type from block meta
    let controller_type = super::obj::meta_get_string(block_meta, META_KEY_CONTROLLER, "");

    if controller_type.is_empty() {
        // No controller type = web/static block, nothing to manage
        return Ok(());
    }

    if is_closing(block_id) {
        return Err(format!("resync_controller: block {block_id} is closing"));
    }

    // Container agents (agentMode == "container") require a subprocess controller for
    // per-turn docker exec. Persistent is incompatible. Override any stale "persistent"
    // value that may have been written before this invariant was enforced at creation time.
    let agent_mode = super::obj::meta_get_string(block_meta, "agentMode", "");
    let controller_type =
        if agent_mode == "container" && controller_type == BLOCK_CONTROLLER_PERSISTENT {
            tracing::warn!(
                block_id = %block_id,
                "container agent has persistent controller in meta — overriding to subprocess"
            );
            BLOCK_CONTROLLER_SUBPROCESS.to_string()
        } else {
            controller_type
        };

    tracing::info!(
        block_id = %block_id,
        controller_type = %controller_type,
        mstore_present = mstore.is_some(),
        event_bus_present = event_bus.is_some(),
        force,
        "[dnd-debug] resync_controller entry"
    );

    // Check if existing controller needs to be replaced
    let existing = get_controller(block_id);
    if let Some(ref ctrl) = existing {
        let needs_replace = if ctrl.controller_type() != controller_type || force {
            true // Type changed or forced restart
        } else {
            let status = ctrl.get_runtime_status();
            // Check if connection changed
            let new_conn = super::obj::meta_get_string(block_meta, META_KEY_CONNECTION, "local");
            status.shellprocconnname != new_conn
        };

        // A forced resync that lands mid-turn on a persistent agent must NOT
        // tear the controller down: doing so kills the CLI process with the
        // user's in-flight message already written to its stdin, and nothing
        // replays it — `stop_process` records a `StopRequested` (which the
        // resume machine reads as an explicit user stop and so suppresses the
        // retry), the old controller's queue is discarded with it, and the
        // replacement "spawns on first message", i.e. does nothing. The turn
        // vanished with no response and no error. Diagnosed live on AgentX,
        // 2026-08-28.
        //
        // Instead the controller restarts itself the moment the turn ends,
        // picking up the new `cmd:args` (rebuilt from block meta at spawn) on
        // the next message. Nothing is lost by waiting: model/effort flags are
        // baked in at spawn, so they could never have applied to the turn
        // already running.
        //
        // Scoped to a runtime-config tweak: the controller must be persistent
        // now AND still be persistent afterwards. A controller-TYPE change is a
        // genuine replacement that has to happen immediately, even mid-turn.
        //
        // Checking only the EXISTING type was a real bug (reagent P2): the
        // container-agent override a few lines above rewrites `controller_type`
        // persistent -> subprocess, so a forced resync landing mid-turn on such
        // a block would have deferred and silently kept the persistent
        // controller — the exact incompatibility that override exists to
        // correct, and the opposite of what this comment claims.
        if needs_replace
            && is_runtime_config_only_replace(ctrl.controller_type(), &controller_type, force)
        {
            if let Some(p) = ctrl
                .as_any()
                .downcast_ref::<persistent::PersistentSubprocessController>()
            {
                if p.request_restart_when_idle() {
                    return Ok(());
                }
            }
        }

        if needs_replace {
            // stop_for_replace + remove_controller_entry_only, NOT stop +
            // delete_controller: a session restart/resync must not tear
            // down the block's shared process tracker — only the old
            // controller's own CLI process should die. See
            // docs/specs/SPEC_BACKGROUND_TASK_TEARDOWN_SURVIVAL_2026_08_20.md.
            let _ = ctrl.stop_for_replace(STATUS_DONE);
            remove_controller_entry_only(block_id);
        } else {
            // Existing controller is fine, just check if it needs starting
            let status = ctrl.get_runtime_status();
            tracing::info!(
                block_id = %block_id,
                status = %status.shellprocstatus,
                "[dnd-debug] existing controller — skipping spawn (no cmd:cwd seed)"
            );
            if status.shellprocstatus == STATUS_INIT {
                return ctrl.start(block_meta.clone(), rt_opts, force);
            }
            if status.shellprocstatus == STATUS_DONE {
                if respawn_if_done {
                    return ctrl.start(block_meta.clone(), rt_opts, force);
                }
                return Err(RESYNC_ERR_ALREADY_EXITED.to_string());
            }
            return Ok(());
        }
    }

    // Create new controller
    match controller_type.as_str() {
        BLOCK_CONTROLLER_SHELL | BLOCK_CONTROLLER_CMD => {
            let ctrl = shell::ShellController::new(
                controller_type.clone(),
                tab_id.to_string(),
                block_id.to_string(),
                broker,
                event_bus,
                mstore,
                filestore,
                auth_key.to_string(),
            )
            .with_config(config);
            let ctrl = Arc::new(ctrl);
            register_controller(block_id, ctrl.clone());
            let result = ctrl.start(block_meta.clone(), rt_opts, force);
            // codex P2 on PR #3219: ShellController::new starts with
            // is_agent_pane: false and start() only sets it (synchronously,
            // before returning — shell/lifecycle.rs's start()) after actually
            // spawning. register_controller's notification above therefore
            // fired before list_agent_panes()'s is_agent() filter would have
            // included this block; notify again now that start() has run so
            // the classified state gets an authoritative announcement too,
            // instead of depending solely on a proxy event or the poll.
            notify_tracked_blocks_changed();
            result
        }
        BLOCK_CONTROLLER_SUBPROCESS => {
            let ctrl = subprocess::SubprocessController::new(
                tab_id.to_string(),
                block_id.to_string(),
                broker,
                event_bus,
                mstore,
                filestore,
                registry,
                boot_id,
            );
            let ctrl = Arc::new(ctrl);
            ctrl.set_self_ref();
            register_controller(block_id, ctrl.clone());
            let result = ctrl.start(block_meta.clone(), rt_opts, force);
            notify_tracked_blocks_changed();
            result
        }
        BLOCK_CONTROLLER_PERSISTENT => {
            let ctrl = persistent::PersistentSubprocessController::new(
                tab_id.to_string(),
                block_id.to_string(),
                broker,
                event_bus,
                mstore,
                filestore,
            )
            .with_identity_stores(id_store, identity_store, auth_key.to_string())
            // One live instance per agent (SPEC_AGENT_SINGLE_LIVE_INSTANCE_2026_09_24).
            .with_agent_lease_store(registry, boot_id);
            let ctrl = Arc::new(ctrl);
            ctrl.set_self_ref();
            register_controller(block_id, ctrl.clone());
            let result = ctrl.start(block_meta.clone(), rt_opts, force);
            notify_tracked_blocks_changed();
            result
        }
        BLOCK_CONTROLLER_ACP => {
            let ctrl = acp::AcpController::new(
                tab_id.to_string(),
                block_id.to_string(),
                broker,
                event_bus,
                mstore,
                filestore,
            );
            let ctrl = Arc::new(ctrl);
            register_controller(block_id, ctrl.clone());
            let result = ctrl.start(block_meta.clone(), rt_opts, force);
            notify_tracked_blocks_changed();
            result
        }
        BLOCK_CONTROLLER_APP_SERVER => {
            let ctrl = app_server_controller::AppServerController::new(
                tab_id.to_string(),
                block_id.to_string(),
                broker,
                event_bus,
                mstore,
                filestore,
            );
            let ctrl = Arc::new(ctrl);
            ctrl.set_self_ref();
            register_controller(block_id, ctrl.clone());
            ctrl.start(block_meta.clone(), rt_opts, force)
        }
        BLOCK_CONTROLLER_TSUNAMI => {
            // Tsunami controller deferred to later phase
            Err("tsunami controller not yet implemented".to_string())
        }
        _ => Err(format!("unknown controller type: {controller_type}")),
    }
}

/// Publish a controller status event via MPS broker. The sole publish point
/// for `controllerstatus`, used by every controller type (persistent CLI,
/// subprocess CLI, ACP agents, plain shell/PTY panes — 13 call sites).
///
/// `persist: 1` so a reconnecting subscriber (WS reconnect, srv restart)
/// replays the last known status instead of seeing nothing until the next
/// live event — mirrors `EVENT_AGENT_FAILURE`'s identical `persist: 1` in
/// `subprocess/host_spawn.rs`. Closes the "missed the one turn-end push,
/// stuck showing Working forever" gap for the reconnect case; see
/// REPORT_LOGIN_PERSIST_FAILURE_AND_STUCK_WORKING_2026_07_27.md §3/§4 item 5.
/// (A same-connection pane remount, which doesn't clear the broker's
/// per-route replay tracking, isn't covered by persist alone — see the
/// focus-triggered reconcile in `agent-view.tsx` for that case.)
pub fn publish_controller_status(
    broker: &super::mps::Broker,
    status: &BlockControllerRuntimeStatus,
) {
    use super::mps::{MuxEvent, EVENT_CONTROLLER_STATUS};

    let event = MuxEvent {
        event: EVENT_CONTROLLER_STATUS.to_string(),
        scopes: vec![format!("block:{}", status.blockid)],
        sender: String::new(),
        persist: 1,
        data: serde_json::to_value(status).ok(),
    };
    broker.publish(event);
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Deferred-restart guard (reagent P2 on PR #2858) ─────────────────

    /// The case the deferral exists for: /model, /effort, /permission on a
    /// pane that stays persistent.
    #[test]
    fn a_forced_persistent_to_persistent_replace_may_be_deferred() {
        assert!(is_runtime_config_only_replace(
            BLOCK_CONTROLLER_PERSISTENT,
            BLOCK_CONTROLLER_PERSISTENT,
            true
        ));
    }

    /// The bug: the container-agent override rewrites the TARGET persistent ->
    /// subprocess. Deferring here would keep a persistent controller on a
    /// container agent, which cannot do per-turn `docker exec` — the exact
    /// incompatibility that override exists to fix.
    #[test]
    fn a_type_change_to_subprocess_is_never_deferred_even_when_forced() {
        assert!(!is_runtime_config_only_replace(
            BLOCK_CONTROLLER_PERSISTENT,
            BLOCK_CONTROLLER_SUBPROCESS,
            true
        ));
    }

    #[test]
    fn a_type_change_from_a_non_persistent_controller_is_never_deferred() {
        assert!(!is_runtime_config_only_replace(
            BLOCK_CONTROLLER_SUBPROCESS,
            BLOCK_CONTROLLER_PERSISTENT,
            true
        ));
        assert!(!is_runtime_config_only_replace(
            BLOCK_CONTROLLER_SHELL,
            BLOCK_CONTROLLER_SHELL,
            true
        ));
    }

    /// Without `force` the replace is driven by a type or connection change,
    /// never by a runtime-config tweak — nothing to defer.
    #[test]
    fn an_unforced_replace_is_never_deferred() {
        assert!(!is_runtime_config_only_replace(
            BLOCK_CONTROLLER_PERSISTENT,
            BLOCK_CONTROLLER_PERSISTENT,
            false
        ));
    }

    /// Test double for the stop_for_replace/remove_controller_entry_only
    /// tests below. Counts calls to `stop()` vs `stop_for_replace()`
    /// separately so a test can assert exactly one of them fired.
    struct CountingController {
        block_id: String,
        controller_type: String,
        stop_calls: std::sync::atomic::AtomicU32,
        stop_for_replace_calls: std::sync::atomic::AtomicU32,
    }

    impl CountingController {
        fn new(block_id: &str, controller_type: &str) -> Self {
            Self {
                block_id: block_id.to_string(),
                controller_type: controller_type.to_string(),
                stop_calls: std::sync::atomic::AtomicU32::new(0),
                stop_for_replace_calls: std::sync::atomic::AtomicU32::new(0),
            }
        }
    }

    impl Controller for CountingController {
        fn start(
            &self,
            _: MetaMapType,
            _: Option<serde_json::Value>,
            _: bool,
        ) -> Result<(), String> {
            Ok(())
        }
        fn stop(&self, _graceful: bool, _new_status: &str) -> Result<(), String> {
            self.stop_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
        fn get_runtime_status(&self) -> BlockControllerRuntimeStatus {
            BlockControllerRuntimeStatus {
                blockid: self.block_id.clone(),
                ..Default::default()
            }
        }
        fn send_input(&self, _: BlockInputUnion, _: Option<u64>) -> Result<(), String> {
            Ok(())
        }
        fn controller_type(&self) -> &str {
            &self.controller_type
        }
        fn block_id(&self) -> &str {
            &self.block_id
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    /// A second test double that overrides `stop_for_replace` (mirroring
    /// `ShellController`'s real override) so the "which method actually
    /// fired" assertion below is meaningful — `CountingController` alone
    /// would pass even if `resync_controller` still called plain `stop()`,
    /// since the DEFAULT `stop_for_replace` delegates to `stop()` too.
    struct OverridingCountingController(CountingController);

    impl Controller for OverridingCountingController {
        fn start(
            &self,
            m: MetaMapType,
            o: Option<serde_json::Value>,
            f: bool,
        ) -> Result<(), String> {
            self.0.start(m, o, f)
        }
        fn stop(&self, g: bool, s: &str) -> Result<(), String> {
            self.0.stop(g, s)
        }
        fn stop_for_replace(&self, new_status: &str) -> Result<(), String> {
            self.0
                .stop_for_replace_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let _ = new_status;
            Ok(())
        }
        fn get_runtime_status(&self) -> BlockControllerRuntimeStatus {
            self.0.get_runtime_status()
        }
        fn send_input(&self, i: BlockInputUnion, s: Option<u64>) -> Result<(), String> {
            self.0.send_input(i, s)
        }
        fn controller_type(&self) -> &str {
            self.0.controller_type()
        }
        fn block_id(&self) -> &str {
            self.0.block_id()
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    #[test]
    fn default_stop_for_replace_delegates_to_stop() {
        let ctrl = CountingController::new("block-default-delegate", "stub");
        ctrl.stop_for_replace(STATUS_DONE).unwrap();
        assert_eq!(ctrl.stop_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn remove_controller_entry_only_removes_the_registry_entry_without_calling_stop() {
        let block_id = "block-remove-entry-only";
        let ctrl = Arc::new(CountingController::new(block_id, "stub"));
        CONTROLLER_REGISTRY
            .write()
            .unwrap()
            .insert(block_id.to_string(), ctrl.clone());
        assert!(get_controller(block_id).is_some());

        remove_controller_entry_only(block_id);

        assert!(
            get_controller(block_id).is_none(),
            "controller must be gone from the registry"
        );
        assert_eq!(
            ctrl.stop_calls.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "remove_controller_entry_only must not itself call stop() — that's the caller's job via stop_for_replace"
        );
    }

    #[test]
    fn resync_controller_replace_path_calls_stop_for_replace_not_stop() {
        use crate::backend::obj::Block;

        let block_id = "block-resync-replace-uses-stop-for-replace";
        let old = Arc::new(OverridingCountingController(CountingController::new(
            block_id, "old-type",
        )));
        register_controller(block_id, old.clone());
        assert!(get_controller(block_id).is_some());

        // A real ShellController with cmd:runonstart=false so resync_controller's
        // replacement construction doesn't open a real PTY — controller_type
        // "shell" != old's "old-type" forces needs_replace=true.
        let mut meta = MetaMapType::new();
        meta.insert(
            META_KEY_CONTROLLER.to_string(),
            serde_json::Value::String("shell".to_string()),
        );
        meta.insert(
            META_KEY_CMD_RUN_ON_START.to_string(),
            serde_json::Value::Bool(false),
        );
        let block = Block {
            oid: block_id.to_string(),
            version: 1,
            meta,
            ..Default::default()
        };

        let result = resync_controller(&block, "tab-1", None, false, true, None, None, None, None, None, None, None, Arc::from("test-boot"), "test-key", None);
        assert!(result.is_ok(), "resync_controller failed: {result:?}");

        assert_eq!(
            old.0
                .stop_for_replace_calls
                .load(std::sync::atomic::Ordering::SeqCst),
            1,
            "the OLD controller's stop_for_replace should have fired exactly once"
        );
        assert_eq!(
            old.0.stop_calls.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "the OLD controller's plain stop() must NOT have fired — that would (on ShellController) SIGTERM the whole process group, taking a declared-background descendant down with it"
        );

        // A new controller now owns the block, replacing the old one.
        let replaced = get_controller(block_id);
        assert!(replaced.is_some());
        assert_eq!(replaced.unwrap().controller_type(), "shell");

        // Cleanup — real teardown, not a replace, so the ordinary path is fine.
        delete_controller(block_id);
    }

    #[test]
    fn test_status_constants() {
        assert_eq!(STATUS_INIT, "init");
        assert_eq!(STATUS_RUNNING, "running");
        assert_eq!(STATUS_DONE, "done");
    }

    #[test]
    fn test_controller_type_constants() {
        assert_eq!(BLOCK_CONTROLLER_SHELL, "shell");
        assert_eq!(BLOCK_CONTROLLER_CMD, "cmd");
        assert_eq!(BLOCK_CONTROLLER_TSUNAMI, "tsunami");
        assert_eq!(BLOCK_CONTROLLER_APP_SERVER, "app-server");
    }

    #[test]
    fn test_meta_key_constants() {
        assert_eq!(META_KEY_CONTROLLER, "controller");
        assert_eq!(META_KEY_CONNECTION, "connection");
        assert_eq!(META_KEY_CMD, "cmd");
        assert_eq!(META_KEY_CMD_RUN_ON_START, "cmd:runonstart");
    }

    #[test]
    fn test_block_input_union_data() {
        let input = BlockInputUnion::data(b"hello".to_vec());
        assert_eq!(input.input_data.as_ref().unwrap(), b"hello");
        assert!(input.sig_name.is_none());
        assert!(input.term_size.is_none());
    }

    #[test]
    fn test_block_input_union_signal() {
        let input = BlockInputUnion::signal("SIGTERM");
        assert!(input.input_data.is_none());
        assert_eq!(input.sig_name.as_ref().unwrap(), "SIGTERM");
        assert!(input.term_size.is_none());
    }

    #[test]
    fn test_block_input_union_resize() {
        let size = TermSize {
            rows: 40,
            cols: 120,
        };
        let input = BlockInputUnion::resize(size.clone());
        assert!(input.input_data.is_none());
        assert!(input.sig_name.is_none());
        let ts = input.term_size.unwrap();
        assert_eq!(ts.rows, 40);
        assert_eq!(ts.cols, 120);
    }

    #[test]
    fn test_runtime_status_default() {
        let status = BlockControllerRuntimeStatus::default();
        assert!(status.blockid.is_empty());
        assert_eq!(status.version, 0);
        assert!(status.shellprocstatus.is_empty());
        assert_eq!(status.shellprocexitcode, 0);
    }

    #[test]
    fn test_runtime_status_serde() {
        let status = BlockControllerRuntimeStatus {
            blockid: "block-123".to_string(),
            version: 3,
            shellprocstatus: STATUS_RUNNING.to_string(),
            shellprocconnname: "local".to_string(),
            shellprocexitcode: 0,
            ..Default::default()
        };
        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("\"blockid\":\"block-123\""));
        assert!(json.contains("\"shellprocstatus\":\"running\""));

        let parsed: BlockControllerRuntimeStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.blockid, "block-123");
        assert_eq!(parsed.version, 3);
    }

    #[test]
    fn test_get_nonexistent_controller() {
        assert!(get_controller("nonexistent-block").is_none());
    }

    #[test]
    fn test_get_block_controller_status_none() {
        assert!(get_block_controller_status("nonexistent").is_none());
    }

    #[test]
    fn test_stop_nonexistent_controller() {
        // Should be ok (no-op)
        assert!(stop_block_controller("nonexistent").is_ok());
    }

    #[test]
    fn test_send_input_no_controller() {
        let result = send_input("nonexistent", BlockInputUnion::data(b"test".to_vec()), None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no controller"));
    }

    #[test]
    fn test_resync_no_controller_type() {
        let block = Block {
            oid: "test-block".to_string(),
            version: 1,
            meta: HashMap::new(),
            ..Default::default()
        };
        // No "controller" key in meta = no-op
        let result = resync_controller(&block, "tab-1", None, false, true, None, None, None, None, None, None, None, std::sync::Arc::from("test-boot"), "test-key", None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_resync_unknown_controller_type() {
        let mut meta = MetaMapType::new();
        meta.insert(
            "controller".to_string(),
            serde_json::Value::String("unknown_type".to_string()),
        );
        let block = Block {
            oid: "test-block".to_string(),
            version: 1,
            meta,
            ..Default::default()
        };
        let result = resync_controller(&block, "tab-1", None, false, true, None, None, None, None, None, None, None, std::sync::Arc::from("test-boot"), "test-key", None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unknown controller type"));
    }

    /// Regression: `publish_controller_status` must persist (not fire-once),
    /// so a reconnecting subscriber picks up the last known `turn_active`
    /// state instead of nothing — see this function's doc comment and
    /// REPORT_LOGIN_PERSIST_FAILURE_AND_STUCK_WORKING_2026_07_27.md §3/§4
    /// item 5.
    #[test]
    fn test_publish_controller_status_persists_for_replay() {
        let broker = super::super::mps::Broker::new();
        let status = BlockControllerRuntimeStatus {
            blockid: "block-persist-test".to_string(),
            turn_active: true,
            ..Default::default()
        };
        publish_controller_status(&broker, &status);

        let history = broker.read_event_history(
            super::super::mps::EVENT_CONTROLLER_STATUS,
            "block:block-persist-test",
            1,
        );
        assert_eq!(
            history.len(),
            1,
            "publish must persist at least the latest event"
        );
        let replayed: BlockControllerRuntimeStatus =
            serde_json::from_value(history[0].data.clone().unwrap()).unwrap();
        assert_eq!(replayed.blockid, "block-persist-test");
        assert!(replayed.turn_active);
    }

    // ── Subprocess agents are unreachable via this primitive alone ──────────
    // Regression lock for
    // docs/reports/REPORT_JEKT_DELIVERY_DROPS_SUBPROCESS_AGENTS_2026_09_02.md.
    // These two tests together are the whole bug: `deliver_agent_message` hands
    // a SubprocessController to the PTY fallback, and a SubprocessController
    // refuses PTY input — so every inter-agent message to one was dropped.
    // `bootstrap::install_agent_turn_delivery` is what closes the gap, by
    // running a real turn instead of falling through to keystrokes.

    fn subprocess_controller(block_id: &str) -> Arc<subprocess::SubprocessController> {
        Arc::new(subprocess::SubprocessController::new(
            "tab-jekt".to_string(),
            block_id.to_string(),
            None,
            None,
            None,
            None,
            None,
            Arc::from("test-boot"),
        ))
    }

    /// Half one: this primitive has no structured route to a subprocess agent.
    /// If someone later teaches it one, this test should be updated together
    /// with the bootstrap installer — not deleted on its own.
    #[test]
    fn deliver_agent_message_has_no_structured_route_to_a_subprocess_agent() {
        let block_id = "block-jekt-subprocess-delivery";
        register_controller(block_id, subprocess_controller(block_id));

        let delivery = deliver_agent_message(block_id, "hello from another agent")
            .expect("controller is registered, so lookup must succeed");

        assert!(
            matches!(delivery, AgentDelivery::Pty),
            "a SubprocessController still falls back to PTY here; the turn-based              route lives in bootstrap::install_agent_turn_delivery",
        );

        remove_controller_entry_only(block_id);
    }

    /// Half two: and that fallback is not merely suboptimal — it is refused, so
    /// the message reaches the agent not at all rather than late.
    #[test]
    fn and_the_pty_fallback_that_implies_is_refused_outright() {
        let ctrl = subprocess_controller("block-jekt-subprocess-refusal");

        let err = ctrl
            .send_input(
                BlockInputUnion::data(b"hello from another agent".to_vec()),
                None,
            )
            .expect_err("subprocess controllers take turns, not keystrokes");

        assert!(
            err.contains("does not accept raw input"),
            "expected the raw-input refusal, got {err:?}",
        );
    }

    // ── App Server agents have no PTY either ─────────────────────────────────
    // ReAgent P1 on PR #3215: `deliver_agent_message` only special-cased
    // Persistent and ACP, so an App Server block fell through to the PTY
    // branch — the same class of bug as the subprocess case above, just for a
    // controller that didn't exist yet when that fix landed.

    #[test]
    fn deliver_agent_message_routes_to_the_app_server_controller_not_pty() {
        let block_id = "block-jekt-app-server-delivery";
        register_controller(
            block_id,
            Arc::new(app_server_controller::AppServerController::new(
                "tab-jekt".to_string(),
                block_id.to_string(),
                None,
                None,
                None,
                None,
            )),
        );

        let err = deliver_agent_message(block_id, "hello from another agent")
            .expect_err("controller has no process yet, so send_message must fail — the point is that it was called at all");

        assert!(
            err.contains("not initialized"),
            "expected AppServerController::send_message's own error (proving the \
             Structured route was taken), got {err:?}",
        );

        remove_controller_entry_only(block_id);
    }

    /// Controllers that DO have a structured channel must keep the behavior they
    /// had — the fix adds a branch, it does not reroute persistent/ACP agents.
    #[test]
    fn a_missing_controller_is_still_an_error_not_a_silent_pty_fallback() {
        let err = deliver_agent_message("block-that-was-never-registered", "hi")
            .expect_err("an unregistered block has nowhere to deliver to");
        assert!(err.contains("no controller for block"), "got {err:?}");
    }
}
