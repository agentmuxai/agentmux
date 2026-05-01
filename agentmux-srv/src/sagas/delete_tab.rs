// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase E.5.7 (Step 5 PR 1) — DeleteTab saga.
//
// Replaces the SQLite-first delete pattern in `service.rs`'s
// `("workspace", "CloseTab")` handler with a reducer-driven saga.
// The legacy handler called `wcore::delete_tab` first and then
// dispatched `Command::DeleteTab` to keep the reducer in sync — a
// short-circuit that pre-dates the saga coordinator + persist
// subscriber pattern (closes gap §4 in
// `docs/retro/reducer-architecture-gaps-2026-05-01.md`).
//
// **Steps:**
// 1. `DeleteTab { workspace_id, tab_id }` — reducer removes the tab
//    from canonical state (cascading to its blocks in-state) and
//    emits `Event::TabDeleted` plus optional `Event::ActiveTabChanged`
//    if the removed tab was active. The persist subscriber writes
//    SQLite via `wcore::delete_tab` (cascades to blocks + layout +
//    PTY controllers).
//
// **Pre-conditions:**
// 1. Tab must exist in the reducer state and be in the named
//    workspace.
// 2. Tab must NOT be the workspace's last tab. Mirrors `TearOffTab`'s
//    "cannot tear off last tab" guard. Removing the last tab leaves
//    an empty workspace whose UI representation is awkward — callers
//    that want full-workspace teardown should issue
//    `DeleteWorkspace` (which is its own saga in Step 5 PR 2).
//
// **Block controller cascade:** the persist subscriber's
// `apply_tab_deleted` invokes `wcore::delete_tab` which calls
// `delete_tab_inner` → `delete_controller(block_id)` for each block.
// The saga therefore does not need to do explicit controller-kill
// like `delete_block.rs` does (DeleteBlock's persist path is
// `wcore::delete_block` which only handles the SQLite + layout
// prune, not controller teardown).
//
// **Compensation:** like DeleteBlock, un-deleting a tab requires
// reconstructing SQLite rows (Tab + LayoutState + cascaded Blocks)
// AND re-spawning the PTY controllers — neither feasible from saga
// state. Pragma per the brief: log warning on failure, no automatic
// re-create. PR 2's restart-recovery scan will surface partial
// failures from the durable saga log.
//
// **Pre-condition source — reducer vs SQLite:** check reducer state.
// The legacy CloseTab path was SQLite-first which made SQLite
// authoritative; the saga inverts this — the reducer's `tab_ids` is
// the source of truth, with the persist subscriber writing SQLite
// in response. Tabs that exist in SQLite but not in the reducer
// indicate a bootstrap-mismatch (separate bug); the saga rejecting
// them is the correct conservative behaviour.

use agentmux_common::ipc::Command;
use serde_json::{json, Value};

use super::{
    alloc_saga_id, classify_run_saga_result, emit_saga_started, emit_terminal, run_saga, SagaCtx,
};
use crate::server::AppState;

/// Run the DeleteTab saga. On success returns
/// `{"workspace_id": "...", "tab_id": "..."}`.
pub async fn run(
    state: &AppState,
    workspace_id: String,
    tab_id: String,
) -> Result<Value, String> {
    // Pre-conditions: tab exists in workspace; not the last tab.
    //
    // **Last-tab pre-check is best-effort.** A reducer-level guard
    // would be atomic but it broke legitimate CreateTab compensation
    // (codex P1 round 2 #633) and frontend keyboard flow (codex P1
    // round 1 #633). Round 3 walked back the reducer guard. The
    // saga keeps a soft pre-check here as a UX guard; the TOCTOU
    // window (two concurrent CloseTabs on different tabs in a 2-tab
    // workspace) is reachable in theory but the user-facing call
    // sites (tabbar close button at `tabbar.tsx:63`, keyboard handler
    // at `keymodel.ts::simpleCloseStaticTab`) gate with
    // `if (allTabs.length <= 1) return`, so user-driven concurrent
    // CloseTabs can't reach this saga. Automated test harnesses
    // could still race; document the limitation, accept it.
    {
        let s = state.srv_state.lock().await;
        let Some(workspace) = s.workspaces.get(&workspace_id) else {
            return Err(format!(
                "DeleteTab: workspace not found: {}",
                workspace_id
            ));
        };
        if !workspace.tab_ids.iter().any(|t| t == &tab_id) {
            return Err(format!(
                "DeleteTab: tab {} is not in workspace {}",
                tab_id, workspace_id
            ));
        }
        if workspace.tab_ids.len() <= 1 {
            return Err(format!(
                "DeleteTab: cannot delete last tab in workspace {}",
                workspace_id
            ));
        }
        if !s.tabs.contains_key(&tab_id) {
            // Inconsistent state — workspace.tab_ids references a
            // tab that's not in state.tabs. Reject; bootstrap-rebuild
            // gap is a separate concern.
            return Err(format!("DeleteTab: tab record missing: {}", tab_id));
        }
    }

    // Capture the tab's block_ids before dispatch — we'll use these
    // to clean up PTY controllers if the saga partially succeeds
    // (reducer dispatched, SQLite apply failed). (codex P2 PR #633
    // round 3.)
    let block_ids_to_cleanup: Vec<String> = {
        let s = state.srv_state.lock().await;
        s.tabs
            .get(&tab_id)
            .map(|t| t.block_ids.clone())
            .unwrap_or_default()
    };

    let saga_id = alloc_saga_id(state);
    if let Err(e) = emit_saga_started(
        state,
        saga_id,
        "delete_tab",
        json!({
            "workspace_id": &workspace_id,
            "tab_id": &tab_id,
        }),
    )
    .await
    {
        return Err(e);
    }
    let ctx = SagaCtx::new(state, saga_id);
    let result = run_saga("delete_tab", run_inner(ctx, workspace_id, tab_id.clone())).await;
    // If the tab is gone from reducer state, the reducer dispatched
    // (whether or not SQLite apply succeeded). Kill controllers for
    // every block that was in the tab. Idempotent on missing
    // controllers. Same pattern as `delete_block::run` (codex P2
    // round 2 + 3).
    {
        let tab_still_in_reducer = state.srv_state.lock().await.tabs.contains_key(&tab_id);
        if !tab_still_in_reducer {
            for block_id in &block_ids_to_cleanup {
                crate::backend::blockcontroller::delete_controller(block_id);
            }
        }
    }
    emit_terminal(state, saga_id, classify_run_saga_result(&result)).await;
    result
}

async fn run_inner(
    ctx: SagaCtx<'_>,
    workspace_id: String,
    tab_id: String,
) -> Result<Value, String> {
    // Step 1: dispatch DeleteTab through the reducer. The persist
    // subscriber sees TabDeleted (and optional ActiveTabChanged) and
    // runs `wcore::delete_tab` which cascades to blocks + layout +
    // PTY controllers.
    if let Err(reason) = ctx
        .dispatch(Command::DeleteTab {
            workspace_id: workspace_id.clone(),
            tab_id: tab_id.clone(),
        })
        .await
    {
        // No automatic compensation. See module doc-comment for
        // rationale.
        tracing::warn!(
            workspace_id = %workspace_id,
            tab_id = %tab_id,
            "[saga] DeleteTab dispatch failed (no automatic compensation): {}",
            reason
        );
        return Err(format!("DeleteTab: {}", reason));
    }

    Ok(json!({
        "workspace_id": workspace_id,
        "tab_id": tab_id,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::obj::{Tab, Workspace};
    use crate::server::tests::test_state;
    use agentmux_common::ipc::Event;

    async fn dispatch_apply(
        state: &crate::server::AppState,
        cmd: agentmux_common::ipc::Command,
    ) -> Vec<agentmux_common::ipc::Event> {
        let events = crate::server::service::dispatch_to_reducer(state, cmd).await;
        for ev in &events {
            crate::persist_subscriber::apply_event_to_wstore(ev, &state.wstore).unwrap();
        }
        events
    }

    /// Seed: a workspace with two tabs (last-tab guard requires >1).
    /// Returns `(state, ws_id, tab_a_id, tab_b_id)`.
    async fn seed_workspace_with_two_tabs() -> (
        crate::server::AppState,
        String,
        String,
        String,
    ) {
        let state = test_state();
        let ws_evs = dispatch_apply(
            &state,
            agentmux_common::ipc::Command::CreateWorkspace { name: "w".into() },
        )
        .await;
        let ws_id = ws_evs
            .iter()
            .find_map(|e| match e {
                Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
                _ => None,
            })
            .unwrap();
        let mut tab_ids = Vec::new();
        for name in &["tab-a", "tab-b"] {
            let tab_evs = dispatch_apply(
                &state,
                agentmux_common::ipc::Command::CreateTab {
                    workspace_id: ws_id.clone(),
                    name: name.to_string(),
                },
            )
            .await;
            tab_ids.push(
                tab_evs
                    .iter()
                    .find_map(|e| match e {
                        Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
                        _ => None,
                    })
                    .unwrap(),
            );
        }
        (state, ws_id, tab_ids[0].clone(), tab_ids[1].clone())
    }

    #[tokio::test]
    async fn happy_path_removes_tab_from_reducer_and_sqlite() {
        let (state, ws_id, tab_a, tab_b) = seed_workspace_with_two_tabs().await;

        // Sanity: both tabs present pre-delete.
        {
            let s = state.srv_state.lock().await;
            assert_eq!(s.workspaces[&ws_id].tab_ids, vec![tab_a.clone(), tab_b.clone()]);
            assert!(s.tabs.contains_key(&tab_a));
        }
        assert!(state.wstore.get::<Tab>(&tab_a).unwrap().is_some());

        let result = run(&state, ws_id.clone(), tab_a.clone()).await.unwrap();
        assert_eq!(result["tab_id"], tab_a);
        assert_eq!(result["workspace_id"], ws_id);

        // Reducer: tab_a gone; workspace.tab_ids has only tab_b.
        let s = state.srv_state.lock().await;
        assert!(!s.tabs.contains_key(&tab_a));
        assert_eq!(s.workspaces[&ws_id].tab_ids, vec![tab_b.clone()]);
        drop(s);

        // SQLite: tab gone; workspace.tabids reflects.
        assert!(state.wstore.get::<Tab>(&tab_a).unwrap().is_none());
        let ws_persist = state.wstore.get::<Workspace>(&ws_id).unwrap().unwrap();
        assert_eq!(ws_persist.tabids, vec![tab_b]);
    }

    #[tokio::test]
    async fn rejects_when_workspace_not_found() {
        let state = test_state();
        let err = run(&state, "ghost-ws".into(), "ghost-tab".into())
            .await
            .unwrap_err();
        assert!(err.contains("workspace not found"), "got: {}", err);
    }

    #[tokio::test]
    async fn rejects_when_tab_not_in_workspace() {
        let (state, ws_id, _tab_a, _tab_b) = seed_workspace_with_two_tabs().await;
        let err = run(&state, ws_id, "ghost-tab".into()).await.unwrap_err();
        assert!(err.contains("not in workspace"), "got: {}", err);
    }

    #[tokio::test]
    async fn rejects_last_tab() {
        let state = test_state();
        let ws_evs = dispatch_apply(
            &state,
            agentmux_common::ipc::Command::CreateWorkspace { name: "w".into() },
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
                name: "only".into(),
            },
        )
        .await;
        let only_tab = tab_evs
            .iter()
            .find_map(|e| match e {
                Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
                _ => None,
            })
            .unwrap();
        let err = run(&state, ws_id, only_tab).await.unwrap_err();
        assert!(err.contains("last tab"), "got: {}", err);
    }
}
