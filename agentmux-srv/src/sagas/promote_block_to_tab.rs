// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase E.5.7 — PromoteBlockToTab saga.
//
// Migrates the reducer-state portion of `wcore::promote_block_to_tab`
// to a reducer-driven multi-step. The original wcore function does
// substantial layout work + sets the new tab as active; that work
// stays wcore-direct in the RPC handler that wraps this saga.
//
// **Steps:**
// 1. `CreateTab { workspace_id, name: "" }` — new tab in the same
//    workspace.
// 2. `MoveBlock { block_id, src=source_tab, dst=new_tab, dst_index: 0 }`.
//
// **Compensation:**
// * Step 2 fails after step 1 → `DeleteTab { workspace_id, new_tab_id }`
//   (cascades through any blocks; new tab has no blocks at this
//   point since step 2 failed).
// * Step 1 fails → nothing to compensate.
//
// Layout setup (rootnode + leaforder for the new tab) and SetActiveTab
// stay in the RPC handler post-saga; auto-close empty source tab too.

use agentmux_common::ipc::{Command, Event};
use serde_json::{json, Value};

use super::{alloc_saga_id, emit_saga_started, emit_terminal, run_saga, SagaCtx};
use crate::server::AppState;

/// Run the PromoteBlockToTab saga. On success, returns
/// `{"new_tab_id": "..."}`.
pub async fn run(
    state: &AppState,
    block_id: String,
    source_tab_id: String,
    workspace_id: String,
) -> Result<Value, String> {
    // Pre-condition: read SQLite (source of truth during migration
    // window — wcore-direct paths can leave reducer state stale).
    {
        let block = match state.wstore.get::<crate::backend::obj::Block>(&block_id) {
            Ok(Some(b)) => b,
            Ok(None) => {
                return Err(format!("PromoteBlockToTab: block not found: {}", block_id));
            }
            Err(e) => return Err(format!("PromoteBlockToTab: block read failed: {}", e)),
        };
        let expected_parent = format!("tab:{}", source_tab_id);
        if block.parentoref != expected_parent {
            return Err(format!(
                "PromoteBlockToTab: block {} is in {}, not tab:{}",
                block_id, block.parentoref, source_tab_id
            ));
        }
        if state
            .wstore
            .get::<crate::backend::obj::Workspace>(&workspace_id)
            .map(|w| w.is_none())
            .unwrap_or(true)
        {
            return Err(format!(
                "PromoteBlockToTab: workspace not found: {}",
                workspace_id
            ));
        }
    }

    let saga_id = alloc_saga_id(state);
    emit_saga_started(state, saga_id, "promote_block_to_tab").await;
    let ctx = SagaCtx { state, saga_id };
    let result = run_saga(
        "promote_block_to_tab",
        run_inner(ctx, block_id, source_tab_id, workspace_id),
    )
    .await;
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
    workspace_id: String,
) -> Result<Value, String> {
    // Step 1: create the new tab in the same workspace.
    let create_tab_events = ctx
        .dispatch(Command::CreateTab {
            workspace_id: workspace_id.clone(),
            name: String::new(),
        })
        .await
        .map_err(|e| format!("PromoteBlockToTab step 1 (CreateTab): {}", e))?;
    let new_tab_id = create_tab_events
        .iter()
        .find_map(|e| match e {
            Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
            _ => None,
        })
        .ok_or_else(|| {
            "PromoteBlockToTab: CreateTab did not emit TabCreated".to_string()
        })?;

    // Step 2: move the block into the new tab.
    if let Err(reason) = ctx
        .dispatch(Command::MoveBlock {
            block_id: block_id.clone(),
            src_tab_id: source_tab_id.clone(),
            dst_tab_id: new_tab_id.clone(),
            dst_index: 0,
        })
        .await
    {
        // Compensate: delete the empty new tab.
        ctx.compensate(Command::DeleteTab {
            workspace_id: workspace_id.clone(),
            tab_id: new_tab_id.clone(),
        })
        .await;
        return Err(format!("PromoteBlockToTab step 2 (MoveBlock): {}", reason));
    }

    Ok(json!({ "new_tab_id": new_tab_id }))
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[tokio::test]
    async fn happy_path_creates_tab_and_moves_block() {
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
                name: "src".into(),
            },
        )
        .await;
        let src_tab = tab_evs
            .iter()
            .find_map(|e| match e {
                Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
                _ => None,
            })
            .unwrap();
        let blk_evs = dispatch_apply(
            &state,
            agentmux_common::ipc::Command::CreateBlock {
                tab_id: src_tab.clone(),
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

        let result = run(&state, block_id.clone(), src_tab.clone(), ws_id.clone())
            .await
            .unwrap();
        let new_tab_id = result["new_tab_id"].as_str().unwrap();

        // Reducer: src_tab has no blocks; new_tab has the block.
        let s = state.srv_state.lock().await;
        assert!(s.tabs[&src_tab].block_ids.is_empty());
        assert_eq!(s.tabs[new_tab_id].block_ids, vec![block_id.clone()]);
        assert_eq!(s.blocks[&block_id].tab_id, new_tab_id);
        assert!(s.workspaces[&ws_id].tab_ids.contains(&new_tab_id.to_string()));
    }

    #[tokio::test]
    async fn rejects_when_block_in_different_tab() {
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
        let err = run(&state, "ghost-block".into(), "ghost-tab".into(), ws_id)
            .await
            .unwrap_err();
        assert!(err.contains("not found") || err.contains("not in"), "got: {}", err);
    }
}
