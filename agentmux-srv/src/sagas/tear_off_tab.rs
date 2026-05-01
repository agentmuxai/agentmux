// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase E.5.5 — TearOffTab saga.
//
// Migrates the tear-off-tab flow from `wcore::tear_off_tab` (which
// did two non-transactional SQLite writes outside the reducer) to a
// reducer-driven multi-step that the persist subscriber writes to
// SQLite event-by-event. Closes the smoke regression where the new
// workspace existed in SQLite but the reducer didn't know about it,
// so the new window's `CreateTab` call would fail.
//
// **Steps (mirrors `wcore::tear_off_tab`'s behaviour, NOT the
// `SPEC_PHASE_E_SAGAS_2026-04-30.md` §6.1 step 3 `CreateWindow`):**
//
// 1. `CreateWorkspace { name: "" }` — empty-name matches wcore's
//    behaviour; user renames after.
// 2. `MoveTab { tab_id, src=source_ws, dst=new_ws, dst_index: 0 }`.
//
// The frontend separately calls the host's `tear_off_pool_promote`
// to open the CEF window, then the new window's renderer registers
// the (window_id, workspace_id) mapping via `WindowService.CreateWindow`.
// Including `Command::CreateWindow` in the saga (per spec §6.1) is
// deferred — it would require pre-assigning the window_id before
// the CEF window opens, which the existing host pool-promote and
// app-init.ts flow doesn't accommodate. See
// `docs/retro/saga-coordinator-location-analysis-2026-04-30.md`
// §4.2 — host CEF window creation stays outside the saga.
//
// **Pre-condition:** the source workspace has more than one tab.
// `wcore::tear_off_tab` enforced this; the reducer's `MoveTab`
// doesn't (intentionally — the same command supports legitimate
// "drain workspace" flows). The saga checks before issuing any
// commands so the user-visible error is clear.
//
// **Compensation:**
// * Step 2 fails after step 1 succeeded → `DeleteWorkspace { new_ws_id }`
//   (the reducer cascades any tabs in it; the new workspace will be
//   empty here since step 2 failed before moving the tab).
// * Step 1 fails → nothing to compensate (reducer rejected without
//   mutating).

use agentmux_common::ipc::{Command, Event};
use serde_json::{json, Value};

use super::{
    alloc_saga_id, classify_run_saga_result, emit_saga_started, emit_terminal, run_saga, SagaCtx,
};
use crate::server::AppState;

/// Run the TearOffTab saga. On success, returns
/// `{"new_workspace_id": "..."}`. On failure, the source workspace
/// is unchanged (compensation has reverted any partial state).
pub async fn run(
    state: &AppState,
    tab_id: String,
    source_workspace_id: String,
) -> Result<Value, String> {
    // Pre-condition: source workspace must have more than one tab.
    // We're about to move one out; if it's the only tab, we'd leave
    // an empty workspace behind, which the UI doesn't represent
    // gracefully. Mirrors `wcore::tear_off_tab`'s "cannot tear off
    // last tab" guard.
    //
    // **Read SQLite, not reducer state.** During the migration
    // window, some tab-creating/moving paths (e.g.
    // `PromoteBlockToTab`) are still wcore-direct — their writes
    // don't flow through the reducer, so `state.workspaces[ws].tab_ids`
    // can lag SQLite. A reducer-state pre-check would falsely reject
    // valid tear-off requests (codex P1 round-2 #621). SQLite is the
    // source of truth; check there. Also include `pinnedtabids` in
    // the membership check — bootstrap merges them into the reducer's
    // `tab_ids`, but legacy SQLite rows may still carry the entry.
    {
        let src_ws = match state.wstore.get::<crate::backend::obj::Workspace>(&source_workspace_id) {
            Ok(Some(ws)) => ws,
            Ok(None) => {
                return Err(format!(
                    "TearOffTab: source workspace not found: {}",
                    source_workspace_id
                ));
            }
            Err(e) => return Err(format!("TearOffTab: workspace read failed: {}", e)),
        };
        let in_workspace = src_ws.tabids.iter().any(|id| id == &tab_id)
            || src_ws.pinnedtabids.iter().any(|id| id == &tab_id);
        if !in_workspace {
            return Err(format!(
                "TearOffTab: tab {} is not in workspace {}",
                tab_id, source_workspace_id
            ));
        }
        let total_tabs = src_ws.tabids.len() + src_ws.pinnedtabids.len();
        if total_tabs <= 1 {
            return Err(format!(
                "TearOffTab: cannot tear off last tab from workspace {}",
                source_workspace_id
            ));
        }
    }

    let saga_id = alloc_saga_id(state);
    emit_saga_started(
        state,
        saga_id,
        "tear_off_tab",
        serde_json::json!({
            "tab_id": &tab_id,
            "source_workspace_id": &source_workspace_id,
        }),
    )
    .await;
    let ctx = SagaCtx::new(state, saga_id);
    let result = run_saga("tear_off_tab", run_inner(ctx, tab_id, source_workspace_id)).await;
    emit_terminal(state, saga_id, classify_run_saga_result(&result)).await;
    result
}

async fn run_inner(
    ctx: SagaCtx<'_>,
    tab_id: String,
    source_workspace_id: String,
) -> Result<Value, String> {
    // Step 1: create the new workspace.
    let create_events = ctx
        .dispatch(Command::CreateWorkspace {
            name: String::new(),
        })
        .await
        .map_err(|e| format!("TearOffTab step 1 (CreateWorkspace): {}", e))?;
    let new_workspace_id = create_events
        .iter()
        .find_map(|e| match e {
            Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
            _ => None,
        })
        .ok_or_else(|| {
            "TearOffTab: CreateWorkspace did not emit WorkspaceCreated".to_string()
        })?;

    // Step 2: move the tab.
    if let Err(reason) = ctx
        .dispatch(Command::MoveTab {
            tab_id: tab_id.clone(),
            src_workspace_id: source_workspace_id.clone(),
            dst_workspace_id: new_workspace_id.clone(),
            dst_index: 0,
        })
        .await
    {
        // Compensate: delete the empty workspace we just created.
        ctx.compensate(Command::DeleteWorkspace {
            workspace_id: new_workspace_id.clone(),
        })
        .await;
        return Err(format!("TearOffTab step 2 (MoveTab): {}", reason));
    }

    Ok(json!({ "new_workspace_id": new_workspace_id }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::obj::Workspace;
    use crate::server::tests::test_state;

    /// Boot a workspace + 2 tabs in the reducer + SQLite, mirroring
    /// what bootstrap would do at startup. Returns `(state, ws_id,
    /// tab_a_id, tab_b_id)`.
    async fn seed_workspace_with_two_tabs() -> (
        crate::server::AppState,
        String,
        String,
        String,
    ) {
        let state = test_state();
        // Use the reducer to create a workspace + 2 tabs so they end
        // up in both reducer state and SQLite (the saga's pre-condition
        // checks read reducer state).
        let ws_events = crate::server::service::dispatch_to_reducer(
            &state,
            agentmux_common::ipc::Command::CreateWorkspace { name: "src".into() },
        )
        .await;
        for ev in &ws_events {
            crate::persist_subscriber::apply_event_to_wstore(ev, &state.wstore).unwrap();
        }
        let ws_id = ws_events
            .iter()
            .find_map(|e| match e {
                Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
                _ => None,
            })
            .unwrap();
        let mut tab_ids = Vec::new();
        for name in &["tab-a", "tab-b"] {
            let tab_events = crate::server::service::dispatch_to_reducer(
                &state,
                agentmux_common::ipc::Command::CreateTab {
                    workspace_id: ws_id.clone(),
                    name: name.to_string(),
                },
            )
            .await;
            for ev in &tab_events {
                crate::persist_subscriber::apply_event_to_wstore(ev, &state.wstore).unwrap();
            }
            tab_ids.push(
                tab_events
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
    async fn happy_path_creates_new_workspace_and_moves_tab() {
        let (state, src_ws, tab_a, tab_b) = seed_workspace_with_two_tabs().await;
        let result = run(&state, tab_a.clone(), src_ws.clone()).await.unwrap();
        let new_ws_id = result["new_workspace_id"].as_str().unwrap();

        // Reducer view: src has just tab_b; new_ws has tab_a.
        let s = state.srv_state.lock().await;
        assert_eq!(s.workspaces[&src_ws].tab_ids, vec![tab_b.clone()]);
        assert_eq!(s.workspaces[new_ws_id].tab_ids, vec![tab_a.clone()]);
        assert_eq!(s.tabs[&tab_a].workspace_id, new_ws_id);

        // SQLite view: same.
        let src_persist = state.wstore.get::<Workspace>(&src_ws).unwrap().unwrap();
        let new_persist = state.wstore.get::<Workspace>(new_ws_id).unwrap().unwrap();
        assert_eq!(src_persist.tabids, vec![tab_b]);
        assert_eq!(new_persist.tabids, vec![tab_a]);
    }

    #[tokio::test]
    async fn rejects_when_source_workspace_missing() {
        let state = test_state();
        let err = run(&state, "tab-1".into(), "no-such-ws".into()).await.unwrap_err();
        assert!(err.contains("source workspace not found"), "got: {}", err);
    }

    #[tokio::test]
    async fn rejects_last_tab() {
        let state = test_state();
        let ws_events = crate::server::service::dispatch_to_reducer(
            &state,
            agentmux_common::ipc::Command::CreateWorkspace { name: "src".into() },
        )
        .await;
        for ev in &ws_events {
            crate::persist_subscriber::apply_event_to_wstore(ev, &state.wstore).unwrap();
        }
        let ws_id = ws_events
            .iter()
            .find_map(|e| match e {
                Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
                _ => None,
            })
            .unwrap();
        let tab_events = crate::server::service::dispatch_to_reducer(
            &state,
            agentmux_common::ipc::Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: "only".into(),
            },
        )
        .await;
        for ev in &tab_events {
            crate::persist_subscriber::apply_event_to_wstore(ev, &state.wstore).unwrap();
        }
        let only_tab = tab_events
            .iter()
            .find_map(|e| match e {
                Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
                _ => None,
            })
            .unwrap();
        let err = run(&state, only_tab, ws_id).await.unwrap_err();
        assert!(err.contains("last tab"), "got: {}", err);
    }
}
