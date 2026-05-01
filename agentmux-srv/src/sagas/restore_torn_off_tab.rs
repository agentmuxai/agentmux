// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase E.5.6 — RestoreTornOffTab saga.
//
// Drops the torn-off tab back into the destination workspace and,
// if the source workspace became empty, deletes it (cascade).
//
// **Steps:**
// 1. `MoveTab { tab_id, src=source_ws, dst=dest_ws, dst_index }`
// 2. If post-move state shows source workspace's `tab_ids` is
//    empty → `DeleteWorkspace { source_ws }` (cascade is built in;
//    no surviving tabs to handle).
//    Otherwise → done.
//
// **Compensation:**
// * Step 1 fails → nothing to compensate (reducer rejected without
//   mutating; tab is still in source).
// * Step 2 fails → tab is already restored to dest; the orphan
//   source workspace persists. Log the failure but return success
//   (the user-visible operation succeeded; the orphan is a soft
//   cleanup gap that the next user action — close window, etc. —
//   will catch).
//
// **Pinning note:** the legacy `was_pinned` arg of `RestoreTornOffTab`
// is ignored. Pinning was a Waveterm feature removed from AgentMux
// (per E.2c.3b). Restored tabs always land in `tab_ids`.

use agentmux_common::ipc::Command;
use serde_json::{json, Value};

use super::{
    alloc_saga_id, classify_run_saga_result, emit_saga_started, emit_terminal, run_saga, SagaCtx,
};
use crate::server::AppState;

/// Run the RestoreTornOffTab saga. On success, returns
/// `{"source_workspace_deleted": <bool>}`.
pub async fn run(
    state: &AppState,
    tab_id: String,
    source_workspace_id: String,
    dest_workspace_id: String,
    insert_index: Option<u32>,
) -> Result<Value, String> {
    // Pre-condition checks read SQLite, not reducer state. During
    // the migration window, wcore-direct paths (PromoteBlockToTab,
    // etc.) leave reducer.tabs / workspaces stale relative to disk;
    // a reducer-state pre-check would falsely reject valid restores
    // (codex P1 round-2 #621).
    {
        let src_ws = match state.wstore.get::<crate::backend::obj::Workspace>(&source_workspace_id) {
            Ok(Some(ws)) => ws,
            Ok(None) => {
                return Err(format!(
                    "RestoreTornOffTab: source workspace not found: {}",
                    source_workspace_id
                ));
            }
            Err(e) => {
                return Err(format!(
                    "RestoreTornOffTab: workspace read failed: {}",
                    e
                ));
            }
        };
        if state
            .wstore
            .get::<crate::backend::obj::Workspace>(&dest_workspace_id)
            .map(|w| w.is_none())
            .unwrap_or(true)
        {
            return Err(format!(
                "RestoreTornOffTab: dest workspace not found: {}",
                dest_workspace_id
            ));
        }
        let in_workspace = src_ws.tabids.iter().any(|id| id == &tab_id)
            || src_ws.pinnedtabids.iter().any(|id| id == &tab_id);
        if !in_workspace {
            return Err(format!(
                "RestoreTornOffTab: tab {} is not in workspace {}",
                tab_id, source_workspace_id
            ));
        }
    }

    let dst_index = insert_index.unwrap_or(u32::MAX);

    let saga_id = alloc_saga_id(state);
    if let Err(e) = emit_saga_started(
        state,
        saga_id,
        "restore_torn_off_tab",
        serde_json::json!({
            "tab_id": &tab_id,
            "source_workspace_id": &source_workspace_id,
            "dest_workspace_id": &dest_workspace_id,
            "insert_index": dst_index,
        }),
    )
    .await
    {
        return Err(e);
    }
    let ctx = SagaCtx::new(state, saga_id);
    let result = run_saga(
        "restore_torn_off_tab",
        run_inner(ctx, tab_id, source_workspace_id, dest_workspace_id, dst_index),
    )
    .await;
    emit_terminal(state, saga_id, classify_run_saga_result(&result)).await;
    result
}

async fn run_inner(
    ctx: SagaCtx<'_>,
    tab_id: String,
    source_workspace_id: String,
    dest_workspace_id: String,
    dst_index: u32,
) -> Result<Value, String> {
    // Step 1: move the tab.
    ctx.dispatch(Command::MoveTab {
        tab_id: tab_id.clone(),
        src_workspace_id: source_workspace_id.clone(),
        dst_workspace_id: dest_workspace_id.clone(),
        dst_index,
    })
    .await
    .map_err(|e| format!("RestoreTornOffTab step 1 (MoveTab): {}", e))?;

    // Step 2: check post-move state, conditionally delete source.
    let source_now_empty = {
        let s = ctx.state_lock().await;
        s.workspaces
            .get(&source_workspace_id)
            .map(|ws| ws.tab_ids.is_empty())
            .unwrap_or(true)
    };

    let mut source_deleted = false;
    if source_now_empty {
        // Best-effort delete; log + soft-fail on reducer rejection
        // (tab is already restored, which is what the user asked for).
        match ctx
            .dispatch(Command::DeleteWorkspace {
                workspace_id: source_workspace_id.clone(),
                // `force: false` — sub-step within restore_torn_off_tab,
                // not the dedicated `delete_workspace` saga (Step 5 PR 2).
                // The workspace is already empty here so cascade-vs-saga
                // distinction is moot; the flag is provenance-only.
                force: false,
            })
            .await
        {
            Ok(_) => source_deleted = true,
            Err(e) => {
                tracing::warn!(
                    saga_id = ctx.saga_id(),
                    "[saga] RestoreTornOffTab: source workspace cleanup failed: {} (orphan workspace {})",
                    e,
                    source_workspace_id
                );
            }
        }
    }

    Ok(json!({ "source_workspace_deleted": source_deleted }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentmux_common::ipc::Event;
    use crate::backend::obj::Workspace;
    use crate::server::tests::test_state;

    async fn dispatch_apply(
        state: &crate::server::AppState,
        cmd: agentmux_common::ipc::Command,
    ) -> Vec<Event> {
        let events = crate::server::service::dispatch_to_reducer(state, cmd).await;
        for ev in &events {
            crate::persist_subscriber::apply_event_to_wstore(ev, &state.wstore).unwrap();
        }
        events
    }

    /// Seeds a "torn-off" workspace (single tab) and a destination
    /// workspace (its own tab). Returns (state, torn_ws, torn_tab,
    /// dest_ws, dest_tab).
    async fn seed_torn_off_state() -> (crate::server::AppState, String, String, String, String) {
        let state = test_state();
        let mut ws_ids = Vec::new();
        let mut tab_ids = Vec::new();
        for ws_name in &["torn", "dest"] {
            let ws_evs = dispatch_apply(
                &state,
                agentmux_common::ipc::Command::CreateWorkspace {
                    name: ws_name.to_string(),
                },
            )
            .await;
            let ws_id = ws_evs
                .iter()
                .find_map(|e| match e {
                    Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
                    _ => None,
                })
                .unwrap();
            let tab_evs = dispatch_apply(
                &state,
                agentmux_common::ipc::Command::CreateTab {
                    workspace_id: ws_id.clone(),
                    name: format!("{}-tab", ws_name),
                },
            )
            .await;
            let tab_id = tab_evs
                .iter()
                .find_map(|e| match e {
                    Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
                    _ => None,
                })
                .unwrap();
            ws_ids.push(ws_id);
            tab_ids.push(tab_id);
        }
        (
            state,
            ws_ids[0].clone(),
            tab_ids[0].clone(),
            ws_ids[1].clone(),
            tab_ids[1].clone(),
        )
    }

    #[tokio::test]
    async fn happy_path_moves_tab_back_and_deletes_empty_source() {
        let (state, torn_ws, torn_tab, dest_ws, _dest_tab) = seed_torn_off_state().await;
        let result = run(&state, torn_tab.clone(), torn_ws.clone(), dest_ws.clone(), Some(0))
            .await
            .unwrap();
        assert_eq!(result["source_workspace_deleted"], true);

        // Reducer: source workspace gone; tab is in dest.
        let s = state.srv_state.lock().await;
        assert!(!s.workspaces.contains_key(&torn_ws));
        assert!(s.workspaces[&dest_ws].tab_ids.contains(&torn_tab));
        assert_eq!(s.tabs[&torn_tab].workspace_id, dest_ws);
        drop(s);

        // SQLite: source workspace row gone too.
        assert!(state.wstore.get::<Workspace>(&torn_ws).unwrap().is_none());
    }

    #[tokio::test]
    async fn skips_delete_when_source_still_has_tabs() {
        let (state, torn_ws, torn_tab, dest_ws, _) = seed_torn_off_state().await;
        // Add a second tab to torn so it doesn't become empty.
        let _ = dispatch_apply(
            &state,
            agentmux_common::ipc::Command::CreateTab {
                workspace_id: torn_ws.clone(),
                name: "extra".into(),
            },
        )
        .await;

        let result = run(&state, torn_tab, torn_ws.clone(), dest_ws, Some(0))
            .await
            .unwrap();
        assert_eq!(
            result["source_workspace_deleted"], false,
            "source ws should survive when it still has tabs"
        );
        // Source still in reducer.
        let s = state.srv_state.lock().await;
        assert!(s.workspaces.contains_key(&torn_ws));
    }
}
