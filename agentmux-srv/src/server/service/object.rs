// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `object` service handler (GetObject, CreateBlock, UpdateObject, …).

use crate::backend::obj::*;
use crate::backend::service::{self, WebCallType, WebReturnType};

use super::super::AppState;
use super::object_helpers::{
    find_tab_for_layout, get_object_by_oref, schedule_agent_zoom_mirror, update_object,
    update_object_meta,
};
use super::reducer_helpers::{compensate_via_reducer, dispatch_to_reducer, publish_events};

pub(super) async fn handle_object_service(state: &AppState, call: &WebCallType) -> WebReturnType {
    let store = &state.wstore;
    let args = &call.args;
    match call.method.as_str() {
        "GetObject" => {
            let oref_str: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            match get_object_by_oref(store, &oref_str) {
                Ok(data) => WebReturnType::success(data),
                Err(e) => WebReturnType::error(e),
            }
        }
        "GetObjects" => {
            let orefs: Vec<String> = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let mut results = Vec::new();
            for oref_str in &orefs {
                match get_object_by_oref(store, oref_str) {
                    Ok(data) => results.push(data),
                    Err(_) => results.push(serde_json::Value::Null),
                }
            }
            WebReturnType::success(serde_json::json!(results))
        }
        "CreateBlock" => {
            let block_def: BlockDef = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            // Optional explicit tab_id at args[2] (args[1] is rtOpts).
            // When present, overrides uicontext.active_tab_id — lets
            // callers like applyTabPreset (frontend) target a specific
            // tab without depending on which tab happens to be active
            // when the RPC's uicontext is serialised. Eliminates the
            // TOCTOU race where the user can switch tabs between the
            // call site and the server-side handler.
            //
            // A *malformed* args[2] (e.g. non-string from a stale SDK)
            // returns an error — silently falling back to uicontext
            // would defeat the explicit-targeting contract and make
            // wrong-tab routing hard to diagnose. Missing/null/empty
            // is fine: treat as "no override" and use uicontext.
            let explicit_tab_id: Option<String> = match service::get_optional_arg::<String>(args, 2) {
                Ok(opt) => opt.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
                Err(e) => return WebReturnType::error(format!("invalid tabId arg: {}", e)),
            };
            let tab_id = match explicit_tab_id {
                Some(id) => id,
                None => match call
                    .uicontext
                    .as_ref()
                    .map(|ctx| ctx.active_tab_id.clone())
                {
                    Some(id) if !id.is_empty() => id,
                    _ => return WebReturnType::error("missing uicontext.activetabid"),
                },
            };
            // Phase E.2c.4 — CreateBlock dispatches through the reducer
            // (forward+compensate on SQLite failure). The reducer
            // assigns the block_id; the persist subscriber's apply
            // path writes the Block row with the caller's meta map.
            let meta_value =
                serde_json::to_value(&block_def.meta).unwrap_or(serde_json::Value::Null);
            let events = dispatch_to_reducer(
                state,
                agentmux_common::ipc::Command::CreateBlock {
                    tab_id: tab_id.clone(),
                    meta: meta_value,
                },
            )
            .await;
            // Surface reducer Error events (tab not found).
            if let Some(err_msg) = events.iter().find_map(|e| match e {
                agentmux_common::ipc::Event::Error { message, .. } => Some(message.clone()),
                _ => None,
            }) {
                return WebReturnType::error(err_msg);
            }
            let block_id = events.iter().find_map(|e| match e {
                agentmux_common::ipc::Event::BlockCreated { block_id, .. } => {
                    Some(block_id.clone())
                }
                _ => None,
            });
            let mut apply_err: Option<String> = None;
            for ev in &events {
                if let Err(e) = crate::persist_subscriber::apply_event_to_wstore(ev, store) {
                    apply_err = Some(e.to_string());
                    break;
                }
            }
            if let Some(err) = apply_err {
                if let Some(bid) = block_id.as_ref() {
                    compensate_via_reducer(
                        state,
                        agentmux_common::ipc::Command::DeleteBlock {
                            tab_id: tab_id.clone(),
                            block_id: bid.clone(),
                        },
                        store,
                    )
                    .await;
                }
                return WebReturnType::error(format!("CreateBlock: SQLite write failed: {}", err));
            }
            publish_events(state, &events);
            match block_id {
                Some(bid) => {
                    let mut updates = vec![];
                    if let Ok(block) = store.must_get::<Block>(&bid) {
                        updates.push(WaveObjUpdate {
                            updatetype: "update".into(),
                            otype: OTYPE_BLOCK.to_string(),
                            oid: bid.clone(),
                            obj: Some(wave_obj_to_value(&block)),
                        });
                    }
                    if let Ok(tab) = store.must_get::<Tab>(&tab_id) {
                        updates.push(WaveObjUpdate {
                            updatetype: "update".into(),
                            otype: OTYPE_TAB.to_string(),
                            oid: tab_id.clone(),
                            obj: Some(wave_obj_to_value(&tab)),
                        });
                    }
                    WebReturnType::success_data_updates(serde_json::json!(bid), updates)
                }
                None => WebReturnType::error(
                    "CreateBlock: reducer did not emit BlockCreated".to_string(),
                ),
            }
        }
        // Phase E.5.7 (Step 5 PR 1) — DeleteBlock saga. The legacy
        // SQLite-first pattern (wcore::delete_block + reducer-sync
        // dispatch) is replaced by `sagas::delete_block::run`, which
        // routes through the reducer + persist subscriber. The saga
        // also handles the controller-kill cascade (matches the old
        // ordering: controller down → reducer dispatch → SQLite).
        "DeleteBlock" => {
            let block_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            // Look up the block's owning tab from server state rather than using
            // uicontext.active_tab_id. Floating pane windows have their own tab
            // context that differs from the block's owning tab, so using the
            // uicontext tab always fails for floating-pane closes.
            let tab_id = {
                let s = state.srv_state.lock().await;
                match s.blocks.get(&block_id) {
                    Some(rec) => rec.tab_id.clone(),
                    None => return WebReturnType::error(format!("DeleteBlock: block not found: {}", block_id)),
                }
            };
            if let Err(reason) = crate::sagas::delete_block::run(state, tab_id, block_id).await {
                return WebReturnType::error(reason);
            }
            WebReturnType::success_empty()
        }
        "UpdateObject" => {
            let wave_obj_value: serde_json::Value = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            // SPEC_864 Phase 2 — an OTYPE_LAYOUT push routes through the
            // reducer as a single `LayoutSetTree` (tree + client slices),
            // replacing the legacy `update_raw` whole-row write PLUS the
            // separate SetFocusedNode/SetMagnifiedNode dispatches (the
            // "double-write": two SQLite writes, two version bumps, per
            // push, with `TabRecord.rootnode` left a stale shadow). One
            // dispatch → one persist-subscriber write → one version bump,
            // and the reducer's in-memory tree stays authoritative.
            //
            // Fallback: an unowned layout row (no tab references it) or a
            // rootnode that fails typed deserialization takes the legacy
            // wcore-direct path below, loudly — never silently drop a push.
            // For an OWNED row whose rootnode failed the typed parse, the
            // legacy path must still dispatch the focus/magnify slice to
            // the reducer (the pre-Phase-2 Option-A behavior) so
            // `TabRecord` focus state can't silently diverge on that
            // branch (reagent P1 #1970, review 3).
            let mut fallback_slice: Option<(String, String, String)> = None;
            if wave_obj_value.get("otype").and_then(|v| v.as_str()) == Some(OTYPE_LAYOUT) {
                let layout_route: Option<(String, Option<agentmux_common::LayoutNode>)> =
                    match wave_obj_value
                        .get("oid")
                        .and_then(|v| v.as_str())
                        .and_then(|layout_oid| find_tab_for_layout(store, layout_oid))
                    {
                        Some(tab_id) => match wave_obj_value.get("rootnode") {
                            None | Some(serde_json::Value::Null) => Some((tab_id, None)),
                            Some(v) => match serde_json::from_value(v.clone()) {
                                Ok(tree) => Some((tab_id, Some(tree))),
                                Err(e) => {
                                    tracing::warn!(
                                        error = %e,
                                        "UpdateObject: layout rootnode failed typed parse; \
                                         falling back to legacy wcore-direct write"
                                    );
                                    let get_str = |key: &str| -> String {
                                        wave_obj_value
                                            .get(key)
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("")
                                            .to_string()
                                    };
                                    fallback_slice = Some((
                                        tab_id,
                                        get_str("focusednodeid"),
                                        get_str("magnifiednodeid"),
                                    ));
                                    None
                                }
                            },
                        },
                        None => {
                            tracing::warn!(
                                "UpdateObject: layout row not owned by any tab; \
                                 falling back to legacy wcore-direct write"
                            );
                            None
                        }
                    };
                if let Some((tab_id, new_tree)) = layout_route {
                    return update_layout_via_reducer(state, wave_obj_value, tab_id, new_tree)
                        .await;
                }
            }
            // Non-layout otypes and the layout fallbacks: legacy wholesale
            // row replace. The unowned-row fallback has no reducer-known
            // tab, so there is nothing to dispatch; the owned-but-unparsable
            // fallback dispatches the focus/magnify slice AFTER the write
            // succeeds (failure-atomicity ordering per codex P2 PR #632).
            match update_object(store, wave_obj_value) {
                Ok((otype, oid, obj_val)) => {
                    if let Some((tab_id, new_focused, new_magnified)) = fallback_slice {
                        let focus_events = dispatch_to_reducer(
                            state,
                            agentmux_common::ipc::Command::SetFocusedNode {
                                tab_id: tab_id.clone(),
                                node_id: new_focused,
                            },
                        )
                        .await;
                        publish_events(state, &focus_events);
                        let mag_events = dispatch_to_reducer(
                            state,
                            agentmux_common::ipc::Command::SetMagnifiedNode {
                                tab_id,
                                node_id: new_magnified,
                            },
                        )
                        .await;
                        publish_events(state, &mag_events);
                    }
                    let update = WaveObjUpdate {
                        updatetype: "update".into(),
                        otype,
                        oid,
                        obj: Some(obj_val),
                    };
                    WebReturnType::success_with_updates(vec![update])
                }
                Err(e) => WebReturnType::error(e),
            }
        }
        // Phase E.5.3 — UpdateObjectMeta migrated through the
        // reducer. Decomposes by otype to the typed Update*Meta
        // command. Reducer is pass-through (validates entity exists;
        // emits event); subscriber's apply_*_meta_updated does the
        // shallow merge against wstore.
        "UpdateObjectMeta" => {
            let oref_str: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let meta_update: MetaMapType = match service::get_arg(args, 1) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let oref = match crate::backend::ORef::parse(&oref_str) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e.to_string()),
            };
            let meta_value = serde_json::to_value(&meta_update).unwrap_or(serde_json::Value::Null);
            let cmd = match oref.otype.as_str() {
                t if t == OTYPE_WORKSPACE => agentmux_common::ipc::Command::UpdateWorkspaceMeta {
                    workspace_id: oref.oid.clone(),
                    meta_patch: meta_value,
                },
                t if t == OTYPE_TAB => agentmux_common::ipc::Command::UpdateTabMeta {
                    tab_id: oref.oid.clone(),
                    meta_patch: meta_value,
                },
                t if t == OTYPE_BLOCK => agentmux_common::ipc::Command::UpdateBlockMeta {
                    block_id: oref.oid.clone(),
                    meta_patch: meta_value,
                },
                t if t == OTYPE_WINDOW => agentmux_common::ipc::Command::UpdateWindowMeta {
                    window_id: oref.oid.clone(),
                    meta_patch: meta_value,
                },
                other => {
                    // Remaining otypes (Layout, Client, Temp) aren't
                    // meta-mutated via the reducer yet; fall back to
                    // wcore for forward-compat. They publish no event,
                    // so the WaveObjUpdate bridge can't see them — the
                    // frontend cache stays stale until next bootstrap
                    // (deemed acceptable since these aren't user-edited).
                    // Future Phase E.5.x migrations can add reducer arms
                    // for any of these following the OTYPE_WINDOW pattern
                    // above (per issue #855 retro).
                    return match update_object_meta(store, &oref_str, &meta_update) {
                        Ok(()) => WebReturnType::success_empty(),
                        Err(e) => WebReturnType::error(format!(
                            "UpdateObjectMeta: unsupported otype {} via reducer; wcore fallback failed: {}",
                            other, e
                        )),
                    };
                }
            };
            let events = dispatch_to_reducer(state, cmd).await;
            if let Some(err_msg) = events.iter().find_map(|e| match e {
                agentmux_common::ipc::Event::Error { message, .. } => Some(message.clone()),
                _ => None,
            }) {
                return WebReturnType::error(err_msg);
            }
            for ev in &events {
                if let Err(e) = crate::persist_subscriber::apply_event_to_wstore(ev, store) {
                    return WebReturnType::error(format!(
                        "UpdateObjectMeta: SQLite write failed: {}",
                        e
                    ));
                }
            }
            publish_events(state, &events);
            // Per-agent zoom persistence (SPEC_AGENT_ZOOM_PERSISTENCE): when an
            // agent block's `term:zoom` changes, mirror it (debounced) into the
            // agent's per-agent `ui:zoom` content so the zoom survives the block
            // and is restored at `agent.open`. Only blocks carrying `agentId`
            // (agent panes) participate; everything else is untouched. A `null`
            // `term:zoom` (the frontend's reset-to-1.0 convention) deletes the
            // saved value so a default agent stores nothing.
            if oref.otype == OTYPE_BLOCK && meta_update.contains_key("term:zoom") {
                if let Ok(block) = store.must_get::<Block>(&oref.oid) {
                    let agent_id = block
                        .meta
                        .get("agentId")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    if !agent_id.is_empty() {
                        let zoom = meta_update.get("term:zoom").and_then(|v| v.as_f64());
                        schedule_agent_zoom_mirror(store.clone(), agent_id, zoom);
                    }
                }
            }
            // Return the updated object so the frontend WOS cache stays in sync.
            if oref.otype == OTYPE_BLOCK {
                if let Ok(block) = store.must_get::<Block>(&oref.oid) {
                    return WebReturnType::success_with_updates(vec![WaveObjUpdate {
                        updatetype: "update".into(),
                        otype: OTYPE_BLOCK.to_string(),
                        oid: oref.oid.clone(),
                        obj: Some(wave_obj_to_value(&block)),
                    }]);
                }
            }
            if oref.otype == OTYPE_TAB {
                if let Ok(tab) = store.must_get::<Tab>(&oref.oid) {
                    return WebReturnType::success_with_updates(vec![WaveObjUpdate {
                        updatetype: "update".into(),
                        otype: OTYPE_TAB.to_string(),
                        oid: oref.oid.clone(),
                        obj: Some(wave_obj_to_value(&tab)),
                    }]);
                }
            }
            WebReturnType::success_empty()
        }
        // Phase E.5.3 — UpdateTabName migrated through the reducer.
        "UpdateTabName" => {
            let tab_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let name: String = match service::get_arg(args, 1) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let events = dispatch_to_reducer(
                state,
                agentmux_common::ipc::Command::RenameTab {
                    tab_id: tab_id.clone(),
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
                        "UpdateTabName: SQLite write failed: {}",
                        e
                    ));
                }
            }
            publish_events(state, &events);
            if let Ok(updated_tab) = store.must_get::<Tab>(&tab_id) {
                let update = WaveObjUpdate {
                    updatetype: "update".into(),
                    otype: OTYPE_TAB.to_string(),
                    oid: tab_id.clone(),
                    obj: Some(wave_obj_to_value(&updated_tab)),
                };
                return WebReturnType::success_with_updates(vec![update]);
            }
            WebReturnType::success_empty()
        }
        _ => WebReturnType::error(format!("unknown object method: {}", call.method)),
    }
}

/// SPEC_864 Phase 2 — route a frontend `UpdateObject` layout push through
/// the reducer as a single `LayoutSetTree` dispatch.
///
/// Dispatch AND SQLite apply happen under ONE hold of the `srv_state`
/// mutex (reagent P1 #1970): with `dispatch_to_reducer`'s usual
/// release-before-I/O contract, two concurrent pushes for the same tab
/// could dispatch in order A→B but persist B→A, leaving `db_layout` on
/// the older tree while `TabRecord` (authoritative as of this route)
/// holds the newer one — the exact coherence this route exists to
/// guarantee, and a regression vs. the legacy single atomic `update_raw`.
/// Holding the lock across the row UPDATE (a sub-ms local SQLite write)
/// makes persist order equal dispatch order; the SQLite-failure rollback
/// re-dispatch also runs inside the same hold so no interleaved dispatch
/// can observe the un-rolled-back state. Publish happens after release
/// (the wave-obj bridge re-reads the row, so publish must follow the
/// write; subscribers never see events out of dispatch order because
/// publish order still matches — see below).
///
/// Not carried: `LayoutState.meta`. The legacy whole-row write persisted
/// it incidentally, but nothing mutates layout meta (`UpdateObjectMeta`
/// rejects OTYPE_LAYOUT and the frontend's `persistToBackend` writes only
/// tree/focus/magnify/leaforder/pendingbackendactions), so the reducer
/// route leaves the stored meta untouched.
async fn update_layout_via_reducer(
    state: &AppState,
    wave_obj_value: serde_json::Value,
    tab_id: String,
    new_tree: Option<agentmux_common::LayoutNode>,
) -> WebReturnType {
    let store = &state.wstore;
    let oid = wave_obj_value
        .get("oid")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    let get_str = |key: &str| -> String {
        wave_obj_value
            .get(key)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    let get_json = |key: &str| -> Option<serde_json::Value> {
        wave_obj_value.get(key).filter(|v| !v.is_null()).cloned()
    };
    let slices = agentmux_common::LayoutClientSlices {
        leaforder: get_json("leaforder"),
        focused_node_id: get_str("focusednodeid"),
        magnified_node_id: get_str("magnifiednodeid"),
        pending_backend_actions: get_json("pendingbackendactions"),
    };
    let cmd = agentmux_common::ipc::Command::LayoutSetTree {
        tab_id: tab_id.clone(),
        new_tree,
        correlation_id: String::new(),
        slices: Some(slices),
    };

    // ── single critical section: snapshot + dispatch + persist (+ rollback) ──
    let mut apply_err: Option<String> = None;
    let mut early_err: Option<String> = None;
    let events = {
        let mut s = state.srv_state.lock().await;
        // Snapshot the current row for the rollback path INSIDE the
        // critical section (reagent P1 #1970, review 4): a pre-lock
        // snapshot could be overtaken by a concurrent push committing
        // between the read and this lock acquisition — a subsequent
        // rollback would then restore the STALE snapshot, clobbering the
        // concurrently-committed newer state in both TabRecord and
        // db_layout. Read under the lock, immediately before the forward
        // dispatch, no interleaving is possible.
        let old_row = match store.get::<LayoutState>(&oid) {
            Ok(Some(row)) => row,
            Ok(None) => {
                early_err = Some(format!("UpdateObject: layout not found: {}", oid));
                LayoutState::default()
            }
            Err(e) => {
                early_err = Some(format!("UpdateObject: {}", e));
                LayoutState::default()
            }
        };
        if early_err.is_some() {
            Vec::new()
        } else {
            let ctx = crate::reducer::Ctx {
                now_rfc3339: chrono::Utc::now().to_rfc3339(),
                registered_pid: None,
            };
            let events = crate::reducer::update(&mut s, cmd, &ctx);
            let has_error = events
                .iter()
                .any(|e| matches!(e, agentmux_common::ipc::Event::Error { .. }));
            if !has_error {
                for ev in &events {
                    if let Err(e) = crate::persist_subscriber::apply_event_to_wstore(ev, store) {
                        apply_err = Some(e.to_string());
                        break;
                    }
                }
                if apply_err.is_some() {
                    // Roll the reducer back to the pre-push row inside the
                    // same lock hold so no concurrent dispatch can observe
                    // the divergent state; best-effort mirror to SQLite.
                    let rollback_cmd = agentmux_common::ipc::Command::LayoutSetTree {
                        tab_id,
                        new_tree: old_row.rootnode.clone(),
                        correlation_id: String::new(),
                        slices: Some(agentmux_common::LayoutClientSlices {
                            leaforder: old_row
                                .leaforder
                                .as_ref()
                                .and_then(|v| serde_json::to_value(v).ok()),
                            focused_node_id: old_row.focusednodeid.clone(),
                            magnified_node_id: old_row.magnifiednodeid.clone(),
                            pending_backend_actions: old_row
                                .pendingbackendactions
                                .as_ref()
                                .and_then(|v| serde_json::to_value(v).ok()),
                        }),
                    };
                    let rb_events = crate::reducer::update(&mut s, rollback_cmd, &ctx);
                    for ev in &rb_events {
                        if let Err(e) =
                            crate::persist_subscriber::apply_event_to_wstore(ev, store)
                        {
                            tracing::warn!(
                                error = %e,
                                "UpdateObject: layout rollback SQLite mirror failed"
                            );
                        }
                    }
                }
            }
            events
        }
    };

    if let Some(err) = early_err {
        return WebReturnType::error(err);
    }
    if let Some(err_msg) = events.iter().find_map(|e| match e {
        agentmux_common::ipc::Event::Error { message, .. } => Some(message.clone()),
        _ => None,
    }) {
        return WebReturnType::error(err_msg);
    }
    if let Some(err) = apply_err {
        return WebReturnType::error(format!("UpdateObject: SQLite write failed: {}", err));
    }
    publish_events(state, &events);

    // Return the committed row (fresh version) so the pusher's WOS cache
    // stays in sync — same response shape as the legacy path.
    match get_object_by_oref(store, &format!("{}:{}", OTYPE_LAYOUT, oid)) {
        Ok(obj_val) => WebReturnType::success_with_updates(vec![WaveObjUpdate {
            updatetype: "update".into(),
            otype: OTYPE_LAYOUT.to_string(),
            oid,
            obj: Some(obj_val),
        }]),
        Err(e) => WebReturnType::error(format!("UpdateObject: re-read failed: {}", e)),
    }
}
