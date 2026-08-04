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

/// SPEC_864 Phase 3 — single-writer layout seeding. Dispatch a
/// `LayoutSetTree` (tree + focus + leaforder as client slices) and persist
/// it inside ONE hold of the reducer mutex, so persist order equals
/// dispatch order — the same contract as `UpdateObject`'s Phase-2 route in
/// `object.rs`. Replaces the wcore-direct seeders
/// (`write_default_three_pane_layout` post-bootstrap caller,
/// `setup_torn_off_block_layout`): the reducer's `TabRecord.rootnode` is
/// authoritative, and the persist subscriber is the sole `db_layout`
/// writer on this path.
///
/// On SQLite apply failure the reducer is rolled back to an empty tree
/// (the seed target is a fresh tab whose row was empty) inside the same
/// lock hold. Publishes events on success. Requires the tab to be known
/// to the reducer — true for every caller (all run after reducer-routed
/// CreateTab / sagas); the pre-bootstrap first-launch seed
/// (`ensure_initial_data` → `seed_default_layout`) deliberately stays
/// store-direct because the reducer isn't hydrated yet (spec site #3).
pub(crate) async fn seed_layout_via_reducer(
    state: &AppState,
    tab_id: &str,
    new_tree: agentmux_common::LayoutNode,
    focused_node_id: String,
    leaforder: Vec<crate::backend::obj::LeafOrderEntry>,
) -> Result<(), String> {
    let store = &state.wstore;
    let slices = agentmux_common::LayoutClientSlices {
        leaforder: serde_json::to_value(&leaforder).ok(),
        focused_node_id,
        magnified_node_id: String::new(),
        // Fresh-tab seed: REPLACE semantics clear any stale queue.
        pending_backend_actions: None,
    };
    let cmd = agentmux_common::ipc::Command::LayoutSetTree {
        tab_id: tab_id.to_string(),
        new_tree: Some(new_tree),
        correlation_id: String::new(),
        slices: Some(slices),
    };

    let mut apply_err: Option<String> = None;
    let events = {
        let mut s = state.srv_state.lock().await;
        let ctx = crate::reducer::Ctx {
            now_rfc3339: chrono::Utc::now().to_rfc3339(),
            registered_pid: None,
        };
        let events = crate::reducer::update(&mut s, cmd, &ctx);
        let has_error = events
            .iter()
            .any(|e| matches!(e, agentmux_common::ipc::Event::Error { .. }));
        if !has_error {
            for ev in &events {
                if let Err(e) = crate::persist_subscriber::apply_event_to_wstore(ev, store) {
                    apply_err = Some(e.to_string());
                    break;
                }
            }
            if apply_err.is_some() {
                // Roll the reducer back to the empty pre-seed tree inside
                // the same lock hold; best-effort SQLite mirror.
                let rollback = agentmux_common::ipc::Command::LayoutSetTree {
                    tab_id: tab_id.to_string(),
                    new_tree: None,
                    correlation_id: String::new(),
                    slices: Some(agentmux_common::LayoutClientSlices::default()),
                };
                let rb_events = crate::reducer::update(&mut s, rollback, &ctx);
                for ev in &rb_events {
                    if let Err(e) = crate::persist_subscriber::apply_event_to_wstore(ev, store) {
                        tracing::warn!(
                            error = %e,
                            "seed_layout_via_reducer: rollback SQLite mirror failed"
                        );
                    }
                }
            }
        }
        events
    };

    if let Some(err_msg) = events.iter().find_map(|e| match e {
        agentmux_common::ipc::Event::Error { message, .. } => Some(message.clone()),
        _ => None,
    }) {
        return Err(err_msg);
    }
    if let Some(err) = apply_err {
        return Err(format!("layout seed SQLite write failed: {}", err));
    }
    publish_events(state, &events);
    Ok(())
}

/// SPEC_864 Phase 4 — single-writer queue append. Dispatch a
/// `LayoutQueueBackendActions` and persist the resulting
/// `LayoutBackendActionsQueued` inside ONE hold of the reducer mutex
/// (same critical-section contract as `seed_layout_via_reducer`), so a
/// concurrent frontend ACK slice (`LayoutSetTree` REPLACE) can't
/// interleave between dispatch and persist. Replaces the five
/// store-direct `pendingbackendactions` writers (`layout_helpers`'
/// insert/split/delete queuers + the two inline app_api sites).
///
/// No rollback arm: the reducer does not model the queue in
/// `TabRecord` (pass-through validation only), so a SQLite apply
/// failure leaves no reducer↔SQLite divergence to repair — just
/// return the error.
///
/// `actions` must be non-empty; the reducer rejects an empty array.
pub(crate) async fn queue_layout_actions_via_reducer(
    state: &AppState,
    tab_id: &str,
    actions: Vec<crate::backend::obj::LayoutActionData>,
) -> Result<(), String> {
    let store = &state.wstore;
    let actions_json = serde_json::to_value(&actions)
        .map_err(|e| format!("queue actions serialize failed: {}", e))?;
    let cmd = agentmux_common::ipc::Command::LayoutQueueBackendActions {
        tab_id: tab_id.to_string(),
        actions: actions_json,
        correlation_id: String::new(),
    };

    let mut apply_err: Option<String> = None;
    let events = {
        let mut s = state.srv_state.lock().await;
        let ctx = crate::reducer::Ctx {
            now_rfc3339: chrono::Utc::now().to_rfc3339(),
            registered_pid: None,
        };
        let events = crate::reducer::update(&mut s, cmd, &ctx);
        let has_error = events
            .iter()
            .any(|e| matches!(e, agentmux_common::ipc::Event::Error { .. }));
        if !has_error {
            for ev in &events {
                if let Err(e) = crate::persist_subscriber::apply_event_to_wstore(ev, store) {
                    apply_err = Some(e.to_string());
                    break;
                }
            }
        }
        events
    };

    if let Some(err_msg) = events.iter().find_map(|e| match e {
        agentmux_common::ipc::Event::Error { message, .. } => Some(message.clone()),
        _ => None,
    }) {
        return Err(err_msg);
    }
    if let Some(err) = apply_err {
        return Err(format!("queue append SQLite write failed: {}", err));
    }
    publish_events(state, &events);
    Ok(())
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
