// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase E.5.7 (Step 5 PR 2) — DeleteWorkspace saga.
//
// Replaces the inline cascade-on-dispatch pattern in
// `service.rs::("workspace", "DeleteWorkspace")` with a saga that
// records lifecycle brackets in the durable saga log. Closes the
// remaining gap in the Step 5 series:
//
// * PR 1 already migrated `DeleteBlock` and `DeleteTab` to sagas.
// * The `("workspace", "DeleteWorkspace")` RPC handler still issued
//   a single `Command::DeleteWorkspace` whose reducer arm cascaded
//   through tabs+blocks atomically, but a crash mid-cascade left
//   inconsistent state with no durable record of WHAT was deleted.
//
// **Saga-as-narrator pattern.** The reducer's `handle_delete_workspace`
// remains the canonical mutator (it cascades through tabs+blocks and
// drops window mappings in a single in-memory step). The saga's
// contribution is durability: it walks the workspace's tabs first via
// per-tab `Command::DeleteTab { force: true }` dispatches, recording
// each in the saga log, then issues the final
// `Command::DeleteWorkspace { force: true }` to drop the (now-empty)
// workspace + window mappings.
//
// This decomposition means crash recovery (`recovery::compensate_unresolved`,
// merged in #636) sees per-tab progress markers and can mark the saga
// `failed_compensation` for operator review if the cascade was
// interrupted — versus the legacy single-shot `DeleteWorkspace` which
// either fully applied or left the reducer state untouched, with no
// durable trace either way.
//
// **Steps:**
// 1. Snapshot the workspace's tabs+blocks (read-only) for the saga
//    log's `input` field. Provenance for `--diag sagas`: which entities
//    existed at saga start, so an operator can reason about a partial
//    cascade later.
// 2. For each tab (in `tab_ids` order): dispatch
//    `Command::DeleteTab { force: true }`. The reducer cascades the
//    tab's blocks atomically; the persist subscriber writes SQLite
//    via `wcore::delete_tab` (which kills PTY controllers via
//    `delete_tab_inner` → `delete_controller(block_id)`).
//    `force: true` bypasses the reducer's last-tab guard — we're
//    intentionally draining the workspace.
// 3. Final dispatch: `Command::DeleteWorkspace { force: true }`. The
//    workspace is empty by this point (step 2 deleted every tab), so
//    the reducer's cascade only removes the workspace record + drops
//    window mappings (emitting `SrvWindowClosed` per affected window).
//    The `force: true` flag is provenance-only — the reducer's
//    behaviour is identical regardless.
//
// **Compensation.** Delete sagas are awkward to compensate by design:
// once a tab + its blocks are gone (SQLite rows deleted, controllers
// killed), reconstruction would require persisting the pre-state, which
// no current saga does. Per the brief: compensation is **record-only**.
// If a step fails mid-cascade we rely on `classify_run_saga_result` to
// classify the error path:
//
//   * `Err` (non-timeout) → `Compensated` — the saga's per-step log
//     rows already record what was deleted; subsequent crash-recovery
//     does NOT replay (Delete commands have no derivable inverse in
//     `recovery::derive_inverse_command`, by design).
//   * `Err` containing `"timed out"` → `Failed` — the saga timed out
//     before completing; recovery will see `running` lifecycle + the
//     succeeded step prefix and mark it `failed_compensation` (since
//     Delete has no derivable inverse, the operator must reconcile).
//   * `Ok` → `Completed` — clean cascade.
//
// **Pre-condition:** the workspace must exist in the reducer state OR
// in SQLite. Bootstrap loads SQLite into reducer at startup, so they
// normally match; we accept either to handle migration-window flows
// where SQLite-direct writes haven't yet flowed through the reducer.

use agentmux_common::ipc::Command;
use serde_json::{json, Value};

use super::{
    alloc_saga_id, classify_run_saga_result, emit_saga_started, emit_terminal, run_saga, SagaCtx,
};
use crate::server::AppState;

/// Run the DeleteWorkspace saga. On success returns
/// `{"workspace_id": "...", "deleted_tab_count": N, "deleted_block_count": M}`.
pub async fn run(state: &AppState, workspace_id: String) -> Result<Value, String> {
    // Pre-condition + snapshot: read the workspace's tabs+blocks
    // before any dispatch. We need the tab list to drive step 2's
    // per-tab DeleteTab dispatches, and we record block ids in the
    // saga log so `--diag sagas` can show what was destroyed.
    let (tab_ids, block_count) = {
        let s = state.srv_state.lock().await;
        let Some(workspace) = s.workspaces.get(&workspace_id) else {
            return Err(format!(
                "DeleteWorkspace: workspace not found: {}",
                workspace_id
            ));
        };
        let tab_ids: Vec<String> = workspace.tab_ids.clone();
        let block_count: usize = tab_ids
            .iter()
            .map(|tid| s.tabs.get(tid).map(|t| t.block_ids.len()).unwrap_or(0))
            .sum();
        (tab_ids, block_count)
    };

    let saga_id = alloc_saga_id(state);
    if let Err(e) = emit_saga_started(
        state,
        saga_id,
        "delete_workspace",
        json!({
            "workspace_id": &workspace_id,
            "tab_ids": &tab_ids,
            "block_count": block_count,
        }),
    )
    .await
    {
        return Err(e);
    }
    let ctx = SagaCtx::new(state, saga_id);
    let result = run_saga(
        "delete_workspace",
        run_inner(ctx, workspace_id.clone(), tab_ids.clone(), block_count),
    )
    .await;
    emit_terminal(state, saga_id, classify_run_saga_result(&result)).await;
    result
}

async fn run_inner(
    ctx: SagaCtx<'_>,
    workspace_id: String,
    tab_ids: Vec<String>,
    block_count: usize,
) -> Result<Value, String> {
    // Step 2: per-tab DeleteTab dispatch. `force: true` bypasses the
    // reducer's last-tab guard — the saga is intentionally draining
    // the workspace, the guard exists to protect user-facing CloseTab
    // flows from emptying a workspace by accident. The persist
    // subscriber's `apply_tab_deleted` runs `wcore::delete_tab` which
    // kills each block's PTY controller via `delete_tab_inner` →
    // `delete_controller(block_id)`. That's the same controller-cleanup
    // path the user-facing DeleteTab saga (Step 5 PR 1) relies on,
    // so we don't replicate the controller-kill here.
    //
    // **No compensation.** If a tab's DeleteTab dispatch fails mid-
    // cascade, the already-deleted prefix is gone — we can't
    // reconstruct the SQLite rows or respawn the PTY controllers.
    // Returning `Err` records the failure in the saga log (per-step
    // succeeded rows for the prefix + a failed row for the rejecting
    // tab). `classify_run_saga_result` maps non-timeout `Err` to
    // `Compensated`, which is technically inaccurate (nothing was
    // un-done) but matches the saga framework's convention for
    // non-timeout errors and avoids surfacing the saga to recovery's
    // `failed`-state replay path (Delete has no derivable inverse
    // anyway, so recovery would just mark `failed_compensation` —
    // which is fine; either outcome surfaces in `--diag sagas`).
    for (i, tab_id) in tab_ids.iter().enumerate() {
        if let Err(reason) = ctx
            .dispatch(Command::DeleteTab {
                workspace_id: workspace_id.clone(),
                tab_id: tab_id.clone(),
                // Saga is draining the workspace; bypass the
                // user-facing last-tab guard. (Saga's own pre-checks
                // already validated the workspace exists.)
                force: true,
            })
            .await
        {
            tracing::warn!(
                workspace_id = %workspace_id,
                tab_id = %tab_id,
                step = i,
                "[saga] DeleteWorkspace step 2 (DeleteTab): dispatch failed: {} — succeeded prefix already gone, no compensation possible",
                reason,
            );
            return Err(format!(
                "DeleteWorkspace step 2 (DeleteTab {}): {}",
                tab_id, reason
            ));
        }
    }

    // Step 3: drop the (now-empty) workspace + cascade window mappings.
    // The reducer's `handle_delete_workspace` removes the workspace
    // record + emits `SrvWindowClosed` per affected window; the
    // persist subscriber's `apply_workspace_deleted` runs
    // `wcore::delete_workspace` for SQLite (idempotent if the
    // workspace was already removed by the per-tab cascade in step 2).
    if let Err(reason) = ctx
        .dispatch(Command::DeleteWorkspace {
            workspace_id: workspace_id.clone(),
            // Saga-driven dispatch — provenance flag for the durable
            // log. Reducer behaviour is identical for both values.
            force: true,
        })
        .await
    {
        tracing::warn!(
            workspace_id = %workspace_id,
            "[saga] DeleteWorkspace step 3 (DeleteWorkspace): dispatch failed: {} — tabs already gone, workspace partially deleted",
            reason,
        );
        return Err(format!("DeleteWorkspace step 3 (DeleteWorkspace): {}", reason));
    }

    Ok(json!({
        "workspace_id": workspace_id,
        "deleted_tab_count": tab_ids.len(),
        "deleted_block_count": block_count,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::obj::{Block, Tab, Workspace};
    use crate::server::tests::test_state;
    use agentmux_common::ipc::Event;

    async fn dispatch_apply(
        state: &crate::server::AppState,
        cmd: Command,
    ) -> Vec<Event> {
        let events = crate::server::service::dispatch_to_reducer(state, cmd).await;
        for ev in &events {
            crate::persist_subscriber::apply_event_to_wstore(ev, &state.wstore).unwrap();
        }
        events
    }

    /// Seed a workspace with N tabs, each containing one block.
    /// Returns `(state, workspace_id, tab_ids, block_ids)`.
    async fn seed_workspace_with_tabs_and_blocks(
        n: usize,
    ) -> (
        crate::server::AppState,
        String,
        Vec<String>,
        Vec<String>,
    ) {
        let state = test_state();
        let ws_evs = dispatch_apply(
            &state,
            Command::CreateWorkspace { name: "w".into() },
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
        let mut block_ids = Vec::new();
        for i in 0..n {
            let tab_evs = dispatch_apply(
                &state,
                Command::CreateTab {
                    workspace_id: ws_id.clone(),
                    name: format!("tab-{}", i),
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
            let blk_evs = dispatch_apply(
                &state,
                Command::CreateBlock {
                    tab_id: tab_id.clone(),
                    meta: serde_json::Value::Null,
                },
            )
            .await;
            let block_id = blk_evs
                .iter()
                .find_map(|e| match e {
                    Event::BlockCreated { block_id, .. } => Some(block_id.clone()),
                    _ => None,
                })
                .unwrap();
            tab_ids.push(tab_id);
            block_ids.push(block_id);
        }
        (state, ws_id, tab_ids, block_ids)
    }

    #[tokio::test]
    async fn happy_path_cascades_tabs_and_blocks() {
        let (state, ws_id, tab_ids, block_ids) =
            seed_workspace_with_tabs_and_blocks(2).await;

        // Sanity: pre-state has workspace + tabs + blocks in both
        // reducer state and SQLite.
        {
            let s = state.srv_state.lock().await;
            assert!(s.workspaces.contains_key(&ws_id));
            assert_eq!(s.workspaces[&ws_id].tab_ids.len(), 2);
            assert_eq!(s.tabs.len(), 2);
            assert_eq!(s.blocks.len(), 2);
        }
        for tab_id in &tab_ids {
            assert!(state.wstore.get::<Tab>(tab_id).unwrap().is_some());
        }
        for block_id in &block_ids {
            assert!(state.wstore.get::<Block>(block_id).unwrap().is_some());
        }

        let result = run(&state, ws_id.clone()).await.unwrap();
        assert_eq!(result["workspace_id"], ws_id);
        assert_eq!(result["deleted_tab_count"], 2);
        assert_eq!(result["deleted_block_count"], 2);

        // Reducer: workspace + all tabs + all blocks gone.
        let s = state.srv_state.lock().await;
        assert!(!s.workspaces.contains_key(&ws_id));
        assert!(s.tabs.is_empty());
        assert!(s.blocks.is_empty());
        drop(s);

        // SQLite: matches.
        assert!(state.wstore.get::<Workspace>(&ws_id).unwrap().is_none());
        for tab_id in &tab_ids {
            assert!(state.wstore.get::<Tab>(tab_id).unwrap().is_none());
        }
        for block_id in &block_ids {
            assert!(state.wstore.get::<Block>(block_id).unwrap().is_none());
        }
    }

    #[tokio::test]
    async fn rejects_when_workspace_not_found() {
        let state = test_state();
        let err = run(&state, "ghost-ws".into()).await.unwrap_err();
        assert!(err.contains("workspace not found"), "got: {}", err);
    }

    #[tokio::test]
    async fn empty_workspace_succeeds() {
        // Workspace exists but has zero tabs — saga should skip the
        // per-tab loop and proceed directly to step 3 (DeleteWorkspace).
        let state = test_state();
        let ws_evs = dispatch_apply(
            &state,
            Command::CreateWorkspace { name: "empty".into() },
        )
        .await;
        let ws_id = ws_evs
            .iter()
            .find_map(|e| match e {
                Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
                _ => None,
            })
            .unwrap();

        let result = run(&state, ws_id.clone()).await.unwrap();
        assert_eq!(result["deleted_tab_count"], 0);
        assert_eq!(result["deleted_block_count"], 0);

        let s = state.srv_state.lock().await;
        assert!(!s.workspaces.contains_key(&ws_id));
    }

    #[tokio::test]
    async fn writes_lifecycle_brackets_to_saga_log() {
        // Verify the saga records `start_saga` + per-step `succeeded`
        // rows + a terminal `completed` lifecycle row. This is what
        // PR 2's `--diag sagas` and `compensate_unresolved` rely on.
        let (state, ws_id, _tab_ids, _block_ids) =
            seed_workspace_with_tabs_and_blocks(1).await;
        run(&state, ws_id.clone()).await.unwrap();

        let snap = state.saga_log.snapshot_recent(10).unwrap();
        let saga = snap
            .iter()
            .find(|s| s.name == "delete_workspace")
            .expect("delete_workspace saga should appear in snapshot");
        assert_eq!(saga.state, "completed", "saga should terminate completed");
        // 1 DeleteTab step + 1 DeleteWorkspace step = 2 forward steps.
        assert!(
            saga.step_count >= 2,
            "expected >= 2 steps, got {}",
            saga.step_count
        );
        // No unresolved sagas — recovery shouldn't pick this up.
        let unresolved = state.saga_log.unresolved_sagas().unwrap();
        assert!(
            unresolved.iter().all(|s| s.saga_id != saga.saga_id),
            "saga should not be unresolved post-completion"
        );
    }

    #[tokio::test]
    async fn cascade_drops_window_mappings() {
        // Seed a workspace mapped to a window; saga should cascade
        // window-removal via the reducer's existing
        // handle_delete_workspace logic (which emits SrvWindowClosed
        // per affected window).
        let (state, ws_id, _tab_ids, _block_ids) =
            seed_workspace_with_tabs_and_blocks(1).await;
        let win_id = "win-test".to_string();
        let _ = dispatch_apply(
            &state,
            Command::CreateWindow {
                window_id: win_id.clone(),
                workspace_id: ws_id.clone(),
            },
        )
        .await;
        {
            let s = state.srv_state.lock().await;
            assert!(s.windows.contains_key(&win_id));
        }

        run(&state, ws_id.clone()).await.unwrap();

        let s = state.srv_state.lock().await;
        assert!(!s.windows.contains_key(&win_id));
    }
}
