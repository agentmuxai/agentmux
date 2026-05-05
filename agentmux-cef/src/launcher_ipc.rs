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

use agentmux_common::ipc::{ClientKind, Command, Event, HostFrame};
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
    //
    // Phase CPD-5 — the reader now accepts `HostFrame` envelopes
    // (CPD-1) so the launcher can push saga-issued `Command`s down
    // alongside `Event`s. Each line is parsed as `HostFrame` first;
    // on parse failure we fall back to raw `Event` JSON for
    // backward-compatibility with launchers that haven't yet
    // adopted the envelope shape (the legacy fanout path emits
    // bare `Event` JSON; CPD-2's `HostPipe::send_event` wraps in
    // `HostFrame::Event`).
    //
    // Saga-issued `Command`s are dispatched via the host-side
    // idempotency LRU (`saga_dispatch::dispatch_host_command`):
    // duplicate `(saga_id, kind)` pairs re-emit the same Report
    // without re-running the action. The reply Commands are pushed
    // through the existing `COMMAND_TX` channel, which the drain
    // task spawned above already serializes to the writer.
    let state_for_reader = std::sync::Arc::clone(&state);
    let saga_lru = std::sync::Arc::new(parking_lot::Mutex::new(
        crate::saga_dispatch::SagaIdempotencyLru::with_default_cap(),
    ));
    // The reply path uses the same `COMMAND_TX` published above.
    // Snapshot a sender clone NOW so the reader doesn't have to
    // probe the OnceLock on every command (which would also race
    // with the unset-OnceLock window during the brief gap between
    // Register flush and `COMMAND_TX.set`).
    let reply_tx = COMMAND_TX
        .get()
        .expect("COMMAND_TX set before reader task spawn")
        .clone();
    let saga_runner = std::sync::Arc::new(crate::saga_dispatch::LiveActionRunner {
        state: std::sync::Arc::clone(&state),
    });
    let reader_task = tokio::spawn(async move {
        let reader = BufReader::new(read_half);
        let mut lines = reader.lines();
        loop {
            match lines.next_line().await {
                Ok(Some(line)) if line.trim().is_empty() => continue,
                Ok(Some(line)) => {
                    // Try the envelope first.
                    match serde_json::from_str::<HostFrame>(&line) {
                        Ok(HostFrame::Event(event)) => {
                            tracing::info!("[launcher-ipc] received event: {:?}", event);
                            apply_event_to_shadow(&state_for_reader, &event);
                        }
                        Ok(HostFrame::Command(cmd)) => {
                            tracing::info!(
                                "[launcher-ipc] received saga command: {:?}",
                                cmd
                            );
                            let outcome = crate::saga_dispatch::dispatch_host_command(
                                &cmd,
                                saga_runner.as_ref(),
                                &saga_lru,
                                &reply_tx,
                            );
                            tracing::debug!(
                                "[launcher-ipc] saga command outcome: {:?}",
                                outcome
                            );
                        }
                        Err(_envelope_err) => {
                            // Backward-compat fallback: pre-CPD-2 launchers
                            // emit bare `Event` JSON without the `HostFrame`
                            // wrapper. Parse as raw `Event`; on failure
                            // log and skip.
                            match serde_json::from_str::<Event>(&line) {
                                Ok(event) => {
                                    tracing::info!(
                                        "[launcher-ipc] received event (legacy bare-Event): {:?}",
                                        event
                                    );
                                    apply_event_to_shadow(&state_for_reader, &event);
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        "[launcher-ipc] could not parse line as HostFrame or Event ({}): {}",
                                        e,
                                        line
                                    );
                                }
                            }
                        }
                    }
                }
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

/// Apply a launcher event to host's shadow projections, then fan
/// the event out to every top-level renderer via the JS bridge.
///
/// Shadows are updated FIRST so any renderer-side code that reads
/// host state via the IPC HTTP path (e.g. `listWindowInstances`)
/// sees a consistent view at the moment the typed event lands.
///
/// Phase B.7.3.3 — the bespoke `window-instances-changed` re-emit
/// is gone; renderers now consume typed events directly via the
/// CEF JS bridge dispatcher (`launcher_event_bridge`).
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
        }
        Event::CorrectiveWindowMove { hwnd, target_rect, reason, .. } => {
            // Phase B.9.2 — pure-reducer self-heal. Reducer detected
            // an off-monitor / sentinel-parked window that the user
            // has never foregrounded, and computed an on-monitor
            // target rect. Apply `SetWindowPos` to move the window
            // before the user notices the orphan taskbar entry.
            // The CEF UI thread is the safe caller for SetWindowPos
            // on a CEF Views-managed window — post a UI task rather
            // than calling from this tokio task directly.
            tracing::warn!(
                target: "wrr",
                "[wrr] CorrectiveWindowMove hwnd={:#x} reason={:?} target=({},{})-({},{})",
                hwnd, reason,
                target_rect.left, target_rect.top, target_rect.right, target_rect.bottom
            );
            crate::ui_tasks::post_corrective_window_move(
                state,
                *hwnd,
                target_rect.left,
                target_rect.top,
                target_rect.right - target_rect.left,
                target_rect.bottom - target_rect.top,
            );
        }
        Event::HostShouldQuit { .. } => {
            // Phase B.9.3 was diagnostic-only; the comment in
            // `agentmux-common/src/ipc.rs::HostShouldQuit` explicitly
            // calls this advisory. v0.33.643 caught the gap: when
            // promoted `window-pool-*` browsers go orphan (in host's
            // browsers map but the launcher's mirror has dropped
            // them), the cascade in `on_before_close` gates on a
            // user_browser_count that wrongly includes them, so the
            // gate fails and the host stays alive forever.
            //
            // Spec: `docs/specs/SPEC_HOST_ORPHAN_RECONCILIATION_2026_05_05.md`.
            //
            // The reconciler runs on this IPC reader's thread but
            // does NOT touch CEF directly — it dispatches via
            // `PostMessageW(WM_CLOSE)` (preferred on Windows) or
            // `cef::post_task(UI, ClosePoolBrowserTask)` (the
            // existing fallback path). Same channels the cascade
            // already uses, so each close funnels back through
            // `on_before_close` and the existing Stage-2
            // `quit_message_loop` fires when `browser_list` empties.
            // Earlier v0.33.491–v0.33.494 attempts failed because
            // they tried to drive UI-thread work *directly* from
            // here — we don't, we hand it to CEF's task queue.
            tracing::warn!(
                target: "wrr",
                "[wrr] HostShouldQuit received — running orphan reconciler"
            );
            crate::commands::orphan_reconcile::reconcile_and_drain(state);
        }
        _ => {}
    }

    // Phase B.7.3.1 — broadcast the typed event to every top-level
    // renderer. The renderer-side dispatcher (`window.__agentmux_launcher_event`,
    // installed by `frontend/util/launcher-events.ts`) feeds the
    // launcher-event-reducer signal that downstream UI subscribes to.
    crate::launcher_event_bridge::dispatch_to_renderers(state, event);
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
    // CPD-1 (schema-only): host's existing `report_pool_window_added`
    // call sites are organic refills (not yet saga-driven); pass
    // `saga_id: None` per spec §3.3. CPD-3 wires the saga-driven
    // path that will pass `Some(N)` through here.
    let cmd = Command::ReportPoolWindowAdded { label, saga_id: None };
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

/// Phase F.5 — sync API: tell the launcher that a pool window is
/// promoting to a user-visible top-level window. Sent BETWEEN
/// `report_pool_window_removed` and `report_window_opened` so the
/// launcher's pool-respawn saga (state-machine bracket around the
/// implicit refill) can correlate the promote event with the
/// subsequent `PoolWindowAdded` for the freshly-spawned replacement
/// pool slot.
///
/// Same no-op-if-disconnected semantics as the other `report_*`
/// helpers — `task dev` mode (no launcher in the loop) silently
/// drops; the host's authoritative state and refill mechanism are
/// unaffected.
pub fn report_pool_window_promoted(label: String) {
    let Some(tx) = COMMAND_TX.get() else {
        return;
    };
    let cmd = Command::ReportPoolWindowPromoted { label };
    if let Err(e) = tx.send(cmd) {
        tracing::warn!("[launcher-ipc] report_pool_window_promoted: channel closed ({})", e);
    }
}

/// Phase F.6 — sync API: tell the launcher that all browser-pane
/// HWNDs belonging to a closing top-level window have been reaped
/// (lifecycle entries drained, pane HWND map cleared, subwindow
/// cascade closes initiated). Sent from `client.rs::on_before_close`
/// AFTER the pane drain step, BEFORE the post-close pool-drain
/// decision is reported.
///
/// The launcher's window-cleanup-cascade saga uses this as the
/// Step 1 → Step 2 transition signal: it marks the implicit pane
/// reap as observed and lets the saga issue its `DrainPoolIfLast`
/// IssueCmd (currently log-only — see `saga/window_cleanup.rs`
/// module docstring for the saga-as-narrator scope decision).
///
/// Same no-op-if-disconnected semantics as the other `report_*`
/// helpers; `task dev` mode silently drops.
pub fn report_panes_reaped(label: String) {
    let Some(tx) = COMMAND_TX.get() else {
        return;
    };
    // CPD-1 (schema-only): existing call sites are organic; CPD-3
    // adds the saga-driven path that fills `saga_id`.
    let cmd = Command::ReportPanesReaped { label, saga_id: None };
    if let Err(e) = tx.send(cmd) {
        tracing::warn!("[launcher-ipc] report_panes_reaped: channel closed ({})", e);
    }
}

/// Phase F.6 — sync API: tell the launcher the result of the
/// post-close drain-pool-if-last decision. `was_last == true` when
/// the closing window was the last user-visible window (Stage 1 of
/// the wrr two-stage close cascade just kicked off in
/// `client.rs::on_before_close`); `false` when other windows
/// remain and the warm pool stays warm.
///
/// Step 2 terminal signal for the launcher's
/// window-cleanup-cascade saga. Both branches close the
/// `SagaStarted` bracket successfully — the saga's job is to
/// narrate the decision, not enforce a particular outcome.
///
/// Same no-op-if-disconnected semantics as the other `report_*`
/// helpers.
pub fn report_pool_drain_decision(label: String, was_last: bool) {
    let Some(tx) = COMMAND_TX.get() else {
        return;
    };
    // CPD-1 (schema-only): existing call sites are organic; CPD-3
    // adds the saga-driven path that fills `saga_id`.
    let cmd = Command::ReportPoolDrainDecision {
        label,
        was_last,
        saga_id: None,
    };
    if let Err(e) = tx.send(cmd) {
        tracing::warn!(
            "[launcher-ipc] report_pool_drain_decision: channel closed ({})",
            e
        );
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

/// Phase B.9.1 (WRR) — sync API: report a Win32 HWND created.
/// Called from the WRR `SetWinEventHook` callback. No-op if the
/// launcher pipe isn't connected (`task dev` mode); reducer arm
/// stashes pending-without-label until reconciliation.
pub fn report_hwnd_opened(
    hwnd: u64,
    class_name: String,
    title: String,
    label_hint: Option<String>,
) {
    let Some(tx) = COMMAND_TX.get() else {
        return;
    };
    let cmd = Command::ReportHwndOpened {
        hwnd,
        class_name,
        title,
        label_hint,
    };
    if let Err(e) = tx.send(cmd) {
        tracing::warn!("[launcher-ipc] report_hwnd_opened: channel closed ({})", e);
    }
}

/// Phase B.9.1 — sync API: report a Win32 HWND destroyed.
pub fn report_hwnd_destroyed(hwnd: u64) {
    let Some(tx) = COMMAND_TX.get() else {
        return;
    };
    let cmd = Command::ReportHwndDestroyed { hwnd };
    if let Err(e) = tx.send(cmd) {
        tracing::warn!("[launcher-ipc] report_hwnd_destroyed: channel closed ({})", e);
    }
}

/// Phase B.9.1 — sync API: report visibility change.
pub fn report_hwnd_visibility_changed(hwnd: u64, visible: bool) {
    let Some(tx) = COMMAND_TX.get() else {
        return;
    };
    let cmd = Command::ReportHwndVisibilityChanged { hwnd, visible };
    if let Err(e) = tx.send(cmd) {
        tracing::warn!("[launcher-ipc] report_hwnd_visibility_changed: channel closed ({})", e);
    }
}

/// Phase B.9.1 — sync API: report foreground change.
pub fn report_hwnd_foreground_changed(hwnd: u64) {
    let Some(tx) = COMMAND_TX.get() else {
        return;
    };
    let cmd = Command::ReportHwndForegroundChanged { hwnd };
    if let Err(e) = tx.send(cmd) {
        tracing::warn!("[launcher-ipc] report_hwnd_foreground_changed: channel closed ({})", e);
    }
}

/// Phase B.9.1 — sync API: report iconic (minimized) change.
pub fn report_hwnd_iconic_changed(hwnd: u64, iconic: bool) {
    let Some(tx) = COMMAND_TX.get() else {
        return;
    };
    let cmd = Command::ReportHwndIconicChanged { hwnd, iconic };
    if let Err(e) = tx.send(cmd) {
        tracing::warn!("[launcher-ipc] report_hwnd_iconic_changed: channel closed ({})", e);
    }
}

/// Phase B.9.1 — sync API: report position change. Caller is
/// responsible for debouncing — see `wrr/position_debounce.rs`.
pub fn report_hwnd_position_changed(hwnd: u64, rect: agentmux_common::ipc::Rect) {
    let Some(tx) = COMMAND_TX.get() else {
        return;
    };
    let cmd = Command::ReportHwndPositionChanged { hwnd, rect };
    if let Err(e) = tx.send(cmd) {
        tracing::warn!("[launcher-ipc] report_hwnd_position_changed: channel closed ({})", e);
    }
}

/// Phase B.9.1 — sync API: report current monitor topology. Sent
/// once at install time; mid-session topology changes are a B.9.2
/// follow-up.
pub fn report_monitor_topology_changed(rects: Vec<agentmux_common::ipc::Rect>) {
    let Some(tx) = COMMAND_TX.get() else {
        return;
    };
    let cmd = Command::ReportMonitorTopologyChanged { rects };
    if let Err(e) = tx.send(cmd) {
        tracing::warn!("[launcher-ipc] report_monitor_topology_changed: channel closed ({})", e);
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
    // Phase H.2.b — reducer-aware label snapshot.
    let unpromoted = state.unpromoted_pool_labels_snapshot();
    let pool = unpromoted.len() as u32;
    let windows = state
        .list_browser_labels()
        .into_iter()
        .filter(|k| !k.starts_with("browser-pane-") && !unpromoted.contains(k.as_str()))
        .count() as u32;
    report_host_counts(windows, pool);
}
