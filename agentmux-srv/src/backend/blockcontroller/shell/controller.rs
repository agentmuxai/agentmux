// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `ShellController` struct + inner state definitions, constructor, and the
//! small accessor / lock / meta helpers. The `Controller` trait impl and the
//! spawn/IO orchestration live in [`super::lifecycle`].

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Instant;

use tokio::sync::mpsc;

use super::super::{
    BlockControllerRuntimeStatus, BlockInputUnion, META_KEY_CMD, META_KEY_CMD_ARGS,
    META_KEY_CMD_CLEAR_ON_START, META_KEY_CMD_CLOSE_ON_EXIT, META_KEY_CMD_CLOSE_ON_EXIT_DELAY,
    META_KEY_CMD_CLOSE_ON_EXIT_FORCE, META_KEY_CMD_RUN_ONCE, META_KEY_CMD_RUN_ON_START,
    META_KEY_CONNECTION, STATUS_INIT,
};
use crate::backend::eventbus::EventBus;
use crate::backend::obj::{self, MetaMapType};
use crate::backend::shellexec::ConnInterface;
use crate::backend::storage::filestore::FileStore;
use crate::backend::storage::store::Store;
use crate::backend::mps;

/// Cap on the out-of-order input reorder buffer (`input_seq_buf`).
///
/// The input channel itself is now **unbounded** (`unbounded_channel`): a
/// bounded `try_send` silently dropped input on burst (large pastes arriving
/// faster than the PTY write loop drains — the original truncation bug), and
/// the proper backpressure remedy (`send().await`) can't be applied here
/// because `send_input` is a synchronous trait method holding a `std::Mutex`.
/// An unbounded channel guarantees no input is ever dropped; terminal input is
/// human-paced (and the frontend now chunks large pastes at 4 KB / 5 ms), and
/// the PTY drain loop keeps up, so the queue does not grow in practice. This
/// constant only bounds the reorder buffer (pathological out-of-order seqs).
pub(super) const SHELL_INPUT_CH_SIZE: usize = 256;

/// Inner state protected by mutex.
/// Grace period (seconds) between SIGTERM and SIGKILL during stop().
#[allow(dead_code)]
pub(super) const KILL_GRACE_SECS: u64 = 5;

pub(super) struct ShellControllerInner {
    /// Current process status.
    pub(super) proc_status: String,
    /// Process exit code.
    pub(super) proc_exit_code: i32,
    /// Status version counter (incremented on each change).
    pub(super) status_version: i32,
    /// Connection name for the shell process.
    pub(super) conn_name: String,
    /// Input channel sender (sends to the PTY input loop). Unbounded so input
    /// is never dropped on burst — see `SHELL_INPUT_CH_SIZE` doc.
    pub(super) input_tx: Option<mpsc::UnboundedSender<BlockInputUnion>>,
    /// Input channel receiver (consumed by the PTY input loop).
    #[allow(dead_code)]
    pub(super) input_rx: Option<mpsc::UnboundedReceiver<BlockInputUnion>>,
    /// OS PID of the running child process, kept for signal delivery in stop().
    pub(super) child_pid: Option<u32>,
    /// Unix timestamp (ms) when the process was spawned; None until first spawn.
    pub(super) spawn_ts_ms: Option<i64>,
    /// Monotonic instant of the most recent PTY read; None until first output.
    pub(super) last_pty_output: Option<Instant>,
    /// True if this pane is running an agent CLI (e.g. claude).
    pub(super) is_agent_pane: bool,
    /// This pane's own jekt/muxbus identity, captured once at spawn time
    /// from `resolve_agent_id_for_jekt` (see that fn's doc comment) — the
    /// independent source of truth `Controller::agent_id()` exposes for
    /// `inject_message_inner`'s recipient-identity check. `None` for a
    /// plain (non-agent) terminal pane.
    pub(super) agent_id: Option<String>,
    /// Next expected input seq number (per-TermViewModel monotonic counter).
    pub(super) input_seq_next: u64,
    /// Out-of-order input packets waiting for their seq slot (capped at SHELL_INPUT_CH_SIZE).
    pub(super) input_seq_buf: std::collections::BTreeMap<u64, BlockInputUnion>,
}

/// Factory function type for creating ConnInterface instances.
/// This allows dependency injection for testing.
pub type ConnFactory =
    Box<dyn Fn(&str, &MetaMapType) -> Result<Box<dyn ConnInterface>, String> + Send + Sync>;

/// ShellController manages one shell or command block.
pub struct ShellController {
    /// Controller type: "shell" or "cmd".
    pub(super) controller_type: String,
    pub(super) tab_id: String,
    pub(super) block_id: String,
    /// Prevents concurrent run() calls.
    pub(super) run_lock: Arc<AtomicBool>,
    /// Protected inner state.
    pub(super) inner: Arc<Mutex<ShellControllerInner>>,
    /// Optional factory for creating ConnInterface (for testing).
    pub(super) conn_factory: Mutex<Option<ConnFactory>>,
    /// MPS broker for publishing events (blockfile, controllerstatus).
    pub(super) broker: Option<Arc<mps::Broker>>,
    /// Event bus (unused for now, reserved for future event routing).
    #[allow(dead_code)]
    pub(super) event_bus: Option<Arc<EventBus>>,
    /// AgentMux object store — used to seed cmd:cwd on shell spawn.
    pub(super) wstore: Option<Arc<Store>>,
    /// FileStore write-through target for PTY output persistence
    /// (SPEC_TERMINAL_SCROLLBACK_PERSISTENCE_2026_07_23.md §2.1) — lets
    /// `handle_append_block_file`'s "term" writes survive a reconnect,
    /// mirroring `PersistentController`/`SubprocessController`'s existing
    /// filestore wiring.
    pub(super) filestore: Option<Arc<FileStore>>,
}

impl ShellController {
    /// Create a new ShellController.
    pub fn new(
        controller_type: String,
        tab_id: String,
        block_id: String,
        broker: Option<Arc<mps::Broker>>,
        event_bus: Option<Arc<EventBus>>,
        wstore: Option<Arc<Store>>,
        filestore: Option<Arc<FileStore>>,
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
                agent_id: None,
                input_seq_next: 0,
                input_seq_buf: std::collections::BTreeMap::new(),
            })),
            conn_factory: Mutex::new(None),
            broker,
            event_bus,
            wstore,
            filestore,
        }
    }

    /// Set a custom ConnInterface factory (for testing).
    #[allow(dead_code)]
    pub fn set_conn_factory(&self, factory: ConnFactory) {
        *self.conn_factory.lock().unwrap() = Some(factory);
    }

    /// Try to acquire the run lock. Returns false if already running.
    pub(super) fn try_lock_run(&self) -> bool {
        self.run_lock
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    /// Release the run lock.
    pub(super) fn unlock_run(&self) {
        self.run_lock.store(false, Ordering::SeqCst);
    }

    /// Update process status and increment version (must hold inner lock).
    pub(super) fn set_status(inner: &mut ShellControllerInner, status: &str) {
        inner.proc_status = status.to_string();
        inner.status_version += 1;
    }

    /// Get the runtime status (snapshot).
    pub(super) fn get_status_snapshot(&self) -> BlockControllerRuntimeStatus {
        let inner = self.inner.lock().unwrap();
        BlockControllerRuntimeStatus {
            blockid: self.block_id.clone(),
            version: inner.status_version,
            shellprocstatus: inner.proc_status.clone(),
            shellprocconnname: inner.conn_name.clone(),
            shellprocexitcode: inner.proc_exit_code,
            spawn_ts_ms: inner.spawn_ts_ms,
            is_agent_pane: inner.is_agent_pane,
            // The shell/PTY controller has no NDJSON-derived health monitor
            // (no structured turn-end marker to key off, unlike
            // persistent.rs/acp.rs) — leave unset rather than guess. Mount
            // reconciliation falls back to today's Idle default for these
            // panes, same as before this field existed.
            turn_active: false,
        }
    }

    /// Seconds since last PTY output, or None if no output yet.
    pub fn last_output_secs_ago(&self) -> Option<u64> {
        self.inner.lock().unwrap().last_pty_output.map(|t| t.elapsed().as_secs())
    }

    /// True if this pane is running an agent CLI.
    #[allow(dead_code)]
    pub fn is_agent_pane(&self) -> bool {
        self.inner.lock().unwrap().is_agent_pane
    }

    /// Check block meta for whether to run on start.
    pub(super) fn should_run_on_start(meta: &MetaMapType) -> bool {
        obj::meta_get_bool(meta, META_KEY_CMD_RUN_ON_START, true)
    }

    /// Check block meta for run-once mode (used in full lifecycle integration).
    #[allow(dead_code)]
    pub(super) fn should_run_once(meta: &MetaMapType) -> bool {
        obj::meta_get_bool(meta, META_KEY_CMD_RUN_ONCE, false)
    }

    /// Check block meta for clear-on-start (used in full lifecycle integration).
    #[allow(dead_code)]
    pub(super) fn should_clear_on_start(meta: &MetaMapType) -> bool {
        obj::meta_get_bool(meta, META_KEY_CMD_CLEAR_ON_START, false)
    }

    /// Check block meta for close-on-exit, as literally set — no default
    /// inference. Use [`effective_close_on_exit`](Self::effective_close_on_exit)
    /// at call sites that need the controller-type-aware default; this raw
    /// getter exists so that default logic has a single, testable "was it
    /// explicitly set" primitive to build on.
    pub(super) fn should_close_on_exit(meta: &MetaMapType) -> bool {
        obj::meta_get_bool(meta, META_KEY_CMD_CLOSE_ON_EXIT, false)
    }

    /// Whether this block should close its pane when the underlying process
    /// exits — [`should_close_on_exit`](Self::should_close_on_exit) layered
    /// with a controller-type-aware default for when `cmd:closeonexit`
    /// isn't set at all. An explicit value in meta always wins either way;
    /// this only decides the unset case.
    ///
    /// Defaults to `true` for `BLOCK_CONTROLLER_SHELL` (the interactive PTY
    /// both the `Terminal` widget and an agent's composer-drawer shell
    /// use) — matches the ordinary expectation that typing `exit` closes
    /// the pane, the default behavior of most terminal apps. Defaults to
    /// `false` for `BLOCK_CONTROLLER_CMD` (one-shot, non-interactive
    /// command execution, e.g. a build command) — that pane exists
    /// specifically to show its output/exit code after the command
    /// finishes, and auto-closing it by default would hide exactly the
    /// information it's for. See
    /// docs/specs/SPEC_TERM_EXIT_RESPAWN_LOOP_2026_09_15.md §10.
    pub(super) fn effective_close_on_exit(meta: &MetaMapType, controller_type: &str) -> bool {
        if meta.contains_key(META_KEY_CMD_CLOSE_ON_EXIT) {
            return Self::should_close_on_exit(meta);
        }
        controller_type == super::super::BLOCK_CONTROLLER_SHELL
    }

    /// Check block meta for force close-on-exit. **Not currently read by
    /// any call site** — kept as a meta getter (parsed, tested, and
    /// available for a future distinction) rather than deleted outright,
    /// because two attempts at giving it a real meaning during this PR
    /// both turned out unworkable, not because the getter itself is wrong:
    ///
    /// - Exit-code-based ("close even on a non-zero exit"): `wait()`'s
    ///   exit-code capture is itself lossy — portable-pty's `ExitStatus`
    ///   doesn't expose a raw code on all platforms, so success/failure is
    ///   all that's reliably known (see the wait/cleanup task's own
    ///   comment in `lifecycle.rs`) — not enough to build a principled
    ///   "force" distinction on.
    /// - Flusher-drain-timeout-based ("close even if a background
    ///   descendant might still be producing output"): tried and reverted
    ///   in this same PR after live-testing on Windows showed
    ///   `FLUSHER_DRAIN_TIMEOUT` (10s) gets hit on essentially every
    ///   ordinary `cmd.exe` exit — ConPTY's own console-host process
    ///   routinely outlives the direct child by design — which made
    ///   gating close-on-exit on it fire almost never, defeating the
    ///   whole feature on the one platform this was tested on. See
    ///   `docs/specs/SPEC_TERM_EXIT_RESPAWN_LOOP_2026_09_15.md` §10.
    #[allow(dead_code)]
    pub(super) fn should_close_on_exit_force(meta: &MetaMapType) -> bool {
        obj::meta_get_bool(meta, META_KEY_CMD_CLOSE_ON_EXIT_FORCE, false)
    }

    /// Get the close-on-exit delay in ms — how long to leave the pane
    /// showing its final output before actually closing it.
    ///
    /// **Defaults to 0 (close immediately).** The knob predates any caller
    /// and its original 2000 was never exercised; when close-on-exit was
    /// first wired up (§10) that 2s was kept as-is, and the reporter's
    /// first working retest asked for it to go — a shell you just typed
    /// `exit` into has already shown you whatever it was going to show, so
    /// the pause reads as lag, not as a chance to read the output. Still
    /// honored when set explicitly via `cmd:closeonexitdelay`, for a pane
    /// that genuinely wants a beat before vanishing. See
    /// docs/specs/SPEC_TERM_EXIT_RESPAWN_LOOP_2026_09_15.md §13.
    pub(super) fn close_on_exit_delay_ms(meta: &MetaMapType) -> u64 {
        match meta.get(META_KEY_CMD_CLOSE_ON_EXIT_DELAY) {
            Some(serde_json::Value::Number(n)) => n.as_u64().unwrap_or(0),
            _ => 0,
        }
    }

    /// Get the connection name from block meta.
    pub(super) fn get_conn_name(meta: &MetaMapType) -> String {
        obj::meta_get_string(meta, META_KEY_CONNECTION, "local")
    }

    /// Get the command string from block meta.
    pub(super) fn get_cmd_str(meta: &MetaMapType) -> String {
        obj::meta_get_string(meta, META_KEY_CMD, "")
    }

    /// Get cmd:args array from block meta.
    pub(super) fn get_cmd_args(meta: &MetaMapType) -> Vec<String> {
        match meta.get(META_KEY_CMD_ARGS) {
            Some(serde_json::Value::Array(arr)) => arr
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect(),
            _ => vec![],
        }
    }

    /// Check if cmd:interactive is set in block meta.
    pub(super) fn is_interactive(meta: &MetaMapType) -> bool {
        obj::meta_get_bool(meta, "cmd:interactive", false)
    }

    /// Publish current controller status via the MPS broker.
    pub(super) fn publish_status(&self) {
        if let Some(ref broker) = self.broker {
            let status = self.get_status_snapshot();
            super::super::publish_controller_status(broker, &status);
        }
    }
}
