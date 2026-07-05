// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `workspace` service handler (workspace + tab lifecycle, drag-and-drop moves).

use crate::backend::obj::*;
use crate::backend::service::{self, CloseTabRtnType, WebCallType, WebReturnType};
use crate::backend::wcore;

use super::super::AppState;
use super::layout_helpers::{
    queue_source_layout_delete, queue_target_layout_insert, queue_target_layout_split,
    setup_torn_off_block_layout,
};
use super::reducer_helpers::{
    compensate_via_reducer, dispatch_to_reducer, publish_events, wstore_workspace_exists,
};

pub(super) async fn handle_workspace_service(state: &AppState, call: &WebCallType) -> WebReturnType {
    let store = &state.wstore;
    let args = &call.args;
    // Phase E.2c.2 — workspace lifecycle dispatches through the
    // srv reducer for event emission (sagas / renderer / persist
    // subscriber consume them) AND synchronously applies the
    // emitted events to SQLite via the subscriber's apply path.
    // Synchronous SQLite writes are required during the migration
    // window because tab/block RPC still hits wcore directly and
    // expects workspaces to be present in SQLite by the time the
    // RPC reply returns (e.g., a CreateTab call right after
    // CreateWorkspace would 404 on the workspace lookup if we
    // only relied on the async subscriber). The subscriber later
    // receives the same event on the broadcast bus and re-applies
    // idempotently — safe because each apply arm checks SQLite
    // state before writing. (Both reagent + codex flagged this
    // race as P1 #615.)
    //
    // Reads (`GetWorkspace` / `ListWorkspaces`) stay on wstore
    // until the tab + block RPC layers also migrate (E.2c.3 +
    // E.2c.4). The reducer's `WorkspaceRecord` doesn't track
    // `pinnedtabids` and its `tabids` / `activetabid` go stale
    // immediately after any wcore-direct tab op — reading from
    // it before tabs are migrated returns wrong data.
    match call.method.as_str() {
        "CreateWorkspace" => {
            let name: String = service::get_arg(args, 0).unwrap_or_default();
            let events = dispatch_to_reducer(
                state,
                agentmux_common::ipc::Command::CreateWorkspace { name: name.clone() },
            )
            .await;
            let workspace_id = events.iter().find_map(|e| match e {
                agentmux_common::ipc::Event::WorkspaceCreated { workspace_id, .. } => {
                    Some(workspace_id.clone())
                }
                _ => None,
            });
            // Apply synchronously to wstore BEFORE publishing or
            // returning. On SQLite failure, dispatch a compensating
            // `DeleteWorkspace` so the reducer's session-only state
            // doesn't carry a ghost workspace that was never
            // persisted (codex P2 #615).
            let mut apply_err: Option<String> = None;
            for ev in &events {
                if let Err(e) = crate::persist_subscriber::apply_event_to_wstore(ev, store) {
                    apply_err = Some(e.to_string());
                    break;
                }
            }
            if let Some(err) = apply_err {
                if let Some(id) = workspace_id.as_ref() {
                    compensate_via_reducer(
                        state,
                        agentmux_common::ipc::Command::DeleteWorkspace {
                            workspace_id: id.clone(),
                            // Internal compensation path for failed
                            // CreateWorkspace SQLite apply — not the
                            // saga (Step 5 PR 2).
                            force: false,
                        },
                        store,
                    )
                    .await;
                }
                return WebReturnType::error(format!(
                    "CreateWorkspace: SQLite write failed: {}",
                    err
                ));
            }
            publish_events(state, &events);
            match workspace_id {
                Some(id) => match wcore::get_workspace(store, &id) {
                    Ok(ws) => {
                        WebReturnType::success(serde_json::to_value(&ws).unwrap_or_default())
                    }
                    Err(e) => WebReturnType::error(format!(
                        "CreateWorkspace: post-write read failed: {}",
                        e
                    )),
                },
                None => WebReturnType::error(
                    "CreateWorkspace: reducer did not emit WorkspaceCreated".to_string(),
                ),
            }
        }
        "GetWorkspace" => {
            let ws_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            // wstore-direct during the migration window (see
            // ("workspace", ...) header comment above for the
            // rationale). Reducer-state reads return on E.2c.3+ once
            // tabs (and pinned tabs) live in the reducer.
            match wcore::get_workspace(store, &ws_id) {
                Ok(ws) => WebReturnType::success(serde_json::to_value(&ws).unwrap_or_default()),
                Err(e) => WebReturnType::error(e.to_string()),
            }
        }
        "DeleteWorkspace" => {
            let ws_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            // Step 5 PR 2 — route the user-initiated DeleteWorkspace
            // through the `delete_workspace` saga. The saga:
            //   1. Snapshots the workspace's tabs+blocks for
            //      provenance in the durable saga log.
            //   2. Dispatches per-tab `DeleteTab { force: true }`
            //      through the reducer (cascades blocks; persist
            //      subscriber writes SQLite + kills controllers via
            //      `wcore::delete_tab_inner`).
            //   3. Dispatches the final
            //      `DeleteWorkspace { force: true }` which removes
            //      the (now-empty) workspace + window mappings.
            //
            // The legacy SQLite-first path here (wcore::delete_workspace
            // followed by Command::DeleteWorkspace dispatch) is replaced
            // by the saga because the durable lifecycle bracket gives
            // crash-recovery a chance to retry/compensate via
            // `recovery::compensate_unresolved` if the cascade is
            // interrupted. Cascade behaviour is preserved 1:1.
            //
            // Pre-condition: workspace must exist (in reducer or
            // SQLite). The saga runs its own existence check; we mirror
            // the legacy NotFound semantics here for backward-compat
            // error messages.
            let exists_in_wstore = match wstore_workspace_exists(store, &ws_id) {
                Ok(v) => v,
                Err(e) => {
                    return WebReturnType::error(format!(
                        "DeleteWorkspace: SQLite read failed: {}",
                        e
                    ))
                }
            };
            if !exists_in_wstore {
                let exists_in_state = state
                    .srv_state
                    .lock()
                    .await
                    .workspaces
                    .contains_key(&ws_id);
                if !exists_in_state {
                    return WebReturnType::error(format!(
                        "DeleteWorkspace: workspace not found: {}",
                        ws_id
                    ));
                }
            }
            match crate::sagas::delete_workspace::run(state, ws_id.clone()).await {
                Ok(_) => WebReturnType::success_empty(),
                Err(e) => WebReturnType::error(format!("DeleteWorkspace failed: {}", e)),
            }
        }
        "ListWorkspaces" => match wcore::list_workspaces(store) {
            Ok(list) => WebReturnType::success(serde_json::to_value(&list).unwrap_or_default()),
            Err(e) => WebReturnType::error(e.to_string()),
        },
        // Phase E.2c.3b — CreateTab dispatches through the reducer.
        // The `pinned` argument from older clients is ignored:
        // pinning was a Waveterm feature removed from AgentMux.
        // Legacy SQLite databases may still have entries in
        // `Workspace.pinnedtabids`; bootstrap merges them into
        // `tab_ids` so they behave as regular tabs.
        "CreateTab" => {
            let ws_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let tab_name: String = service::get_arg(args, 1).unwrap_or_default();
            let activate: bool = service::get_arg(args, 2).unwrap_or(true);
            // args[3] (`pinned`) intentionally ignored.
            // Auto-generate a `tab{N}` name when the caller passed
            // empty so behaviour matches the prior wcore path. Counts
            // both `tabids` and any leftover `pinnedtabids` from
            // legacy data so the numbering doesn't collide with
            // pre-removal entries that bootstrap will surface as
            // regular tabs.
            let resolved_name = if tab_name.is_empty() {
                match store.get::<Workspace>(&ws_id) {
                    Ok(Some(ws)) => {
                        format!("tab{}", ws.tabids.len() + ws.pinnedtabids.len() + 1)
                    }
                    _ => "tab1".to_string(),
                }
            } else {
                tab_name.clone()
            };
            let events = dispatch_to_reducer(
                state,
                agentmux_common::ipc::Command::CreateTab {
                    workspace_id: ws_id.clone(),
                    name: resolved_name,
                },
            )
            .await;
            // Surface reducer Error events (e.g., workspace not
            // found) before any persistence work — they're not
            // bug events, they're caller-visible failures, and the
            // generic "did not emit TabCreated" message below would
            // mask the real reason. Matches the SetActiveTab pattern.
            // (reagent P1 #616.)
            if let Some(err_msg) = events.iter().find_map(|e| match e {
                agentmux_common::ipc::Event::Error { message, .. } => Some(message.clone()),
                _ => None,
            }) {
                return WebReturnType::error(err_msg);
            }
            let tab_id = events.iter().find_map(|e| match e {
                agentmux_common::ipc::Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
                _ => None,
            });
            // Apply synchronously to wstore (forward+compensate on
            // failure — same pattern as CreateWorkspace in E.2c.2).
            let mut apply_err: Option<String> = None;
            for ev in &events {
                if let Err(e) = crate::persist_subscriber::apply_event_to_wstore(ev, store) {
                    apply_err = Some(e.to_string());
                    break;
                }
            }
            if let Some(err) = apply_err {
                if let Some(tid) = tab_id.as_ref() {
                    compensate_via_reducer(
                        state,
                        agentmux_common::ipc::Command::DeleteTab {
                            workspace_id: ws_id.clone(),
                            tab_id: tid.clone(),
                            // Compensation must bypass the last-tab
                            // guard to roll back a just-created sole
                            // tab when its persist failed (codex P1
                            // round 2 + P2 round 4 PR #633).
                            force: true,
                        },
                        store,
                    )
                    .await;
                }
                return WebReturnType::error(format!("CreateTab: SQLite write failed: {}", err));
            }
            publish_events(state, &events);
            // If `activate=true` and the reducer didn't auto-activate
            // this as the first tab, dispatch SetActiveTab.
            let auto_activated = events
                .iter()
                .any(|e| matches!(e, agentmux_common::ipc::Event::ActiveTabChanged { .. }));
            if activate && !auto_activated {
                if let Some(tid) = tab_id.as_ref() {
                    let active_events = dispatch_to_reducer(
                        state,
                        agentmux_common::ipc::Command::SetActiveTab {
                            workspace_id: ws_id.clone(),
                            tab_id: tid.clone(),
                        },
                    )
                    .await;
                    let mut active_err: Option<String> = None;
                    for ev in &active_events {
                        if let Err(e) =
                            crate::persist_subscriber::apply_event_to_wstore(ev, store)
                        {
                            active_err = Some(e.to_string());
                            break;
                        }
                    }
                    if active_err.is_none() {
                        publish_events(state, &active_events);
                    }
                    // SetActiveTab failure is non-fatal here — the
                    // tab exists; activation can be retried by the
                    // caller. Log if it happened.
                    if let Some(err) = active_err {
                        tracing::warn!(
                            "CreateTab: post-create activate failed: {}",
                            err
                        );
                    }
                }
            }
            match tab_id {
                Some(id) => {
                    let mut updates = vec![];
                    if let Ok(tab) = store.must_get::<Tab>(&id) {
                        updates.push(WaveObjUpdate {
                            updatetype: "update".into(),
                            otype: OTYPE_TAB.to_string(),
                            oid: id.clone(),
                            obj: Some(wave_obj_to_value(&tab)),
                        });
                    }
                    if let Ok(ws) = store.must_get::<Workspace>(&ws_id) {
                        updates.push(WaveObjUpdate {
                            updatetype: "update".into(),
                            otype: OTYPE_WORKSPACE.to_string(),
                            oid: ws_id.clone(),
                            obj: Some(wave_obj_to_value(&ws)),
                        });
                    }
                    WebReturnType::success_data_updates(
                        serde_json::to_value(&id).unwrap_or_default(),
                        updates,
                    )
                }
                None => WebReturnType::error(
                    "CreateTab: reducer did not emit TabCreated".to_string(),
                ),
            }
        }
        // Phase E.2c.3 — SetActiveTab routes through the reducer.
        // Read-through reads (e.g., GetWorkspace) still hit wstore
        // during the migration window, so the synchronous
        // apply-to-wstore keeps them consistent.
        "SetActiveTab" => {
            let ws_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let tab_id: String = match service::get_arg(args, 1) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            // Self-heal the layout before activating — remove any
            // orphaned block nodes that would render as blank panes.
            // (codex P1 PR #632 round 2) heal_layout clears
            // focusednodeid in SQLite when rootnode drops to empty,
            // bypassing the reducer. Sync the post-heal state through
            // the reducer so its tabs[tab_id].focused_node_id mirror
            // matches SQLite.
            let healed = wcore::heal_layout(store, &tab_id).unwrap_or(false);
            if healed {
                if let Ok(tab) = store.must_get::<Tab>(&tab_id) {
                    if !tab.layoutstate.is_empty() {
                        if let Ok(Some(layout)) = store.get::<LayoutState>(&tab.layoutstate) {
                            // Best-effort dispatch — failures here
                            // don't block SetActiveTab.
                            let focus_events = dispatch_to_reducer(
                                state,
                                agentmux_common::ipc::Command::SetFocusedNode {
                                    tab_id: tab_id.clone(),
                                    node_id: layout.focusednodeid.clone(),
                                },
                            )
                            .await;
                            publish_events(state, &focus_events);
                            let mag_events = dispatch_to_reducer(
                                state,
                                agentmux_common::ipc::Command::SetMagnifiedNode {
                                    tab_id: tab_id.clone(),
                                    node_id: layout.magnifiednodeid.clone(),
                                },
                            )
                            .await;
                            publish_events(state, &mag_events);
                        }
                    }
                }
            }

            // Phase E.2c.3b — pinning was removed from AgentMux
            // (Waveterm legacy). All tabs are regular; dispatch
            // straight through the reducer. Bootstrap merges any
            // legacy `pinnedtabids` into the reducer's `tab_ids` so
            // tabs from older databases are reachable as normal tabs.
            let events = dispatch_to_reducer(
                state,
                agentmux_common::ipc::Command::SetActiveTab {
                    workspace_id: ws_id.clone(),
                    tab_id: tab_id.clone(),
                },
            )
            .await;
            // Reducer emits Event::Error on unknown workspace/tab —
            // surface as RPC error.
            if let Some(err_msg) = events.iter().find_map(|e| match e {
                agentmux_common::ipc::Event::Error { message, .. } => Some(message.clone()),
                _ => None,
            }) {
                return WebReturnType::error(err_msg);
            }
            // Apply synchronously. SetActiveTab is reversible at the
            // reducer level (just write back the previous active id),
            // but we don't track the previous id here; if SQLite
            // fails, return the error and accept short-lived
            // divergence on this RPC path. (Acceptable: SetActiveTab
            // is a UI-driven action; the user can retry.)
            let mut apply_err: Option<String> = None;
            for ev in &events {
                if let Err(e) = crate::persist_subscriber::apply_event_to_wstore(ev, store) {
                    apply_err = Some(e.to_string());
                    break;
                }
            }
            if let Some(err) = apply_err {
                return WebReturnType::error(format!("SetActiveTab: SQLite write failed: {}", err));
            }
            publish_events(state, &events);
            if let Ok(ws) = store.must_get::<Workspace>(&ws_id) {
                let update = WaveObjUpdate {
                    updatetype: "update".into(),
                    otype: OTYPE_WORKSPACE.to_string(),
                    oid: ws_id.clone(),
                    obj: Some(wave_obj_to_value(&ws)),
                };
                WebReturnType::success_with_updates(vec![update])
            } else {
                WebReturnType::success_empty()
            }
        }
        // Phase E.5.7 (Step 5 PR 1) — CloseTab via DeleteTab saga.
        // Replaces the legacy SQLite-first pattern (wcore::delete_tab
        // followed by reducer-sync dispatch) with a saga-driven
        // reducer + persist-subscriber flow. The saga also enforces
        // the not-the-last-tab pre-condition mirrored from
        // TearOffTab; user-facing CloseTab now refuses to drain a
        // workspace to zero tabs (callers wanting full teardown
        // should issue DeleteWorkspace instead — that path migrates
        // in Step 5 PR 2).
        "CloseTab" => {
            let ws_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let tab_id: String = match service::get_arg(args, 1) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            if let Err(reason) =
                crate::sagas::delete_tab::run(state, ws_id.clone(), tab_id.clone()).await
            {
                return WebReturnType::error(reason);
            }
            let rtn = CloseTabRtnType {
                closewindow: false,
                newactivetabid: String::new(),
            };
            let mut updates = vec![WaveObjUpdate {
                updatetype: "delete".into(),
                otype: OTYPE_TAB.to_string(),
                oid: tab_id.clone(),
                obj: None,
            }];
            if let Ok(ws) = store.must_get::<Workspace>(&ws_id) {
                updates.push(WaveObjUpdate {
                    updatetype: "update".into(),
                    otype: OTYPE_WORKSPACE.to_string(),
                    oid: ws_id.clone(),
                    obj: Some(wave_obj_to_value(&ws)),
                });
            }
            WebReturnType::success_data_updates(
                serde_json::to_value(&rtn).unwrap_or_default(),
                updates,
            )
        }
        // Phase E.5.3 — UpdateWorkspace migrated through the reducer.
        // Currently only handles rename (the only field this RPC ever
        // mutated). Meta-only updates are dispatched as
        // UpdateWorkspaceMeta separately by frontends.
        "UpdateWorkspace" => {
            let ws_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let name: Option<String> = service::get_optional_arg(args, 1).unwrap_or(None);
            let Some(name) = name else {
                return WebReturnType::success_empty();
            };
            let events = dispatch_to_reducer(
                state,
                agentmux_common::ipc::Command::RenameWorkspace {
                    workspace_id: ws_id.clone(),
                    name,
                },
            )
            .await;
            if let Some(err_msg) = events.iter().find_map(|e| match e {
                agentmux_common::ipc::Event::Error { message, .. } => Some(message.clone()),
                _ => None,
            }) {
                return WebReturnType::error(err_msg);
            }
            for ev in &events {
                if let Err(e) = crate::persist_subscriber::apply_event_to_wstore(ev, store) {
                    return WebReturnType::error(format!(
                        "UpdateWorkspace: SQLite write failed: {}",
                        e
                    ));
                }
            }
            publish_events(state, &events);
            WebReturnType::success_empty()
        }
        // Phase E.5.3 — UpdateTabIds migrated to ReorderTabsBulk
        // through the reducer. The legacy `pinned_tab_ids` arg is
        // ignored: pinning was a Waveterm feature removed from
        // AgentMux. Bootstrap merged any legacy `pinnedtabids` into
        // the reducer's `tab_ids`. The subscriber's
        // `apply_tabs_reordered_bulk` rewrites `Workspace.tabids`
        // and drains any leftover `Workspace.pinnedtabids` so the
        // UI's `[...pinnedtabids, ...tabids]` combine never
        // double-counts a tab once a workspace's tabs are
        // reordered through the reducer.
        "UpdateTabIds" => {
            let ws_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let tab_ids: Vec<String> = match service::get_arg(args, 1) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            // args[2] (pinned_tab_ids) intentionally ignored.
            let events = dispatch_to_reducer(
                state,
                agentmux_common::ipc::Command::ReorderTabsBulk {
                    workspace_id: ws_id.clone(),
                    tab_ids,
                },
            )
            .await;
            if let Some(err_msg) = events.iter().find_map(|e| match e {
                agentmux_common::ipc::Event::Error { message, .. } => Some(message.clone()),
                _ => None,
            }) {
                return WebReturnType::error(err_msg);
            }
            for ev in &events {
                if let Err(e) = crate::persist_subscriber::apply_event_to_wstore(ev, store) {
                    return WebReturnType::error(format!(
                        "UpdateTabIds: SQLite write failed: {}",
                        e
                    ));
                }
            }
            publish_events(state, &events);
            if let Ok(updated_ws) = store.must_get::<Workspace>(&ws_id) {
                let update = WaveObjUpdate {
                    updatetype: "update".into(),
                    otype: OTYPE_WORKSPACE.to_string(),
                    oid: ws_id.clone(),
                    obj: Some(wave_obj_to_value(&updated_ws)),
                };
                return WebReturnType::success_with_updates(vec![update]);
            }
            WebReturnType::success_empty()
        }
        // Phase E.5.7 — MoveBlockToTab migrated to dispatch
        // Command::MoveBlock through the reducer. Auto-close empty
        // source tab still uses Command::DeleteTab. ws_id arg kept
        // for backward compat — used only for the post-op SQLite
        // refresh + auto-close workspace check.
        "MoveBlockToTab" => {
            let ws_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let block_id: String = match service::get_arg(args, 1) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let source_tab_id: String = match service::get_arg(args, 2) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let dest_tab_id: String = match service::get_arg(args, 3) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let auto_close: bool = service::get_arg(args, 4).unwrap_or(true);
            tracing::info!(ws_id = %ws_id, block_id = %block_id, source_tab = %source_tab_id, dest_tab = %dest_tab_id, "[dnd:svc] MoveBlockToTab via reducer");
            // codex P2 #622: same-tab requests were no-ops in the
            // prior wcore handler. The reducer's MoveBlock treats
            // same source = dest as an in-tab reorder; with
            // `dst_index: u32::MAX` it would silently move the block
            // to the end of the list. Short-circuit to preserve the
            // prior contract — a `MoveBlockToTab` whose dest equals
            // the source is a UI quirk (e.g. drop on origin tab),
            // not an intentional reorder.
            if source_tab_id == dest_tab_id {
                return WebReturnType::success_empty();
            }
            // Move the block via the reducer. dst_index 0 to mirror
            // wcore::move_block_to_tab which appended at end... wait,
            // wcore appends, so end-of-list. The reducer's MoveBlock
            // clamps dst_index to dst.block_ids.len(); use u32::MAX
            // to land at the end.
            let events = dispatch_to_reducer(
                state,
                agentmux_common::ipc::Command::MoveBlock {
                    block_id: block_id.clone(),
                    src_tab_id: source_tab_id.clone(),
                    dst_tab_id: dest_tab_id.clone(),
                    dst_index: u32::MAX,
                },
            )
            .await;
            if let Some(err_msg) = events.iter().find_map(|e| match e {
                agentmux_common::ipc::Event::Error { message, .. } => Some(message.clone()),
                _ => None,
            }) {
                return WebReturnType::error(err_msg);
            }
            for ev in &events {
                if let Err(e) = crate::persist_subscriber::apply_event_to_wstore(ev, store) {
                    return WebReturnType::error(format!(
                        "MoveBlockToTab: SQLite write failed: {}",
                        e
                    ));
                }
            }
            publish_events(state, &events);
            // Auto-close empty source tab (mirrors wcore::move_block_to_tab).
            if auto_close {
                let should_close = match store.must_get::<Tab>(&source_tab_id) {
                    Ok(t) => t.blockids.is_empty(),
                    Err(_) => false,
                };
                if should_close {
                    let total_tabs = match store.must_get::<Workspace>(&ws_id) {
                        Ok(ws) => ws.tabids.len() + ws.pinnedtabids.len(),
                        Err(_) => 0,
                    };
                    if total_tabs > 1 {
                        let close_events = dispatch_to_reducer(
                            state,
                            agentmux_common::ipc::Command::DeleteTab {
                                workspace_id: ws_id.clone(),
                                tab_id: source_tab_id.clone(),
                                // Auto-close already gated on
                                // `total_tabs > 1` above; reducer's
                                // last-tab guard is defense-in-depth
                                // for the race window.
                                force: false,
                            },
                        )
                        .await;
                        for ev in &close_events {
                            let _ = crate::persist_subscriber::apply_event_to_wstore(ev, store);
                        }
                        publish_events(state, &close_events);
                    }
                }
            }
            let mut updates = vec![];
            if let Ok(src) = store.must_get::<Tab>(&source_tab_id) {
                updates.push(WaveObjUpdate {
                    updatetype: "update".into(),
                    otype: OTYPE_TAB.to_string(),
                    oid: source_tab_id.clone(),
                    obj: Some(wave_obj_to_value(&src)),
                });
            }
            if let Ok(dst) = store.must_get::<Tab>(&dest_tab_id) {
                updates.push(WaveObjUpdate {
                    updatetype: "update".into(),
                    otype: OTYPE_TAB.to_string(),
                    oid: dest_tab_id.clone(),
                    obj: Some(wave_obj_to_value(&dst)),
                });
            }
            if let Ok(ws) = store.must_get::<Workspace>(&ws_id) {
                updates.push(WaveObjUpdate {
                    updatetype: "update".into(),
                    otype: OTYPE_WORKSPACE.to_string(),
                    oid: ws_id.clone(),
                    obj: Some(wave_obj_to_value(&ws)),
                });
            }
            WebReturnType::success_with_updates(updates)
        }
        // Phase E.5.7 — PromoteBlockToTab migrated to saga
        // (CreateTab + MoveBlock). Layout setup + SetActiveTab +
        // auto-close source tab stay wcore-direct here (E.4 layout
        // territory). Same shape as TearOffBlock's RPC handler.
        "PromoteBlockToTab" => {
            let ws_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let block_id: String = match service::get_arg(args, 1) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let source_tab_id: String = match service::get_arg(args, 2) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let auto_close: bool = service::get_arg(args, 3).unwrap_or(true);
            tracing::info!(ws_id = %ws_id, block_id = %block_id, source_tab = %source_tab_id, "[dnd:svc] PromoteBlockToTab via saga");
            let saga_result = crate::sagas::promote_block_to_tab::run(
                state,
                block_id.clone(),
                source_tab_id.clone(),
                ws_id.clone(),
            )
            .await;
            let new_tab_oid = match saga_result {
                Ok(v) => v
                    .get("new_tab_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                Err(reason) => return WebReturnType::error(reason),
            };

            // Layout setup: rootnode + leaforder for the new tab so
            // the frontend renders the moved block correctly. Same
            // helper TearOffBlock uses.
            if let Err(e) = setup_torn_off_block_layout(state, &new_tab_oid, &block_id).await {
                tracing::warn!(new_tab = %new_tab_oid, "PromoteBlockToTab: layout setup failed: {}", e);
            }
            // Source tab: queue layout-delete action.
            if let Err(e) = queue_source_layout_delete(store, &source_tab_id, &block_id) {
                tracing::warn!(source_tab = %source_tab_id, "PromoteBlockToTab: source layout delete-action enqueue failed: {}", e);
            }
            // Set the new tab as active in the workspace via reducer.
            let active_events = dispatch_to_reducer(
                state,
                agentmux_common::ipc::Command::SetActiveTab {
                    workspace_id: ws_id.clone(),
                    tab_id: new_tab_oid.clone(),
                },
            )
            .await;
            for ev in &active_events {
                if let Err(e) = crate::persist_subscriber::apply_event_to_wstore(ev, store) {
                    tracing::warn!("PromoteBlockToTab: SetActiveTab apply failed: {}", e);
                }
            }
            publish_events(state, &active_events);

            // Auto-close empty source tab (mirrors wcore behaviour).
            if auto_close {
                let should_close = match store.must_get::<Tab>(&source_tab_id) {
                    Ok(t) => t.blockids.is_empty(),
                    Err(_) => false,
                };
                if should_close {
                    let total_tabs = match store.must_get::<Workspace>(&ws_id) {
                        Ok(ws) => ws.tabids.len() + ws.pinnedtabids.len(),
                        Err(_) => 0,
                    };
                    if total_tabs > 1 {
                        let close_events = dispatch_to_reducer(
                            state,
                            agentmux_common::ipc::Command::DeleteTab {
                                workspace_id: ws_id.clone(),
                                tab_id: source_tab_id.clone(),
                                // Auto-close already gated on
                                // `total_tabs > 1` above; reducer's
                                // last-tab guard is defense-in-depth
                                // for the race window.
                                force: false,
                            },
                        )
                        .await;
                        for ev in &close_events {
                            let _ = crate::persist_subscriber::apply_event_to_wstore(ev, store);
                        }
                        publish_events(state, &close_events);
                    }
                }
            }

            let mut updates = vec![];
            if let Ok(new_tab) = store.must_get::<Tab>(&new_tab_oid) {
                updates.push(WaveObjUpdate {
                    updatetype: "update".into(),
                    otype: OTYPE_TAB.to_string(),
                    oid: new_tab_oid.clone(),
                    obj: Some(wave_obj_to_value(&new_tab)),
                });
            }
            if let Ok(src) = store.must_get::<Tab>(&source_tab_id) {
                updates.push(WaveObjUpdate {
                    updatetype: "update".into(),
                    otype: OTYPE_TAB.to_string(),
                    oid: source_tab_id.clone(),
                    obj: Some(wave_obj_to_value(&src)),
                });
            }
            if let Ok(ws) = store.must_get::<Workspace>(&ws_id) {
                updates.push(WaveObjUpdate {
                    updatetype: "update".into(),
                    otype: OTYPE_WORKSPACE.to_string(),
                    oid: ws_id.clone(),
                    obj: Some(wave_obj_to_value(&ws)),
                });
            }
            WebReturnType::success_data_updates(
                serde_json::to_value(&new_tab_oid).unwrap_or_default(),
                updates,
            )
        }
        // Phase E.2c.3b — ReorderTab dispatches through the reducer.
        // Forward+compensate isn't useful here (reorder is its own
        // inverse), so on SQLite apply failure we just surface the
        // error and the reducer state ends up ahead of disk for the
        // remainder of the session — converges back at next restart
        // via bootstrap.
        "ReorderTab" => {
            let ws_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let tab_id: String = match service::get_arg(args, 1) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let new_index: usize = match service::get_arg(args, 2) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            tracing::info!(ws_id = %ws_id, tab_id = %tab_id, new_index = %new_index, "[dnd:svc] ReorderTab");
            // Clamp to u32::MAX rather than truncating via `as u32`.
            // The reducer further clamps to `tab_ids.len() - 1` so an
            // absurd usize ends up at the last position — matching
            // the prior `wcore::reorder_tab` behaviour where any
            // out-of-range usize clamped to the end. (codex P3 #617.)
            let new_index_u32 = u32::try_from(new_index).unwrap_or(u32::MAX);
            let events = dispatch_to_reducer(
                state,
                agentmux_common::ipc::Command::ReorderTab {
                    workspace_id: ws_id.clone(),
                    tab_id: tab_id.clone(),
                    new_index: new_index_u32,
                },
            )
            .await;
            // Surface reducer Error events.
            if let Some(err_msg) = events.iter().find_map(|e| match e {
                agentmux_common::ipc::Event::Error { message, .. } => Some(message.clone()),
                _ => None,
            }) {
                return WebReturnType::error(err_msg);
            }
            let mut apply_err: Option<String> = None;
            for ev in &events {
                if let Err(e) = crate::persist_subscriber::apply_event_to_wstore(ev, store) {
                    apply_err = Some(e.to_string());
                    break;
                }
            }
            if let Some(err) = apply_err {
                return WebReturnType::error(format!("ReorderTab: SQLite write failed: {}", err));
            }
            publish_events(state, &events);
            if let Ok(ws) = store.must_get::<Workspace>(&ws_id) {
                let update = WaveObjUpdate {
                    updatetype: "update".into(),
                    otype: OTYPE_WORKSPACE.to_string(),
                    oid: ws_id.clone(),
                    obj: Some(wave_obj_to_value(&ws)),
                };
                WebReturnType::success_with_updates(vec![update])
            } else {
                WebReturnType::success_empty()
            }
        }
        // Phase E.5.5 — MoveTabToWorkspace migrated to dispatch
        // Command::MoveTab through the reducer. Closes codex P1 #621
        // (the saga's reducer-state pre-check rejected tear-off after
        // a wcore-direct cross-window drag had left state.tabs stale)
        // by routing all tab moves through the reducer so its view
        // always matches SQLite.
        "MoveTabToWorkspace" => {
            let tab_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let source_ws_id: String = match service::get_arg(args, 1) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let dest_ws_id: String = match service::get_arg(args, 2) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let insert_index: Option<u32> = service::get_arg::<usize>(args, 3)
                .ok()
                .map(|v| v.try_into().unwrap_or(u32::MAX));
            tracing::info!(tab_id = %tab_id, source_ws = %source_ws_id, dest_ws = %dest_ws_id, insert_index = ?insert_index, "[dnd:svc] MoveTabToWorkspace via reducer");
            // Same-workspace short-circuit matches wcore behaviour.
            // The reducer rejects same-workspace moves outright (use
            // ReorderTab instead); for the RPC contract, treat it as
            // a no-op success so existing callers don't see a
            // behavioural regression.
            if source_ws_id == dest_ws_id {
                return WebReturnType::success_empty();
            }
            // Last-tab guard mirrors wcore::move_tab_to_workspace —
            // the reducer's MoveTab doesn't enforce this (intentionally,
            // for sagas that legitimately drain a workspace to delete
            // it). Keep the guard at the RPC layer where the policy
            // belongs. **Read SQLite, not reducer state** — during the
            // migration window, wcore-direct tab paths
            // (PromoteBlockToTab, etc.) leave reducer.tab_ids stale,
            // so a reducer-state guard would falsely reject valid
            // moves. SQLite is the source of truth (codex P1 round-2
            // #621).
            match store.get::<Workspace>(&source_ws_id) {
                Ok(Some(src_ws)) => {
                    let total_tabs = src_ws.tabids.len() + src_ws.pinnedtabids.len();
                    if total_tabs <= 1 {
                        return WebReturnType::error(
                            "cannot move last tab out of workspace".to_string(),
                        );
                    }
                }
                Ok(None) => {
                    return WebReturnType::error(format!(
                        "MoveTabToWorkspace: source workspace not found: {}",
                        source_ws_id
                    ));
                }
                Err(e) => {
                    return WebReturnType::error(format!(
                        "MoveTabToWorkspace: workspace read failed: {}",
                        e
                    ));
                }
            }
            let dst_index = insert_index.unwrap_or(u32::MAX);
            let events = dispatch_to_reducer(
                state,
                agentmux_common::ipc::Command::MoveTab {
                    tab_id: tab_id.clone(),
                    src_workspace_id: source_ws_id.clone(),
                    dst_workspace_id: dest_ws_id.clone(),
                    dst_index,
                },
            )
            .await;
            if let Some(err_msg) = events.iter().find_map(|e| match e {
                agentmux_common::ipc::Event::Error { message, .. } => Some(message.clone()),
                _ => None,
            }) {
                return WebReturnType::error(err_msg);
            }
            for ev in &events {
                if let Err(e) = crate::persist_subscriber::apply_event_to_wstore(ev, store) {
                    return WebReturnType::error(format!(
                        "MoveTabToWorkspace: SQLite write failed: {}",
                        e
                    ));
                }
            }
            publish_events(state, &events);
            let mut updates = Vec::new();
            if let Ok(src_ws) = store.must_get::<Workspace>(&source_ws_id) {
                updates.push(WaveObjUpdate {
                    updatetype: "update".into(),
                    otype: OTYPE_WORKSPACE.to_string(),
                    oid: source_ws_id.clone(),
                    obj: Some(wave_obj_to_value(&src_ws)),
                });
            }
            if let Ok(dst_ws) = store.must_get::<Workspace>(&dest_ws_id) {
                updates.push(WaveObjUpdate {
                    updatetype: "update".into(),
                    otype: OTYPE_WORKSPACE.to_string(),
                    oid: dest_ws_id.clone(),
                    obj: Some(wave_obj_to_value(&dst_ws)),
                });
            }
            WebReturnType::success_with_updates(updates)
        }
        // Phase E.5.6 — RestoreTornOffTab migrated to saga (MoveTab
        // back + conditional DeleteWorkspaceCascade if source becomes
        // empty). The legacy `was_pinned` arg is ignored — pinning
        // was removed from AgentMux in E.2c.3b; restored tabs always
        // land in `tab_ids`.
        "RestoreTornOffTab" => {
            let tab_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let source_ws_id: String = match service::get_arg(args, 1) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let dest_ws_id: String = match service::get_arg(args, 2) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let insert_index: Option<u32> = service::get_arg::<usize>(args, 3)
                .ok()
                .map(|v| v.try_into().unwrap_or(u32::MAX));
            tracing::info!(tab_id = %tab_id, source_ws = %source_ws_id, dest_ws = %dest_ws_id, insert_index = ?insert_index, "[dnd:svc] RestoreTornOffTab via saga");
            let saga_result = crate::sagas::restore_torn_off_tab::run(
                state,
                tab_id,
                source_ws_id.clone(),
                dest_ws_id.clone(),
                insert_index,
            )
            .await;
            match saga_result {
                Ok(_) => {
                    let mut updates = Vec::new();
                    match store.get::<Workspace>(&source_ws_id) {
                        Ok(Some(src_ws)) => {
                            updates.push(WaveObjUpdate {
                                updatetype: "update".into(),
                                otype: OTYPE_WORKSPACE.to_string(),
                                oid: source_ws_id.clone(),
                                obj: Some(wave_obj_to_value(&src_ws)),
                            });
                        }
                        Ok(None) => {
                            updates.push(WaveObjUpdate {
                                updatetype: "delete".into(),
                                otype: OTYPE_WORKSPACE.to_string(),
                                oid: source_ws_id.clone(),
                                obj: None,
                            });
                        }
                        Err(_) => {}
                    }
                    if let Ok(dst_ws) = store.must_get::<Workspace>(&dest_ws_id) {
                        updates.push(WaveObjUpdate {
                            updatetype: "update".into(),
                            otype: OTYPE_WORKSPACE.to_string(),
                            oid: dest_ws_id.clone(),
                            obj: Some(wave_obj_to_value(&dst_ws)),
                        });
                    }
                    WebReturnType::success_with_updates(updates)
                }
                Err(reason) => WebReturnType::error(reason),
            }
        }
        // Phase E.5.5 — TearOffBlock migrated to saga (reducer-state
        // portion: CreateWorkspace + CreateTab + MoveBlock). Layout
        // tree setup on the new tab + queueing the source tab's
        // layout-delete action stay wcore-direct here — layout state
        // is E.4 work, separately scoped. The saga's atomicity is
        // limited to the reducer-state portion; layout writes are
        // best-effort and can leave a torn-off block with a malformed
        // layout if the post-saga step fails. Acceptable trade-off
        // for the smoke regression fix; full atomicity is a Phase F+
        // gap (see saga-coordinator-location-analysis-2026-04-30.md
        // §4.2).
        "TearOffBlock" => {
            let block_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let source_tab_id: String = match service::get_arg(args, 1) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let source_ws_id: String = match service::get_arg(args, 2) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let auto_close: bool = service::get_arg(args, 3).unwrap_or(true);
            tracing::info!(block_id = %block_id, source_tab = %source_tab_id, source_ws = %source_ws_id, "[dnd:svc] TearOffBlock via saga");
            let saga_result = crate::sagas::tear_off_block::run(
                state,
                block_id.clone(),
                source_tab_id.clone(),
                source_ws_id.clone(),
            )
            .await;
            let (new_ws_oid, new_tab_oid) = match saga_result {
                Ok(value) => {
                    let new_ws_oid = value
                        .get("new_workspace_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let new_tab_oid = value
                        .get("new_tab_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    (new_ws_oid, new_tab_oid)
                }
                Err(reason) => return WebReturnType::error(reason),
            };

            // Layout setup for the new tab — make the moved block its
            // single root node so the frontend renders it. Mirrors
            // wcore::tear_off_block. Best-effort; layout migration is
            // E.4 territory and not yet reducer-routed.
            if let Err(e) = setup_torn_off_block_layout(state, &new_tab_oid, &block_id).await {
                tracing::warn!(new_tab = %new_tab_oid, "TearOffBlock: layout setup failed: {} (block in tab but layout malformed)", e);
            }
            // Source tab: queue a layout-delete action so the source
            // window's frontend removes the node from its tree.
            if let Err(e) = queue_source_layout_delete(store, &source_tab_id, &block_id) {
                tracing::warn!(source_tab = %source_tab_id, "TearOffBlock: source layout delete-action enqueue failed: {}", e);
            }

            // Auto-close empty source tab. Route through the reducer
            // (DeleteTab cascade is built in; the tab has no blocks
            // at this point — we just moved the only one out). Skip
            // when source workspace would become empty.
            if auto_close {
                let should_close = match store.must_get::<Tab>(&source_tab_id) {
                    Ok(t) => t.blockids.is_empty(),
                    Err(_) => false,
                };
                if should_close {
                    let total_tabs = match store.must_get::<Workspace>(&source_ws_id) {
                        Ok(ws) => ws.tabids.len() + ws.pinnedtabids.len(),
                        Err(_) => 0,
                    };
                    if total_tabs > 1 {
                        tracing::info!(source_tab = %source_tab_id, "[dnd:svc] auto-closing empty source tab after TearOffBlock");
                        let close_events = dispatch_to_reducer(
                            state,
                            agentmux_common::ipc::Command::DeleteTab {
                                workspace_id: source_ws_id.clone(),
                                tab_id: source_tab_id.clone(),
                                // Auto-close already gated on
                                // total_tabs > 1.
                                force: false,
                            },
                        )
                        .await;
                        for ev in &close_events {
                            let _ = crate::persist_subscriber::apply_event_to_wstore(ev, store);
                        }
                        publish_events(state, &close_events);
                    }
                }
            }

            let mut updates = Vec::new();
            if let Ok(src_tab) = store.must_get::<Tab>(&source_tab_id) {
                updates.push(WaveObjUpdate {
                    updatetype: "update".into(),
                    otype: OTYPE_TAB.to_string(),
                    oid: source_tab_id.clone(),
                    obj: Some(wave_obj_to_value(&src_tab)),
                });
            }
            if let Ok(src_ws) = store.must_get::<Workspace>(&source_ws_id) {
                updates.push(WaveObjUpdate {
                    updatetype: "update".into(),
                    otype: OTYPE_WORKSPACE.to_string(),
                    oid: source_ws_id.clone(),
                    obj: Some(wave_obj_to_value(&src_ws)),
                });
            }
            if let Ok(new_ws) = store.must_get::<Workspace>(&new_ws_oid) {
                updates.push(WaveObjUpdate {
                    updatetype: "update".into(),
                    otype: OTYPE_WORKSPACE.to_string(),
                    oid: new_ws_oid.clone(),
                    obj: Some(wave_obj_to_value(&new_ws)),
                });
            }
            WebReturnType::success_data_updates(
                serde_json::to_value(&new_ws_oid).unwrap_or_default(),
                updates,
            )
        }
        // Floating-pane Phase 4a — re-dock a floater's block back into
        // an existing tab in another workspace. Same shape as
        // TearOffBlock's RPC handler: saga handles the reducer-state
        // move (MoveBlock); layout writes are wcore-direct (the
        // target's layout grows a leaf; the source's layout enqueues
        // a delete action). Source floater closes via PR #1089's
        // empty-tab watcher once its tab.blockids is empty.
        // Spec: docs/specs/SPEC_FLOATING_PANE_REDOCK_2026-05-27.md
        "RedockFloatingPane" => {
            let block_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let source_tab_id: String = match service::get_arg(args, 1) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let source_ws_id: String = match service::get_arg(args, 2) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let target_tab_id: String = match service::get_arg(args, 3) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let target_ws_id: String = match service::get_arg(args, 4) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            // Phase 4b — optional direction hint from the ghost overlay.
            // Both must be present; if either is absent fall back to InsertNode.
            let target_block_id: Option<String> = service::get_optional_arg(args, 5)
                .unwrap_or(None);
            let direction: Option<u8> = service::get_optional_arg(args, 6)
                .unwrap_or(None);
            tracing::info!(
                block_id = %block_id,
                source_tab = %source_tab_id,
                source_ws = %source_ws_id,
                target_tab = %target_tab_id,
                target_ws = %target_ws_id,
                target_block_id = ?target_block_id,
                direction = ?direction,
                "[dnd:svc] RedockFloatingPane via saga"
            );
            let saga_result = crate::sagas::redock_floating_pane::run(
                state,
                block_id.clone(),
                source_tab_id.clone(),
                source_ws_id.clone(),
                target_tab_id.clone(),
                target_ws_id.clone(),
                None,
            )
            .await;
            if let Err(reason) = saga_result {
                return WebReturnType::error(reason);
            }

            // Target layout: enqueue the appropriate action on its
            // `pendingbackendactions`. Phase 4b: when a direction hint is
            // present, queue SplitHorizontal/SplitVertical so the block lands
            // in the exact slot the ghost previewed. Otherwise fall back to
            // the generic InsertNode (Phase 4a behavior).
            // Layout writes are required before the Tab broadcast — if either
            // fails the block becomes invisible. Return error so the caller can
            // retry; no visible change has propagated to the renderers yet.
            let target_layout_result = match (target_block_id.as_deref(), direction) {
                (Some(tbid), Some(dir)) => {
                    queue_target_layout_split(store, &target_tab_id, &block_id, tbid, dir)
                }
                _ => queue_target_layout_insert(store, &target_tab_id, &block_id),
            };
            if let Err(e) = target_layout_result {
                tracing::error!(
                    target_tab = %target_tab_id,
                    "RedockFloatingPane: target layout action failed — aborting broadcast: {}",
                    e
                );
                return WebReturnType::error(format!(
                    "redock layout action failed: {e}"
                ));
            }
            if let Err(e) = queue_source_layout_delete(store, &source_tab_id, &block_id) {
                tracing::error!(
                    source_tab = %source_tab_id,
                    "RedockFloatingPane: source layout delete failed — aborting broadcast: {}",
                    e
                );
                return WebReturnType::error(format!(
                    "redock layout delete failed: {e}"
                ));
            }

            let mut updates = Vec::new();
            // Layout updates MUST come first — `append_block_to_target_layout`
            // and `queue_source_layout_delete` write straight to wstore via
            // `store.update`, which is NOT auto-broadcast (only the SQLite
            // row gets a new version). Without these entries in the response,
            // the target window's frontend never sees the new leaf and
            // renders nothing; the source's pending delete action never
            // gets pulled either. Both layouts are read AFTER the helpers
            // run so we capture the fresh state.
            if let Ok(src_tab) = store.must_get::<Tab>(&source_tab_id) {
                if let Ok(src_layout) = store.must_get::<LayoutState>(&src_tab.layoutstate) {
                    updates.push(WaveObjUpdate {
                        updatetype: "update".into(),
                        otype: OTYPE_LAYOUT.to_string(),
                        oid: src_tab.layoutstate.clone(),
                        obj: Some(wave_obj_to_value(&src_layout)),
                    });
                }
                updates.push(WaveObjUpdate {
                    updatetype: "update".into(),
                    otype: OTYPE_TAB.to_string(),
                    oid: source_tab_id.clone(),
                    obj: Some(wave_obj_to_value(&src_tab)),
                });
            }
            if let Ok(dst_tab) = store.must_get::<Tab>(&target_tab_id) {
                if let Ok(dst_layout) = store.must_get::<LayoutState>(&dst_tab.layoutstate) {
                    updates.push(WaveObjUpdate {
                        updatetype: "update".into(),
                        otype: OTYPE_LAYOUT.to_string(),
                        oid: dst_tab.layoutstate.clone(),
                        obj: Some(wave_obj_to_value(&dst_layout)),
                    });
                }
                updates.push(WaveObjUpdate {
                    updatetype: "update".into(),
                    otype: OTYPE_TAB.to_string(),
                    oid: target_tab_id.clone(),
                    obj: Some(wave_obj_to_value(&dst_tab)),
                });
            }
            // CRITICAL: WaveObjUpdates in the response only reach the
            // CALLING renderer (the floater that's about to close). The
            // TARGET window's renderer is a different process and won't
            // see the layout change unless we explicitly broadcast on
            // the event bus. Mirrors the pattern in `app_api.rs:399-410`.
            // Without this the target tab.blockids includes the new
            // block but its layout.leaforder doesn't → block invisible.
            for update in &updates {
                let oref = format!("{}:{}", update.otype, update.oid);
                if let Ok(data) = serde_json::to_value(update) {
                    state.event_bus.broadcast_event(
                        &crate::backend::eventbus::WSEventType {
                            eventtype: "waveobj:update".to_string(),
                            oref,
                            data: Some(data),
                        },
                    );
                }
            }

            WebReturnType::success_data_updates(
                serde_json::json!({
                    "redocked": true,
                    "block_id": block_id,
                    "target_tab_id": target_tab_id,
                }),
                updates,
            )
        }

        // Phase E.5.5 — TearOffTab migrated to saga. Closes the
        // smoke regression where wcore::tear_off_tab created the new
        // workspace bypassing the reducer, leaving the new window's
        // CreateTab/etc. calls failing on "workspace not found"
        // checks against the reducer's stale view.
        "TearOffTab" => {
            let tab_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let source_ws_id: String = match service::get_arg(args, 1) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            tracing::info!(tab_id = %tab_id, source_ws = %source_ws_id, "[dnd:svc] TearOffTab via saga");
            match crate::sagas::tear_off_tab::run(state, tab_id, source_ws_id.clone()).await {
                Ok(saga_result) => {
                    let new_ws_oid = saga_result
                        .get("new_workspace_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let mut updates = Vec::new();
                    if let Ok(src_ws) = store.must_get::<Workspace>(&source_ws_id) {
                        updates.push(WaveObjUpdate {
                            updatetype: "update".into(),
                            otype: OTYPE_WORKSPACE.to_string(),
                            oid: source_ws_id.clone(),
                            obj: Some(wave_obj_to_value(&src_ws)),
                        });
                    }
                    if let Ok(new_ws) = store.must_get::<Workspace>(&new_ws_oid) {
                        updates.push(WaveObjUpdate {
                            updatetype: "update".into(),
                            otype: OTYPE_WORKSPACE.to_string(),
                            oid: new_ws_oid.clone(),
                            obj: Some(wave_obj_to_value(&new_ws)),
                        });
                    }
                    WebReturnType::success_data_updates(
                        serde_json::to_value(&new_ws_oid).unwrap_or_default(),
                        updates,
                    )
                }
                Err(reason) => WebReturnType::error(reason),
            }
        }
        _ => WebReturnType::error(format!("unknown workspace method: {}", call.method)),
    }
}
