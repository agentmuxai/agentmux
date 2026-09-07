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
//
// This module holds connection setup (platform-specific
// `connect_to_launcher` + `LauncherIpcHandle`) and the shadow
// projection that keeps the host's read-side state in sync with the
// launcher's authoritative event stream. The uniform, stateless
// `report_*` sync API that pushes facts back up to the launcher lives
// in the sibling `reporters` module (re-exported below so existing
// `crate::launcher_ipc::report_*` call sites are unaffected).

use std::sync::Arc;
use std::sync::OnceLock;

use agentmux_common::ipc::{ClientKind, Command, Event, HostFrame};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, Mutex};

#[cfg(target_os = "windows")]
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};

mod reporters;
pub(crate) use reporters::*;

/// Phase B.4 — global outbound command channel. Set once when the
/// launcher pipe connects; sync callers (CEF lifecycle callbacks
/// fire on the UI thread) post `Command`s without needing a
/// tokio runtime handle. Drained by a task spawned in
/// `connect_to_launcher` that writes to the pipe.
///
/// `None` semantics: launcher IPC isn't connected (e.g. `task dev`
/// mode where launcher isn't in the loop). `report_window_*` calls
/// are silently no-ops; the host runs as before.
///
/// `pub(crate)` — the `reporters` submodule reads this directly
/// (via `super::COMMAND_TX`) to stay a flat, stateless "build a
/// Command and send it" family without needing an accessor fn.
pub(crate) static COMMAND_TX: OnceLock<mpsc::UnboundedSender<Command>> = OnceLock::new();

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

#[cfg(unix)]
pub struct LauncherIpcHandle {
    /// Held purely as a keepalive; same shape as the Windows variant.
    #[allow(dead_code)]
    writer: Arc<Mutex<tokio::io::WriteHalf<tokio::net::UnixStream>>>,
    /// Read-half task handle; A1.2 — the body of the reader task is
    /// shared with the Windows path (parses HostFrame envelopes, falls
    /// back to bare Event for legacy launchers, dispatches saga
    /// Commands).
    #[allow(dead_code)]
    reader_task: tokio::task::JoinHandle<()>,
}

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
    // SPEC_PILLAR1_STEP4 Phase 1 — request the launcher's live window
    // snapshot on every connect (cold start AND a post-crash respawn look
    // identical here; see the spec for why no separate "is this a restart"
    // signal is needed). Phase 1 is observe-only: the reader task logs what
    // this would drive but doesn't recreate anything yet.
    request_snapshot();
    // Workstream 0 Phase 1 — tell the launcher whether background-service
    // mode is on, once per connect (see `report_background_service_enabled`).
    report_background_service_enabled(state.host_state.lock().background_service_enabled);
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
                        Ok(HostFrame::Command(Command::ProbeUiThread { nonce })) => {
                            // SPEC_LAUNCHER_TEARDOWN_BACKSTOP Phase 1 — NOT a
                            // saga command (no saga_id, no idempotency LRU:
                            // duplicate probes are harmless, replies are
                            // at-most-once per posted task). The reply MUST
                            // come from the UI thread — replying from here
                            // (the tokio reader) would prove nothing about
                            // the thread the backstop actually cares about.
                            // If the UI thread isn't pumping (wedged, or the
                            // pre-ready post_task silent drop), the posted
                            // task never executes and no reply is sent —
                            // silence IS the signal.
                            crate::ui_tasks::post_probe_ui_thread_reply(nonce);
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

/// A1.2 — Unix-domain-socket variant. Mirrors the Windows path above:
/// connect → Register → spawn drain task → spawn reader task → return
/// handle. Protocol on the wire is identical (newline-delimited JSON
/// `Command` / `Event` / `HostFrame`) so the launcher's
/// `handle_connection` generic body serves both transports unchanged.
///
/// Env var name: reuses `AGENTMUX_LAUNCHER_PIPE` even though the
/// underlying resource is a Unix-domain socket. Lets the 17 host-side
/// `report_*` call sites in this file stay platform-agnostic. The
/// launcher's `spawn_host_unix` exports the socket path under this
/// name (see `agentmux-launcher/src/main.rs::run_unix`).
#[cfg(unix)]
pub async fn connect_to_launcher(
    state: std::sync::Arc<crate::state::AppState>,
) -> Option<LauncherIpcHandle> {
    use tokio::net::UnixStream;

    let sock_path = match std::env::var("AGENTMUX_LAUNCHER_PIPE") {
        Ok(p) if !p.is_empty() => p,
        _ => {
            tracing::info!(
                "AGENTMUX_LAUNCHER_PIPE unset — running without launcher IPC (dev mode)"
            );
            return None;
        }
    };

    let stream = match UnixStream::connect(&sock_path).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(
                "[launcher-ipc] failed to connect {}: {} — continuing without launcher IPC",
                sock_path,
                e
            );
            return None;
        }
    };
    tracing::info!("[launcher-ipc] connected to {}", sock_path);

    let (read_half, write_half) = tokio::io::split(stream);
    let writer = Arc::new(Mutex::new(write_half));

    // Send Register FIRST — same race-avoidance discipline as the
    // Windows path: COMMAND_TX is only published after Register
    // flushes successfully.
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

    // Outbound command drain task — same shape as Windows variant.
    let (tx, mut rx) = mpsc::unbounded_channel::<Command>();
    if COMMAND_TX.set(tx).is_err() {
        tracing::warn!("[launcher-ipc] COMMAND_TX already set — connect_to_launcher called twice");
    }
    // SPEC_PILLAR1_STEP4 Phase 1 — see the Windows variant's identical call
    // for the rationale.
    request_snapshot();
    // Workstream 0 Phase 1 — see the Windows variant's identical call.
    report_background_service_enabled(state.host_state.lock().background_service_enabled);
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

    // Reader task — identical body to the Windows variant.
    let state_for_reader = std::sync::Arc::clone(&state);
    let saga_lru = std::sync::Arc::new(parking_lot::Mutex::new(
        crate::saga_dispatch::SagaIdempotencyLru::with_default_cap(),
    ));
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
                    match serde_json::from_str::<HostFrame>(&line) {
                        Ok(HostFrame::Event(event)) => {
                            tracing::info!("[launcher-ipc] received event: {:?}", event);
                            apply_event_to_shadow(&state_for_reader, &event);
                        }
                        Ok(HostFrame::Command(Command::ProbeUiThread { nonce })) => {
                            // SPEC_LAUNCHER_TEARDOWN_BACKSTOP Phase 1 — NOT a
                            // saga command (no saga_id, no idempotency LRU:
                            // duplicate probes are harmless, replies are
                            // at-most-once per posted task). The reply MUST
                            // come from the UI thread — replying from here
                            // (the tokio reader) would prove nothing about
                            // the thread the backstop actually cares about.
                            // If the UI thread isn't pumping (wedged, or the
                            // pre-ready post_task silent drop), the posted
                            // task never executes and no reply is sent —
                            // silence IS the signal.
                            crate::ui_tasks::post_probe_ui_thread_reply(nonce);
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
                    tracing::info!("[launcher-ipc] launcher socket EOF — connection closed");
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

/// Non-Windows + non-Unix fallback (e.g., WASM if we ever ship it). Should be
/// unreachable in practice — every Tier-1 platform we ship is either Windows
/// or Unix.
#[cfg(not(any(target_os = "windows", unix)))]
pub struct LauncherIpcHandle;

#[cfg(not(any(target_os = "windows", unix)))]
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
/// Pure shadow-state projection — extracted from `apply_event_to_shadow`
/// for property testing per master spec §8.14 (subscriber idempotency
/// contract). Mutates ONLY the shadow Mutex<HashMap> fields:
/// `shadow_instance_registry`, `shadow_window_meta`,
/// `shadow_backend_window_ids`. No UI tasks, no renderer dispatch, no
/// CEF calls — those live in the wrapper `apply_event_to_shadow`.
///
/// Idempotent by construction: every arm is `HashMap::insert` /
/// `HashMap::remove` keyed by `label`, both naturally idempotent past
/// the first call. The drift-compare branch in `WindowOpened` is
/// read-only (logs a warning, doesn't mutate). Locked in by the
/// `shadow_projection_idempotent_under_replay` proptest.
pub(crate) fn apply_shadow_projection(state: &std::sync::Arc<crate::state::AppState>, event: &Event) {
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
            // + drift compare. The wire `WindowKind` IS the host's
            // `crate::state::WindowKind` (a re-export, `state/window_meta.rs`),
            // so the variant-by-variant mapping this used to do is gone.
            let host_kind = *kind;
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
        // Side-effect arms (UI tasks, reconciler) handled in the
        // `apply_event_to_shadow` wrapper. Excluded here so this
        // function stays pure-projection and trivially idempotent.
        _ => {}
    }
}

fn apply_event_to_shadow(state: &std::sync::Arc<crate::state::AppState>, event: &Event) {
    apply_shadow_projection(state, event);
    match event {
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
            // The launcher emits this when it detects the
            // last-user-visible-window-closed-but-host-alive state.
            // The host's reconciler closes any orphan
            // `window-pool-*` browsers so the existing on_before_close
            // cascade can drain to `browser_list.is_empty()` and
            // fire `quit_message_loop`.
            //
            // Spec: `docs/specs/SPEC_HOST_ORPHAN_RECONCILIATION_2026_05_05.md`.
            //
            // We're on the launcher IPC reader thread; CEF
            // Browser/BrowserHost calls aren't safe here. The
            // reconciler does state-snapshot + classification only,
            // then posts a UI-thread task that does the HWND probe
            // and `host.close_browser(force=1)`. Earlier v0.33.491–
            // v0.33.494 attempts at driving UI shutdown directly
            // from this handler hung CEF; using the task scheduler
            // is the difference.
            tracing::warn!(
                target: "wrr",
                "[wrr] HostShouldQuit received — running orphan reconciler"
            );
            crate::commands::orphan_reconcile::reconcile_and_drain(state);
        }
        // SPEC_PILLAR1_STEP4 Phase 2 — logs what this snapshot shows, then
        // drives the fast-path reproject (recreate any window beyond
        // "main" the launcher still remembers — e.g. after a host-only
        // crash the launcher survived). Deliberately returns before the
        // generic broadcast below: this is a large, host-internal
        // request/response payload, not a typed delta the frontend's
        // launcher-event reducer expects — forwarding it would be either
        // dead weight or a mis-parse.
        Event::Snapshot { version, windows, backend_window_ids, .. } => {
            tracing::info!(
                target: "reproject",
                version,
                window_count = windows.len(),
                windows = ?windows
                    .iter()
                    .map(|w| (w.label.as_str(), w.kind, w.parent_label.as_deref(), w.last_rect))
                    .collect::<Vec<_>>(),
                "[reproject] launcher snapshot received"
            );
            // This event can arrive (and this arm can run) before CEF's
            // UI-thread message loop has started pumping — the launcher-ipc
            // reader task runs on its own tokio runtime, independent of
            // `run_message_loop()`. Posting `CreateWindowTask` via
            // `post_task(ThreadId::UI, ...)` before that point is a silent
            // no-op — verified live: the task's `execute()` never runs and
            // the window is never created, even though every reducer-side
            // bookkeeping call still succeeds. Stash and let `"main"`'s own
            // registration (proof the UI thread is alive) drain and replay.
            //
            // The decision and the stash happen under ONE lock acquisition
            // (`ui_thread_gate`), not a load-then-separate-lock pair — see
            // `UiThreadGate::on_snapshot`'s doc comment for the TOCTOU this
            // closes, and for what each outcome means.
            let action = {
                let mut gate = state.ui_thread_gate.lock();
                let action = gate.on_snapshot();
                if action == crate::state::SnapshotAction::Stash {
                    gate.stashed = Some(windows.clone());
                    gate.stashed_backend_window_ids = Some(backend_window_ids.clone());
                }
                action
            };
            match action {
                crate::state::SnapshotAction::Stash => {
                    tracing::info!(
                        target: "reproject",
                        "[reproject] UI thread not ready yet — stashing snapshot for replay after \"main\" registers"
                    );
                }
                crate::state::SnapshotAction::RunFastPath => {
                    tracing::info!(
                        target: "reproject",
                        window_count = windows.len(),
                        "[reproject] fast-path snapshot arrived while slow path was pending — using it instead"
                    );
                    crate::commands::window::reproject_from_snapshot_and_stage_closures(
                        state,
                        windows,
                        backend_window_ids,
                    );
                }
                crate::state::SnapshotAction::Skip => {
                    tracing::info!(
                        target: "reproject",
                        "[reproject] snapshot arrived after reproject already ran — skipping"
                    );
                }
            }
            return;
        }
        _ => {}
    }

    // Phase B.7.3.1 — broadcast the typed event to every top-level
    // renderer. The renderer-side dispatcher (`window.__agentmux_launcher_event`,
    // installed by `frontend/util/launcher-events.ts`) feeds the
    // launcher-event-reducer signal that downstream UI subscribes to.
    crate::launcher_event_bridge::dispatch_to_renderers(state, event);
}

/// SPEC_PILLAR1_STEP4 Phase 1 — request the launcher's live window
/// snapshot (`Event::Snapshot`, handled in `apply_event_to_shadow` above).
/// Called once, right after `COMMAND_TX` is published, from both platform
/// variants of `connect_to_launcher`. No-op if the launcher pipe isn't
/// connected (`task dev` mode), same semantics as every other `report_*`
/// helper in the sibling `reporters` module.
fn request_snapshot() {
    let Some(tx) = COMMAND_TX.get() else {
        return;
    };
    if let Err(e) = tx.send(Command::GetSnapshot) {
        tracing::warn!("[launcher-ipc] request_snapshot: channel closed ({})", e);
    }
}

#[cfg(test)]
mod shadow_projection_tests {
    //! Master spec §8.14 — subscriber idempotency property tests for
    //! the host's shadow projection. The launcher event channel may
    //! deliver duplicates (re-dispatch, resync, replay); the contract
    //! is that subscribers fold them into a no-op past the first
    //! application.
    //!
    //! These tests target `apply_shadow_projection`, the pure
    //! HashMap-mutation slice of `apply_event_to_shadow`. Side-effect
    //! arms (`CorrectiveWindowMove`, `HostShouldQuit`) are excluded
    //! from the projection function and tested separately at the
    //! integration level.

    use super::*;
    use agentmux_common::ipc::{Event, WindowKind};
    use proptest::prelude::*;
    use std::sync::Arc;

    fn snapshot_shadow_state(
        state: &Arc<crate::state::AppState>,
    ) -> (
        std::collections::HashMap<String, crate::state::WindowMeta>,
        std::collections::HashMap<String, String>,
        std::collections::HashMap<String, u32>,
    ) {
        (
            state.shadow_window_meta.lock().clone(),
            state.shadow_backend_window_ids.lock().clone(),
            state.shadow_instance_registry.lock().clone(),
        )
    }

    /// Strategy: events the projection actually mutates. Side-effect
    /// arms (CorrectiveWindowMove, HostShouldQuit) excluded — they
    /// don't reach `apply_shadow_projection`'s match.
    ///
    /// Labels drawn from `[a-c]{1,3}` so duplicates (open then close
    /// for same label) are common. The launcher's strict-pairing
    /// semantic means real production never sends close-without-open,
    /// but the projection itself doesn't enforce that, and replaying
    /// either order through `apply_shadow_projection` should still
    /// converge to the same shadow state.
    fn arb_projection_event() -> impl Strategy<Value = Event> {
        prop_oneof![
            3 => (
                "[a-c]{1,3}",
                prop_oneof![Just(WindowKind::FullInstance), Just(WindowKind::Subwindow)],
                prop_oneof![Just(None::<String>), Just(Some("a".into()))],
            )
                .prop_map(|(label, kind, parent_label)| Event::WindowOpened {
                    label,
                    kind,
                    parent_label,
                    version: 0,
                }),
            3 => "[a-c]{1,3}".prop_map(|label| Event::WindowClosed {
                label,
                version: 0,
                crash_detected: false,
            }),
            2 => ("[a-c]{1,3}", 1u32..100u32).prop_map(|(label, num)| {
                Event::WindowInstanceAssigned { label, num, version: 0 }
            }),
            1 => ("[a-c]{1,3}", 1u32..100u32).prop_map(|(label, num)| {
                Event::WindowInstanceReleased { label, num, version: 0 }
            }),
            2 => ("[a-c]{1,3}", "[0-9a-f]{4,8}").prop_map(|(label, window_id)| {
                Event::BackendWindowIdRegistered { label, window_id, version: 0 }
            }),
            1 => ("[a-c]{1,3}", "[0-9a-f]{4,8}").prop_map(|(label, window_id)| {
                Event::BackendWindowIdUnregistered { label, window_id, version: 0 }
            }),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 64,
            ..ProptestConfig::default()
        })]

        /// Master spec §8.14 — replaying any event sequence twice
        /// converges to the same shadow state. Property: for any
        /// sequence S of events, `apply(S) == apply(S; S)`.
        ///
        /// Drift-storm regression class (PR #708): if a launcher
        /// event reaches the host's `apply_event_to_shadow` 600
        /// times instead of once, the shadow projection MUST stay
        /// in the same final state. By construction
        /// (HashMap::insert/remove are idempotent for the same key),
        /// this holds — the proptest locks it.
        #[test]
        fn shadow_projection_idempotent_under_replay(
            events in proptest::collection::vec(arb_projection_event(), 0..40)
        ) {
            let state = Arc::new(crate::state::AppState::default());

            // Apply once.
            for e in &events {
                apply_shadow_projection(&state, e);
            }
            let (meta1, ids1, registry1) = snapshot_shadow_state(&state);

            // Apply the SAME sequence again — every event is a duplicate.
            for e in &events {
                apply_shadow_projection(&state, e);
            }
            let (meta2, ids2, registry2) = snapshot_shadow_state(&state);

            prop_assert_eq!(meta1, meta2, "shadow_window_meta diverged on replay");
            prop_assert_eq!(ids1, ids2, "shadow_backend_window_ids diverged on replay");
            prop_assert_eq!(registry1, registry2, "shadow_instance_registry diverged on replay");
        }

        /// Stronger property: arbitrary per-event duplication count
        /// (1..5x per event) produces the same final state as
        /// applying each event once. This is the §8.14 contract
        /// directly — duplicates must fold to no-op.
        #[test]
        fn shadow_projection_collapses_arbitrary_duplicates(
            events in proptest::collection::vec(arb_projection_event(), 1..30),
            dup_counts in proptest::collection::vec(1u8..6, 1..30),
        ) {
            // Pair events with duplication counts, taking the shorter length.
            let n = events.len().min(dup_counts.len());

            // Run 1: apply each event once.
            let state_once = Arc::new(crate::state::AppState::default());
            for e in events.iter().take(n) {
                apply_shadow_projection(&state_once, e);
            }
            let baseline = snapshot_shadow_state(&state_once);

            // Run 2: apply each event N times (1..5x).
            let state_dup = Arc::new(crate::state::AppState::default());
            for (e, &dups) in events.iter().take(n).zip(dup_counts.iter().take(n)) {
                for _ in 0..dups {
                    apply_shadow_projection(&state_dup, e);
                }
            }
            let inflated = snapshot_shadow_state(&state_dup);

            prop_assert_eq!(baseline.0, inflated.0, "shadow_window_meta diverged under duplicate-bursting");
            prop_assert_eq!(baseline.1, inflated.1, "shadow_backend_window_ids diverged under duplicate-bursting");
            prop_assert_eq!(baseline.2, inflated.2, "shadow_instance_registry diverged under duplicate-bursting");
        }
    }

    /// Anti-vacuity guard (per `feedback_property_test_input_must_match_sut_filter`):
    /// confirm the projection actually mutates the shadow state under
    /// the strategy's events. If the strategy ever drifts away from
    /// the SUT (e.g. event variants renamed), this test fails loudly
    /// instead of letting the property hold vacuously.
    #[test]
    fn projection_actually_mutates_state_for_strategy_events() {
        let state = Arc::new(crate::state::AppState::default());
        apply_shadow_projection(
            &state,
            &Event::WindowOpened {
                label: "a".to_string(),
                kind: WindowKind::FullInstance,
                parent_label: None,
                version: 0,
            },
        );
        assert!(
            state.shadow_window_meta.lock().contains_key("a"),
            "projection failed to mutate shadow_window_meta — strategy/SUT drift?"
        );
        apply_shadow_projection(
            &state,
            &Event::BackendWindowIdRegistered {
                label: "a".to_string(),
                window_id: "w-1".to_string(),
                version: 0,
            },
        );
        assert!(
            state.shadow_backend_window_ids.lock().contains_key("a"),
            "projection failed to mutate shadow_backend_window_ids"
        );
        apply_shadow_projection(
            &state,
            &Event::WindowInstanceAssigned {
                label: "a".to_string(),
                num: 7,
                version: 0,
            },
        );
        assert_eq!(
            state.shadow_instance_registry.lock().get("a"),
            Some(&7),
            "projection failed to mutate shadow_instance_registry"
        );
    }
}
