// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! ShellNodeRunner — spawns a shell command and streams output to the
//! frontend as `shell_chunk` WPS events scoped to the agent's block.
//!
//! Launched by `handle_shell_create` (server/mod.rs) via `tokio::spawn`.
//! The runner is fire-and-forget; the HTTP handler returns the `shell_id`
//! to the MCP caller immediately while output streams asynchronously.
//!
//! Phase 2 of SPEC_PERSISTENT_SHELL_NODE_2026_06_11.md. Uses
//! `tokio::process::Command` with captured pipes (no PTY in this phase;
//! PTY support is a Phase 3 follow-up that requires portable-pty wiring
//! similar to the existing ShellController).

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use parking_lot::Mutex;
use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::sync::{mpsc, oneshot};

use crate::backend::wps::{Broker, WaveEvent, EVENT_SHELL_CHUNK};
use agentmux_common::api_types::ShellInputFailure;

fn now_ms() -> u64 {
    agentmux_common::time::now_ms_u64()
}

/// Live status of a running (or recently exited) shell. Updated by
/// `ShellNodeRunner` as output arrives and on exit.
///
/// `block_id`/`cmd`/`title`/`started_at` are populated once at spawn (see
/// `ShellNodeRunner::run`) and never change — kept here (rather than a
/// separate lookup) so `list_active` (SPEC_SWARM_LONG_RUNNING_PROCESS_
/// ROWS_2026_07_20) can answer "which shells belong to block X" from the
/// same map `get_status`/`ShellStatus` already query, with no new registry
/// or second source of truth.
#[derive(Clone, Default)]
pub struct ShellStatusInfo {
    pub running: bool,
    pub exit_code: Option<i32>,
    pub line_count: u64,
    pub block_id: String,
    pub cmd: String,
    pub title: String,
    pub started_at: u64,
}

/// Snapshot of one currently-running shell for the Swarm pane's per-agent
/// long-running-process rows. See `ShellSessionRegistry::list_active`.
/// Carries `block_id` (unfiltered list — same shape as `subagent.ListActive`/
/// `ActiveSubagent.parent_block_id`) so the Swarm pane's `buildTree()` groups
/// by block client-side in one pass, instead of one RPC call per tracked
/// agent block.
#[derive(Clone, Serialize)]
pub struct ShellSummary {
    pub shell_id: String,
    pub block_id: String,
    pub cmd: String,
    pub title: String,
    pub started_at: u64,
    pub line_count: u64,
}

// Per-shell registry entry: stop handle + optional stdin channel (Phase 3b).
// The stdin_tx is an mpsc sender to a relay task that owns the real ChildStdin.
// Sending text is non-blocking (unbounded channel); the relay handles the
// actual write. When all senders are dropped, the channel closes, the relay
// exits, ChildStdin is dropped, and the child sees EOF on stdin.
struct ShellEntry {
    stop_tx: oneshot::Sender<()>,
    // None for entries created by unit tests (no relay task / ChildStdin).
    stdin_tx: Option<mpsc::UnboundedSender<String>>,
}

/// Per-shell stop handles + status. `ShellStop` fires the oneshot; `ShellInput`
/// sends to the stdin relay; `ShellStatus` reads the live status arc.
/// Lives in `AppState.shell_sessions`.
/// Max number of EXITED shell statuses retained for post-exit ShellStatus
/// queries. Running shells are always kept regardless of this cap; only
/// exited entries beyond it are evicted oldest-first. Bounds memory for
/// long-lived sidecars that spawn many short shells.
const MAX_EXITED_STATUS: usize = 512;

#[derive(Default)]
pub struct ShellSessionRegistry {
    shells: Mutex<HashMap<String, ShellEntry>>,
    // Status entries persist after exit so ShellStatus can query final
    // exit_code/line_count. Exited entries are capped at MAX_EXITED_STATUS
    // (oldest-first eviction via status_order); running entries are never
    // evicted. stop_all() clears everything on srv shutdown.
    status_map: Mutex<HashMap<String, Arc<Mutex<ShellStatusInfo>>>>,
    // Spawn-order queue of shell_ids, used to evict the oldest exited status
    // entries first when over the cap.
    status_order: Mutex<VecDeque<String>>,
}

impl ShellSessionRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    // Used by unit tests — registers a stop handle with no stdin relay/status.
    fn insert(&self, shell_id: String, tx: oneshot::Sender<()>) {
        self.shells.lock().insert(shell_id, ShellEntry { stop_tx: tx, stdin_tx: None });
    }

    // Full registration called by the runner after spawning the child.
    // stdin_tx is None when capture_stdin is false (stdin is /dev/null).
    // Returns shell_ids evicted by the prune pass — the caller (which holds
    // the Broker, unlike this registry) purges their `shell:<id>` persisted
    // WaveEvent history so the broker's persist_map key set stays bounded
    // in step with this registry's own MAX_EXITED_STATUS cap.
    //
    // `pub(crate)` (rather than private) so `server::shell_handlers`'s own
    // tests can seed a running/exited shell directly, without spawning a
    // real child process, to exercise the `shellstatus` RPC handler's JSON
    // shape end-to-end for both branches — the same pre-tested primitive
    // this module's own tests already use, not duplicated logic.
    pub(crate) fn register_full(
        &self,
        shell_id: String,
        stop_tx: oneshot::Sender<()>,
        stdin_tx: Option<mpsc::UnboundedSender<String>>,
        status: Arc<Mutex<ShellStatusInfo>>,
    ) -> Vec<String> {
        self.shells.lock().insert(shell_id.clone(), ShellEntry { stop_tx, stdin_tx });
        self.status_map.lock().insert(shell_id.clone(), status);
        self.status_order.lock().push_back(shell_id);
        // Bound retained exited statuses; every spawn is a natural prune trigger.
        self.prune_exited_statuses()
    }

    /// Evict oldest EXITED status entries beyond `MAX_EXITED_STATUS`. Running
    /// shells are always retained (their status is read live via ShellStatus).
    /// Also drops any order-queue ids whose status entry is already gone.
    /// Returns the shell_ids actually evicted (status entry removed) so the
    /// caller can purge their broker-side persisted history too.
    fn prune_exited_statuses(&self) -> Vec<String> {
        let mut map = self.status_map.lock();
        let mut order = self.status_order.lock();
        let mut evicted = Vec::new();
        let mut exited = map
            .values()
            .filter(|arc| !arc.lock().running)
            .count();
        let mut i = 0;
        while exited > MAX_EXITED_STATUS && i < order.len() {
            let id = order[i].clone();
            match map.get(&id) {
                // Exited entry over the cap → evict it.
                Some(arc) if !arc.lock().running => {
                    map.remove(&id);
                    order.remove(i);
                    exited -= 1;
                    evicted.push(id);
                }
                // Still running → keep in place, advance.
                Some(_) => i += 1,
                // Stale order id (already removed) → drop from queue.
                None => {
                    order.remove(i);
                }
            }
        }
        evicted
    }

    /// Request stop of a running shell. Returns false if unknown or already exited.
    /// Dropping the stdin_tx here closes the relay channel → ChildStdin drops →
    /// child sees EOF, unblocking any stdin-reading process before the kill.
    pub fn stop(&self, shell_id: &str) -> bool {
        // Do NOT remove from status_map here — the runner task is still live
        // and will persist the final exit status after child.wait() returns.
        if let Some(entry) = self.shells.lock().remove(shell_id) {
            // stdin_tx dropped here → relay closes → child gets stdin EOF
            let _ = entry.stop_tx.send(());
            true
        } else {
            false
        }
    }

    /// Called by the runner on natural exit (stdout/stderr closed). Removes the
    /// shells entry (dropping stdin_tx → child gets stdin EOF). Status entry is
    /// intentionally kept so ShellStatus can query the final exit code/line count.
    fn remove(&self, shell_id: &str) {
        self.shells.lock().remove(shell_id);
        // status_map entry is NOT removed — it persists so get_status() can
        // return the final running:false / exit_code / line_count after exit.
    }

    /// Stop every running shell. For srv-shutdown cleanup.
    pub fn stop_all(&self) -> usize {
        self.status_map.lock().clear();
        self.status_order.lock().clear();
        let drained: Vec<_> = self.shells.lock().drain().map(|(_, e)| e.stop_tx).collect();
        let n = drained.len();
        for tx in drained {
            let _ = tx.send(());
        }
        n
    }

    /// Resolve where a `ShellInput` write should go, distinguishing two
    /// failure modes:
    /// - `Ok(tx)`            — running with captured stdin
    /// - `Err(StdinNotCaptured)` — running but created without capture_stdin
    /// - `Err(NotRunning)`   — unknown id or already exited
    pub fn resolve_stdin(
        &self,
        shell_id: &str,
    ) -> Result<mpsc::UnboundedSender<String>, ShellInputFailure> {
        let shells = self.shells.lock();
        match shells.get(shell_id) {
            Some(entry) => match &entry.stdin_tx {
                Some(tx) => Ok(tx.clone()),
                None => Err(ShellInputFailure::StdinNotCaptured),
            },
            None => Err(ShellInputFailure::NotRunning),
        }
    }

    /// Return a snapshot of the shell's status. Returns `running: false` if unknown.
    /// Existing, documented contract for `POST /api/v1/shell/status` (the MCP
    /// `ShellStatus` tool) — do not change: an agent calling this for an id
    /// it doesn't recognize expects a plain "not running" answer, not an error.
    pub fn get_status(&self, shell_id: &str) -> ShellStatusInfo {
        self.status_map.lock()
            .get(shell_id)
            .map(|arc| arc.lock().clone())
            .unwrap_or_default()
    }

    /// Like [`Self::get_status`], but `None` when no entry exists at all —
    /// distinguishing "genuinely unknown" from "known, and not running."
    ///
    /// Needed by the `shellstatus` RPC command (unlike the MCP tool above,
    /// which is fine collapsing "unknown" into "not running"): `register_full`
    /// only runs after the runner task spawns the child process, but
    /// `shell_node_create` is published to the frontend BEFORE that task is
    /// even scheduled (`handle_shell_create` in server/mod.rs). A caller that
    /// checks status immediately after seeing `shell_node_create` can race
    /// ahead of registration — reagent P1 on PR #2770: collapsing that race
    /// window into `running: false` was indistinguishable from a genuinely
    /// already-exited shell, so a fast, still-registering, genuinely LIVE
    /// shell (e.g. a real `task dev`) could be shown as failed in the
    /// Activity Dock for its entire run.
    pub(crate) fn get_status_if_known(&self, shell_id: &str) -> Option<ShellStatusInfo> {
        self.status_map.lock().get(shell_id).map(|arc| arc.lock().clone())
    }

    /// List every currently-RUNNING shell across all blocks — the Swarm
    /// pane's data source for its per-agent long-running-process rows
    /// (SPEC_SWARM_LONG_RUNNING_PROCESS_ROWS_2026_07_20). Unfiltered, like
    /// `subagent.ListActive`/`ListDispatches` — the Swarm pane fetches once
    /// and groups by `block_id` client-side in `buildTree()`, rather than
    /// one RPC per tracked agent block. Exited shells are intentionally
    /// excluded: Phase 1's scope is "what's happening now," not a shell
    /// history view.
    pub fn list_active(&self) -> Vec<ShellSummary> {
        self.status_map
            .lock()
            .iter()
            .filter_map(|(shell_id, arc)| {
                let s = arc.lock();
                if s.running {
                    Some(ShellSummary {
                        shell_id: shell_id.clone(),
                        block_id: s.block_id.clone(),
                        cmd: s.cmd.clone(),
                        title: s.title.clone(),
                        started_at: s.started_at,
                        line_count: s.line_count,
                    })
                } else {
                    None
                }
            })
            .collect()
    }
}

/// Kill the entire process tree rooted at `pid`. `Child::kill` / `kill_on_drop`
/// only reap the wrapper shell (`cmd /C` / `sh -c`); a `task dev` spawns
/// `task.exe` → `cargo`/`node` grandchildren that survive otherwise.
///
/// Scoped strictly to OUR `pid` — never by image name. Killing `task.exe`
/// by name is the cross-instance hazard that let agent "Mazs" nuke peer
/// dev servers (see SPEC_PERSISTENT_SHELL_PHASE3_STOP §1).
fn kill_tree(pid: u32) {
    // Group-kill with SIGTERM → 300ms → SIGKILL escalation on Unix (the child
    // is spawned via process_group(0), so the negative-pid signal reaches its
    // descendants); `taskkill /F /T /PID` on Windows. The implementation is
    // shared with the other by-PID kill sites in `agentmux_common::process`.
    agentmux_common::process::kill_process_group(pid);
}

pub struct ShellNodeRunner {
    pub shell_id: String,
    pub block_id: String,
    pub cmd: String,
    /// Caller-supplied display title, defaulting to `cmd` — mirrors
    /// `handle_shell_create`'s own `title` local, threaded through so
    /// `ShellStatusInfo`/`list_active` can show it without a second lookup.
    pub title: String,
    pub cwd: Option<String>,
    pub extra_env: HashMap<String, String>,
    pub broker: Arc<Broker>,
    /// Stop registry — the runner registers its `shell_id` here so
    /// `ShellStop` can tree-kill it, and removes itself on natural exit.
    pub registry: Arc<ShellSessionRegistry>,
    /// If true, pipe stdin and start the relay task so ShellInput() works.
    /// If false (default), stdin is /dev/null — avoids blocking programs that
    /// read stdin to EOF (e.g. `cat` with no args).
    pub capture_stdin: bool,
}

impl ShellNodeRunner {
    pub async fn run(self) {
        let shell_id = self.shell_id.clone();
        let block_id = self.block_id.clone();
        let broker = self.broker.clone();

        let mut child_cmd = if cfg!(windows) {
            let mut c = tokio::process::Command::new("cmd");
            c.args(["/C", &self.cmd]);
            c
        } else {
            let mut c = tokio::process::Command::new("sh");
            c.args(["-c", &self.cmd]);
            c
        };

        // Only set the working directory if it actually exists. The cwd is
        // normalized upstream (handle_shell_create → base::normalize_working_dir),
        // but a stale or mistyped path would otherwise make the spawn fail hard
        // with os error 267 (ERROR_DIRECTORY) on Windows. Mirror the agent-CLI
        // spawn's graceful fallback (subprocess.rs): warn via a system chunk and
        // run in the server's cwd rather than killing the shell before it starts.
        if let Some(ref cwd) = self.cwd {
            if std::path::Path::new(cwd).is_dir() {
                child_cmd.current_dir(cwd);
            } else {
                publish_chunk(
                    &broker,
                    &block_id,
                    &shell_id,
                    "system",
                    &format!("[cwd not found: {cwd} — running in the server's working directory]"),
                    now_ms(),
                );
            }
        }
        for (k, v) in &self.extra_env {
            child_cmd.env(k, v);
        }

        child_cmd.stdout(std::process::Stdio::piped());
        child_cmd.stderr(std::process::Stdio::piped());
        // stdin: piped only when capture_stdin=true (opt-in). Default is null so
        // programs that read stdin to EOF (e.g. `cat` with no args) don't block
        // forever. When piped, a relay task forwards ShellInput() writes; when all
        // senders drop the relay exits, ChildStdin drops, and the child sees EOF.
        if self.capture_stdin {
            child_cmd.stdin(std::process::Stdio::piped());
        } else {
            child_cmd.stdin(std::process::Stdio::null());
        }
        // Windows: suppress the console window that Windows auto-creates for
        // CUI-subsystem processes (cmd.exe). stdout/stderr are piped so no
        // output is lost — the window was decorative noise only.
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt as _;
            use agentmux_common::win32::CREATE_NO_WINDOW;
            child_cmd.creation_flags(CREATE_NO_WINDOW);
        }
        // Backstop: reap the wrapper shell if this runner task is ever dropped.
        child_cmd.kill_on_drop(true);
        // Unix: own process group so kill_tree can signal the whole group
        // (-pgid). Windows uses taskkill /T on the pid instead.
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt as _;
            child_cmd.process_group(0);
        }

        let mut child = match child_cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                publish_chunk(&broker, &block_id, &shell_id, "system", &format!("[spawn error: {e}]"), now_ms());
                publish_exit(&broker, &block_id, &shell_id, 1, false, now_ms());
                return;
            }
        };

        let pid = child.id();
        tracing::info!(shell_id = %shell_id, pid = ?pid, capture_stdin = self.capture_stdin, "shell.spawn");

        // Spawn stdin relay only when capture_stdin=true.
        let stdin_tx: Option<mpsc::UnboundedSender<String>> = if self.capture_stdin {
            let child_stdin = child.stdin.take().expect("stdin piped");
            let (tx, mut rx) = mpsc::unbounded_channel::<String>();
            tokio::spawn(async move {
                let mut writer = BufWriter::new(child_stdin);
                while let Some(text) = rx.recv().await {
                    if writer.write_all(text.as_bytes()).await.is_err() { break; }
                    if writer.flush().await.is_err() { break; }
                }
                // All senders dropped or write error → ChildStdin dropped → child EOF
            });
            Some(tx)
        } else {
            None
        };

        let status_arc = Arc::new(Mutex::new(ShellStatusInfo {
            running: true,
            exit_code: None,
            line_count: 0,
            block_id: block_id.clone(),
            cmd: self.cmd.clone(),
            title: self.title.clone(),
            started_at: now_ms(),
        }));
        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
        let evicted = self.registry.register_full(
            shell_id.clone(),
            cancel_tx,
            stdin_tx,
            Arc::clone(&status_arc),
        );
        for evicted_id in evicted {
            broker.purge_scope(&format!("shell:{evicted_id}"));
        }
        let kill_task = tokio::spawn(async move {
            match cancel_rx.await {
                Ok(()) => {
                    if let Some(pid) = pid {
                        let _ = tokio::task::spawn_blocking(move || kill_tree(pid)).await;
                    }
                    true // stopped by request
                }
                Err(_) => false, // sender dropped → natural exit
            }
        });

        let stdout = child.stdout.take().expect("stdout piped");
        let stderr = child.stderr.take().expect("stderr piped");

        // Collect both stdout and stderr into a single ordered channel.
        // Each task owns its half of the pipe; the channel closes when both
        // tasks finish, at which point the main loop exits and we wait for exit.
        type Line = (&'static str, String, u64);
        let (tx, mut rx) = mpsc::unbounded_channel::<Line>();

        let tx_out = tx.clone();
        let t_stdout = tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let ts = now_ms();
                let _ = tx_out.send(("stdout", line, ts));
            }
        });

        let tx_err = tx.clone();
        let t_stderr = tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let ts = now_ms();
                let _ = tx_err.send(("stderr", line, ts));
            }
        });

        // Drop original sender so channel closes once both reader tasks finish.
        drop(tx);

        let mut line_count: u64 = 0;
        while let Some((kind, content, ts)) = rx.recv().await {
            line_count += 1;
            status_arc.lock().line_count = line_count;
            publish_chunk(&broker, &block_id, &shell_id, kind, &content, ts);
        }

        let _ = tokio::join!(t_stdout, t_stderr);

        // Drop our stop handle (no-op if ShellStop already removed it) so the
        // natural-exit path lets `kill_task` resolve.
        self.registry.remove(&shell_id);

        let exit_code = match child.wait().await {
            Ok(status) => status.code().unwrap_or(-1),
            Err(_) => -1,
        };

        let was_stopped = kill_task.await.unwrap_or(false);
        tracing::info!(shell_id = %shell_id, exit_code, was_stopped, line_count, "shell.exit");

        // Persist final status in the status_map so ShellStatus can still query it.
        {
            let mut s = status_arc.lock();
            s.running = false;
            s.exit_code = Some(exit_code);
            s.line_count = line_count;
        }

        publish_exit(&broker, &block_id, &shell_id, exit_code, was_stopped, now_ms());
    }
}

// Every shell_chunk / exit event is published under a SINGLE scope:
//   - `shell:<shell_id>`  → a per-shell persistence ring buffer (1024 entries).
//
// Single-scope delivery (chosen over the fallback of strengthening the reducer's
// dedup): an earlier design also published every chunk under `block:<block_id>`
// for "live" delivery, so output produced before the frontend's `shell:<id>`
// subscription established (the common case — process spawn + first lines beat
// the WS resub round-trip) arrived LIVE via the block scope AND then again in the
// replay burst when the broker replayed the whole `shell:<id>` ring on first
// subscribe. The reducer's last-chunk-only `isDuplicate` let those non-adjacent
// dups through → doubled output (full duplication on WS reconnect).
//
// Now chunks/exit go ONLY to `shell:<shell_id>`. The frontend subscribes to that
// scope when it sees the (block-scoped, persist:64) `shell_node_create`. Because
// the broker persists the ring regardless of subscribers (wps.rs persist_event
// runs inside publish whenever persist>0), any output produced before the
// subscription establishes is retained in the persist:1024 ring and replayed
// exactly once on subscribe (guarded by the broker's per-route+event+scope
// `replayed` set). No chunk is ever delivered via two paths → no duplication.
//
// This still preserves the reason the per-shell ring was introduced: each shell
// has its OWN ring, so a chatty shell can't evict another shell's `exit` event
// (the bug that left a sibling's row stuck `running` after a remount when all
// shells shared the block ring).
fn shell_scopes(shell_id: &str) -> Vec<String> {
    vec![format!("shell:{shell_id}")]
}

fn publish_chunk(broker: &Broker, _block_id: &str, shell_id: &str, kind: &str, content: &str, ts: u64) {
    broker.publish(WaveEvent {
        event: EVENT_SHELL_CHUNK.to_string(),
        scopes: shell_scopes(shell_id),
        sender: String::new(),
        persist: 1024,
        data: Some(serde_json::json!({
            "shell_id": shell_id,
            "op": "chunk",
            "kind": kind,
            "content": content,
            "timestamp": ts,
        })),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stop_signals_then_is_idempotent() {
        let reg = ShellSessionRegistry::new();
        let (tx, rx) = oneshot::channel::<()>();
        reg.insert("s1".to_string(), tx);
        assert!(reg.stop("s1")); // fires the channel
        assert!(rx.await.is_ok());
        assert!(!reg.stop("s1")); // second stop: unknown id
    }

    #[tokio::test]
    async fn remove_drops_sender_without_signalling() {
        let reg = ShellSessionRegistry::new();
        let (tx, rx) = oneshot::channel::<()>();
        reg.insert("s2".to_string(), tx);
        reg.remove("s2"); // natural-exit path
        assert!(rx.await.is_err()); // sender dropped → Err, not Ok
        assert!(!reg.stop("s2"));
    }

    #[tokio::test]
    async fn stop_all_fires_every_handle() {
        let reg = ShellSessionRegistry::new();
        let (tx1, rx1) = oneshot::channel::<()>();
        let (tx2, rx2) = oneshot::channel::<()>();
        reg.insert("a".to_string(), tx1);
        reg.insert("b".to_string(), tx2);
        reg.stop_all();
        assert!(rx1.await.is_ok());
        assert!(rx2.await.is_ok());
    }

    fn register_exited(reg: &ShellSessionRegistry, id: &str, running: bool) {
        let (tx, _rx) = oneshot::channel::<()>();
        let status = Arc::new(Mutex::new(ShellStatusInfo {
            running,
            exit_code: if running { None } else { Some(0) },
            line_count: 1,
            ..Default::default()
        }));
        // _rx is intentionally dropped; we only exercise the status-map cap.
        reg.register_full(id.to_string(), tx, None, status);
    }

    #[tokio::test]
    async fn exited_statuses_are_capped_oldest_first() {
        let reg = ShellSessionRegistry::new();
        let total = MAX_EXITED_STATUS + 10;
        for i in 0..total {
            register_exited(&reg, &format!("s{i}"), false);
        }
        // Never exceeds the cap.
        assert_eq!(reg.status_map.lock().len(), MAX_EXITED_STATUS);
        // Oldest 10 evicted → get_status returns default (exit_code None).
        assert!(reg.get_status("s0").exit_code.is_none());
        assert!(reg.get_status("s9").exit_code.is_none());
        // Newest retained with its real exit code.
        assert_eq!(reg.get_status(&format!("s{}", total - 1)).exit_code, Some(0));
    }

    #[tokio::test]
    async fn running_statuses_are_never_evicted() {
        let reg = ShellSessionRegistry::new();
        let total = MAX_EXITED_STATUS + 5;
        for i in 0..total {
            register_exited(&reg, &format!("r{i}"), true); // all running
        }
        // Running shells exceed the exited cap but none are evicted.
        assert_eq!(reg.status_map.lock().len(), total);
        assert!(reg.get_status("r0").running);
    }

    fn register_running_for_block(reg: &ShellSessionRegistry, id: &str, block_id: &str, cmd: &str) {
        let (tx, _rx) = oneshot::channel::<()>();
        let status = Arc::new(Mutex::new(ShellStatusInfo {
            running: true,
            block_id: block_id.to_string(),
            cmd: cmd.to_string(),
            title: cmd.to_string(),
            started_at: 1_000,
            ..Default::default()
        }));
        reg.register_full(id.to_string(), tx, None, status);
    }

    #[tokio::test]
    async fn list_active_returns_only_running_shells_across_all_blocks() {
        let reg = ShellSessionRegistry::new();
        register_running_for_block(&reg, "s1", "block-a", "npm run dev");
        register_running_for_block(&reg, "s2", "block-b", "task dev");
        register_exited(&reg, "s3", false); // exited — must not appear

        let mut active = reg.list_active();
        active.sort_by(|a, b| a.shell_id.cmp(&b.shell_id));

        assert_eq!(active.len(), 2);
        assert_eq!(active[0].shell_id, "s1");
        assert_eq!(active[0].block_id, "block-a");
        assert_eq!(active[0].cmd, "npm run dev");
        assert_eq!(active[1].shell_id, "s2");
        assert_eq!(active[1].block_id, "block-b");
    }
}

fn publish_exit(broker: &Broker, _block_id: &str, shell_id: &str, exit_code: i32, stopped: bool, ts: u64) {
    broker.publish(WaveEvent {
        event: EVENT_SHELL_CHUNK.to_string(),
        scopes: shell_scopes(shell_id),
        sender: String::new(),
        persist: 1024,
        data: Some(serde_json::json!({
            "shell_id": shell_id,
            "op": "exit",
            "exit_code": exit_code,
            // True when the exit was caused by ShellStop (tree-killed), so the
            // frontend renders the grey "stopped" status instead of exited-err.
            "stopped": stopped,
            "timestamp": ts,
        })),
    });
}
