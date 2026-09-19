// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// ClosePane saga — closes every block in a pane, each one stopped before its
// records are deleted. docs/specs/SPEC_AGENT_PANE_CLOSE_GRACEFUL_SHUTDOWN_2026_09_18.md
//
// A pane (layout leaf) can hold several blocks as tabs (`block_stack`). The
// pane's × used to send one `DeleteBlock` for the visible tab only, leaving
// every other tab's agent running with no pane (§2 of the spec), and the
// `ClosePane` MCP tool had the same gap (#3202). This saga takes the whole set.
//
// **Steps, per block (§4.2):**
// 1. `shutdown_before_delete` — refuse respawn and messaging delivery, stop
//    the controller and drop its process tracker, then save final state
//    (`session:active_pid` cleared, instance row `stopped` + `ended_at`)
//    while the block record still exists.
// 2. `DeleteBlock { tab_id, block_id }` through the reducer.
//
// **Then, once for the pane:** if every member of the leaf is gone, delete
// the leaf (`LayoutDeleteNode`) and queue one frontend `delete` for it.
// A leaf that still has a live member is left alone — closing one tab of a
// stack (`closeBlockInStack`) must never take its siblings with it; the
// frontend has already pushed the shortened stack, and the reducer's
// `prune_dangling_stack_members` covers any stale copy.
//
// Phase 1 of the spec: the stop is still the immediate hard kill
// (`delete_controller`). Phase 2 makes it graceful and waits for exit.
//
// **Compensation:** none, same as `delete_block` — a stopped process and a
// deleted record cannot be recreated from saga state. A block whose
// `DeleteBlock` dispatch fails stays in the tab with its process stopped;
// closing it again finishes the job.

use agentmux_common::ipc::Command;
use serde_json::{json, Value};

use super::{
    alloc_saga_id, classify_run_saga_result, emit_saga_started, emit_terminal, run_saga, SagaCtx,
};
use crate::backend::blockcontroller;
use crate::server::AppState;

/// Close `block_ids` (normally every member of one pane's stack). Ids that no
/// longer exist are skipped; ids in different tabs are rejected. On success
/// returns `{"tab_id": "...", "block_ids": [...closed...], "leaf_removed": bool}`.
pub async fn run(state: &AppState, block_ids: Vec<String>) -> Result<Value, String> {
    let mut ids: Vec<String> = Vec::new();
    for id in block_ids {
        if !id.is_empty() && !ids.contains(&id) {
            ids.push(id);
        }
    }
    if ids.is_empty() {
        return Err("ClosePane: no block ids".to_string());
    }

    let (tab_id, present, leaf_to_delete) = {
        let s = state.srv_state.lock().await;
        let mut tab_id: Option<String> = None;
        let mut present = Vec::new();
        for id in &ids {
            let Some(block) = s.blocks.get(id) else {
                continue; // already gone
            };
            match &tab_id {
                None => tab_id = Some(block.tab_id.clone()),
                Some(t) if *t != block.tab_id => {
                    return Err(format!(
                        "ClosePane: blocks span tabs ({} is in {}, not {})",
                        id, block.tab_id, t
                    ));
                }
                Some(_) => {}
            }
            present.push(id.clone());
        }
        let Some(tab_id) = tab_id else {
            return Err(format!("ClosePane: block not found: {}", ids.join(", ")));
        };
        if !s.tabs.contains_key(&tab_id) {
            return Err(format!("ClosePane: tab not found: {}", tab_id));
        }
        // The leaf to remove: the one holding these blocks, and only if none
        // of its members survives this close.
        let leaf_to_delete = s
            .tabs
            .get(&tab_id)
            .and_then(|t| t.rootnode.as_ref())
            .and_then(|root| {
                present
                    .iter()
                    .find_map(|id| crate::backend::layout::find_leaf_containing_block(root, id))
            })
            .and_then(|leaf| {
                let data = leaf.data.as_ref()?;
                let all_closing = crate::backend::layout::leaf_members(data)
                    .iter()
                    .all(|m| ids.contains(m) || !s.blocks.contains_key(m));
                all_closing.then(|| (leaf.id.clone(), data.block_id.clone()))
            });
        (tab_id, present, leaf_to_delete)
    };

    let saga_id = alloc_saga_id(state);
    emit_saga_started(
        state,
        saga_id,
        "close_pane",
        json!({
            "tab_id": &tab_id,
            "block_ids": &present,
        }),
    )
    .await?;
    let ctx = SagaCtx::new(state, saga_id);
    let result = run_saga(
        "close_pane",
        run_inner(ctx, tab_id, present.clone(), leaf_to_delete),
    )
    .await;
    for id in &present {
        finish_close(state, id).await;
    }
    emit_terminal(state, saga_id, classify_run_saga_result(&result)).await;
    result
}

async fn run_inner(
    ctx: SagaCtx<'_>,
    tab_id: String,
    block_ids: Vec<String>,
    leaf_to_delete: Option<(String, String)>,
) -> Result<Value, String> {
    let mut failures = Vec::new();
    for block_id in &block_ids {
        shutdown_before_delete(ctx.state, block_id);
        if let Err(reason) = ctx
            .dispatch(Command::DeleteBlock {
                tab_id: tab_id.clone(),
                block_id: block_id.clone(),
            })
            .await
        {
            tracing::warn!(
                tab_id = %tab_id,
                block_id = %block_id,
                "[saga] ClosePane: DeleteBlock dispatch failed (process already stopped): {}",
                reason
            );
            failures.push(format!("{block_id}: {reason}"));
        }
    }

    let leaf_removed = leaf_to_delete.is_some();
    if let Some((node_id, visible_block_id)) = leaf_to_delete {
        // Best-effort, as in `delete_block`: the blocks are already gone.
        if let Err(reason) = ctx
            .dispatch(Command::LayoutDeleteNode {
                tab_id: tab_id.clone(),
                node_id: node_id.clone(),
                correlation_id: String::new(),
            })
            .await
        {
            tracing::warn!(
                tab_id = %tab_id,
                node_id = %node_id,
                "[saga] ClosePane: LayoutDeleteNode failed (best-effort): {}",
                reason
            );
        }
        // One frontend prune for the pane, keyed on its visible block — not
        // one per member, which is what produced the "could not find leaf
        // node with blockId" errors when each member sent its own.
        if let Err(reason) = crate::server::service::layout_helpers::queue_source_layout_delete(
            ctx.state,
            &tab_id,
            &visible_block_id,
        )
        .await
        {
            tracing::warn!(
                tab_id = %tab_id,
                block_id = %visible_block_id,
                "[saga] ClosePane: queue_source_layout_delete failed (best-effort): {}",
                reason
            );
        }
    }

    if !failures.is_empty() {
        return Err(format!("ClosePane: {}", failures.join("; ")));
    }
    Ok(json!({
        "tab_id": tab_id,
        "block_ids": block_ids,
        "leaf_removed": leaf_removed,
    }))
}

/// §4.2 steps 1 and 4–6 for one block, run while its record still exists.
/// Shared with `delete_block` so a tab's own × and the pane's × stop an
/// agent identically.
pub(crate) fn shutdown_before_delete(state: &AppState, block_id: &str) {
    // 1. Stop routing input: no respawn (resync_controller refuses), no
    //    jekt/muxbus delivery. The controller leaves CONTROLLER_REGISTRY in
    //    the next step, so AgentInput finds nothing to write to.
    blockcontroller::mark_closing(block_id);
    state.reactive_handler.unregister_block(block_id);
    // 4–5. Stop the process, then drop its tracker (kills its descendants).
    blockcontroller::delete_controller(block_id);
    // 6. Save final state.
    save_final_state(state, block_id);
}

/// Clear `session:active_pid` (so the next boot doesn't read this as an
/// interrupted session) and mark the local instance row stopped. The session
/// id itself is left everywhere it is — block meta, instance row, shared
/// registry — so reopening the agent resumes it (spec §4.2 step 6).
fn save_final_state(state: &AppState, block_id: &str) {
    use crate::backend::blockcontroller::session_recovery;

    let has_active_pid = state
        .mstore
        .get::<crate::backend::obj::Block>(block_id)
        .ok()
        .flatten()
        .is_some_and(|b| {
            b.meta
                .get(session_recovery::META_SESSION_ACTIVE_PID)
                .is_some_and(|v| !v.is_null())
        });
    if has_active_pid {
        session_recovery::clear_active_pid(&state.mstore, block_id);
    }

    match state.mstore.instance_get_by_block_id(block_id) {
        Ok(Some(instance)) => {
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64;
            let upd = crate::backend::storage::agents::InstanceUpdate {
                status: Some(
                    crate::backend::storage::agents::InstanceStatus::Stopped
                        .as_str()
                        .to_string(),
                ),
                ended_at: Some(now_ms),
                ..Default::default()
            };
            if let Err(e) = state.mstore.instance_update_partial(&instance.id, &upd) {
                tracing::warn!(
                    block_id = %block_id,
                    instance_id = %instance.id,
                    error = %e,
                    "ClosePane: failed to mark instance stopped"
                );
            }
        }
        Ok(None) => {}
        Err(e) => {
            tracing::debug!(block_id = %block_id, error = %e, "ClosePane: instance lookup failed");
        }
    }
}

/// After the saga: purge the block's persisted broker history and lift the
/// respawn guard. If the block somehow survived (dispatch failed), it keeps
/// its stopped state and can be closed again.
pub(crate) async fn finish_close(state: &AppState, block_id: &str) {
    let gone = !state.srv_state.lock().await.blocks.contains_key(block_id);
    if gone {
        state.broker.purge_scope(&format!("block:{}", block_id));
    }
    blockcontroller::unmark_closing(block_id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::obj::Block;
    use crate::server::tests::test_state;
    use agentmux_common::ipc::Event;

    async fn dispatch_apply(state: &AppState, cmd: Command) -> Vec<Event> {
        let events = crate::server::service::dispatch_to_reducer(state, cmd).await;
        for ev in &events {
            crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore).unwrap();
        }
        events
    }

    async fn seed_tab(state: &AppState) -> (String, String) {
        let ws_id = dispatch_apply(state, Command::CreateWorkspace { name: "w".into() })
            .await
            .iter()
            .find_map(|e| match e {
                Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
                _ => None,
            })
            .unwrap();
        let tab_id = dispatch_apply(
            state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: "t".into(),
            },
        )
        .await
        .iter()
        .find_map(|e| match e {
            Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
            _ => None,
        })
        .unwrap();
        (ws_id, tab_id)
    }

    async fn seed_block(state: &AppState, tab_id: &str) -> String {
        dispatch_apply(
            state,
            Command::CreateBlock {
                tab_id: tab_id.to_string(),
                meta: serde_json::Value::Null,
            },
        )
        .await
        .iter()
        .find_map(|e| match e {
            Event::BlockCreated { block_id, .. } => Some(block_id.clone()),
            _ => None,
        })
        .unwrap()
    }

    fn stack_leaf(id: &str, stack: &[&str], visible: &str) -> agentmux_common::LayoutNode {
        agentmux_common::LayoutNode {
            id: id.into(),
            data: Some(agentmux_common::LayoutNodeData {
                block_id: visible.to_string(),
                block_stack: stack.iter().map(|s| s.to_string()).collect(),
                active_block_id: visible.to_string(),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    async fn set_tree(state: &AppState, tab_id: &str, tree: agentmux_common::LayoutNode) {
        dispatch_apply(
            state,
            Command::LayoutSetTree {
                tab_id: tab_id.to_string(),
                new_tree: Some(tree),
                correlation_id: String::new(),
                slices: None,
            },
        )
        .await;
    }

    fn queued_deletes(state: &AppState, tab_id: &str) -> Vec<String> {
        let tab = state.mstore.must_get::<crate::backend::obj::Tab>(tab_id).unwrap();
        let layout = state
            .mstore
            .must_get::<crate::backend::obj::LayoutState>(&tab.layoutstate)
            .unwrap();
        layout
            .pendingbackendactions
            .unwrap_or_default()
            .into_iter()
            .filter(|a| a.actiontype == "delete")
            .map(|a| a.blockid)
            .collect()
    }

    /// The incident (spec §2): a pane holding [A, B, C] with A visible. Closing
    /// it must remove all three blocks, not just A, and prune the leaf once.
    #[tokio::test]
    async fn closing_a_stacked_pane_removes_every_member() {
        let state = test_state();
        let (_ws, tab_id) = seed_tab(&state).await;
        let a = seed_block(&state, &tab_id).await;
        let b = seed_block(&state, &tab_id).await;
        let c = seed_block(&state, &tab_id).await;
        set_tree(&state, &tab_id, stack_leaf("leaf", &[&a, &b, &c], &a)).await;

        let out = run(&state, vec![a.clone(), b.clone(), c.clone()]).await.unwrap();
        assert_eq!(out["leaf_removed"], true);

        {
            let s = state.srv_state.lock().await;
            for id in [&a, &b, &c] {
                assert!(!s.blocks.contains_key(id), "block {id} must be deleted");
            }
            assert!(s.tabs[&tab_id].block_ids.is_empty(), "no member left in tab.blockids");
            assert!(s.tabs[&tab_id].rootnode.is_none(), "leaf pruned");
        }
        for id in [&a, &b, &c] {
            assert!(state.mstore.get::<Block>(id).unwrap().is_none());
            assert!(!blockcontroller::is_closing(id), "respawn guard lifted after close");
        }
        assert_eq!(queued_deletes(&state, &tab_id), vec![a.clone()], "one frontend prune, for the visible block");
    }

    /// Closing one tab of a stack (the tab's own ×) must leave the pane and its
    /// other tabs alone — even when the closed tab is the visible one in the
    /// backend's copy of the tree.
    #[tokio::test]
    async fn closing_one_member_keeps_the_pane_and_its_siblings() {
        let state = test_state();
        let (_ws, tab_id) = seed_tab(&state).await;
        let a = seed_block(&state, &tab_id).await;
        let b = seed_block(&state, &tab_id).await;
        set_tree(&state, &tab_id, stack_leaf("leaf", &[&a, &b], &a)).await;

        let out = run(&state, vec![a.clone()]).await.unwrap();
        assert_eq!(out["leaf_removed"], false);

        let s = state.srv_state.lock().await;
        assert!(!s.blocks.contains_key(&a));
        assert!(s.blocks.contains_key(&b), "sibling survives");
        assert_eq!(s.tabs[&tab_id].block_ids, vec![b.clone()]);
        assert!(s.tabs[&tab_id].rootnode.is_some(), "pane survives");
        drop(s);
        assert!(queued_deletes(&state, &tab_id).is_empty());
    }

    fn sample_agent(id: &str) -> crate::backend::storage::agents::AgentDefinition {
        crate::backend::storage::agents::AgentDefinition {
            conversation_visibility: crate::backend::storage::agents::default_conversation_visibility(),
            id: id.to_string(),
            slug: id.to_string(),
            name: id.to_string(),
            icon: "✦".to_string(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: String::new(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 0,
            agent_type: "host".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 0,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 0,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
            memory_id: String::new(),
        }
    }

    /// Ordering (spec §4.2): final state is saved while the block still
    /// exists — `session:active_pid` cleared and the instance row marked
    /// stopped — and the session id is kept.
    #[tokio::test]
    async fn final_state_is_saved_before_the_block_is_deleted() {
        use crate::backend::storage::agents::{AgentInstance, InstanceStatus};
        let state = test_state();
        let (_ws, tab_id) = seed_tab(&state).await;
        let a = seed_block(&state, &tab_id).await;

        let mut meta = crate::backend::obj::MetaMapType::new();
        meta.insert(
            crate::backend::blockcontroller::session_recovery::META_SESSION_ACTIVE_PID.to_string(),
            serde_json::json!(4242),
        );
        crate::server::service::update_object_meta(&state.mstore, &format!("block:{a}"), &meta).unwrap();

        let mut def = sample_agent("def-close");
        state.mstore.agent_def_insert(&mut def).unwrap();
        state
            .mstore
            .instance_create(&AgentInstance {
                id: "def-close".to_string(),
                definition_id: "def-close".to_string(),
                parent_instance_id: String::new(),
                block_id: a.clone(),
                session_id: "sess-keep".to_string(),
                status: InstanceStatus::Running.as_str().to_string(),
                github_context: String::new(),
                started_at: 1000,
                ended_at: 0,
                created_at: 1000,
                identity_id: String::new(),
                memory_id: String::new(),
                instance_name: String::new(),
                working_directory: String::new(),
                display_hidden: false,
            })
            .unwrap();

        // Run only the pre-delete half and observe the block while it exists.
        shutdown_before_delete(&state, &a);
        let block = state.mstore.get::<Block>(&a).unwrap().expect("block still exists");
        assert!(
            block
                .meta
                .get(crate::backend::blockcontroller::session_recovery::META_SESSION_ACTIVE_PID)
                .is_none_or(|v| v.is_null()),
            "active pid cleared before delete"
        );
        let inst = state.mstore.instance_get_by_block_id(&a).unwrap().unwrap();
        assert_eq!(inst.status, "stopped");
        assert!(inst.ended_at > 0);
        assert_eq!(inst.session_id, "sess-keep", "session id kept for resume");
        assert!(blockcontroller::is_closing(&a), "respawn refused mid-close");
        finish_close(&state, &a).await;
    }

    #[tokio::test]
    async fn rejects_blocks_from_different_tabs() {
        let state = test_state();
        let (ws, tab1) = seed_tab(&state).await;
        let tab2 = dispatch_apply(
            &state,
            Command::CreateTab {
                workspace_id: ws,
                name: "t2".into(),
            },
        )
        .await
        .iter()
        .find_map(|e| match e {
            Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
            _ => None,
        })
        .unwrap();
        let a = seed_block(&state, &tab1).await;
        let b = seed_block(&state, &tab2).await;
        let err = run(&state, vec![a.clone(), b.clone()]).await.unwrap_err();
        assert!(err.contains("span tabs"), "got: {err}");
        let s = state.srv_state.lock().await;
        assert!(s.blocks.contains_key(&a) && s.blocks.contains_key(&b), "nothing closed");
    }

    #[tokio::test]
    async fn skips_ids_that_are_already_gone() {
        let state = test_state();
        let (_ws, tab_id) = seed_tab(&state).await;
        let a = seed_block(&state, &tab_id).await;
        let out = run(&state, vec!["ghost".into(), a.clone()]).await.unwrap();
        assert_eq!(out["block_ids"], json!([a]));
        let err = run(&state, vec!["ghost".into()]).await.unwrap_err();
        assert!(err.contains("block not found"), "got: {err}");
    }

    #[test]
    fn closing_guard_blocks_resync() {
        let block = Block {
            oid: "closing-guard-blk".to_string(),
            parentoref: String::new(),
            version: 1,
            runtimeopts: None,
            stickers: None,
            meta: {
                let mut m = crate::backend::obj::MetaMapType::new();
                m.insert("controller".to_string(), json!("shell"));
                m
            },
            subblockids: None,
        };
        blockcontroller::mark_closing(&block.oid);
        let err = blockcontroller::resync_controller(
            &block, "tab-1", None, false, true, None, None, None, None, None,
            std::sync::Arc::from("test-boot"),
        )
        .unwrap_err();
        blockcontroller::unmark_closing(&block.oid);
        assert!(err.contains("is closing"), "got: {err}");
        assert!(blockcontroller::get_controller(&block.oid).is_none());
    }
}
