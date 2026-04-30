// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase E.2 — persist subscriber: bridges the srv reducer's event
// stream to SQLite. Listens on the broadcast bus, writes
// reducer-emitted events to wstore.
//
// Design (per spec §6):
//   * Reducer is canonical IN-MEMORY. SQLite is a derived projection.
//   * Bootstrap reads SQLite into reducer state at startup (§6.1
//     phase 1). Phase 2 (replay-from-HWM via event log) is deferred
//     to a later sub-phase; E.2 ships the in-memory side only.
//   * Persist subscriber writes via the same wstore APIs that
//     wcore::* uses, so the on-disk format is unchanged.
//
// Idempotency: the subscriber checks for existing rows before
// inserting (SQLite INSERT would constraint-fail otherwise).
// DELETE is naturally idempotent. Required for safe replay once
// event-log-replay-at-bootstrap lands; harmless overhead until then.

use std::sync::Arc;

use agentmux_common::ipc::Event;
use tokio::sync::Mutex;

use crate::backend::obj::Workspace;
use crate::backend::storage::wstore::WaveStore;
use crate::state::{State, WorkspaceRecord};

/// Phase E.2 — load workspaces from SQLite into the reducer state.
/// Called once at srv startup before the IPC server starts accepting
/// commands.
///
/// Errors are logged but non-fatal: if SQLite read fails (e.g. fresh
/// install with empty DB), the reducer starts with an empty
/// workspace map. Subsequent reducer commands populate state as
/// before; subscribers (renderer, host, future Tools) see what the
/// reducer has.
pub async fn bootstrap_state_from_wstore(state: &Arc<Mutex<State>>, wstore: &WaveStore) {
    let workspaces = match wstore.get_all::<Workspace>() {
        Ok(ws) => ws,
        Err(e) => {
            tracing::warn!(
                target: "srv-persist",
                "[srv-persist] bootstrap: failed to load workspaces from wstore: {} — reducer starts empty",
                e
            );
            return;
        }
    };
    // Acquire the state mutex via `.lock().await`. Async because
    // we're inside a tokio runtime — `blocking_lock` would panic
    // ("Cannot block the current thread from within a runtime").
    // No actual await suspension expected since bootstrap runs
    // before the IPC server starts accepting commands.
    let mut state = state.lock().await;
    for ws in workspaces {
        state.workspaces.insert(
            ws.oid.clone(),
            WorkspaceRecord {
                workspace_id: ws.oid,
                name: ws.name,
            },
        );
    }
    tracing::info!(
        target: "srv-persist",
        "[srv-persist] bootstrap loaded {} workspace(s) from wstore",
        state.workspaces.len()
    );
}

/// Phase E.2 — persist subscriber task. Subscribes to the srv
/// broadcast bus, mirrors workspace events to SQLite. Idempotent
/// w.r.t. duplicate events (replay-safe by construction once
/// event-log replay lands).
///
/// Receives the broadcast receiver pre-subscribed (per the same
/// pattern as run_disk_writer + saga coordinator — subscribe before
/// spawn to avoid losing events between construction and first
/// recv).
pub async fn run_persist_subscriber(
    wstore: Arc<WaveStore>,
    state: Arc<Mutex<State>>,
    mut events_rx: tokio::sync::broadcast::Receiver<Event>,
) {
    tracing::info!(target: "srv-persist", "[srv-persist] subscriber started");
    // Phase E.2 (codex P1 #611) — once we lag the broadcast bus, we
    // don't know which event versions we missed. Setting this flag
    // freezes `persistence_hwm` at its pre-lag value: subsequent
    // events still get written to SQLite (idempotent), but HWM stops
    // advancing so future bootstrap-replay (later sub-phase) re-runs
    // from before the lag. Replay is idempotent by construction
    // (insert-if-missing, delete-if-present), so re-applying known-
    // persisted events is safe.
    let mut lagged_since_start = false;
    loop {
        match events_rx.recv().await {
            Ok(event) => {
                if let Err(e) =
                    apply_event_to_wstore(&wstore, &state, &event, lagged_since_start).await
                {
                    tracing::warn!(
                        target: "srv-persist",
                        "[srv-persist] write failed for {:?}: {}",
                        event_kind_name(&event),
                        e
                    );
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                lagged_since_start = true;
                tracing::error!(
                    target: "srv-persist",
                    "[srv-persist] lagged event bus, missed {} events — freezing persistence_hwm so future replay re-runs from before the gap (events still applied to SQLite, idempotent)",
                    n
                );
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                tracing::info!(target: "srv-persist", "[srv-persist] subscriber stopping (bus closed)");
                return;
            }
        }
    }
}

// `lagged_since_start`: when true, the persist subscriber lagged
// earlier in this session and we don't know which events were
// missed. SQLite writes still happen (idempotent), but HWM stays
// frozen so future bootstrap-replay re-runs from before the gap.
// (codex P1 #611.)
async fn apply_event_to_wstore(
    wstore: &Arc<WaveStore>,
    state: &Arc<Mutex<State>>,
    event: &Event,
    lagged_since_start: bool,
) -> Result<(), String> {
    match event {
        Event::WorkspaceCreated {
            workspace_id,
            name,
            version,
        } => {
            // Idempotent insert: skip if a row with this oid already
            // exists in SQLite. Once event-log-replay lands, this
            // becomes important; for E.2 it's defensive belt.
            let exists = wstore.must_get::<Workspace>(workspace_id).is_ok();
            if !exists {
                let mut ws = Workspace {
                    oid: workspace_id.clone(),
                    name: name.clone(),
                    tabids: vec![],
                    pinnedtabids: vec![],
                    activetabid: String::new(),
                    meta: crate::backend::obj::MetaMapType::new(),
                    ..Default::default()
                };
                wstore
                    .insert(&mut ws)
                    .map_err(|e| format!("workspace insert: {}", e))?;
            }
            if !lagged_since_start {
                advance_hwm(state, *version).await;
            }
            Ok(())
        }
        Event::WorkspaceDeleted {
            workspace_id,
            version,
        } => {
            // Phase E.2 (codex P1 #611) — use wcore::delete_workspace
            // for cascading semantics (deletes child tabs+blocks).
            // Plain wstore::delete<Workspace> would leave orphan
            // tab/block rows in SQLite; the legacy RPC path
            // (wcore::delete_workspace) handles the cascade and
            // we mirror that behavior here.
            //
            // Idempotent: wcore::delete_workspace returns
            // StoreError::NotFound on a missing workspace, which we
            // swallow as a no-op (the reducer already checked
            // existence before emitting the event).
            match crate::backend::wcore::delete_workspace(wstore, workspace_id) {
                Ok(()) => {}
                Err(crate::backend::storage::StoreError::NotFound) => {
                    tracing::debug!(
                        target: "srv-persist",
                        "[srv-persist] workspace {} already absent at delete time — idempotent no-op",
                        workspace_id
                    );
                }
                Err(e) => return Err(format!("workspace cascade delete: {}", e)),
            }
            if !lagged_since_start {
                advance_hwm(state, *version).await;
            }
            Ok(())
        }
        // Phase E.2 only handles workspace lifecycle. Other event
        // variants are no-ops here; later sub-phases add tab / block
        // / layout persistence.
        _ => Ok(()),
    }
}

async fn advance_hwm(state: &Arc<Mutex<State>>, version: u64) {
    let mut state = state.lock().await;
    if version > state.persistence_hwm {
        state.persistence_hwm = version;
    }
}

fn event_kind_name(e: &Event) -> &'static str {
    match e {
        Event::WorkspaceCreated { .. } => "WorkspaceCreated",
        Event::WorkspaceDeleted { .. } => "WorkspaceDeleted",
        _ => "Other",
    }
}
