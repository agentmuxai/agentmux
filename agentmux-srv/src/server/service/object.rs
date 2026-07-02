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
            // Phase E.4 (Option A) — when a LayoutState update lands,
            // route the focused/magnified slice through the srv reducer
            // so its canonical state matches what the frontend just
            // pushed and the persist subscriber emits the new
            // FocusedNodeChanged / MagnifiedNodeChanged events for E.6
            // dispatcher consumption. The remaining LayoutState fields
            // (rootnode/leaforder/pendingbackendactions) keep the
            // wcore-direct write below per the deferred Option B
            // decision in `SPEC_PHASE_E4_LAYOUT_REDUCER_2026-05-01.md`.
            //
            // (codex P2 PR #632) Capture the slice now but DO NOT
            // dispatch yet — reducer + subscriber updates must happen
            // ONLY AFTER update_object succeeds. Otherwise an
            // UpdateObject failure would leave reducer state and
            // FocusedNodeChanged/MagnifiedNodeChanged events fired for
            // a request that returned an error, breaking failure
            // atomicity.
            let layout_slice: Option<(String, String, String)> = if wave_obj_value
                .get("otype")
                .and_then(|v| v.as_str())
                == Some(OTYPE_LAYOUT)
            {
                wave_obj_value
                    .get("oid")
                    .and_then(|v| v.as_str())
                    .and_then(|layout_oid| find_tab_for_layout(store, layout_oid))
                    .map(|tab_id| {
                        let new_focused = wave_obj_value
                            .get("focusednodeid")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let new_magnified = wave_obj_value
                            .get("magnifiednodeid")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        (tab_id, new_focused, new_magnified)
                    })
            } else {
                None
            };
            match update_object(store, wave_obj_value) {
                Ok((otype, oid, obj_val)) => {
                    // DB write succeeded — now dispatch the layout
                    // reducer updates so reducer state and persist-
                    // subscriber events stay aligned with the
                    // committed wstore state. (codex P2 PR #632)
                    if let Some((tab_id, new_focused, new_magnified)) = layout_slice {
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
