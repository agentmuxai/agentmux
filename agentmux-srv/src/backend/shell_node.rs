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

use std::collections::HashMap;
use std::sync::Arc;
use parking_lot::Mutex;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{mpsc, oneshot};

use crate::backend::wps::{Broker, WaveEvent, EVENT_SHELL_CHUNK};

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Per-shell stop handles. `ShellStop` (MCP tool / UI button) looks up a
/// `shell_id` and fires the oneshot, which makes the owning `ShellNodeRunner`
/// tree-kill its child. Mirrors `InstallSessionRegistry`; lives in
/// `AppState.shell_sessions`. Phase 3 of SPEC_PERSISTENT_SHELL_NODE.
#[derive(Default)]
pub struct ShellSessionRegistry {
    shells: Mutex<HashMap<String, oneshot::Sender<()>>>,
}

impl ShellSessionRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn insert(&self, shell_id: String, tx: oneshot::Sender<()>) {
        self.shells.lock().insert(shell_id, tx);
    }

    /// Request stop of a running shell. Returns false if the id is unknown
    /// (never started, or already exited). Removing here also closes the
    /// window for the runner's own natural-exit `remove`.
    pub fn stop(&self, shell_id: &str) -> bool {
        if let Some(tx) = self.shells.lock().remove(shell_id) {
            let _ = tx.send(());
            true
        } else {
            false
        }
    }

    /// Drop a shell's handle without signalling — called by the runner on
    /// natural exit so its `kill_task` resolves `Err` (= not stopped).
    fn remove(&self, shell_id: &str) {
        self.shells.lock().remove(shell_id);
    }

    /// Stop every running shell. For srv-shutdown cleanup so long-running
    /// children (`task dev` → `task.exe`/`node`) don't orphan and hold ports.
    /// Returns the number of shells signalled (so the caller can skip the
    /// grace-period sleep when there was nothing to stop).
    pub fn stop_all(&self) -> usize {
        let drained: Vec<_> = self.shells.lock().drain().map(|(_, tx)| tx).collect();
        let n = drained.len();
        for tx in drained {
            let _ = tx.send(());
        }
        n
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
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    #[cfg(unix)]
    {
        // Negative pid targets the process group created via process_group(0).
        let pgid = pid as i32;
        unsafe { libc::kill(-pgid, libc::SIGTERM) };
        std::thread::sleep(std::time::Duration::from_millis(300));
        unsafe { libc::kill(-pgid, libc::SIGKILL) };
    }
}

pub struct ShellNodeRunner {
    pub shell_id: String,
    pub block_id: String,
    pub cmd: String,
    pub cwd: Option<String>,
    pub extra_env: HashMap<String, String>,
    pub broker: Arc<Broker>,
    /// Stop registry — the runner registers its `shell_id` here so
    /// `ShellStop` can tree-kill it, and removes itself on natural exit.
    pub registry: Arc<ShellSessionRegistry>,
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
        // Null stdin (CRITICAL). The shell runs non-interactively and the srv's
        // own stdin is not a usable TTY. Without this the child inherits the srv
        // stdin, and tools that probe/read stdin at startup HANG before doing any
        // work — e.g. `npm run dev` spawned npm-cli.js but it never launched the
        // vite script (no output, never bound its port). Confirmed: the same
        // command with `< NUL` starts vite instantly (agentx/fix-shell-dev-server).
        // (When ShellInput lands — Phase 3b — this becomes a piped stdin.)
        child_cmd.stdin(std::process::Stdio::null());
        // Windows: suppress the console window that Windows auto-creates for
        // CUI-subsystem processes (cmd.exe). stdout/stderr are piped so no
        // output is lost — the window was decorative noise only.
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt as _;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
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

        // Register a stop handle. `ShellStop` fires `cancel_rx`, and `kill_task`
        // tree-kills this child. On natural exit we drop the sender (via
        // registry.remove) so `kill_task` resolves Err → "not stopped".
        let pid = child.id();
        tracing::info!(shell_id = %shell_id, pid = ?pid, "shell.spawn");
        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
        self.registry.insert(shell_id.clone(), cancel_tx);
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
