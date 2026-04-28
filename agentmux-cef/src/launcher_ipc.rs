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
pub async fn connect_to_launcher(
    state: std::sync::Arc<crate::state::AppState>,
) -> Option<LauncherIpcHandle> {
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

    // Spawn a read-loop task. Logs every event and feeds the host's
    // shadow projections (`shadow_instance_registry` for now;
    // `shadow_*` for other migrated maps in subsequent B.5 sub-PRs)
    // from the launcher's authoritative event stream. Host has no
    // local `WindowInstanceRegistry` post-B.5e — the shadow IS the
    // read path.
    let state_for_reader = std::sync::Arc::clone(&state);
    let reader_task = tokio::spawn(async move {
        let reader = BufReader::new(read_half);
        let mut lines = reader.lines();
        loop {
            match lines.next_line().await {
                Ok(Some(line)) if line.trim().is_empty() => continue,
                Ok(Some(line)) => match serde_json::from_str::<Event>(&line) {
                    Ok(event) => {
                        tracing::info!("[launcher-ipc] received event: {:?}", event);
                        apply_event_to_shadow(&state_for_reader, &event);
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
pub async fn connect_to_launcher(
    _state: std::sync::Arc<crate::state::AppState>,
) -> Option<LauncherIpcHandle> {
    None
}

/// Phase B.5e — apply launcher events to host's shadow projections.
/// Currently handles instance-number events; future B.5 sub-PRs will
/// add more shadow handlers as additional host maps migrate. Other
/// event types are no-ops here (they're already logged at the call
/// site).
fn apply_event_to_shadow(state: &std::sync::Arc<crate::state::AppState>, event: &Event) {
    match event {
        Event::WindowInstanceAssigned { label, num, .. } => {
            // Phase B.5e — host's `WindowInstanceRegistry` was
            // deleted. The drift compare that used to live here
            // (B.5b/c/d) is gone; the launcher is the sole
            // authority and the shadow is the host's projection.
            state
                .shadow_instance_registry
                .lock()
                .insert(label.clone(), *num);
            // Phase B.5d — re-emit `window-instances-changed` so the
            // frontend's InstancePanel catches up after the brief
            // race between the synchronous emit at the mutation site
            // (which carries the pre-event count) and the launcher's
            // `WindowInstanceAssigned` arrival here.
            //
            // Phase B.7.1 — payload now carries the resolved
            // `entries` array (label + windowId pairs from launcher
            // shadows), letting the frontend update its atoms
            // directly without a follow-up `list_window_instances`
            // RPC and without the retry-on-stale polling that
            // existed only because the synchronous emit fired
            // before state.browsers settled. Sync emits still
            // ship count-only payloads; the frontend keeps the
            // legacy refresh path as a fallback for those.
            emit_window_instances_changed_with_entries(state);
        }
        Event::WindowOpened { label, kind, parent_label, .. } => {
            // Phase B.5 (window_meta step b) — shadow projection
            // + drift compare. Map the wire `WindowKind` back to
            // host's `crate::state::WindowKind` (one-to-one).
            let host_kind = match kind {
                agentmux_common::ipc::WindowKind::FullInstance => crate::state::WindowKind::FullInstance,
                agentmux_common::ipc::WindowKind::Subwindow => crate::state::WindowKind::Subwindow,
            };
            let shadow_meta = crate::state::WindowMeta {
                label: label.clone(),
                kind: host_kind,
                parent_instance_id: parent_label.clone(),
            };
            // Drift compare to host's authoritative `window_meta`.
            // Race-tolerant: host's local insert (in
            // `on_after_created`) may not have run yet if the
            // launcher event raced ahead. Skip in that case.
            let host_meta = state.window_meta.lock().get(label).cloned();
            if let Some(host_meta) = host_meta {
                if host_meta.kind != shadow_meta.kind
                    || host_meta.parent_instance_id != shadow_meta.parent_instance_id
                {
                    tracing::warn!(
                        target: "launcher-ipc:drift",
                        label = %label,
                        launcher_kind = ?shadow_meta.kind,
                        launcher_parent = ?shadow_meta.parent_instance_id,
                        host_kind = ?host_meta.kind,
                        host_parent = ?host_meta.parent_instance_id,
                        "[launcher-ipc] window_meta drift: host and launcher disagree"
                    );
                }
            }
            state
                .shadow_window_meta
                .lock()
                .insert(label.clone(), shadow_meta);
        }
        Event::WindowClosed { label, .. } => {
            // Phase B.5 (window_meta step b) — symmetric drop.
            // Drift compare on close is omitted: the launcher's
            // strict-pairing semantic (PR #577 round-2) means we
            // only see this event when the open was paired, so a
            // host-side missing entry would have surfaced at
            // open time.
            state.shadow_window_meta.lock().remove(label);
        }
        Event::BackendWindowIdRegistered { label, window_id, .. } => {
            // Phase B.5 (window_id_map step e) — host's
            // `window_id_map` was deleted; the drift compare is
            // gone with it. Shadow is sole source of truth.
            state
                .shadow_backend_window_ids
                .lock()
                .insert(label.clone(), window_id.clone());
        }
        Event::BackendWindowIdUnregistered { label, .. } => {
            // Phase B.5 (window_id_map step e) — drift compare gone.
            state.shadow_backend_window_ids.lock().remove(label);
        }
        Event::WindowInstanceReleased { label, .. } => {
            // Phase B.5e — host's `WindowInstanceRegistry` was
            // deleted; the drift compare is gone with it.
            state.shadow_instance_registry.lock().remove(label);
            // Phase B.5d / B.7.1 — re-emit with the resolved
            // entries array (see WindowInstanceAssigned arm above
            // for rationale).
            emit_window_instances_changed_with_entries(state);
        }
        _ => {}
    }
}

/// Phase B.7.1 — emit `window-instances-changed` with a resolved
/// `entries` array so the frontend can update its atoms directly,
/// bypassing both the `list_window_instances` RPC round-trip and
/// the historical retry-on-stale polling in `app-init.ts`.
///
/// Called from the launcher-driven re-emit sites (post-shadow
/// update) where the `shadow_instance_registry` and
/// `shadow_backend_window_ids` projections are guaranteed to
/// reflect the launcher's view. Pool windows and `browser-pane-*`
/// labels are filtered out here so the payload matches the
/// `list_window_instances` shape the frontend already understands.
///
/// Sync emit sites (window.rs / drag.rs / window_pool.rs / client.rs)
/// continue to ship count-only payloads in this PR — they fire
/// before host state has settled and don't have access to the
/// launcher's resolved view. B.7.2 retires them in favor of the
/// launcher-driven path.
fn emit_window_instances_changed_with_entries(state: &std::sync::Arc<crate::state::AppState>) {
    let labels: Vec<String> = {
        let pool_labels = state.unpromoted_pool_labels.lock();
        let registry = state.shadow_instance_registry.lock();
        registry
            .keys()
            .filter(|l| !pool_labels.contains(*l) && !l.starts_with("browser-pane-"))
            .cloned()
            .collect()
    };
    let entries: Vec<serde_json::Value> = labels
        .iter()
        .map(|l| {
            serde_json::json!({
                "label": l,
                "windowId": state.backend_window_id(l),
            })
        })
        .collect();
    let count = entries.len();
    crate::events::emit_event_all_windows(
        state,
        "window-instances-changed",
        &serde_json::json!({
            "count": count,
            "entries": entries,
        }),
    );
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

/// Phase B.4 follow-up — sync API: report a pool window being added
/// to the warm pool inventory. Called from `spawn_pool_window` on
/// the UI thread. No-op when launcher pipe is absent.
pub fn report_pool_window_added(label: String) {
    let Some(tx) = COMMAND_TX.get() else {
        return;
    };
    let cmd = Command::ReportPoolWindowAdded { label };
    if let Err(e) = tx.send(cmd) {
        tracing::warn!("[launcher-ipc] report_pool_window_added: channel closed ({})", e);
    }
}

/// Phase B.4 follow-up — sync API: report a pool window leaving the
/// pool (promote, destroy). On promote callers should also call
/// `report_window_opened` so the label transitions atomically (from
/// the launcher's perspective) from `pool` to `windows`.
pub fn report_pool_window_removed(label: String) {
    let Some(tx) = COMMAND_TX.get() else {
        return;
    };
    let cmd = Command::ReportPoolWindowRemoved { label };
    if let Err(e) = tx.send(cmd) {
        tracing::warn!("[launcher-ipc] report_pool_window_removed: channel closed ({})", e);
    }
}

/// Phase B.4 follow-up — sync API: report the host's current
/// authoritative counts so the launcher reducer can compare against
/// its mirror and emit `Event::DriftDetected` on mismatch. Callers
/// invoke this AFTER each window-level transition so the launcher
/// gets a fresh snapshot to compare against its just-applied
/// transition.
pub fn report_host_counts(windows: u32, pool: u32) {
    let Some(tx) = COMMAND_TX.get() else {
        return;
    };
    let cmd = Command::ReportHostCounts { windows, pool };
    if let Err(e) = tx.send(cmd) {
        tracing::warn!("[launcher-ipc] report_host_counts: channel closed ({})", e);
    }
}

/// Phase B.5 (window_id_map step b) — sync API: report the
/// frontend's `register_backend_window` IPC to the launcher.
/// Called from `commands/window.rs::register_backend_window`
/// after the host's local `window_id_map` insert. No-op if the
/// launcher pipe isn't connected.
pub fn report_backend_window_id_registered(label: String, window_id: String) {
    let Some(tx) = COMMAND_TX.get() else {
        return;
    };
    let cmd = Command::ReportBackendWindowIdRegistered { label, window_id };
    if let Err(e) = tx.send(cmd) {
        tracing::warn!(
            "[launcher-ipc] report_backend_window_id_registered: channel closed ({})",
            e
        );
    }
}

/// Phase B.5 (window_id_map step b) — sync API: report a window's
/// backend ID being dropped (close path). Called from
/// `client.rs::on_before_close` after the host's local
/// `window_id_map.remove`. No-op if launcher pipe absent.
pub fn report_backend_window_id_unregistered(label: String) {
    let Some(tx) = COMMAND_TX.get() else {
        return;
    };
    let cmd = Command::ReportBackendWindowIdUnregistered { label };
    if let Err(e) = tx.send(cmd) {
        tracing::warn!(
            "[launcher-ipc] report_backend_window_id_unregistered: channel closed ({})",
            e
        );
    }
}

/// Phase B.4 follow-up — sync API: report the host's pool count
/// only. Used by `spawn_pool_window` where the windows dimension
/// is mid-flight relative to the launcher mirror (refill happens
/// during a close path that hasn't sent `ReportWindowClosed` yet);
/// snapshotting only the pool dimension preserves the
/// check-every-transition guarantee without producing false
/// windows-drift. (codex P2 PR #578 round-3.)
pub fn report_host_pool_count(count: u32) {
    let Some(tx) = COMMAND_TX.get() else {
        return;
    };
    let cmd = Command::ReportHostPoolCount { count };
    if let Err(e) = tx.send(cmd) {
        tracing::warn!("[launcher-ipc] report_host_pool_count: channel closed ({})", e);
    }
}

/// Phase B.4 follow-up — compute the host's authoritative counts
/// from `AppState` and report them. Callers invoke this AFTER
/// each window/pool transition.
///
/// Atomic snapshot: holds both `unpromoted_pool_labels` and
/// `browsers` simultaneously so the reported `(windows, pool)`
/// pair is from one consistent state. Without this, a concurrent
/// mutation between the two lock acquisitions (CEF lifecycle on
/// the UI thread vs. IPC handler in `commands/drag.rs`) could
/// produce a mismatched snapshot and trigger a spurious
/// `Event::DriftDetected`. (codex P2 PR #578 round-1.)
///
/// Lock order: `unpromoted_pool_labels` first, then `browsers`.
/// Matches the existing snapshot pattern in
/// `client.rs::on_before_close` (line ~418) and is the only place
/// in the codebase that holds both locks simultaneously, so no
/// other path can race in the reverse order.
///
/// Counts (matching the launcher's mirror semantics):
/// * `windows` — top-level user-visible windows in `browsers`,
///   excluding `browser-pane-*` child HWNDs and any label still
///   in `unpromoted_pool_labels`.
/// * `pool` — pre-promote pool labels (`unpromoted_pool_labels.len()`).
///
/// **Why this reads host's `browsers` and `unpromoted_pool_labels`
/// directly (not the shadow):** this fn IS the source for the
/// launcher's mirror — its output is what gets compared against
/// `state.windows.len()` / `state.pool.len()` in the drift-detection
/// path. Reading from the shadow would compare the shadow against
/// itself (always agrees) and defeat the entire B.4 drift-detection
/// design. Once the host reducer arrives in Phase F (see
/// `docs/retro/multi-reducer-proposal-2026-04-28.md`), this becomes
/// "report host's authoritative reducer-state to the launcher."
pub fn compute_and_report_host_counts(state: &std::sync::Arc<crate::state::AppState>) {
    let unpromoted = state.unpromoted_pool_labels.lock();
    let browsers = state.browsers.lock();
    let pool = unpromoted.len() as u32;
    let windows = browsers
        .keys()
        .filter(|k| !k.starts_with("browser-pane-") && !unpromoted.contains(*k))
        .count() as u32;
    drop(browsers);
    drop(unpromoted);
    report_host_counts(windows, pool);
}
