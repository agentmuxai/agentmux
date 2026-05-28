// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// RedockFloatingPane saga — moves a block from a floating pane's
// (single-block) workspace into an existing tab in a different
// workspace. The inverse of `tear_off_block`.
//
// Used by the floating-pane re-dock flow (Phase 4a per
// `docs/specs/SPEC_FLOATING_PANE_REDOCK_2026-05-27.md`): the user
// drops a floater over another agentmux window's tile area; that
// window's frontend computes the drop and calls this RPC.
//
// **Steps:**
// 1. `MoveBlock { block_id, src=source_tab, dst=target_tab,
//    dst_index: <position in target> }`
//
// After the move:
// * Source workspace's tab is empty → frontend's empty-tab watcher
//   (PR #1089) closes the floating window.
// * Target tab's `blockids` now includes the moved block. The frontend
//   target window updates its layout local-state to add a leaf and
//   persists via the standard `UpdateObject` path. The saga itself
//   does not touch LayoutState — same pattern as within-window pane
//   drag (frontend mutates layout locally, persists, backend just
//   stores).
//
// **Compensation:** single-step saga, so no in-saga compensation
// chain needed. If MoveBlock fails, the reducer's validation rejects
// up-front and nothing has changed yet.

use agentmux_common::ipc::Command;
use serde_json::{json, Value};

use super::{
    alloc_saga_id, classify_run_saga_result, emit_saga_started, emit_terminal, run_saga, SagaCtx,
};
use crate::server::AppState;

/// Run the RedockFloatingPane saga. On success, returns
/// `{ "block_id": "...", "target_tab_id": "..." }`.
///
/// `dst_index` defaults to the end of the target tab's blockids if
/// callers don't have a specific drop position (Phase 4a MVP). Phase
/// 4b polish will let callers specify a drop position relative to a
/// target leaf + direction (left/right/top/bottom/into).
pub async fn run(
    state: &AppState,
    block_id: String,
    source_tab_id: String,
    source_workspace_id: String,
    target_tab_id: String,
    target_workspace_id: String,
    dst_index: Option<u32>,
) -> Result<Value, String> {
    // Pre-conditions: block exists and belongs to source_tab; source
    // tab is in source_workspace; target tab is in target_workspace;
    // source != target (callers should already check, but a no-op
    // re-dock is a programmer mistake worth surfacing).
    let dst_index_resolved = {
        let s = state.srv_state.lock().await;
        match s.blocks.get(&block_id) {
            None => {
                return Err(format!(
                    "RedockFloatingPane: block not found: {}",
                    block_id
                ));
            }
            Some(block) if block.tab_id != source_tab_id => {
                return Err(format!(
                    "RedockFloatingPane: block {} is in tab {}, not {}",
                    block_id, block.tab_id, source_tab_id
                ));
            }
            _ => {}
        }
        match s.tabs.get(&source_tab_id) {
            None => {
                return Err(format!(
                    "RedockFloatingPane: source tab not found: {}",
                    source_tab_id
                ));
            }
            Some(tab) if tab.workspace_id != source_workspace_id => {
                return Err(format!(
                    "RedockFloatingPane: source tab {} is in workspace {}, not {}",
                    source_tab_id, tab.workspace_id, source_workspace_id
                ));
            }
            _ => {}
        }
        let target_tab = match s.tabs.get(&target_tab_id) {
            None => {
                return Err(format!(
                    "RedockFloatingPane: target tab not found: {}",
                    target_tab_id
                ));
            }
            Some(tab) if tab.workspace_id != target_workspace_id => {
                return Err(format!(
                    "RedockFloatingPane: target tab {} is in workspace {}, not {}",
                    target_tab_id, tab.workspace_id, target_workspace_id
                ));
            }
            Some(tab) => tab,
        };
        if source_tab_id == target_tab_id {
            return Err(
                "RedockFloatingPane: source and target are the same tab — use MoveBlockToTab"
                    .to_string(),
            );
        }
        // Default insertion: append at the end of the target tab's
        // blockids.
        dst_index.unwrap_or(target_tab.block_ids.len() as u32)
    };

    let saga_id = alloc_saga_id(state);
    if let Err(e) = emit_saga_started(
        state,
        saga_id,
        "redock_floating_pane",
        json!({
            "block_id": &block_id,
            "source_tab_id": &source_tab_id,
            "target_tab_id": &target_tab_id,
            "dst_index": dst_index_resolved,
        }),
    )
    .await
    {
        return Err(e);
    }
    let ctx = SagaCtx::new(state, saga_id);
    let result = run_saga(
        "redock_floating_pane",
        run_inner(
            ctx,
            block_id,
            source_tab_id,
            target_tab_id,
            dst_index_resolved,
        ),
    )
    .await;
    emit_terminal(state, saga_id, classify_run_saga_result(&result)).await;
    result
}

async fn run_inner(
    ctx: SagaCtx<'_>,
    block_id: String,
    source_tab_id: String,
    target_tab_id: String,
    dst_index: u32,
) -> Result<Value, String> {
    ctx.dispatch(Command::MoveBlock {
        block_id: block_id.clone(),
        src_tab_id: source_tab_id,
        dst_tab_id: target_tab_id.clone(),
        dst_index,
    })
    .await
    .map_err(|e| format!("RedockFloatingPane MoveBlock: {}", e))?;

    Ok(json!({
        "block_id": block_id,
        "target_tab_id": target_tab_id,
        "dst_index": dst_index,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::obj::{Block, Tab};
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

    async fn seed_workspace_with_block(
        state: &crate::server::AppState,
        ws_name: &str,
    ) -> (String, String, String) {
        let ws_evs = dispatch_apply(
            state,
            Command::CreateWorkspace {
                name: ws_name.into(),
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
            state,
            Command::CreateTab {
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
            state,
            Command::CreateBlock {
                tab_id: tab_id.clone(),
                meta: Value::Null,
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
        (ws_id, tab_id, block_id)
    }

    #[tokio::test]
    async fn happy_path_moves_block_between_workspaces() {
        let state = test_state();
        let (src_ws, src_tab, block_id) = seed_workspace_with_block(&state, "src").await;
        // Seed a destination workspace with one tab (no blocks yet).
        let dst_ws_evs = dispatch_apply(
            &state,
            Command::CreateWorkspace { name: "dst".into() },
        )
        .await;
        let dst_ws = dst_ws_evs
            .iter()
            .find_map(|e| match e {
                Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
                _ => None,
            })
            .unwrap();
        let dst_tab_evs = dispatch_apply(
            &state,
            Command::CreateTab {
                workspace_id: dst_ws.clone(),
                name: "dt".into(),
            },
        )
        .await;
        let dst_tab = dst_tab_evs
            .iter()
            .find_map(|e| match e {
                Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
                _ => None,
            })
            .unwrap();

        let result = run(
            &state,
            block_id.clone(),
            src_tab.clone(),
            src_ws.clone(),
            dst_tab.clone(),
            dst_ws.clone(),
            None,
        )
        .await
        .unwrap();
        assert_eq!(result["block_id"], block_id);
        assert_eq!(result["target_tab_id"], dst_tab);

        // Reducer: source tab is empty; target tab has the block.
        let s = state.srv_state.lock().await;
        assert!(s.tabs[&src_tab].block_ids.is_empty());
        assert_eq!(s.tabs[&dst_tab].block_ids, vec![block_id.clone()]);
        assert_eq!(s.blocks[&block_id].tab_id, dst_tab);

        // SQLite: matches.
        drop(s);
        let dst_tab_obj = state.wstore.get::<Tab>(&dst_tab).unwrap().unwrap();
        assert_eq!(dst_tab_obj.blockids, vec![block_id.clone()]);
        let block = state.wstore.get::<Block>(&block_id).unwrap().unwrap();
        assert_eq!(block.parentoref, format!("tab:{}", dst_tab));
    }

    #[tokio::test]
    async fn rejects_when_source_and_target_are_same_tab() {
        let state = test_state();
        let (ws, tab, block_id) = seed_workspace_with_block(&state, "w").await;
        let err = run(
            &state,
            block_id,
            tab.clone(),
            ws.clone(),
            tab,
            ws,
            None,
        )
        .await
        .unwrap_err();
        assert!(
            err.contains("source and target are the same tab"),
            "got: {}",
            err
        );
    }

    #[tokio::test]
    async fn rejects_when_block_not_in_source_tab() {
        let state = test_state();
        let (ws, tab, _block_id) = seed_workspace_with_block(&state, "w").await;
        let dst_ws_evs = dispatch_apply(
            &state,
            Command::CreateWorkspace { name: "dst".into() },
        )
        .await;
        let dst_ws = dst_ws_evs
            .iter()
            .find_map(|e| match e {
                Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
                _ => None,
            })
            .unwrap();
        let dst_tab_evs = dispatch_apply(
            &state,
            Command::CreateTab {
                workspace_id: dst_ws.clone(),
                name: "dt".into(),
            },
        )
        .await;
        let dst_tab = dst_tab_evs
            .iter()
            .find_map(|e| match e {
                Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
                _ => None,
            })
            .unwrap();
        let err = run(
            &state,
            "ghost-block".into(),
            tab,
            ws,
            dst_tab,
            dst_ws,
            None,
        )
        .await
        .unwrap_err();
        assert!(err.contains("block not found"), "got: {}", err);
    }
}
