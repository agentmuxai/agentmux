// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! WaveObjUpdate broadcast bridge.
//!
//! Subscribes to `srv_events_tx` (the internal sidecar event bus that the
//! reducer publishes mutations to) and translates each event into one or
//! more `WaveObjUpdate` records, broadcast to all connected WS clients via
//! the existing `event_bus.broadcast_event(...)` plumbing — the same path
//! that `service.rs:39-52`'s response-broadcast loop uses.
//!
//! Why this exists: per-RPC handlers were responsible for attaching
//! `WaveObjUpdate`s to their responses (`success_with_updates(...)`).
//! Forgetting that call left the frontend WOS cache stale (e.g. workspace
//! renames not propagating to the OS title or the InstancePanel — see
//! `docs/specs/SPEC_REACTIVE_WORKSPACE_SYNC_2026-05-14.md`).
//!
//! With this bridge in place, any reducer event automatically reaches the
//! frontend, so the per-handler convention becomes belt-and-suspenders
//! instead of load-bearing.
//!
//! Spec: `docs/specs/SPEC_OBJ_UPDATE_BRIDGE_2026-05-14.md`.
//!
//! Phase 1 scope (this implementation): workspace events only —
//! immediately fixes the user-reported bug. Phase 2 expands to tabs /
//! blocks / windows / layouts; Phase 3 retires the per-handler
//! `success_with_updates(...)` calls now that the bridge covers them.

use std::sync::Arc;

use agentmux_common::ipc::Event;
use tokio::sync::broadcast;

use crate::backend::eventbus::{EventBus, WSEventType};
use crate::backend::obj::{wave_obj_to_value, OTYPE_WORKSPACE};
use crate::backend::storage::wstore::WaveStore;

/// JSON shape that gets broadcast as the `data` payload of a
/// `waveobj:update` WS event. Matches the shape of `WaveObjUpdate` in
/// `agentmux-srv/src/backend/obj.rs:465-474` so the frontend's existing
/// `updateWaveObject` handler accepts it without changes.
fn build_update_payload(
    updatetype: &str,
    otype: &str,
    oid: &str,
    obj: Option<serde_json::Value>,
) -> serde_json::Value {
    let mut map = serde_json::Map::with_capacity(4);
    map.insert("updatetype".into(), serde_json::Value::String(updatetype.into()));
    map.insert("otype".into(), serde_json::Value::String(otype.into()));
    map.insert("oid".into(), serde_json::Value::String(oid.into()));
    if let Some(o) = obj {
        map.insert("obj".into(), o);
    }
    serde_json::Value::Object(map)
}

/// Push one `WaveObjUpdate` payload to all connected WS clients via the
/// shared event_bus. Mirrors the response-broadcast loop in
/// `service.rs:39-52`.
fn emit(event_bus: &EventBus, otype: &str, oid: &str, payload: serde_json::Value) {
    let oref = format!("{otype}:{oid}");
    event_bus.broadcast_event(&WSEventType {
        eventtype: "waveobj:update".to_string(),
        oref,
        data: Some(payload),
    });
}

/// Translate one reducer event into zero or more `waveobj:update` broadcasts.
///
/// Phase 1 covers the four workspace events. Other variants intentionally
/// fall through to the catch-all `_ => {}` arm — they're either not
/// WaveObj-affecting (saga lifecycle, OS facts, etc.) or scheduled for
/// Phase 2 expansion.
///
/// **Read source — post-event state guarantee:**
/// For Phase 1 (workspace events), every workspace mutation flows through
/// the HTTP `service.rs:UpdateWorkspace` handler which calls
/// `apply_event_to_wstore` synchronously (`service.rs:1297-1304`) before
/// `publish_events` (`service.rs:1305`). So when the bridge receives a
/// workspace event, SQLite is already up-to-date.
///
/// **Phase 2 caveat:** for tab/block events that the launcher → IPC path
/// in `srv_ipc/server.rs:295` may publish without first applying to
/// SQLite, the persist subscriber and bridge race. Phase 2 should either
/// (a) make the IPC path apply synchronously like the HTTP path does, or
/// (b) read from the in-memory `srv_state` reducer rather than SQLite.
/// Tracked in `SPEC_OBJ_UPDATE_BRIDGE §11.1` follow-up.
///
/// **Lock discipline (per ReAgent P1 on PR #852):** `WaveStore::get<T>()`
/// acquires `std::sync::Mutex<Connection>` (blocking). Even though the
/// hold is brief in steady state, a long reducer transaction could block
/// this tokio worker thread. We therefore do the SQLite read inside
/// `tokio::task::spawn_blocking` so the async runtime stays responsive.
async fn dispatch_event(event: Event, wstore: Arc<WaveStore>, event_bus: Arc<EventBus>) {
    use crate::backend::obj::Workspace;

    match event {
        Event::WorkspaceRenamed { workspace_id, .. }
        | Event::WorkspaceMetaUpdated { workspace_id, .. }
        | Event::WorkspaceCreated { workspace_id, .. } => {
            // Offload the blocking SQLite read to the blocking thread pool
            // so async worker threads stay free for I/O-bound work.
            let id = workspace_id.clone();
            let store = Arc::clone(&wstore);
            let result = tokio::task::spawn_blocking(move || store.get::<Workspace>(&id)).await;
            match result {
                Ok(Ok(Some(ws))) => {
                    // "update" for both create and update — the frontend's
                    // updateWaveObject (`wos.ts:259-279`) treats anything
                    // not "delete" identically. Sending "update" uniformly
                    // simplifies the bridge's logic and matches what the
                    // existing response-broadcast loop emits for the same
                    // events.
                    let payload = build_update_payload(
                        "update",
                        OTYPE_WORKSPACE,
                        &workspace_id,
                        Some(wave_obj_to_value(&ws)),
                    );
                    emit(&event_bus, OTYPE_WORKSPACE, &workspace_id, payload);
                }
                Ok(Ok(None)) => {
                    tracing::warn!(
                        target: "wave-obj-bridge",
                        workspace_id = %workspace_id,
                        "workspace event for missing workspace; skipping broadcast"
                    );
                }
                Ok(Err(e)) => {
                    tracing::error!(
                        target: "wave-obj-bridge",
                        workspace_id = %workspace_id,
                        error = %e,
                        "wstore.get::<Workspace> failed; skipping broadcast"
                    );
                }
                Err(join_err) => {
                    tracing::error!(
                        target: "wave-obj-bridge",
                        workspace_id = %workspace_id,
                        error = %join_err,
                        "spawn_blocking join failed (likely panicked); skipping broadcast"
                    );
                }
            }
        }

        Event::WorkspaceDeleted { workspace_id, .. } => {
            // No object to fetch — frontend just needs the oid + delete tag.
            let payload = build_update_payload("delete", OTYPE_WORKSPACE, &workspace_id, None);
            emit(&event_bus, OTYPE_WORKSPACE, &workspace_id, payload);
        }

        // All other event variants are either not WaveObj-affecting (saga
        // lifecycle, OS facts, …) or pending Phase 2 coverage. Silent skip
        // is correct — the catch-all _ arm makes this future-proof for new
        // event variants the reducer may add.
        _ => {}
    }
}

/// Spawn the bridge task. Returns the `JoinHandle` so callers can keep it
/// alive (typically forever — the task lives for the lifetime of the srv
/// process). Per ReAgent P1 on PR #852: the loop is panic-resilient — a
/// panic inside `dispatch_event` is caught and logged, and the loop
/// continues processing subsequent events. Without this, a single
/// malformed event could silently kill the entire bridge task and
/// frontend WOS would stop seeing updates.
///
/// Subscribe ordering: per `SPEC §11.1` the bridge can subscribe in any
/// order relative to the persist subscriber. For Phase 1's workspace
/// events the HTTP RPC handler applies SQLite synchronously before
/// publishing the event, so the bridge always sees post-event state.
pub fn spawn_wave_obj_bridge(
    events_rx: broadcast::Receiver<Event>,
    wstore: Arc<WaveStore>,
    event_bus: Arc<EventBus>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(run_wave_obj_bridge(events_rx, wstore, event_bus))
}

async fn run_wave_obj_bridge(
    mut events_rx: broadcast::Receiver<Event>,
    wstore: Arc<WaveStore>,
    event_bus: Arc<EventBus>,
) {
    tracing::info!(target: "wave-obj-bridge", "[wave-obj-bridge] started (Phase 1: workspace events)");
    loop {
        match events_rx.recv().await {
            Ok(event) => {
                // Per-event panic isolation (ReAgent P1 on PR #852): use
                // FuturesUnordered with a catch_unwind future would be the
                // textbook fix, but for a single event-at-a-time loop the
                // simpler pattern is to spawn the dispatch as its own task
                // and observe the JoinError if it panics. We `await` it
                // immediately so events still process serially (matching
                // the broadcast channel's send order), but a panic in one
                // event can't kill the bridge.
                let store = Arc::clone(&wstore);
                let bus = Arc::clone(&event_bus);
                let event_dbg = format!("{:?}", &event);
                let join = tokio::spawn(dispatch_event(event, store, bus)).await;
                if let Err(join_err) = join {
                    if join_err.is_panic() {
                        tracing::error!(
                            target: "wave-obj-bridge",
                            event = %event_dbg,
                            "dispatch_event panicked; bridge continues with next event. Panic: {}",
                            join_err,
                        );
                    } else {
                        tracing::error!(
                            target: "wave-obj-bridge",
                            event = %event_dbg,
                            error = %join_err,
                            "dispatch_event task aborted unexpectedly"
                        );
                    }
                }
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                // The broadcast channel has 1024 capacity (main.rs:624).
                // If we lag, frontend WOS state diverges silently — log it
                // loudly so operators can correlate with user-visible drift
                // (e.g. the InstancePanel/title showing stale names).
                // No automatic recovery; the next event resyncs the affected
                // object and frontend reads everything else from its cache.
                tracing::error!(
                    target: "wave-obj-bridge",
                    skipped = n,
                    "broadcast channel lagged; some waveobj:update events were dropped — frontend WOS may show stale state until the affected object is mutated again"
                );
            }
            Err(broadcast::error::RecvError::Closed) => {
                tracing::info!(target: "wave-obj-bridge", "events channel closed; bridge exiting");
                return;
            }
        }
    }
}
