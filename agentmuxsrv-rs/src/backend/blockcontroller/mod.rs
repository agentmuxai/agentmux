// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

//! Block controller: manages lifecycle of each block (terminal, command, web app).
//! Port of Go's pkg/blockcontroller/blockcontroller.go.

#![allow(dead_code)]
//!
//! Architecture:
//! - Global controller registry maps block_id → Controller
//! - Each controller manages the lifecycle of one block
//! - ShellController handles "shell" and "cmd" block types
//! - Controllers dispatch I/O between the user and the process/service

pub mod shell;

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};

use super::eventbus::EventBus;
use super::storage::wstore::WaveStore;
use super::waveobj::{Block, MetaMapType, TermSize};
use super::wps::Broker;

// ---- Controller status constants (match Go) ----

pub const STATUS_INIT: &str = "init";
pub const STATUS_RUNNING: &str = "running";
pub const STATUS_DONE: &str = "done";

// ---- Controller type constants (match Go) ----

pub const BLOCK_CONTROLLER_SHELL: &str = "shell";
pub const BLOCK_CONTROLLER_CMD: &str = "cmd";
pub const BLOCK_CONTROLLER_TSUNAMI: &str = "tsunami";

// ---- Block metadata key constants (match Go) ----

pub const META_KEY_CONTROLLER: &str = "controller";
pub const META_KEY_CONNECTION: &str = "connection";
pub const META_KEY_CMD: &str = "cmd";
pub const META_KEY_CMD_CWD: &str = "cmd:cwd";
pub const META_KEY_CMD_SHELL: &str = "cmd:shell";
pub const META_KEY_CMD_ARGS: &str = "cmd:args";
pub const META_KEY_CMD_ENV: &str = "cmd:env";
pub const META_KEY_CMD_JWT: &str = "cmd:jwt";
pub const META_KEY_CMD_NO_WSH: &str = "cmd:nowsh";
pub const META_KEY_CMD_RUN_ON_START: &str = "cmd:runonstart";
pub const META_KEY_CMD_RUN_ONCE: &str = "cmd:runonce";
pub const META_KEY_CMD_CLEAR_ON_START: &str = "cmd:clearonstart";
pub const META_KEY_CMD_CLOSE_ON_EXIT: &str = "cmd:closeonexit";
pub const META_KEY_CMD_CLOSE_ON_EXIT_FORCE: &str = "cmd:closeonexitforce";
pub const META_KEY_CMD_CLOSE_ON_EXIT_DELAY: &str = "cmd:closeonexitdelay";
pub const META_KEY_CMD_INIT_SCRIPT: &str = "cmd:initscript";
pub const META_KEY_CMD_INIT_SCRIPT_BASH: &str = "cmd:initscript.bash";
pub const META_KEY_CMD_INIT_SCRIPT_ZSH: &str = "cmd:initscript.zsh";
pub const META_KEY_CMD_INIT_SCRIPT_FISH: &str = "cmd:initscript.fish";
pub const META_KEY_CMD_INIT_SCRIPT_PWSH: &str = "cmd:initscript.pwsh";
pub const META_KEY_TERM_LOCAL_SHELL_PATH: &str = "term:localshellpath";
pub const META_KEY_TERM_LOCAL_SHELL_OPTS: &str = "term:localshellopts";

// ---- Default timeouts ----

/// Default controller operation timeout in milliseconds.
pub const DEFAULT_TIMEOUT_MS: u64 = 2000;

/// Grace period before forceful kill in milliseconds.
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

    /// Get the current runtime status.
    fn get_runtime_status(&self) -> BlockControllerRuntimeStatus;

    /// Send input (terminal data, signal, or resize) to the controller.
    fn send_input(&self, input: BlockInputUnion) -> Result<(), String>;

    /// Get the controller type (e.g., "shell", "cmd").
    fn controller_type(&self) -> &str;

    /// Get the block ID.
    fn block_id(&self) -> &str;
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

/// Unregister (delete) a controller by block ID.
pub fn delete_controller(block_id: &str) {
    CONTROLLER_REGISTRY.write().unwrap().remove(block_id);
}

/// Get all controllers (snapshot).
pub fn get_all_controllers() -> HashMap<String, Arc<dyn Controller>> {
    CONTROLLER_REGISTRY.read().unwrap().clone()
}

/// Stop all running controllers gracefully.
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
pub fn stop_block_controller(block_id: &str) -> Result<(), String> {
    match get_controller(block_id) {
        Some(ctrl) => ctrl.stop(true, STATUS_DONE),
        None => Ok(()), // No controller = already stopped
    }
}

/// Send input to a block's controller.
pub fn send_input(block_id: &str, input: BlockInputUnion) -> Result<(), String> {
    match get_controller(block_id) {
        Some(ctrl) => ctrl.send_input(input),
        None => Err(format!("no controller for block {block_id}")),
    }
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
    wstore: Option<Arc<WaveStore>>,
) -> Result<(), String> {
    let block_id = &block.oid;
    let block_meta = &block.meta;

    // Get controller type from block meta
    let controller_type = super::waveobj::meta_get_string(block_meta, META_KEY_CONTROLLER, "");

    if controller_type.is_empty() {
        // No controller type = web/static block, nothing to manage
        return Ok(());
    }

    // Check if existing controller needs to be replaced
    let existing = get_controller(block_id);
    if let Some(ref ctrl) = existing {
        let needs_replace = if ctrl.controller_type() != controller_type || force {
            true // Type changed or forced restart
        } else {
            let status = ctrl.get_runtime_status();
            // Check if connection changed
            let new_conn =
                super::waveobj::meta_get_string(block_meta, META_KEY_CONNECTION, "local");
            status.shellprocconnname != new_conn
        };

        if needs_replace {
            let _ = ctrl.stop(true, STATUS_DONE);
            delete_controller(block_id);
        } else {
            // Existing controller is fine, just check if it needs starting
            let status = ctrl.get_runtime_status();
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

/// Publish a controller status event via WPS broker.
pub fn publish_controller_status(
    broker: &super::wps::Broker,
    status: &BlockControllerRuntimeStatus,
) {
    use super::wps::{WaveEvent, EVENT_CONTROLLER_STATUS};

    let event = WaveEvent {
        event: EVENT_CONTROLLER_STATUS.to_string(),
        scopes: vec![format!("block:{}", status.blockid)],
        sender: String::new(),
        persist: 0,
        data: serde_json::to_value(status).ok(),
    };
    broker.publish(event);
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let result = send_input("nonexistent", BlockInputUnion::data(b"test".to_vec()));
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
        let result = resync_controller(&block, "tab-1", None, false, None, None, None);
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
        let result = resync_controller(&block, "tab-1", None, false, None, None, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unknown controller type"));
    }
}
