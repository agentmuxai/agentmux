// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase E.5.7 (Step 5 PR 1) — DeleteBlock saga.
//
// Replaces the SQLite-first delete pattern in `service.rs`'s
// `("object", "DeleteBlock")` handler with a reducer-driven saga.
// The legacy handler called `wcore::delete_block` first and then
// dispatched `Command::DeleteBlock` to keep the reducer in sync — a
// short-circuit that pre-dates the saga coordinator + persist
// subscriber pattern (closes gap §4 in
// `docs/retro/reducer-architecture-gaps-2026-05-01.md`).
//
// **Steps:**
// 1. `DeleteBlock { tab_id, block_id }` — reducer removes the block
//    from canonical state and emits `Event::BlockDeleted`. The
//    persist subscriber writes SQLite via `wcore::delete_block`
//    (cascades to layout pruning).
//
// **Block controller side-effect:** the legacy RPC handler killed
// the block's PTY/controller via `blockcontroller::delete_controller`
// BEFORE the wcore SQLite delete. The saga preserves that ordering
// — controller-kill happens in this function before the reducer
// dispatch, since the persist subscriber's `wcore::delete_block`
// only handles SQLite and layout pruning, not process teardown. We
// still drop the controller even if the saga later short-circuits
// (block-not-found): the controller registry is a process-local
// map, idempotent on missing keys.
//
// **Compensation:** delete sagas are awkward to compensate — once
// the block row + controller are gone, "un-delete" requires
// reconstructing both the SQLite row and the PTY/process subtree,
// neither of which is meaningful from saga state. We follow the
// brief's pragma: log a warning on dispatch failure, no automatic
// re-create. The reducer's `DeleteBlock` is silent-no-op on missing
// inputs (see reducer.rs handle_delete_block), so the only failure
// path is wstore write errors surfaced by the persist subscriber —
// in which case the controller is already gone (intentional; the
// PTY can't be partially-killed) and the SQLite row may or may not
// have been written. PR 2's `compensate_unresolved` resume scan
// surfaces these via the durable saga log for operator follow-up.
//
// **Pre-condition:** the block must exist in the reducer state.
// Without this, the reducer would silently no-op (handle_delete_block
// returns an empty event vec on missing tab/block) and the user
// would see "delete succeeded" while nothing happened. The saga
// surfaces a clear "block not found" error instead. We check the
// reducer state (not SQLite) because `Command::DeleteBlock` carries
// `tab_id` from the RPC's `uicontext.activetabid` and we want to
// validate against the reducer's view of (tab → blocks) — that's
// what the dispatch will mutate.

use agentmux_common::ipc::Command;
use serde_json::{json, Value};

use super::{
    alloc_saga_id, classify_run_saga_result, emit_saga_started, emit_terminal, run_saga, SagaCtx,
};
use crate::server::AppState;

/// Run the DeleteBlock saga. On success returns
/// `{"block_id": "...", "tab_id": "..."}`.
pub async fn run(
    state: &AppState,
    tab_id: String,
    block_id: String,
) -> Result<Value, String> {
    // Pre-condition: block exists and is in the named tab. Reducer
    // would silent-no-op otherwise (see handle_delete_block); the
    // saga surfaces a clear error instead.
    {
        let s = state.srv_state.lock().await;
        match s.blocks.get(&block_id) {
            None => {
                return Err(format!("DeleteBlock: block not found: {}", block_id));
            }
            Some(block) if block.tab_id != tab_id => {
                return Err(format!(
                    "DeleteBlock: block {} is in tab {}, not {}",
                    block_id, block.tab_id, tab_id
                ));
            }
            _ => {}
        }
        if !s.tabs.contains_key(&tab_id) {
            return Err(format!("DeleteBlock: tab not found: {}", tab_id));
        }
    }

    let saga_id = alloc_saga_id(state);
    if let Err(e) = emit_saga_started(
        state,
        saga_id,
        "delete_block",
        json!({
            "tab_id": &tab_id,
            "block_id": &block_id,
        }),
    )
    .await
    {
        return Err(e);
    }
    let ctx = SagaCtx::new(state, saga_id);
    let result = run_saga("delete_block", run_inner(ctx, tab_id, block_id.clone())).await;
    // Kill the controller AFTER the saga's reducer + SQLite writes
    // succeed. (reagent P2 PR #633.) The earlier round killed the
    // controller before emit_saga_started, which left an
    // unrecoverable side-effect leak if start_saga rejected a
    // collision: PTY dead, block still in reducer + SQLite. Doing
    // it here means: success → controller gone, block gone, all
    // consistent; saga failure → controller still alive (we don't
    // touch it), block stays — also consistent. Idempotent on
    // missing controller.
    if result.is_ok() {
        crate::backend::blockcontroller::delete_controller(&block_id);
    }
    emit_terminal(state, saga_id, classify_run_saga_result(&result)).await;
    result
}

async fn run_inner(
    ctx: SagaCtx<'_>,
    tab_id: String,
    block_id: String,
) -> Result<Value, String> {
    // Step 1: dispatch DeleteBlock through the reducer. The persist
    // subscriber sees the BlockDeleted event and runs
    // `wcore::delete_block` (SQLite delete + layout prune).
    if let Err(reason) = ctx
        .dispatch(Command::DeleteBlock {
            tab_id: tab_id.clone(),
            block_id: block_id.clone(),
        })
        .await
    {
        // No automatic compensation — un-deleting a block requires
        // reconstructing both SQLite + PTY which we cannot do from
        // saga state. Surface the failure; PR 2's restart-recovery
        // scan picks up the durable log row for operator review.
        tracing::warn!(
            tab_id = %tab_id,
            block_id = %block_id,
            "[saga] DeleteBlock dispatch failed (no automatic compensation): {}",
            reason
        );
        return Err(format!("DeleteBlock: {}", reason));
    }

    Ok(json!({
        "tab_id": tab_id,
        "block_id": block_id,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::obj::Block;
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

    /// Seed a workspace + tab + block and return their ids.
    async fn seed() -> (
        crate::server::AppState,
        String, // workspace_id
        String, // tab_id
        String, // block_id
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
        let tab_evs = dispatch_apply(
            &state,
            agentmux_common::ipc::Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: "t".into(),
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
            agentmux_common::ipc::Command::CreateBlock {
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
        (state, ws_id, tab_id, block_id)
    }

    #[tokio::test]
    async fn happy_path_removes_block_from_reducer_and_sqlite() {
        let (state, _ws_id, tab_id, block_id) = seed().await;

        // Sanity: block is present pre-delete.
        {
            let s = state.srv_state.lock().await;
            assert!(s.blocks.contains_key(&block_id));
            assert_eq!(s.tabs[&tab_id].block_ids, vec![block_id.clone()]);
        }
        assert!(state.wstore.get::<Block>(&block_id).unwrap().is_some());

        let result = run(&state, tab_id.clone(), block_id.clone()).await.unwrap();
        assert_eq!(result["block_id"], block_id);
        assert_eq!(result["tab_id"], tab_id);

        // Reducer: block gone, tab's block_ids empty.
        let s = state.srv_state.lock().await;
        assert!(!s.blocks.contains_key(&block_id));
        assert!(s.tabs[&tab_id].block_ids.is_empty());
        drop(s);

        // SQLite: block gone.
        assert!(state.wstore.get::<Block>(&block_id).unwrap().is_none());
    }

    #[tokio::test]
    async fn rejects_when_block_not_found() {
        let (state, _ws_id, tab_id, _block_id) = seed().await;
        let err = run(&state, tab_id, "ghost-block".into()).await.unwrap_err();
        assert!(err.contains("block not found"), "got: {}", err);
    }

    #[tokio::test]
    async fn rejects_when_block_in_different_tab() {
        let (state, _ws_id, tab_id, block_id) = seed().await;
        // Create a second tab; ask to delete block via that tab's id.
        let tab_evs = dispatch_apply(
            &state,
            agentmux_common::ipc::Command::CreateTab {
                workspace_id: _ws_id.clone(),
                name: "other".into(),
            },
        )
        .await;
        let other_tab = tab_evs
            .iter()
            .find_map(|e| match e {
                Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
                _ => None,
            })
            .unwrap();
        let err = run(&state, other_tab, block_id.clone()).await.unwrap_err();
        assert!(
            err.contains("is in tab") && err.contains(&tab_id),
            "got: {}",
            err
        );
        // Block must still be present (saga rejected pre-dispatch).
        let s = state.srv_state.lock().await;
        assert!(s.blocks.contains_key(&block_id));
    }
}
