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
use super::storage::filestore::FileStore;
use super::storage::store::Store;
use super::obj::{Block, MetaMapType, TermSize};
use super::wps::Broker;

// ---- Controller status constants (match Go) ----

pub const STATUS_INIT: &str = "init";
pub const STATUS_RUNNING: &str = "running";
pub const STATUS_DONE: &str = "done";

// ---- Controller type constants (match Go) ----

pub const BLOCK_CONTROLLER_SHELL: &str = "shell";
pub const BLOCK_CONTROLLER_CMD: &str = "cmd";
pub const BLOCK_CONTROLLER_TSUNAMI: &str = "tsunami";
pub const BLOCK_CONTROLLER_SUBPROCESS: &str = "subprocess";
pub const BLOCK_CONTROLLER_PERSISTENT: &str = "persistent";
pub const BLOCK_CONTROLLER_ACP: &str = "acp";

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
    /// True if this pane is running an agent CLI (e.g. claude, codex, gemini, kimi, openclaw, pi).
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_agent_pane: bool,
    /// True if a turn is currently in flight (message sent, no terminating
    /// `"result"` event observed yet). For `PersistentSubprocessController`
    /// this is the only signal that distinguishes "actively generating a
    /// turn" from "process alive, idle between turns" — `shellprocstatus`
    /// stays `"running"` for the whole process lifetime either way. Backed
    /// by `HealthMonitor.active_turn` (see `blockcontroller/health.rs`).
    /// Frontend seeds `TurnPhase` from this at mount instead of always
    /// defaulting to `Idle` — see
    /// docs/specs/REPORT_AGENT_PANE_STATE_RECONCILIATION_2026_07_07.md
    /// Finding 1.
    #[serde(default, skip_serializing_if = "is_false")]
    pub turn_active: bool,
}

// ---- Controller trait ----

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

    /// This controller's `HealthMonitor`, if it owns one. Default `None`,
    /// mirroring [`agent_id`](Controller::agent_id)'s default-None pattern
    /// — only controller types actually wired to a `HealthMonitor`
    /// (persistent, host_spawn/subprocess, container_spawn) override this.
    /// Used by `handle_wps_publish` (`server/mod.rs`) to forward a
    /// `compaction_started` WPS event into the right block's health
    /// monitor — see
    /// docs/specs/SPEC_UNRESPONSIVE_FALSE_POSITIVE_DURING_COMPACTION_2026_08_22.md.
    fn health_monitor(&self) -> Option<Arc<health::HealthMonitor>> {
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
    CONTROLLER_REGISTRY
        .read()
        .unwrap()
        .get(block_id)
        .cloned()
}

/// Register a controller, stopping any previous one for the same block.
pub fn register_controller(block_id: &str, controller: Arc<dyn Controller>) {
    let mut registry = CONTROLLER_REGISTRY.write().unwrap();
    if let Some(old) = registry.remove(block_id) {
        // Stop the old controller before replacing
        let _ = old.stop(true, STATUS_DONE);
    }
    registry.insert(block_id.to_string(), controller);
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

/// Unregister (delete) a controller by block ID, stopping it first.
/// Removes from the registry before calling stop() so no new callers can reach it.
pub fn delete_controller(block_id: &str) {
    let ctrl = CONTROLLER_REGISTRY.write().unwrap().remove(block_id);
    if let Some(ctrl) = ctrl {
        let _ = ctrl.stop(true, STATUS_DONE);
    }
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

/// How a controller-aware agent message was delivered.
pub enum AgentDelivery {
    /// Delivered on the controller's structured input channel — a persistent
    /// stream-json stdin line or an ACP `session/prompt`. No PTY keystrokes are
    /// needed, and the message lands on the live channel so the agent picks it up
    /// mid-turn (steering) instead of only when idle.
    Structured,
    /// The controller is PTY/terminal-based (shell/term) or otherwise has no
    /// structured input channel. The caller should fall back to keystroke
    /// injection.
    Pty,
}

/// Deliver an inter-agent / muxbus message to a running agent the way its controller
/// expects.
///
/// - **Persistent** (stream-json) agents have no PTY: the message is written as a
///   `{type:"user",…}` line on the live stdin, which steers the agent mid-turn.
/// - **ACP** agents receive the message as a `session/prompt` (the ACP controller's
///   `send_input` already wraps raw input that way).
/// - Everything else (shell/term PTY agents, one-shot subprocess agents) is reported
///   as [`AgentDelivery::Pty`] so the caller uses keystroke injection — preserving
///   today's behavior.
///
/// This is the controller-aware delivery primitive muxbus Tier-1 needs: PTY
/// keystrokes silently fail to reach a persistent stream-json agent (it rejects raw
/// input). Spec: docs/specs/SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md §6 (Phase 3).
pub fn deliver_agent_message(block_id: &str, message: &str) -> Result<AgentDelivery, String> {
    let ctrl = get_controller(block_id)
        .ok_or_else(|| format!("no controller for block {block_id}"))?;

    if let Some(persistent_ctrl) = ctrl
        .as_any()
        .downcast_ref::<persistent::PersistentSubprocessController>()
    {
        persistent_ctrl.send_user_message(message.to_string())?;
        return Ok(AgentDelivery::Structured);
    }

    if ctrl.controller_type() == BLOCK_CONTROLLER_ACP {
        ctrl.send_input(BlockInputUnion::data(message.as_bytes().to_vec()), None)?;
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
pub fn resync_controller(
    block: &Block,
    tab_id: &str,
    rt_opts: Option<serde_json::Value>,
    force: bool,
    broker: Option<Arc<Broker>>,
    event_bus: Option<Arc<EventBus>>,
    wstore: Option<Arc<Store>>,
    filestore: Option<Arc<FileStore>>,
    registry: Option<Arc<crate::registry::Registry>>,
    boot_id: Arc<str>,
) -> Result<(), String> {
    let block_id = &block.oid;
    let block_meta = &block.meta;

    // Get controller type from block meta
    let controller_type = super::obj::meta_get_string(block_meta, META_KEY_CONTROLLER, "");

    if controller_type.is_empty() {
        // No controller type = web/static block, nothing to manage
        return Ok(());
    }

    // Container agents (agentMode == "container") require a subprocess controller for
    // per-turn docker exec. Persistent is incompatible. Override any stale "persistent"
    // value that may have been written before this invariant was enforced at creation time.
    let agent_mode = super::obj::meta_get_string(block_meta, "agentMode", "");
    let controller_type = if agent_mode == "container" && controller_type == BLOCK_CONTROLLER_PERSISTENT {
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
        wstore_present = wstore.is_some(),
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
            let new_conn =
                super::obj::meta_get_string(block_meta, META_KEY_CONNECTION, "local");
            status.shellprocconnname != new_conn
        };

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
            if status.shellprocstatus == STATUS_INIT || status.shellprocstatus == STATUS_DONE {
                return ctrl.start(block_meta.clone(), rt_opts, force);
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
                wstore,
                filestore,
            );
            let ctrl = Arc::new(ctrl);
            register_controller(block_id, ctrl.clone());
            ctrl.start(block_meta.clone(), rt_opts, force)
        }
        BLOCK_CONTROLLER_SUBPROCESS => {
            let ctrl = subprocess::SubprocessController::new(
                tab_id.to_string(),
                block_id.to_string(),
                broker,
                event_bus,
                wstore,
                filestore,
                registry,
                boot_id,
            );
            let ctrl = Arc::new(ctrl);
            ctrl.set_self_ref();
            register_controller(block_id, ctrl.clone());
            ctrl.start(block_meta.clone(), rt_opts, force)
        }
        BLOCK_CONTROLLER_PERSISTENT => {
            let ctrl = persistent::PersistentSubprocessController::new(
                tab_id.to_string(),
                block_id.to_string(),
                broker,
                event_bus,
                wstore,
                filestore,
            );
            let ctrl = Arc::new(ctrl);
            ctrl.set_self_ref();
            register_controller(block_id, ctrl.clone());
            ctrl.start(block_meta.clone(), rt_opts, force)
        }
        BLOCK_CONTROLLER_ACP => {
            let ctrl = acp::AcpController::new(
                tab_id.to_string(),
                block_id.to_string(),
                broker,
                event_bus,
                wstore,
                filestore,
            );
            let ctrl = Arc::new(ctrl);
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

/// Publish a controller status event via WPS broker. The sole publish point
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
    broker: &super::wps::Broker,
    status: &BlockControllerRuntimeStatus,
) {
    use super::wps::{WaveEvent, EVENT_CONTROLLER_STATUS};

    let event = WaveEvent {
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
        fn start(&self, _: MetaMapType, _: Option<serde_json::Value>, _: bool) -> Result<(), String> {
            Ok(())
        }
        fn stop(&self, _graceful: bool, _new_status: &str) -> Result<(), String> {
            self.stop_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
        fn get_runtime_status(&self) -> BlockControllerRuntimeStatus {
            BlockControllerRuntimeStatus { blockid: self.block_id.clone(), ..Default::default() }
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
        fn start(&self, m: MetaMapType, o: Option<serde_json::Value>, f: bool) -> Result<(), String> {
            self.0.start(m, o, f)
        }
        fn stop(&self, g: bool, s: &str) -> Result<(), String> {
            self.0.stop(g, s)
        }
        fn stop_for_replace(&self, new_status: &str) -> Result<(), String> {
            self.0.stop_for_replace_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
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
        CONTROLLER_REGISTRY.write().unwrap().insert(block_id.to_string(), ctrl.clone());
        assert!(get_controller(block_id).is_some());

        remove_controller_entry_only(block_id);

        assert!(get_controller(block_id).is_none(), "controller must be gone from the registry");
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
        let old = Arc::new(OverridingCountingController(CountingController::new(block_id, "old-type")));
        register_controller(block_id, old.clone());
        assert!(get_controller(block_id).is_some());

        // A real ShellController with cmd:runonstart=false so resync_controller's
        // replacement construction doesn't open a real PTY — controller_type
        // "shell" != old's "old-type" forces needs_replace=true.
        let mut meta = MetaMapType::new();
        meta.insert(META_KEY_CONTROLLER.to_string(), serde_json::Value::String("shell".to_string()));
        meta.insert(META_KEY_CMD_RUN_ON_START.to_string(), serde_json::Value::Bool(false));
        let block = Block { oid: block_id.to_string(), version: 1, meta, ..Default::default() };

        let result = resync_controller(&block, "tab-1", None, false, None, None, None, None, None, Arc::from("test-boot"));
        assert!(result.is_ok(), "resync_controller failed: {result:?}");

        assert_eq!(
            old.0.stop_for_replace_calls.load(std::sync::atomic::Ordering::SeqCst),
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
        let size = TermSize { rows: 40, cols: 120 };
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
        let result = resync_controller(&block, "tab-1", None, false, None, None, None, None, None, std::sync::Arc::from("test-boot"));
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
        let result = resync_controller(&block, "tab-1", None, false, None, None, None, None, None, std::sync::Arc::from("test-boot"));
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
        let broker = super::super::wps::Broker::new();
        let status = BlockControllerRuntimeStatus {
            blockid: "block-persist-test".to_string(),
            turn_active: true,
            ..Default::default()
        };
        publish_controller_status(&broker, &status);

        let history = broker.read_event_history(
            super::super::wps::EVENT_CONTROLLER_STATUS,
            "block:block-persist-test",
            1,
        );
        assert_eq!(history.len(), 1, "publish must persist at least the latest event");
        let replayed: BlockControllerRuntimeStatus =
            serde_json::from_value(history[0].data.clone().unwrap()).unwrap();
        assert_eq!(replayed.blockid, "block-persist-test");
        assert!(replayed.turn_active);
    }
}
