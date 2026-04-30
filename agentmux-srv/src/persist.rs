// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase E.2 — bootstrap helper: load SQLite-persistent state into
// the srv reducer at startup. The reducer's state is a SESSION-only
// projection in E.2 — it's populated from SQLite at boot, mutated
// by pipe-originated commands during the session, and discarded on
// restart (the next bootstrap re-reads SQLite).
//
// HTTP/WS RPC continues to write to SQLite directly via wcore. So
// SQLite stays authoritative for the duration of the session even
// though the reducer's view diverges as soon as a pipe command
// runs. That's intentional: pipe-originated commands have no
// client populating them yet (saga coordinator is empty in E.1a;
// E.5+ adds saga consumers). Once those exist, E.2c adds the
// persist subscriber that mirrors pipe-event effects back to SQLite.
//
// This module DOES NOT define a persist subscriber. The HWM /
// broadcast-lag concerns codex flagged are deferred to E.2c when
// the subscriber actually exists.

use std::sync::Arc;

use tokio::sync::Mutex;

use crate::backend::obj::Workspace;
use crate::backend::storage::wstore::WaveStore;
use crate::state::{State, WorkspaceRecord};

/// Phase E.2 — load workspaces from SQLite into the reducer state.
/// Called once at srv startup before the IPC server starts accepting
/// commands. Async because we're inside the tokio runtime.
///
/// Errors are logged but non-fatal: if SQLite read fails (fresh
/// install, empty DB), the reducer starts with an empty workspace
/// map. Subsequent reducer commands populate state as before.
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
