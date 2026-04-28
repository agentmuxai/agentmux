// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase B.2: host-side client for the launcher's named-pipe IPC
// server. Connects on host startup, sends `Register`, holds the
// connection open for the host's lifetime.
//
// Today (B.2) the connection is fire-and-hold — we Register and
// then leave the read half idle. B.3 will start receiving `Event`s
// from the launcher (state changes), B.4 will send `Command`s up,
// B.7 will bridge events to the renderer process.
//
// Activated only when `AGENTMUX_LAUNCHER_PIPE` is set (production
// portable / installed paths after PR #571 + this PR). Absent →
// `task dev` mode where launcher isn't in the loop; host runs as
// before.

use std::sync::Arc;
use std::sync::OnceLock;

use agentmux_common::ipc::{ClientKind, Command, Event};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, Mutex};

#[cfg(target_os = "windows")]
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};

/// Phase B.4 — global outbound command channel. Set once when the
/// launcher pipe connects; sync callers (CEF lifecycle callbacks
/// fire on the UI thread) post `Command`s without needing a
/// tokio runtime handle. Drained by a task spawned in
/// `connect_to_launcher` that writes to the pipe.
///
/// `None` semantics: launcher IPC isn't connected (e.g. `task dev`
/// mode where launcher isn't in the loop). `report_window_*` calls
/// are silently no-ops; the host runs as before.
static COMMAND_TX: OnceLock<mpsc::UnboundedSender<Command>> = OnceLock::new();

/// Handle the host holds for its lifetime so the launcher's IPC
/// pipe stays open. Dropping it closes the connection (launcher
/// logs as a disconnect).
///
/// Cross-platform shape: the Windows variant carries the actual
/// pipe writer + reader-task handles; the non-Windows variant is
/// a unit-struct stub so `connect_to_launcher` keeps a uniform
/// `Option<LauncherIpcHandle>` return type. Phase 7's cross-
/// platform IPC (Unix domain sockets) will fill the non-Windows
/// shape later. (reagent / codex P1 PR #573 round-1.)
#[cfg(target_os = "windows")]
pub struct LauncherIpcHandle {
    /// Held purely as a keepalive; B.3+ will replace with a real
    /// command-sender that uses this writer.
    #[allow(dead_code)]
    writer: Arc<Mutex<tokio::io::WriteHalf<NamedPipeClient>>>,
    /// Read-half task handle. Currently just consumes bytes off the
    /// wire and logs them. B.4+ replaces with an event dispatcher.
    #[allow(dead_code)]
    reader_task: tokio::task::JoinHandle<()>,
}

#[cfg(not(target_os = "windows"))]
pub struct LauncherIpcHandle;

/// If `AGENTMUX_LAUNCHER_PIPE` is set, connect, Register as Host,
/// and return a handle the caller (host main.rs) holds for the
/// host's lifetime. If unset → return None and the host runs in
/// pre-Phase-B mode (no launcher connection).
///
/// Errors are logged but non-fatal: a launcher-IPC failure should
/// NOT prevent the host from running, since the launcher's
/// authoritative state is still in environment / files for B.2.
/// Phase B.5+ will tighten this when the host actually depends on
/// IPC for state.
#[cfg(target_os = "windows")]
pub async fn connect_to_launcher() -> Option<LauncherIpcHandle> {
    let pipe_path = match std::env::var("AGENTMUX_LAUNCHER_PIPE") {
        Ok(p) if !p.is_empty() => p,
        _ => {
            tracing::info!(
                "AGENTMUX_LAUNCHER_PIPE unset — running without launcher IPC (dev mode)"
            );
            return None;
        }
    };

    // Open the named pipe (client side). The launcher created it
    // with `first_pipe_instance(true)` and is accept-looping; this
    // call blocks briefly until accept lands. tokio retries
    // ERROR_PIPE_BUSY internally.
    let client = match ClientOptions::new().open(&pipe_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(
                "[launcher-ipc] failed to open {}: {} — continuing without launcher IPC",
                pipe_path,
                e
            );
            return None;
        }
    };
    tracing::info!("[launcher-ipc] connected to {}", pipe_path);

    let (read_half, write_half) = tokio::io::split(client);
    let writer = Arc::new(Mutex::new(write_half));

    // Send Register FIRST. Server's `enforce_register_first` requires
    // it as the very first message on the wire, and a fatal-close
    // on violation would permanently disable the mirror (no
    // reconnect path in B.4). The drain task that processes
    // outbound commands gets spawned + the global sender published
    // only AFTER this succeeds. (reagent P1 + codex P2 PR #576
    // round-1 — race where a `report_window_*` call between
    // `COMMAND_TX.set` and the Register flush could land first on
    // the wire.)
    let register = Command::Register {
        kind: ClientKind::Host,
        pid: std::process::id(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    };
    let mut buf = match serde_json::to_vec(&register) {
        Ok(b) => b,
        Err(e) => {
            tracing::error!(
                "[launcher-ipc] failed to serialize Register: {} — continuing without IPC",
                e
            );
            return None;
        }
    };
    buf.push(b'\n');
    {
        let mut w = writer.lock().await;
        if let Err(e) = w.write_all(&buf).await {
            tracing::error!(
                "[launcher-ipc] failed to send Register: {} — continuing without IPC",
                e
            );
            return None;
        }
        if let Err(e) = w.flush().await {
            tracing::error!(
                "[launcher-ipc] failed to flush Register: {} — continuing without IPC",
                e
            );
            return None;
        }
    }

    // Phase B.4 — Register confirmed on the wire; safe to expose the
    // outbound command channel. Sync callers push Commands; a
    // dedicated drain task writes them as newline-delimited JSON.
    // Ordered (preserves report order from the UI thread). Bounded?
    // No — UnboundedSender so `try_send` can stay non-blocking on
    // the UI thread; the drain task is fast (single async write)
    // and the volume is one event per window create/close, not
    // high-frequency.
    let (tx, mut rx) = mpsc::unbounded_channel::<Command>();
    if COMMAND_TX.set(tx).is_err() {
        tracing::warn!("[launcher-ipc] COMMAND_TX already set — connect_to_launcher called twice");
    }
    let writer_for_drain = Arc::clone(&writer);
    tokio::spawn(async move {
        while let Some(cmd) = rx.recv().await {
            let mut buf = match serde_json::to_vec(&cmd) {
                Ok(b) => b,
                Err(e) => {
                    tracing::warn!("[launcher-ipc] failed to serialize {:?}: {}", cmd, e);
                    continue;
                }
            };
            buf.push(b'\n');
            let mut w = writer_for_drain.lock().await;
            if let Err(e) = w.write_all(&buf).await {
                tracing::warn!(
                    "[launcher-ipc] write failed for {:?}: {} — dropping further commands",
                    cmd, e
                );
                return;
            }
            if let Err(e) = w.flush().await {
                tracing::warn!("[launcher-ipc] flush failed: {}", e);
                return;
            }
        }
    });

    // Spawn a read-loop task. For B.2 it just logs every Event the
    // server sends; B.3+ will dispatch them into host state.
    let reader_task = tokio::spawn(async move {
        let reader = BufReader::new(read_half);
        let mut lines = reader.lines();
        loop {
            match lines.next_line().await {
                Ok(Some(line)) if line.trim().is_empty() => continue,
                Ok(Some(line)) => match serde_json::from_str::<Event>(&line) {
                    Ok(event) => {
                        tracing::info!("[launcher-ipc] received event: {:?}", event);
                    }
                    Err(e) => {
                        tracing::warn!(
                            "[launcher-ipc] could not parse event line ({}): {}",
                            e,
                            line
                        );
                    }
                },
                Ok(None) => {
                    tracing::info!("[launcher-ipc] launcher pipe EOF — connection closed");
                    return;
                }
                Err(e) => {
                    tracing::warn!("[launcher-ipc] read error: {}", e);
                    return;
                }
            }
        }
    });

    Some(LauncherIpcHandle {
        writer,
        reader_task,
    })
}

#[cfg(not(target_os = "windows"))]
pub async fn connect_to_launcher() -> Option<LauncherIpcHandle> {
    None
}

/// Phase B.4 — sync API: report a window open to the launcher's
/// state mirror. Called from CEF lifecycle callbacks on the UI
/// thread. No-op if the launcher pipe isn't connected (`task dev`
/// mode); failures to enqueue (channel closed, drain task died)
/// are logged but don't propagate — the host's authoritative state
/// is unaffected, the mirror just falls behind. B.5 tightens.
pub fn report_window_opened(
    label: String,
    kind: agentmux_common::ipc::WindowKind,
    parent_label: Option<String>,
) {
    let Some(tx) = COMMAND_TX.get() else {
        return; // launcher not in the loop
    };
    let cmd = Command::ReportWindowOpened {
        label,
        kind,
        parent_label,
    };
    if let Err(e) = tx.send(cmd) {
        tracing::warn!("[launcher-ipc] report_window_opened: channel closed ({})", e);
    }
}

/// Phase B.4 — sync API: report a window close to the launcher's
/// state mirror. Same no-op-if-disconnected semantics as
/// `report_window_opened`.
pub fn report_window_closed(label: String) {
    let Some(tx) = COMMAND_TX.get() else {
        return;
    };
    let cmd = Command::ReportWindowClosed { label };
    if let Err(e) = tx.send(cmd) {
        tracing::warn!("[launcher-ipc] report_window_closed: channel closed ({})", e);
    }
}
