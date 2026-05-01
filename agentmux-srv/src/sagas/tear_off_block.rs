// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase E.5.5 — TearOffBlock saga.
//
// Migrates the reducer-state portion of `wcore::tear_off_block` to
// a reducer-driven multi-step. The original wcore function does
// substantial layout work (rebuilds the new tab's `LayoutState`
// rootnode + leaforder, queues a layout-delete action on the source
// tab's pendingbackendactions) — that's E.4 territory and stays
// wcore-direct in the RPC handler that wraps this saga. The
// reducer/SQLite portion is what fixes the smoke regression
// (the new workspace was invisible to the reducer because wcore
// bypassed it).
//
// **Steps:**
// 1. `CreateWorkspace { name: "" }`
// 2. `CreateTab { workspace_id: new_ws_id, name: "" }`
// 3. `MoveBlock { block_id, src=source_tab, dst=new_tab, dst_index: 0 }`
//
// Auto-close-source-tab and layout setup are NOT here — see the
// RPC handler in `service.rs` for those steps.
//
// **Compensation:**
// * Step 3 fails after step 1+2 → `DeleteWorkspace { new_ws_id }`
//   (cascades through tabs to blocks; no blocks landed in the new
//   tab at this point).
// * Step 2 fails after step 1 → `DeleteWorkspace { new_ws_id }`.
// * Step 1 fails → nothing to compensate.

use agentmux_common::ipc::{Command, Event};
use serde_json::{json, Value};

use super::{alloc_saga_id, emit_saga_started, emit_terminal, run_saga, SagaCtx};
use crate::server::AppState;

/// Run the TearOffBlock saga. On success, returns
/// `{"new_workspace_id": "...", "new_tab_id": "..."}`.
pub async fn run(
    state: &AppState,
    block_id: String,
    source_tab_id: String,
    source_workspace_id: String,
) -> Result<Value, String> {
    // Pre-condition: block exists and belongs to source_tab; source
    // tab is in source_workspace. Reducer would catch the structural
    // mismatch via MoveBlock validation, but check up-front so we
    // don't allocate a workspace + tab and then have to compensate.
    {
        let s = state.srv_state.lock().await;
        match s.blocks.get(&block_id) {
            None => {
                return Err(format!("TearOffBlock: block not found: {}", block_id));
            }
            Some(block) if block.tab_id != source_tab_id => {
                return Err(format!(
                    "TearOffBlock: block {} is in tab {}, not {}",
                    block_id, block.tab_id, source_tab_id
                ));
            }
            _ => {}
        }
        match s.tabs.get(&source_tab_id) {
            None => {
                return Err(format!(
                    "TearOffBlock: source tab not found: {}",
                    source_tab_id
                ));
            }
            Some(tab) if tab.workspace_id != source_workspace_id => {
                return Err(format!(
                    "TearOffBlock: tab {} is in workspace {}, not {}",
                    source_tab_id, tab.workspace_id, source_workspace_id
                ));
            }
            _ => {}
        }
    }

    let saga_id = alloc_saga_id(state);
    emit_saga_started(state, saga_id, "tear_off_block").await;
    let ctx = SagaCtx::new(state, saga_id);
    let result = run_saga("tear_off_block", run_inner(ctx, block_id, source_tab_id)).await;
    emit_terminal(
        state,
        saga_id,
        match &result {
            Ok(_) => Ok(()),
            Err(r) => Err(r.as_str()),
        },
    )
    .await;
    result
}

async fn run_inner(
    ctx: SagaCtx<'_>,
    block_id: String,
    source_tab_id: String,
) -> Result<Value, String> {
    // Step 1: new workspace.
    let create_ws_events = ctx
        .dispatch(Command::CreateWorkspace {
            name: String::new(),
        })
        .await
        .map_err(|e| format!("TearOffBlock step 1 (CreateWorkspace): {}", e))?;
    let new_workspace_id = create_ws_events
        .iter()
        .find_map(|e| match e {
            Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
            _ => None,
        })
        .ok_or_else(|| {
            "TearOffBlock: CreateWorkspace did not emit WorkspaceCreated".to_string()
        })?;

    // Step 2: new tab in the new workspace.
    let create_tab_events = match ctx
        .dispatch(Command::CreateTab {
            workspace_id: new_workspace_id.clone(),
            name: String::new(),
        })
        .await
    {
        Ok(evs) => evs,
        Err(reason) => {
            ctx.compensate(Command::DeleteWorkspace {
                workspace_id: new_workspace_id.clone(),
            })
            .await;
            return Err(format!("TearOffBlock step 2 (CreateTab): {}", reason));
        }
    };
    let new_tab_id = create_tab_events
        .iter()
        .find_map(|e| match e {
            Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
            _ => None,
        })
        .ok_or_else(|| {
            "TearOffBlock: CreateTab did not emit TabCreated".to_string()
        })?;

    // Step 3: move the block.
    if let Err(reason) = ctx
        .dispatch(Command::MoveBlock {
            block_id: block_id.clone(),
            src_tab_id: source_tab_id.clone(),
            dst_tab_id: new_tab_id.clone(),
            dst_index: 0,
        })
        .await
    {
        // Compensate: delete the workspace (cascades the empty tab).
        ctx.compensate(Command::DeleteWorkspace {
            workspace_id: new_workspace_id.clone(),
        })
        .await;
        return Err(format!("TearOffBlock step 3 (MoveBlock): {}", reason));
    }

    Ok(json!({
        "new_workspace_id": new_workspace_id,
        "new_tab_id": new_tab_id,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::obj::{Block, Tab};
    use crate::server::tests::test_state;

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

    #[tokio::test]
    async fn happy_path_creates_workspace_tab_and_moves_block() {
        let state = test_state();
        // Seed: workspace with one tab containing a block.
        let ws_evs = dispatch_apply(
            &state,
            agentmux_common::ipc::Command::CreateWorkspace { name: "src".into() },
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

        let result = run(&state, block_id.clone(), tab_id.clone(), ws_id.clone())
            .await
            .unwrap();
        let new_ws_id = result["new_workspace_id"].as_str().unwrap();
        let new_tab_id = result["new_tab_id"].as_str().unwrap();

        // Reducer: source tab has no blocks; new tab has the block.
        let s = state.srv_state.lock().await;
        assert!(s.tabs[&tab_id].block_ids.is_empty());
        assert_eq!(
            s.tabs[new_tab_id].block_ids,
            vec![block_id.clone()],
            "block should be in new tab"
        );
        assert_eq!(s.blocks[&block_id].tab_id, new_tab_id);
        assert_eq!(s.workspaces[new_ws_id].tab_ids, vec![new_tab_id.to_string()]);

        // SQLite: matches.
        drop(s);
        let new_tab = state.wstore.get::<Tab>(new_tab_id).unwrap().unwrap();
        assert_eq!(new_tab.blockids, vec![block_id.clone()]);
        let block = state.wstore.get::<Block>(&block_id).unwrap().unwrap();
        assert_eq!(block.parentoref, format!("tab:{}", new_tab_id));
    }

    #[tokio::test]
    async fn rejects_when_block_not_in_source_tab() {
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
        let _ = dispatch_apply(
            &state,
            agentmux_common::ipc::Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: "t".into(),
            },
        )
        .await;
        let err = run(
            &state,
            "ghost-block".into(),
            "ghost-tab".into(),
            ws_id,
        )
        .await
        .unwrap_err();
        assert!(err.contains("block not found") || err.contains("source tab not found"), "got: {}", err);
    }
}
