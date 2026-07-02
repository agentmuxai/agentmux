// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Phase E.2c.2 reducer-dispatch helpers shared by the service handlers and
//! (via `crate::server::service::…`) the sagas.

use crate::backend::obj::Workspace;
use crate::backend::storage::store::Store;
use crate::backend::storage::StoreError;

use super::super::AppState;

/// Dispatch a command into the srv reducer and return the emitted
/// events. Locks the reducer mutex briefly; the lock is released
/// before any I/O (caller is responsible for publishing the events
/// to the broadcast bus).
pub(crate) async fn dispatch_to_reducer(
    state: &AppState,
    cmd: agentmux_common::ipc::Command,
) -> Vec<agentmux_common::ipc::Event> {
    let now = chrono::Utc::now().to_rfc3339();
    let mut s = state.srv_state.lock().await;
    let ctx = crate::reducer::Ctx {
        now_rfc3339: now,
        // RPC-originated dispatch has no IPC connection — sentinel.
        conn_id: 0,
        registered_pid: None,
    };
    crate::reducer::update(&mut s, cmd, &ctx)
}

/// Publish each event on the srv broadcast bus. Failures (no
/// subscribers) are non-fatal.
pub(crate) fn publish_events(state: &AppState, events: &[agentmux_common::ipc::Event]) {
    for event in events {
        let _ = state.srv_events_tx.send(event.clone());
    }
}

/// Compensation helper: dispatch a command into the reducer and
/// apply its emitted events to wstore best-effort. Used when an
/// earlier sync apply partially wrote SQLite and we need to undo
/// the leaked rows. SQLite errors during compensation are logged
/// but ignored — the caller is already returning an error to the
/// client; throwing on the cleanup just hides the original cause.
/// (codex P1 + reagent P2 #616 — partial-write cleanup.)
pub(crate) async fn compensate_via_reducer(
    state: &AppState,
    cmd: agentmux_common::ipc::Command,
    store: &Store,
) {
    let events = dispatch_to_reducer(state, cmd).await;
    for ev in &events {
        if let Err(e) = crate::persist_subscriber::apply_event_to_wstore(ev, store) {
            tracing::warn!(
                "compensation: SQLite cleanup failed for event {:?}: {}",
                std::mem::discriminant(ev),
                e
            );
        }
    }
}

/// Existence check used by `DeleteWorkspace` to decide whether to
/// run the wcore delete path. Propagates `StoreError` so the caller
/// can surface real I/O / corruption failures instead of
/// misclassifying them as "not found" (codex P2 #615 carryover —
/// the prior `bool` return collapsed `Err(_)` into `false`, which
/// led to silent successes when SQLite was unhealthy: reducer would
/// delete its own copy and report success while the disk row was
/// never touched).
pub(crate) fn wstore_workspace_exists(
    store: &Store,
    workspace_id: &str,
) -> Result<bool, StoreError> {
    Ok(store.get::<Workspace>(workspace_id)?.is_some())
}

// `build_workspace_from_state` removed in E.2c.2. The reducer's
// WorkspaceRecord can't faithfully render a Workspace during the
// migration window (no pinnedtabids; tabids/activetabid go stale
// vs wcore-direct tab ops). It will be reintroduced in E.2c.3
// when tabs migrate into the reducer and pinned/active state is
// authoritative there. (reagent + codex P1 #615.)
