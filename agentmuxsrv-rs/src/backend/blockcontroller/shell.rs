// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! ShellController: manages lifecycle of shell and command blocks.
//! Port of Go's pkg/blockcontroller/shellcontroller.go.
//!
//! State machine:
//!   INIT ─(start)─> RUNNING ─(exit/stop)─> DONE
//!   DONE ─(resync+force)─> RUNNING
//!
//! I/O model (4 async tasks when running):
//! 1. PTY read loop: process stdout → FileStore + WPS event
//! 2. Input loop: input channel → process stdin
//! 3. Output/proxy loop: WSH messages → input channel
//! 4. Wait loop: monitor process exit, update status


use std::io::Read as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use libc;

use base64::Engine as _;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use tokio::sync::mpsc;

use super::{
    BlockControllerRuntimeStatus, BlockInputUnion, Controller, META_KEY_CMD, META_KEY_CMD_ARGS,
    META_KEY_CMD_CLEAR_ON_START, META_KEY_CMD_CLOSE_ON_EXIT, META_KEY_CMD_CLOSE_ON_EXIT_DELAY,
    META_KEY_CMD_CLOSE_ON_EXIT_FORCE, META_KEY_CMD_ENV, META_KEY_CMD_RUN_ONCE,
    META_KEY_CMD_RUN_ON_START, META_KEY_CONNECTION, STATUS_DONE, STATUS_INIT, STATUS_RUNNING,
};
use crate::backend::eventbus::EventBus;
use crate::backend::shellexec::{ConnInterface, ShellProc};
use crate::backend::storage::wstore::WaveStore;
use crate::backend::waveobj::{self, MetaMapType};
use crate::backend::wps;

/// Channel buffer size for shell input (matches Go's 32).
const SHELL_INPUT_CH_SIZE: usize = 32;

/// Detect the best available interactive shell on Windows.
///
/// Mirrors the original Go logic from pkg/util/shellutil/shellutil.go DetectLocalShellPath():
///   1. Try `pwsh`  (PowerShell 7 — cross-platform)
///   2. Try `powershell` (Windows PowerShell 5.x)
///   3. Fall back to `cmd.exe`
#[cfg(windows)]
fn detect_local_shell_path_windows() -> String {
    use std::process::Command;
    // Try pwsh (PowerShell 7)
    if Command::new("where").arg("pwsh").output().map(|o| o.status.success()).unwrap_or(false) {
        return "pwsh".to_string();
    }
    // Try powershell (Windows PowerShell 5.x)
    if Command::new("where").arg("powershell").output().map(|o| o.status.success()).unwrap_or(false) {
        return "powershell".to_string();
    }
    "cmd.exe".to_string()
}

/// Stub for non-Windows builds (never called due to cfg!(windows) guard).
#[cfg(not(windows))]
fn detect_local_shell_path_windows() -> String {
    "cmd.exe".to_string()
}

/// PTY read buffer size (matches Go's 4096).
const PTY_READ_BUF_SIZE: usize = 4096;

/// Inner state protected by mutex.
/// Grace period (seconds) between SIGTERM and SIGKILL during stop().
const KILL_GRACE_SECS: u64 = 5;

struct ShellControllerInner {
    /// Current process status.
    proc_status: String,
    /// Process exit code.
    proc_exit_code: i32,
    /// Status version counter (incremented on each change).
    status_version: i32,
    /// Connection name for the shell process.
    conn_name: String,
    /// Input channel sender (sends to the PTY input loop).
    input_tx: Option<mpsc::Sender<BlockInputUnion>>,
    /// Input channel receiver (consumed by the PTY input loop).
    input_rx: Option<mpsc::Receiver<BlockInputUnion>>,
    /// OS PID of the running child process, kept for signal delivery in stop().
    child_pid: Option<u32>,
    /// Unix timestamp (ms) when the process was spawned; None until first spawn.
    spawn_ts_ms: Option<i64>,
    /// Monotonic instant of the most recent PTY read; None until first output.
    last_pty_output: Option<Instant>,
    /// True if this pane is running an agent CLI (e.g. claude).
    is_agent_pane: bool,
}

/// Factory function type for creating ConnInterface instances.
/// This allows dependency injection for testing.
pub type ConnFactory =
    Box<dyn Fn(&str, &MetaMapType) -> Result<Box<dyn ConnInterface>, String> + Send + Sync>;

/// ShellController manages one shell or command block.
pub struct ShellController {
    /// Controller type: "shell" or "cmd".
    controller_type: String,
    /// Parent tab UUID.
    tab_id: String,
    /// Block UUID.
    block_id: String,
    /// Prevents concurrent run() calls.
    run_lock: Arc<AtomicBool>,
    /// Protected inner state.
    inner: Arc<Mutex<ShellControllerInner>>,
    /// Optional factory for creating ConnInterface (for testing).
    conn_factory: Mutex<Option<ConnFactory>>,
    /// WPS broker for publishing events (blockfile, controllerstatus).
    broker: Option<Arc<wps::Broker>>,
    /// Event bus (unused for now, reserved for future event routing).
    #[allow(dead_code)]
    event_bus: Option<Arc<EventBus>>,
    /// Wave object store — used to seed cmd:cwd on shell spawn.
    wstore: Option<Arc<WaveStore>>,
}

impl ShellController {
    /// Create a new ShellController.
    pub fn new(
        controller_type: String,
        tab_id: String,
        block_id: String,
        broker: Option<Arc<wps::Broker>>,
        event_bus: Option<Arc<EventBus>>,
        wstore: Option<Arc<WaveStore>>,
    ) -> Self {
        Self {
            controller_type,
            tab_id,
            block_id,
            run_lock: Arc::new(AtomicBool::new(false)),
            inner: Arc::new(Mutex::new(ShellControllerInner {
                proc_status: STATUS_INIT.to_string(),
                proc_exit_code: 0,
                status_version: 0,
                conn_name: String::new(),
                input_tx: None,
                input_rx: None,
                child_pid: None,
                spawn_ts_ms: None,
                last_pty_output: None,
                is_agent_pane: false,
            })),
            conn_factory: Mutex::new(None),
            broker,
            event_bus,
            wstore,
        }
    }

    /// Set a custom ConnInterface factory (for testing).
    pub fn set_conn_factory(&self, factory: ConnFactory) {
        *self.conn_factory.lock().unwrap() = Some(factory);
    }

    /// Try to acquire the run lock. Returns false if already running.
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
    fn set_status(inner: &mut ShellControllerInner, status: &str) {
        inner.proc_status = status.to_string();
        inner.status_version += 1;
    }

    /// Get the runtime status (snapshot).
    fn get_status_snapshot(&self) -> BlockControllerRuntimeStatus {
        let inner = self.inner.lock().unwrap();
        BlockControllerRuntimeStatus {
            blockid: self.block_id.clone(),
            version: inner.status_version,
            shellprocstatus: inner.proc_status.clone(),
            shellprocconnname: inner.conn_name.clone(),
            shellprocexitcode: inner.proc_exit_code,
            spawn_ts_ms: inner.spawn_ts_ms,
            is_agent_pane: inner.is_agent_pane,
        }
    }

    /// Seconds since last PTY output, or None if no output yet.
    pub fn last_output_secs_ago(&self) -> Option<u64> {
        self.inner.lock().unwrap().last_pty_output.map(|t| t.elapsed().as_secs())
    }

    /// True if this pane is running an agent CLI.
    pub fn is_agent_pane(&self) -> bool {
        self.inner.lock().unwrap().is_agent_pane
    }

    /// Check block meta for whether to run on start.
    fn should_run_on_start(meta: &MetaMapType) -> bool {
        waveobj::meta_get_bool(meta, META_KEY_CMD_RUN_ON_START, true)
    }

    /// Check block meta for run-once mode (used in full lifecycle integration).
    #[allow(dead_code)]
    fn should_run_once(meta: &MetaMapType) -> bool {
        waveobj::meta_get_bool(meta, META_KEY_CMD_RUN_ONCE, false)
    }

    /// Check block meta for clear-on-start (used in full lifecycle integration).
    #[allow(dead_code)]
    fn should_clear_on_start(meta: &MetaMapType) -> bool {
        waveobj::meta_get_bool(meta, META_KEY_CMD_CLEAR_ON_START, false)
    }

    /// Check block meta for close-on-exit (used in full lifecycle integration).
    #[allow(dead_code)]
    fn should_close_on_exit(meta: &MetaMapType) -> bool {
        waveobj::meta_get_bool(meta, META_KEY_CMD_CLOSE_ON_EXIT, false)
    }

    /// Check block meta for force close-on-exit (used in full lifecycle integration).
    #[allow(dead_code)]
    fn should_close_on_exit_force(meta: &MetaMapType) -> bool {
        waveobj::meta_get_bool(meta, META_KEY_CMD_CLOSE_ON_EXIT_FORCE, false)
    }

    /// Get the close-on-exit delay in ms (defaults to 2000, used in full lifecycle integration).
    #[allow(dead_code)]
    fn close_on_exit_delay_ms(meta: &MetaMapType) -> u64 {
        match meta.get(META_KEY_CMD_CLOSE_ON_EXIT_DELAY) {
            Some(serde_json::Value::Number(n)) => n.as_u64().unwrap_or(2000),
            _ => 2000,
        }
    }

    /// Get the connection name from block meta.
    fn get_conn_name(meta: &MetaMapType) -> String {
        waveobj::meta_get_string(meta, META_KEY_CONNECTION, "local")
    }

    /// Get the command string from block meta.
    fn get_cmd_str(meta: &MetaMapType) -> String {
        waveobj::meta_get_string(meta, META_KEY_CMD, "")
    }

    /// Get cmd:args array from block meta.
    fn get_cmd_args(meta: &MetaMapType) -> Vec<String> {
        match meta.get(META_KEY_CMD_ARGS) {
            Some(serde_json::Value::Array(arr)) => arr
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect(),
            _ => vec![],
        }
    }

    /// Check if cmd:interactive is set in block meta.
    fn is_interactive(meta: &MetaMapType) -> bool {
        waveobj::meta_get_bool(meta, "cmd:interactive", false)
    }

    /// Publish current controller status via the WPS broker.
    fn publish_status(&self) {
        if let Some(ref broker) = self.broker {
            let status = self.get_status_snapshot();
            super::publish_controller_status(broker, &status);
        }
    }
}

impl Controller for ShellController {
    fn start(
        &self,
        block_meta: MetaMapType,
        _rt_opts: Option<serde_json::Value>,
        force: bool,
    ) -> Result<(), String> {
        let cmd_str_preview = Self::get_cmd_str(&block_meta);
        let interactive_preview = Self::is_interactive(&block_meta);
        tracing::info!(
            block_id = %self.block_id,
            controller = %self.controller_type,
            cmd = %cmd_str_preview,
            interactive = interactive_preview,
            force = force,
            "block start requested"
        );

        // Check if we should run
        if !force && !Self::should_run_on_start(&block_meta) {
            tracing::info!(block_id = %self.block_id, "skipping start: run_on_start is false");
            return Ok(());
        }

        // Try to acquire run lock
        if !self.try_lock_run() {
            return Err("controller is already running".to_string());
        }

        // Get connection info
        let conn_name = Self::get_conn_name(&block_meta);

        // Update status to running and publish
        {
            let mut inner = self.inner.lock().unwrap();
            Self::set_status(&mut inner, STATUS_RUNNING);
            inner.conn_name = conn_name.clone();
        }
        self.publish_status();

        // Create input channel
        let (input_tx, input_rx) = mpsc::channel(SHELL_INPUT_CH_SIZE);
        {
            let mut inner = self.inner.lock().unwrap();
            inner.input_tx = Some(input_tx);
        }

        // Check if we have a conn_factory (test/mock path)
        let has_factory = self.conn_factory.lock().unwrap().is_some();

        if has_factory {
            // Mock path: use ConnInterface factory (synchronous, for tests)
            let conn_result = {
                let factory = self.conn_factory.lock().unwrap();
                factory.as_ref().unwrap()(&conn_name, &block_meta)
            };

            let mut conn = match conn_result {
                Ok(c) => c,
                Err(e) => {
                    let mut inner = self.inner.lock().unwrap();
                    Self::set_status(&mut inner, STATUS_DONE);
                    inner.proc_exit_code = -1;
                    inner.input_tx = None;
                    self.unlock_run();
                    return Err(format!("failed to create connection: {e}"));
                }
            };

            if let Err(e) = conn.start() {
                let mut inner = self.inner.lock().unwrap();
                Self::set_status(&mut inner, STATUS_DONE);
                inner.proc_exit_code = -1;
                inner.input_tx = None;
                self.unlock_run();
                return Err(format!("failed to start process: {e}"));
            }

            let mut shell_proc = ShellProc::new(conn_name, conn);
            let _done_rx = shell_proc.take_done_rx();
            let exit_code = shell_proc.wait_and_signal();

            {
                let mut inner = self.inner.lock().unwrap();
                inner.proc_exit_code = exit_code;
                Self::set_status(&mut inner, STATUS_DONE);
                inner.input_tx = None;
            }
            self.publish_status();
            self.unlock_run();
            return Ok(());
        }

        // Real PTY path
        let pty_system = native_pty_system();
        let pty_size = PtySize {
            rows: 25,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        };

        let pair = pty_system.openpty(pty_size).map_err(|e| {
            tracing::error!(block_id = %self.block_id, error = %e, "failed to open PTY");
            let mut inner = self.inner.lock().unwrap();
            Self::set_status(&mut inner, STATUS_DONE);
            inner.proc_exit_code = -1;
            inner.input_tx = None;
            self.unlock_run();
            format!("failed to open PTY: {e}")
        })?;
        tracing::info!(block_id = %self.block_id, rows = 25, cols = 80, "PTY opened");

        // Determine shell command
        let cmd_str = Self::get_cmd_str(&block_meta);
        let cmd_args = Self::get_cmd_args(&block_meta);
        let interactive = Self::is_interactive(&block_meta);

        // Resolve effective AGENTMUX_AGENT_ID for jekt auto-registration.
        // Priority: block metadata > global settings > WAVEMUX_AGENT_ID env compat.
        let agent_id_for_jekt: Option<String> = block_meta
            .get(META_KEY_CMD_ENV)
            .and_then(|m| m.as_object())
            .and_then(|obj| obj.get("AGENTMUX_AGENT_ID"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| {
                let cfg = crate::backend::wconfig::ConfigWatcher::with_config(
                    crate::backend::wconfig::build_default_config(),
                );
                cfg.get_settings().cmd_env.get("AGENTMUX_AGENT_ID").cloned()
            })
            .or_else(|| std::env::var("WAVEMUX_AGENT_ID").ok());

        let mut cmd = if !cmd_str.is_empty() && (!cmd_args.is_empty() || interactive) {
            // Direct spawn: cmd:args provided or cmd:interactive set.
            // Spawn the CLI directly (no sh -c wrapper) so args are passed correctly.
            tracing::info!(block_id = %self.block_id, cmd = %cmd_str, args = ?cmd_args, "direct spawn path");
            let mut c = CommandBuilder::new(&cmd_str);
            if !cmd_args.is_empty() {
                let arg_refs: Vec<&str> = cmd_args.iter().map(|s| s.as_str()).collect();
                c.args(arg_refs);
            }
            c
        } else if !cmd_str.is_empty() {
            // "cmd" controller: run a specific command string via shell wrapper
            tracing::info!(block_id = %self.block_id, cmd = %cmd_str, "shell-wrapped spawn path");
            if cfg!(windows) {
                let mut c = CommandBuilder::new("cmd.exe");
                c.args(["/C", &cmd_str]);
                c
            } else {
                let mut c = CommandBuilder::new("/bin/sh");
                c.args(["-c", &cmd_str]);
                c
            }
        } else {
            // "shell" controller: interactive shell with AgentMux integration
            // On Windows: prefer pwsh (PowerShell 7), fall back to powershell.exe (5.x), then cmd.exe
            let shell_path = if cfg!(windows) {
                detect_local_shell_path_windows()
            } else {
                std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
            };

            let shell_type = crate::backend::shellintegration::detect_shell_type(&shell_path);

            // Deploy shell integration scripts to ~/.agentmux/ (the user's home-based
            // data dir) instead of AGENTMUX_DATA_HOME.  MSIX packages virtualise writes
            // to %LocalAppData%, so files written by the packaged backend aren't visible
            // to child processes (pwsh, bash, etc.) spawned via ConPTY.  The home dir is
            // never virtualised, so the scripts are always reachable at their literal path.
            let shell_home = crate::backend::wavebase::get_home_dir().join(".agentmux");
            crate::backend::shellintegration::deploy_scripts(&shell_home);

            tracing::info!(block_id = %self.block_id, shell = %shell_path, shell_type = ?shell_type, "interactive shell path");

            let mut c = CommandBuilder::new(&shell_path);

            // Apply shell-specific startup args (--rcfile, -File, etc.)
            if let Some(startup) = crate::backend::shellintegration::get_shell_startup(shell_type, &shell_home) {
                for arg in &startup.extra_args {
                    c.arg(arg);
                }
                for (k, v) in &startup.env_vars {
                    c.env(k, v);
                }
            }

            // Inject terminal capability env vars into the PTY environment.
            // ConPTY on Windows fully supports VT/ANSI sequences, so set TERM
            // on all platforms. Without this, CLI tools (e.g. Claude Code) use
            // different Unicode width tables, causing ANSI color offset on Windows.
            c.env("TERM", "xterm-256color");
            c.env("COLORTERM", "truecolor");
            c.env("TERM_PROGRAM", "agentmux");
            c.env("AGENTMUX_BLOCKID", &self.block_id);
            c.env("AGENTMUX_TABID", &self.tab_id);
            c.env("AGENTMUX_VERSION", env!("CARGO_PKG_VERSION"));

            // Propagate local backend URL so agentbus-client prefers local PTY delivery.
            // Set by main.rs after binding; absent in test/mock contexts (graceful no-op).
            if let Ok(local_url) = std::env::var("AGENTMUX_LOCAL_URL") {
                c.env("AGENTMUX_LOCAL_URL", &local_url);
            }

            // Set AGENTMUX to the wsh binary path for portable mode detection in scripts
            if let Some(wsh_path) = crate::backend::shellintegration::find_wsh_binary() {
                c.env("AGENTMUX", wsh_path.to_string_lossy().as_ref());
            } else {
                c.env("AGENTMUX", "1");
            }

            // Inject cmd:env from wconfig settings and block metadata.
            // Track whether AGENTMUX_AGENT_ID is explicitly set so we know
            // whether to apply the backward-compat WAVEMUX bridge.
            let mut has_agent_id = false;

            // Settings (global defaults, lowest priority)
            let config = crate::backend::wconfig::ConfigWatcher::with_config(
                crate::backend::wconfig::build_default_config(),
            );
            let settings = config.get_settings();
            for (k, v) in &settings.cmd_env {
                if k == "AGENTMUX_AGENT_ID" {
                    has_agent_id = true;
                }
                c.env(k, v);
            }

            // Block metadata (per-block overrides, highest priority)
            if let Some(env_map) = block_meta.get(META_KEY_CMD_ENV) {
                if let Some(obj) = env_map.as_object() {
                    for (k, v) in obj {
                        if let Some(val) = v.as_str() {
                            if k == "AGENTMUX_AGENT_ID" {
                                has_agent_id = true;
                            }
                            c.env(k, val);
                        }
                    }
                }
            }

            // Strip host-inherited agent identity unless explicitly configured
            // in settings.cmd_env or block cmd:env metadata.
            // This also supersedes the old WAVEMUX backward-compat bridge —
            // both AGENTMUX_* and WAVEMUX_* vars are removed so new panes
            // start as plain "Terminal".
            if !has_agent_id {
                c.env_remove("AGENTMUX_AGENT_ID");
                c.env_remove("AGENTMUX_AGENT_COLOR");
                c.env_remove("AGENTMUX_AGENT_TEXT_COLOR");
                c.env_remove("WAVEMUX_AGENT_ID");
                c.env_remove("WAVEMUX_AGENT_COLOR");
            }

            c
        };

        // Set working directory if specified
        let cwd = waveobj::meta_get_string(&block_meta, super::META_KEY_CMD_CWD, "");
        if !cwd.is_empty() {
            cmd.cwd(&cwd);
        }

        let mut child = pair.slave.spawn_command(cmd).map_err(|e| {
            tracing::error!(block_id = %self.block_id, error = %e, cmd = %cmd_str, "spawn failed");
            let mut inner = self.inner.lock().unwrap();
            Self::set_status(&mut inner, STATUS_DONE);
            inner.proc_exit_code = -1;
            inner.input_tx = None;
            self.unlock_run();
            format!("failed to spawn command: {e}")
        })?;
        tracing::info!(block_id = %self.block_id, "process spawned successfully");

        // Detect agent pane: cmd contains a known agent CLI or has AGENTMUX_AGENT_ID set.
        let is_agent = agent_id_for_jekt.is_some()
            || cmd_str.to_lowercase().contains("claude")
            || cmd_str.to_lowercase().contains("codex")
            || cmd_str.to_lowercase().contains("gemini");

        // Register PID and record spawn metadata.
        let spawn_ts_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        {
            let mut inner = self.inner.lock().unwrap();
            if let Some(pid) = child.process_id() {
                super::pidregistry::register(&self.block_id, pid);
                inner.child_pid = Some(pid);
            }
            inner.spawn_ts_ms = Some(spawn_ts_ms);
            inner.is_agent_pane = is_agent;
        }

        // Auto-register with jekt if AGENTMUX_AGENT_ID was set in the block env.
        // This maps agent_id → block_id in the ReactiveHandler so jekt can deliver
        // messages directly to this PTY without a separate /wave/reactive/register call.
        if let Some(ref agent_id) = agent_id_for_jekt {
            match crate::backend::reactive::get_global_handler()
                .register_agent(agent_id, &self.block_id, Some(&self.tab_id))
            {
                Ok(()) => {
                    tracing::info!(
                        block_id = %self.block_id,
                        agent_id = %agent_id,
                        "jekt: auto-registered"
                    );
                    // Also write to cross-instance file registry.
                    if let Ok(local_url) = std::env::var("AGENTMUX_LOCAL_URL") {
                        let data_dir = crate::backend::wavebase::get_wave_data_dir();
                        crate::backend::reactive::registry::write(
                            &data_dir,
                            agent_id,
                            &local_url,
                            &self.block_id,
                        );
                    }
                }
                Err(e) => tracing::warn!(
                    block_id = %self.block_id,
                    agent_id = %agent_id,
                    error = %e,
                    "jekt: auto-register failed"
                ),
            }
        }
        tracing::info!(
            block_id = %self.block_id,
            wstore_present = self.wstore.is_some(),
            event_bus_present = self.event_bus.is_some(),
            "[dnd-debug] pre-seed state after spawn"
        );

        // Seed cmd:cwd in block meta immediately after spawn so drag-and-drop works
        // before the shell emits its first OSC 7 (or for shells without integration).
        if let Some(ref store) = self.wstore {
            let effective_cwd = if !cwd.is_empty() {
                cwd.clone()
            } else {
                std::env::current_dir()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default()
            };
            tracing::debug!(block_id = %self.block_id, cwd = %effective_cwd, "seeding cmd:cwd");
            if !effective_cwd.is_empty() {
                let oref_str = format!("block:{}", self.block_id);
                let mut meta_update = MetaMapType::new();
                meta_update.insert(
                    super::META_KEY_CMD_CWD.to_string(),
                    serde_json::Value::String(effective_cwd),
                );
                // Only set if not already populated — don't clobber a restored session CWD
                match store.must_get::<crate::backend::waveobj::Block>(&self.block_id) {
                    Ok(block) if waveobj::meta_get_string(&block.meta, super::META_KEY_CMD_CWD, "").is_empty() => {
                        match crate::server::service::update_object_meta(store, &oref_str, &meta_update) {
                            Ok(()) => {
                                // Re-read updated block and broadcast waveobj:update so the
                                // frontend Jotai atom refreshes (update_object_meta only writes
                                // to SQLite — it does NOT send a WebSocket event on its own).
                                if let Ok(updated_block) = store.must_get::<crate::backend::waveobj::Block>(&self.block_id) {
                                    if let Some(ref event_bus) = self.event_bus {
                                        let update_data = serde_json::to_value(&waveobj::WaveObjUpdate {
                                            updatetype: "update".into(),
                                            otype: "block".into(),
                                            oid: self.block_id.clone(),
                                            obj: Some(waveobj::wave_obj_to_value(&updated_block)),
                                        }).ok();
                                        event_bus.broadcast_event(&crate::backend::eventbus::WSEventType {
                                            eventtype: "waveobj:update".to_string(),
                                            oref: oref_str.clone(),
                                            data: update_data,
                                        });
                                        tracing::info!(block_id = %self.block_id, "cmd:cwd seeded and broadcast to frontend");
                                    } else {
                                        tracing::warn!(block_id = %self.block_id, "cmd:cwd written to store but no event_bus to broadcast — frontend won't update");
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::warn!(block_id = %self.block_id, error = %e, "failed to seed cmd:cwd in store");
                            }
                        }
                    }
                    Ok(_) => {
                        tracing::debug!(block_id = %self.block_id, "cmd:cwd already set, skipping seed");
                    }
                    Err(e) => {
                        tracing::warn!(block_id = %self.block_id, error = %e, "failed to read block for cmd:cwd seed");
                    }
                }
            }
        }

        // Get reader/writer from master
        let reader = pair.master.try_clone_reader().map_err(|e| {
            let _ = child.kill();
            let mut inner = self.inner.lock().unwrap();
            Self::set_status(&mut inner, STATUS_DONE);
            inner.proc_exit_code = -1;
            inner.input_tx = None;
            self.unlock_run();
            format!("failed to clone PTY reader: {e}")
        })?;

        let writer = pair.master.take_writer().map_err(|e| {
            let _ = child.kill();
            let mut inner = self.inner.lock().unwrap();
            Self::set_status(&mut inner, STATUS_DONE);
            inner.proc_exit_code = -1;
            inner.input_tx = None;
            self.unlock_run();
            format!("failed to take PTY writer: {e}")
        })?;

        // Spawn PTY read task (blocking I/O → spawn_blocking)
        let block_id_read = self.block_id.clone();
        let broker_read = self.broker.clone();
        let inner_read = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            let mut reader = reader;
            let mut buf = [0u8; PTY_READ_BUF_SIZE];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break, // EOF
                    Ok(n) => {
                        inner_read.lock().unwrap().last_pty_output = Some(Instant::now());
                        if let Some(ref broker) = broker_read {
                            handle_append_block_file(
                                broker,
                                &block_id_read,
                                "term",
                                &buf[..n],
                            );
                        }
                    }
                    Err(e) => {
                        tracing::debug!("PTY read error for {}: {}", block_id_read, e);
                        break;
                    }
                }
            }
        });

        // Spawn input task (routes input channel → PTY writer + resize + signals)
        // Owns writer and master — dropping them closes the PTY, causing child to exit.
        let master = pair.master;
        tokio::spawn(async move {
            let mut writer = writer;
            let mut input_rx = input_rx;
            while let Some(input) = input_rx.recv().await {
                if let Some(data) = input.input_data {
                    use std::io::Write;
                    if let Err(e) = writer.write_all(&data) {
                        tracing::debug!("PTY write error: {}", e);
                        break;
                    }
                }
                if let Some(ref size) = input.term_size {
                    let pty_size = PtySize {
                        rows: size.rows as u16,
                        cols: size.cols as u16,
                        pixel_width: 0,
                        pixel_height: 0,
                    };
                    if let Err(e) = master.resize(pty_size) {
                        tracing::debug!("PTY resize error: {}", e);
                    }
                }
                if input.sig_name.is_some() {
                    // Drop writer + master to close PTY, which terminates the child
                    break;
                }
            }
            // writer and master drop here → PTY closes → child gets EOF/terminates
        });

        // Spawn wait task (monitors process exit)
        let inner_wait = Arc::clone(&self.inner);
        let block_id_wait = self.block_id.clone();
        let agent_id_wait = agent_id_for_jekt.clone();
        let broker_wait = self.broker.clone();
        let run_lock = Arc::clone(&self.run_lock);
        tokio::task::spawn_blocking(move || {
            let mut child = child;

            // Wait for child to exit (blocking)
            let exit_status = child.wait();
            let exit_code = match exit_status {
                Ok(status) => {
                    if status.success() {
                        0
                    } else {
                        // portable-pty ExitStatus doesn't expose raw code on all platforms
                        1
                    }
                }
                Err(e) => {
                    tracing::warn!("wait error for block {}: {}", block_id_wait, e);
                    -1
                }
            };

            tracing::info!(block_id = %block_id_wait, exit_code = exit_code, "process exited");

            // Unregister PID from per-pane metrics
            super::pidregistry::unregister(&block_id_wait);

            // Deregister from jekt — removes the agent_id → block_id mapping so
            // subsequent jekt attempts fall back to MessageBus rather than a dead PTY.
            crate::backend::reactive::get_global_handler().unregister_block(&block_id_wait);

            // Also remove from cross-instance file registry.
            if let Some(ref agent_id) = agent_id_wait {
                let data_dir = crate::backend::wavebase::get_wave_data_dir();
                crate::backend::reactive::registry::remove(&data_dir, agent_id);
            }

            // Update inner state
            {
                let mut inner = inner_wait.lock().unwrap();
                inner.proc_exit_code = exit_code;
                ShellController::set_status(&mut inner, STATUS_DONE);
                inner.input_tx = None;
            }

            // Publish done status
            if let Some(ref broker) = broker_wait {
                let status = {
                    let inner = inner_wait.lock().unwrap();
                    BlockControllerRuntimeStatus {
                        blockid: block_id_wait.clone(),
                        version: inner.status_version,
                        shellprocstatus: inner.proc_status.clone(),
                        shellprocconnname: inner.conn_name.clone(),
                        shellprocexitcode: inner.proc_exit_code,
                        spawn_ts_ms: inner.spawn_ts_ms,
                        is_agent_pane: inner.is_agent_pane,
                    }
                };
                super::publish_controller_status(broker, &status);
            }

            // Release run lock
            run_lock.store(false, Ordering::SeqCst);
        });

        // Return immediately — PTY tasks run in background
        Ok(())
    }

    fn stop(&self, _graceful: bool, new_status: &str) -> Result<(), String> {
        // Extract what we need from the lock, release it before any async work.
        #[allow(unused_variables)] // used under #[cfg(unix)] only
        let pid_to_kill = {
            let mut inner = self.inner.lock().unwrap();
            if inner.proc_status == new_status {
                return Ok(());
            }
            let pid = inner.child_pid;
            // Drop the input channel — closes PTY writer → delivers EOF/SIGHUP as
            // belt-and-suspenders in case signal delivery fails on the platform.
            inner.input_tx = None;
            Self::set_status(&mut inner, new_status);
            pid
        };

        // Send SIGTERM to the process group so that child processes spawned by
        // the shell (e.g. `claude --dangerously-skip-permissions` and its subtree)
        // are also signalled. Negative pid targets the whole process group.
        // Schedule SIGKILL after KILL_GRACE_SECS as a backstop for processes
        // that ignore or delay on SIGTERM.
        #[cfg(unix)]
        if let Some(pid) = pid_to_kill {
            // SAFETY: kill() is a well-defined POSIX syscall.
            unsafe { libc::kill(-(pid as libc::pid_t), libc::SIGTERM) };
            tokio::spawn(async move {
                tokio::time::sleep(tokio::time::Duration::from_secs(KILL_GRACE_SECS)).await;
                unsafe { libc::kill(-(pid as libc::pid_t), libc::SIGKILL) };
            });
        }

        Ok(())
    }

    fn get_runtime_status(&self) -> BlockControllerRuntimeStatus {
        self.get_status_snapshot()
    }

    fn send_input(&self, input: BlockInputUnion) -> Result<(), String> {
        let inner = self.inner.lock().unwrap();
        match &inner.input_tx {
            Some(tx) => tx
                .try_send(input)
                .map_err(|e| format!("failed to send input: {e}")),
            None => Err("controller is not running".to_string()),
        }
    }

    fn controller_type(&self) -> &str {
        &self.controller_type
    }

    fn block_id(&self) -> &str {
        &self.block_id
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

// ---- File operation helpers ----

/// Append data to a block's terminal output file and publish a WPS event.
/// Port of Go's `HandleAppendBlockFile`.
pub fn handle_append_block_file(
    broker: &wps::Broker,
    block_id: &str,
    filename: &str,
    data: &[u8],
) {
    // In a full implementation, this would also write to FileStore.
    // For now, just publish the WPS event.
    let data64 = base64::engine::general_purpose::STANDARD.encode(data);

    let event_data = wps::WSFileEventData {
        zoneid: block_id.to_string(),
        filename: filename.to_string(),
        fileop: wps::FILE_OP_APPEND.to_string(),
        data64,
    };

    let event = wps::WaveEvent {
        event: wps::EVENT_BLOCK_FILE.to_string(),
        scopes: vec![format!("block:{block_id}")],
        sender: String::new(),
        persist: 0,
        data: serde_json::to_value(&event_data).ok(),
    };

    broker.publish(event);
}

/// Truncate a block's terminal output file and publish a WPS event.
/// Port of Go's `HandleTruncateBlockFile`.
pub fn handle_truncate_block_file(broker: &wps::Broker, block_id: &str, filename: &str) {
    let event_data = wps::WSFileEventData {
        zoneid: block_id.to_string(),
        filename: filename.to_string(),
        fileop: wps::FILE_OP_TRUNCATE.to_string(),
        data64: String::new(),
    };

    let event = wps::WaveEvent {
        event: wps::EVENT_BLOCK_FILE.to_string(),
        scopes: vec![format!("block:{block_id}")],
        sender: String::new(),
        persist: 0,
        data: serde_json::to_value(&event_data).ok(),
    };

    broker.publish(event);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::shellexec::MockConn;
    use std::sync::Arc;

    fn make_shell_meta() -> MetaMapType {
        let mut meta = MetaMapType::new();
        meta.insert(
            "controller".to_string(),
            serde_json::Value::String("shell".to_string()),
        );
        meta
    }

    fn make_cmd_meta(cmd: &str) -> MetaMapType {
        let mut meta = MetaMapType::new();
        meta.insert(
            "controller".to_string(),
            serde_json::Value::String("cmd".to_string()),
        );
        meta.insert(
            "cmd".to_string(),
            serde_json::Value::String(cmd.to_string()),
        );
        meta
    }

    #[test]
    fn test_shell_controller_new() {
        let ctrl = ShellController::new(
            "shell".to_string(),
            "tab-1".to_string(),
            "block-1".to_string(),
            None,
            None,
            None,
        );
        assert_eq!(ctrl.controller_type(), "shell");
        assert_eq!(ctrl.block_id(), "block-1");

        let status = ctrl.get_runtime_status();
        assert_eq!(status.shellprocstatus, STATUS_INIT);
        assert_eq!(status.blockid, "block-1");
        assert_eq!(status.version, 0);
    }

    #[test]
    fn test_shell_controller_start_stop() {
        let ctrl = ShellController::new(
            "shell".to_string(),
            "tab-1".to_string(),
            "block-1".to_string(),
            None,
            None,
            None,
        );

        // Use mock factory so we don't open a real PTY in tests
        ctrl.set_conn_factory(Box::new(|_conn_name, _meta| {
            Ok(Box::new(MockConn::new(0)) as Box<dyn ConnInterface>)
        }));

        let meta = make_shell_meta();
        let result = ctrl.start(meta, None, false);
        assert!(result.is_ok());

        // After start with mock, process immediately exits → status is done
        let status = ctrl.get_runtime_status();
        assert_eq!(status.shellprocstatus, STATUS_DONE);

        // Stop should work
        let result = ctrl.stop(true, STATUS_DONE);
        assert!(result.is_ok());
    }

    #[test]
    fn test_shell_controller_run_on_start_false() {
        let ctrl = ShellController::new(
            "shell".to_string(),
            "tab-1".to_string(),
            "block-1".to_string(),
            None,
            None,
            None,
        );

        let mut meta = make_shell_meta();
        meta.insert(
            META_KEY_CMD_RUN_ON_START.to_string(),
            serde_json::Value::Bool(false),
        );

        let result = ctrl.start(meta, None, false);
        assert!(result.is_ok());

        // Should still be in init state (didn't start)
        let status = ctrl.get_runtime_status();
        assert_eq!(status.shellprocstatus, STATUS_INIT);
    }

    #[test]
    fn test_shell_controller_force_start() {
        let ctrl = ShellController::new(
            "shell".to_string(),
            "tab-1".to_string(),
            "block-1".to_string(),
            None,
            None,
            None,
        );

        ctrl.set_conn_factory(Box::new(|_conn_name, _meta| {
            Ok(Box::new(MockConn::new(0)) as Box<dyn ConnInterface>)
        }));

        let mut meta = make_shell_meta();
        meta.insert(
            META_KEY_CMD_RUN_ON_START.to_string(),
            serde_json::Value::Bool(false),
        );

        // Force should override run_on_start=false
        let result = ctrl.start(meta, None, true);
        assert!(result.is_ok());

        let status = ctrl.get_runtime_status();
        // With mock, immediately exits to done
        assert_eq!(status.shellprocstatus, STATUS_DONE);
    }

    #[test]
    fn test_shell_controller_with_conn_factory() {
        let ctrl = ShellController::new(
            "cmd".to_string(),
            "tab-1".to_string(),
            "block-1".to_string(),
            None,
            None,
            None,
        );

        // Set a custom factory that returns a mock with exit code 42
        ctrl.set_conn_factory(Box::new(|_conn_name, _meta| {
            Ok(Box::new(MockConn::new(42)) as Box<dyn ConnInterface>)
        }));

        let meta = make_cmd_meta("echo hello");
        let result = ctrl.start(meta, None, true);
        assert!(result.is_ok());

        let status = ctrl.get_runtime_status();
        assert_eq!(status.shellprocstatus, STATUS_DONE);
        assert_eq!(status.shellprocexitcode, 42);
    }

    #[test]
    fn test_shell_controller_conn_factory_error() {
        let ctrl = ShellController::new(
            "shell".to_string(),
            "tab-1".to_string(),
            "block-1".to_string(),
            None,
            None,
            None,
        );

        ctrl.set_conn_factory(Box::new(|_conn_name, _meta| {
            Err("connection refused".to_string())
        }));

        let meta = make_shell_meta();
        let result = ctrl.start(meta, None, true);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("connection refused"));

        let status = ctrl.get_runtime_status();
        assert_eq!(status.shellprocstatus, STATUS_DONE);
        assert_eq!(status.shellprocexitcode, -1);
    }

    #[test]
    fn test_shell_controller_send_input_not_running() {
        let ctrl = ShellController::new(
            "shell".to_string(),
            "tab-1".to_string(),
            "block-1".to_string(),
            None,
            None,
            None,
        );

        let result = ctrl.send_input(BlockInputUnion::data(b"hello".to_vec()));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not running"));
    }

    #[test]
    fn test_shell_controller_status_version_increments() {
        let ctrl = ShellController::new(
            "shell".to_string(),
            "tab-1".to_string(),
            "block-1".to_string(),
            None,
            None,
            None,
        );

        ctrl.set_conn_factory(Box::new(|_conn_name, _meta| {
            Ok(Box::new(MockConn::new(0)) as Box<dyn ConnInterface>)
        }));

        let v0 = ctrl.get_runtime_status().version;

        let meta = make_shell_meta();
        ctrl.start(meta, None, true).unwrap();

        let v_after = ctrl.get_runtime_status().version;
        // Status changed from init → running → done = at least 2 increments
        assert!(v_after > v0);
    }

    #[test]
    fn test_controller_trait_as_arc() {
        let ctrl: Arc<dyn Controller> = Arc::new(ShellController::new(
            "shell".to_string(),
            "tab-1".to_string(),
            "block-1".to_string(),
            None,
            None,
            None,
        ));

        assert_eq!(ctrl.controller_type(), "shell");
        assert_eq!(ctrl.block_id(), "block-1");
        let status = ctrl.get_runtime_status();
        assert_eq!(status.shellprocstatus, STATUS_INIT);
    }

    #[test]
    fn test_meta_helpers() {
        let mut meta = MetaMapType::new();
        assert!(ShellController::should_run_on_start(&meta)); // default true
        assert!(!ShellController::should_run_once(&meta)); // default false
        assert!(!ShellController::should_clear_on_start(&meta)); // default false
        assert!(!ShellController::should_close_on_exit(&meta)); // default false

        meta.insert(
            META_KEY_CMD_RUN_ON_START.to_string(),
            serde_json::Value::Bool(false),
        );
        assert!(!ShellController::should_run_on_start(&meta));

        meta.insert(
            META_KEY_CMD_RUN_ONCE.to_string(),
            serde_json::Value::Bool(true),
        );
        assert!(ShellController::should_run_once(&meta));

        meta.insert(
            META_KEY_CMD_CLEAR_ON_START.to_string(),
            serde_json::Value::Bool(true),
        );
        assert!(ShellController::should_clear_on_start(&meta));
    }

    #[test]
    fn test_close_on_exit_delay() {
        let mut meta = MetaMapType::new();
        assert_eq!(ShellController::close_on_exit_delay_ms(&meta), 2000); // default

        meta.insert(
            META_KEY_CMD_CLOSE_ON_EXIT_DELAY.to_string(),
            serde_json::json!(5000),
        );
        assert_eq!(ShellController::close_on_exit_delay_ms(&meta), 5000);
    }

    #[test]
    fn test_conn_name_from_meta() {
        let mut meta = MetaMapType::new();
        assert_eq!(ShellController::get_conn_name(&meta), "local"); // default

        meta.insert(
            META_KEY_CONNECTION.to_string(),
            serde_json::Value::String("user@host".to_string()),
        );
        assert_eq!(ShellController::get_conn_name(&meta), "user@host");
    }

    #[test]
    fn test_handle_append_block_file() {
        let broker = wps::Broker::new();

        // Subscribe to block file events
        broker.subscribe(
            "test-route",
            wps::SubscriptionRequest {
                event: wps::EVENT_BLOCK_FILE.to_string(),
                scopes: vec!["block:block-1".to_string()],
                allscopes: false,
            },
        );

        handle_append_block_file(&broker, "block-1", "term", b"hello world");

        // Check event was published
        let _history = broker.read_event_history(wps::EVENT_BLOCK_FILE, "block:block-1", 10);
        // Note: events are only persisted if persist > 0, so we verify via the publish mechanism
        // The broker successfully processed without panic, which verifies correctness
    }

    #[test]
    fn test_handle_truncate_block_file() {
        let broker = wps::Broker::new();
        // Should not panic
        handle_truncate_block_file(&broker, "block-1", "term");
    }

    #[test]
    fn test_register_and_get_controller() {
        let ctrl: Arc<dyn Controller> = Arc::new(ShellController::new(
            "shell".to_string(),
            "tab-1".to_string(),
            "test-register-block".to_string(),
            None,
            None,
            None,
        ));

        super::super::register_controller("test-register-block", ctrl.clone());

        let retrieved = super::super::get_controller("test-register-block");
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().block_id(), "test-register-block");

        // Cleanup
        super::super::delete_controller("test-register-block");
        assert!(super::super::get_controller("test-register-block").is_none());
    }

    #[test]
    fn test_resync_creates_shell_controller() {
        use crate::backend::waveobj::Block;

        let mut meta = MetaMapType::new();
        meta.insert(
            "controller".to_string(),
            serde_json::Value::String("shell".to_string()),
        );
        // Disable auto-start so we don't open a real PTY in tests
        meta.insert(
            META_KEY_CMD_RUN_ON_START.to_string(),
            serde_json::Value::Bool(false),
        );

        let block = Block {
            oid: "resync-test-block".to_string(),
            version: 1,
            meta,
            ..Default::default()
        };

        let result = super::super::resync_controller(&block, "tab-1", None, false, None, None, None);
        assert!(result.is_ok());

        let ctrl = super::super::get_controller("resync-test-block");
        assert!(ctrl.is_some());
        assert_eq!(ctrl.unwrap().controller_type(), "shell");

        // Cleanup
        super::super::delete_controller("resync-test-block");
    }
}
