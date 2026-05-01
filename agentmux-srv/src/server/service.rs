
use axum::{extract::State, response::Json};
use serde_json::json;

use crate::backend::blockcontroller;
use crate::backend::service::{self, CloseTabRtnType, WebCallType, WebReturnType};
use crate::backend::storage::wstore::WaveStore;
use crate::backend::obj::*;
use crate::backend::wcore;

use super::AppState;

pub(super) async fn handle_service(
    State(state): State<AppState>,
    body: axum::body::Bytes,
) -> Json<WebReturnType> {
    let service_start = std::time::Instant::now();
    let call: WebCallType = match serde_json::from_slice(&body) {
        Ok(c) => c,
        Err(e) => return Json(WebReturnType::error(format!("invalid request body: {e}"))),
    };
    let result = dispatch_service(&state, &call).await;
    let elapsed = service_start.elapsed();
    tracing::info!(
        "[http-perf] {}.{}: {:.2}ms",
        call.service,
        call.method,
        elapsed.as_secs_f64() * 1000.0,
    );

    // Broadcast every WaveObjUpdate the handler returned so other
    // clients (additional windows, test harnesses, etc.) learn about
    // changes they didn't initiate. The calling HTTP client also gets
    // `updates` in the response body — this broadcast is for
    // everybody else on the event bus. Before this, only a handful
    // of handlers (agent.open, blockcontroller events) broadcast
    // manually, so an external harness's CreateTab / UpdateObject
    // were invisible to the frontend.
    if let Some(updates) = &result.updates {
        for update in updates {
            if let Ok(data) = serde_json::to_value(update) {
                let oref = format!("{}:{}", update.otype, update.oid);
                state.event_bus.broadcast_event(
                    &crate::backend::eventbus::WSEventType {
                        eventtype: "waveobj:update".to_string(),
                        oref,
                        data: Some(data),
                    },
                );
            }
        }
    }

    Json(result)
}

async fn dispatch_service(state: &AppState, call: &WebCallType) -> WebReturnType {
    let store = &state.wstore;
    let args = &call.args;

    match (call.service.as_str(), call.method.as_str()) {
        // ---- ObjectService ----
        ("object", "GetObject") => {
            let oref_str: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            match get_object_by_oref(store, &oref_str) {
                Ok(data) => WebReturnType::success(data),
                Err(e) => WebReturnType::error(e),
            }
        }
        ("object", "GetObjects") => {
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
        ("object", "CreateBlock") => {
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
        // Phase E.2c.4 — DeleteBlock SQLite-first via wcore (cascades
        // for PTY/controller already handled before the wcore call),
        // then dispatch reducer's DeleteBlock to keep state in sync.
        ("object", "DeleteBlock") => {
            let tab_id = match call
                .uicontext
                .as_ref()
                .map(|ctx| ctx.active_tab_id.clone())
            {
                Some(id) if !id.is_empty() => id,
                _ => return WebReturnType::error("missing uicontext.activetabid"),
            };
            let block_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            // Stop and remove the block controller before removing from DB so the PTY
            // and child process are torn down and the registry entry is cleared
            // regardless of DB outcome.
            blockcontroller::delete_controller(&block_id);
            if let Err(e) = wcore::delete_block(store, &tab_id, &block_id) {
                return WebReturnType::error(e.to_string());
            }
            // Reducer dispatch is silent on missing — safe to call
            // unconditionally now that SQLite is consistent.
            let events = dispatch_to_reducer(
                state,
                agentmux_common::ipc::Command::DeleteBlock {
                    tab_id: tab_id.clone(),
                    block_id: block_id.clone(),
                },
            )
            .await;
            publish_events(state, &events);
            WebReturnType::success_empty()
        }
        ("object", "UpdateObject") => {
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
        ("object", "UpdateObjectMeta") => {
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
                other => {
                    // Layouts + Windows aren't meta-mutated via this path
                    // today; fall back to wcore for forward-compat.
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
        ("object", "UpdateTabName") => {
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
        // ---- ClientService ----
        ("client", "GetClientData") => match wcore::get_client(store) {
            Ok(client) => {
                WebReturnType::success(serde_json::to_value(&client).unwrap_or_default())
            }
            Err(e) => WebReturnType::error(e.to_string()),
        },
        ("client", "GetTab") => {
            let tab_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            match store.must_get::<Tab>(&tab_id) {
                Ok(tab) => WebReturnType::success(serde_json::to_value(&tab).unwrap_or_default()),
                Err(e) => WebReturnType::error(e.to_string()),
            }
        }
        ("client", "FocusWindow") => {
            let window_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            match wcore::focus_window(store, &window_id) {
                Ok(()) => WebReturnType::success_empty(),
                Err(e) => WebReturnType::error(e.to_string()),
            }
        }
        ("client", "AgreeTos") => match wcore::get_client(store) {
            Ok(mut client) => {
                client.tosagreed = chrono::Utc::now().timestamp_millis();
                match store.update(&mut client) {
                    Ok(_) => WebReturnType::success_empty(),
                    Err(e) => WebReturnType::error(e.to_string()),
                }
            }
            Err(e) => WebReturnType::error(e.to_string()),
        },
        ("client", "GetAllConnStatus") => {
            // Return empty — connection manager not yet wired
            // Go returns success with no data (nil slice omitted by omitempty)
            WebReturnType::success_empty()
        }
        ("client", "TelemetryUpdate") => {
            // Accept but ignore — telemetry not implemented
            WebReturnType::success_empty()
        }

        // ---- WindowService ----
        ("window", "GetWindow") => {
            let window_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            match store.must_get::<Window>(&window_id) {
                Ok(win) => WebReturnType::success(serde_json::to_value(&win).unwrap_or_default()),
                Err(e) => WebReturnType::error(e.to_string()),
            }
        }
        // Phase E.5.8 — CreateWindow migrated through the reducer.
        // Two paths: (1) empty workspace_id → CreateWorkspace +
        // CreateTab + CreateWindow as a multi-step dispatch (mirrors
        // wcore::create_window_full's "fresh workspace" path); (2)
        // existing workspace_id → just CreateWindow. The subscriber's
        // apply_srv_window_opened handles `Client.windowids` updates
        // and Window-row creation. Layout setup for the new tab uses
        // the apply_tab_created provisioning (E.4 layout migration is
        // separate; default rootnode = None matches wcore behaviour).
        ("window", "CreateWindow") => {
            let requested_ws_id: String = service::get_arg(args, 1).unwrap_or_default();
            // Resolve / create the workspace.
            let (ws_id, fresh_workspace_events): (String, Vec<agentmux_common::ipc::Event>) =
                if requested_ws_id.is_empty() {
                    // Step 1: create workspace.
                    let ws_events = dispatch_to_reducer(
                        state,
                        agentmux_common::ipc::Command::CreateWorkspace {
                            name: String::new(),
                        },
                    )
                    .await;
                    if let Some(err_msg) = ws_events.iter().find_map(|e| match e {
                        agentmux_common::ipc::Event::Error { message, .. } => {
                            Some(message.clone())
                        }
                        _ => None,
                    }) {
                        return WebReturnType::error(err_msg);
                    }
                    for ev in &ws_events {
                        if let Err(e) =
                            crate::persist_subscriber::apply_event_to_wstore(ev, store)
                        {
                            return WebReturnType::error(format!(
                                "CreateWindow: SQLite write failed: {}",
                                e
                            ));
                        }
                    }
                    let new_ws_id = ws_events
                        .iter()
                        .find_map(|e| match e {
                            agentmux_common::ipc::Event::WorkspaceCreated {
                                workspace_id, ..
                            } => Some(workspace_id.clone()),
                            _ => None,
                        })
                        .unwrap_or_default();
                    // Step 2: create tab.
                    let tab_events = dispatch_to_reducer(
                        state,
                        agentmux_common::ipc::Command::CreateTab {
                            workspace_id: new_ws_id.clone(),
                            name: String::new(),
                        },
                    )
                    .await;
                    if let Some(err_msg) = tab_events.iter().find_map(|e| match e {
                        agentmux_common::ipc::Event::Error { message, .. } => {
                            Some(message.clone())
                        }
                        _ => None,
                    }) {
                        // Compensate: delete the empty workspace.
                        let comp = dispatch_to_reducer(
                            state,
                            agentmux_common::ipc::Command::DeleteWorkspace {
                                workspace_id: new_ws_id.clone(),
                            },
                        )
                        .await;
                        for ev in &comp {
                            let _ = crate::persist_subscriber::apply_event_to_wstore(ev, store);
                        }
                        publish_events(state, &comp);
                        return WebReturnType::error(err_msg);
                    }
                    for ev in &tab_events {
                        if let Err(e) =
                            crate::persist_subscriber::apply_event_to_wstore(ev, store)
                        {
                            return WebReturnType::error(format!(
                                "CreateWindow: SQLite write failed: {}",
                                e
                            ));
                        }
                    }
                    let mut combined = ws_events;
                    combined.extend(tab_events);
                    (new_ws_id, combined)
                } else {
                    // Existing workspace — verify it's in the reducer
                    // (or SQLite), but no creation needed.
                    let exists_in_sqlite = match store.get::<Workspace>(&requested_ws_id) {
                        Ok(opt) => opt.is_some(),
                        Err(e) => {
                            return WebReturnType::error(format!(
                                "CreateWindow: workspace lookup failed: {}",
                                e
                            ));
                        }
                    };
                    if !exists_in_sqlite {
                        return WebReturnType::error(format!(
                            "CreateWindow: workspace not found: {}",
                            requested_ws_id
                        ));
                    }
                    (requested_ws_id, Vec::new())
                };

            // Step 3: register the window in the reducer.
            let window_id = uuid::Uuid::new_v4().to_string();
            let win_events = dispatch_to_reducer(
                state,
                agentmux_common::ipc::Command::CreateWindow {
                    window_id: window_id.clone(),
                    workspace_id: ws_id.clone(),
                },
            )
            .await;
            if let Some(err_msg) = win_events.iter().find_map(|e| match e {
                agentmux_common::ipc::Event::Error { message, .. } => Some(message.clone()),
                _ => None,
            }) {
                // Compensate the fresh workspace if we created one.
                if !fresh_workspace_events.is_empty() {
                    let comp = dispatch_to_reducer(
                        state,
                        agentmux_common::ipc::Command::DeleteWorkspace {
                            workspace_id: ws_id.clone(),
                        },
                    )
                    .await;
                    for ev in &comp {
                        let _ = crate::persist_subscriber::apply_event_to_wstore(ev, store);
                    }
                    publish_events(state, &comp);
                }
                return WebReturnType::error(err_msg);
            }
            for ev in &win_events {
                if let Err(e) = crate::persist_subscriber::apply_event_to_wstore(ev, store) {
                    return WebReturnType::error(format!(
                        "CreateWindow: SQLite write failed: {}",
                        e
                    ));
                }
            }
            // Mark the window as `isnew` so the host's first-paint
            // signaling logic still applies — wcore::create_window_full
            // set this; the subscriber's default is `isnew: false`.
            if let Ok(mut win) = store.must_get::<Window>(&window_id) {
                if !win.isnew {
                    win.isnew = true;
                    let _ = store.update(&mut win);
                }
            }
            // Publish all events from this multi-step (workspace + tab + window).
            let mut all_events = fresh_workspace_events;
            all_events.extend(win_events);
            publish_events(state, &all_events);
            // Return the Window struct (matches the prior RPC contract).
            match store.must_get::<Window>(&window_id) {
                Ok(win) => WebReturnType::success(serde_json::to_value(&win).unwrap_or_default()),
                Err(e) => WebReturnType::error(format!(
                    "CreateWindow: window read-back failed: {}",
                    e
                )),
            }
        }
        // Phase E.5.8 — CloseWindow migrated through the reducer.
        // Sequence:
        //   1. Look up the window's workspace (for cascade decision).
        //   2. Dispatch Command::CloseWindowInternal — emits
        //      SrvWindowClosed; subscriber prunes Client.windowids.
        //   3. If no other window points at the same workspace, dispatch
        //      Command::DeleteWorkspace which cascades through tabs+blocks.
        // Mirrors `wcore::close_window` behaviour where each window
        // owns one workspace, but uses the reducer-routed conditional
        // pattern so future multi-window-on-same-workspace flows
        // don't accidentally drop user state.
        ("window", "CloseWindow") => {
            let window_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            // Look up the window's workspace before we close it.
            // Read SQLite (source of truth during migration).
            let ws_id: Option<String> = match store.get::<Window>(&window_id) {
                Ok(Some(w)) => Some(w.workspaceid.clone()),
                Ok(None) => None,
                Err(e) => {
                    return WebReturnType::error(format!(
                        "CloseWindow: window lookup failed: {}",
                        e
                    ));
                }
            };
            // Step 1: drop the window mapping in reducer.
            let close_events = dispatch_to_reducer(
                state,
                agentmux_common::ipc::Command::CloseWindowInternal {
                    window_id: window_id.clone(),
                },
            )
            .await;
            // reagent P1 #622: surface reducer rejection before
            // applying / publishing. Every other primary dispatch in
            // this PR follows this pattern; CloseWindow was missing it.
            if let Some(err_msg) = close_events.iter().find_map(|e| match e {
                agentmux_common::ipc::Event::Error { message, .. } => Some(message.clone()),
                _ => None,
            }) {
                return WebReturnType::error(err_msg);
            }
            for ev in &close_events {
                if let Err(e) = crate::persist_subscriber::apply_event_to_wstore(ev, store) {
                    return WebReturnType::error(format!(
                        "CloseWindow: SQLite write failed: {}",
                        e
                    ));
                }
            }
            publish_events(state, &close_events);
            // Step 2: cascade delete the workspace if no other window
            // points at it. The reducer keeps state.windows updated;
            // check there.
            if let Some(ws_id) = ws_id {
                let any_other_window = {
                    let s = state.srv_state.lock().await;
                    s.windows.values().any(|w| w.workspace_id == ws_id)
                };
                if !any_other_window {
                    let del_events = dispatch_to_reducer(
                        state,
                        agentmux_common::ipc::Command::DeleteWorkspace {
                            workspace_id: ws_id.clone(),
                        },
                    )
                    .await;
                    for ev in &del_events {
                        if let Err(e) = crate::persist_subscriber::apply_event_to_wstore(ev, store)
                        {
                            tracing::warn!(
                                "CloseWindow: workspace cascade SQLite write failed: {}",
                                e
                            );
                        }
                    }
                    publish_events(state, &del_events);
                }
            }
            // Subscriber's apply_srv_window_closed already pruned
            // Client.windowids and deleted the Window row; nothing
            // more for the handler to do.
            WebReturnType::success_empty()
        }
        // Phase E.5.8 — SwitchWorkspace migrated to single-step
        // reducer dispatch. The reducer validates window + workspace
        // both exist + emits SrvWindowWorkspaceChanged; subscriber
        // writes Window.workspaceid in SQLite.
        ("window", "SwitchWorkspace") => {
            let window_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let ws_id: String = match service::get_arg(args, 1) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let events = dispatch_to_reducer(
                state,
                agentmux_common::ipc::Command::SwitchWorkspace {
                    window_id: window_id.clone(),
                    workspace_id: ws_id.clone(),
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
                        "SwitchWorkspace: SQLite write failed: {}",
                        e
                    ));
                }
            }
            publish_events(state, &events);
            WebReturnType::success_empty()
        }
        ("window", "SetWindowPosAndSize") => {
            let window_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let pos: Option<Point> = service::get_optional_arg(args, 1).unwrap_or(None);
            let size: Option<WinSize> = service::get_optional_arg(args, 2).unwrap_or(None);
            match store.must_get::<Window>(&window_id) {
                Ok(mut win) => {
                    if let Some(p) = pos {
                        win.pos = p;
                    }
                    if let Some(s) = size {
                        win.winsize = s;
                    }
                    match store.update(&mut win) {
                        Ok(_) => WebReturnType::success_empty(),
                        Err(e) => WebReturnType::error(e.to_string()),
                    }
                }
                Err(e) => WebReturnType::error(e.to_string()),
            }
        }

        // ---- WorkspaceService ----
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
        ("workspace", "CreateWorkspace") => {
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
        ("workspace", "GetWorkspace") => {
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
        ("workspace", "DeleteWorkspace") => {
            let ws_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            // Phase E.2c.2 — DeleteWorkspace applies SQLite FIRST,
            // THEN dispatches the reducer command. Inverse order
            // vs CreateWorkspace because Delete cascades touch many
            // records (tabs, blocks, layouts) and rolling back the
            // reducer on a partial wcore failure is impractical.
            // SQLite-first means: if wcore fails, the reducer is
            // untouched (no divergence). If wcore succeeds, the
            // reducer dispatch + bus publish keep the rest of the
            // system in sync. (codex P2 #615 — divergence-on-failure.)
            let exists_in_wstore = match wstore_workspace_exists(store, &ws_id) {
                Ok(v) => v,
                Err(e) => {
                    return WebReturnType::error(format!(
                        "DeleteWorkspace: SQLite read failed: {}",
                        e
                    ))
                }
            };
            if exists_in_wstore {
                if let Err(e) = wcore::delete_workspace(store, &ws_id) {
                    return WebReturnType::error(format!(
                        "DeleteWorkspace: SQLite delete failed: {}",
                        e
                    ));
                }
            } else {
                // Not in SQLite — confirm the reducer doesn't know
                // about it either before surfacing NotFound. (Bootstrap
                // populates the reducer from SQLite, so a ws absent
                // from SQLite is normally absent from the reducer too;
                // this guard handles future races where the orderings
                // could diverge.)
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
            // Reducer dispatch is silent on missing — safe to call
            // unconditionally now that SQLite is consistent.
            let events = dispatch_to_reducer(
                state,
                agentmux_common::ipc::Command::DeleteWorkspace {
                    workspace_id: ws_id.clone(),
                },
            )
            .await;
            publish_events(state, &events);
            if events.iter().any(|e| matches!(e, agentmux_common::ipc::Event::Error { .. })) {
                return WebReturnType::error(format!("DeleteWorkspace failed: {}", ws_id));
            }
            WebReturnType::success_empty()
        }
        ("workspace", "ListWorkspaces") => match wcore::list_workspaces(store) {
            Ok(list) => WebReturnType::success(serde_json::to_value(&list).unwrap_or_default()),
            Err(e) => WebReturnType::error(e.to_string()),
        },
        // Phase E.2c.3b — CreateTab dispatches through the reducer.
        // The `pinned` argument from older clients is ignored:
        // pinning was a Waveterm feature removed from AgentMux.
        // Legacy SQLite databases may still have entries in
        // `Workspace.pinnedtabids`; bootstrap merges them into
        // `tab_ids` so they behave as regular tabs.
        ("workspace", "CreateTab") => {
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
        ("workspace", "SetActiveTab") => {
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
        // Phase E.2c.3b — CloseTab applies SQLite first via wcore
        // (cascades to blocks + layouts), then dispatches the
        // reducer's DeleteTab to keep state in sync. Same SQLite-
        // first pattern as DeleteWorkspace — Delete cascades touch
        // many records and rolling back the reducer on a partial
        // wcore failure is impractical.
        ("workspace", "CloseTab") => {
            let ws_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let tab_id: String = match service::get_arg(args, 1) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            if let Err(e) = wcore::delete_tab(store, &ws_id, &tab_id) {
                return WebReturnType::error(e.to_string());
            }
            // Reducer dispatch is silent on missing — safe to call
            // unconditionally now that SQLite is consistent.
            let events = dispatch_to_reducer(
                state,
                agentmux_common::ipc::Command::DeleteTab {
                    workspace_id: ws_id.clone(),
                    tab_id: tab_id.clone(),
                },
            )
            .await;
            publish_events(state, &events);
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
        ("workspace", "UpdateWorkspace") => {
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
        ("workspace", "UpdateTabIds") => {
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
        ("workspace", "MoveBlockToTab") => {
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
        ("workspace", "PromoteBlockToTab") => {
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
            if let Err(e) = setup_torn_off_block_layout(store, &new_tab_oid, &block_id) {
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
        ("workspace", "ReorderTab") => {
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
        ("workspace", "MoveTabToWorkspace") => {
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
        ("workspace", "RestoreTornOffTab") => {
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
        ("workspace", "TearOffBlock") => {
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
            if let Err(e) = setup_torn_off_block_layout(store, &new_tab_oid, &block_id) {
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
        // Phase E.5.5 — TearOffTab migrated to saga. Closes the
        // smoke regression where wcore::tear_off_tab created the new
        // workspace bypassing the reducer, leaving the new window's
        // CreateTab/etc. calls failing on "workspace not found"
        // checks against the reducer's stale view.
        ("workspace", "TearOffTab") => {
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
        // ---- UserInputService ----
        ("userinput", "SendUserInputResponse") => {
            // Accept but drop — user input routing not yet wired
            WebReturnType::success_empty()
        }

        // ---- BlockService ----
        ("block", "GetControllerStatus") => {
            let block_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            match crate::backend::blockcontroller::get_block_controller_status(&block_id) {
                Some(status) => WebReturnType::success(
                    serde_json::to_value(&status).unwrap_or(serde_json::Value::Null),
                ),
                None => {
                    let default_status = crate::backend::blockcontroller::BlockControllerRuntimeStatus {
                        blockid: block_id,
                        ..Default::default()
                    };
                    WebReturnType::success(
                        serde_json::to_value(&default_status).unwrap_or(serde_json::Value::Null),
                    )
                }
            }
        }
        ("block", "SendCommand") | ("block", "SaveTerminalState") => {
            WebReturnType::success_empty()
        }

        // ---- SubagentService ----
        ("subagent", "ListActive") => {
            let subagents = state.subagent_watcher.list_active();
            WebReturnType::success(serde_json::to_value(&subagents).unwrap_or_default())
        }
        ("subagent", "GetHistory") => {
            let agent_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let limit: usize = service::get_arg(args, 1).unwrap_or(100);
            let history = state.subagent_watcher.get_history(&agent_id, limit);
            WebReturnType::success(serde_json::to_value(&history).unwrap_or_default())
        }
        // ---- HistoryService ----
        ("history", "List") => {
            let provider: Option<String> = service::get_optional_arg(args, 0).unwrap_or(None);
            let project: Option<String> = service::get_optional_arg(args, 1).unwrap_or(None);
            let offset: usize = service::get_arg(args, 2).unwrap_or(0);
            let limit: usize = service::get_arg(args, 3).unwrap_or(50);
            let sort_by: String = service::get_arg(args, 4).unwrap_or_else(|_| "modified_at".to_string());
            let sort_dir: String = service::get_arg(args, 5).unwrap_or_else(|_| "desc".to_string());
            let result = state.history_service.list(
                provider.as_deref(),
                project.as_deref(),
                offset,
                limit,
                &sort_by,
                &sort_dir,
            );
            WebReturnType::success(result)
        }
        ("history", "Get") => {
            let session_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let result = state.history_service.get(&session_id);
            WebReturnType::success(result)
        }
        ("history", "Refresh") => {
            let result = state.history_service.refresh();
            WebReturnType::success(result)
        }

        ("subagent", "WatchAgent") => {
            let agent_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let config_dir: String = match service::get_arg(args, 1) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            state.subagent_watcher.watch_agent(&agent_id, std::path::PathBuf::from(config_dir));
            WebReturnType::success_empty()
        }

        _ => WebReturnType::error(format!(
            "unknown service method: {}.{}",
            call.service, call.method
        )),
    }
}

/// Phase E.4 (Option A) — reverse lookup: given a `LayoutState.oid`,
/// find the `Tab.oid` that owns it (i.e., the tab whose `layoutstate`
/// field matches). Returns `None` when the layout is unowned (legacy
/// or partially-migrated row) or the wstore read fails — caller treats
/// either as "skip the reducer dispatch and fall through to the wcore
/// write." Linear scan over all tabs; acceptable here because the
/// layout-update path is low-frequency relative to drag-resize and
/// the reducer mutex itself is held for sub-millisecond intervals.
fn find_tab_for_layout(store: &WaveStore, layout_oid: &str) -> Option<String> {
    let tabs = store.get_all::<Tab>().ok()?;
    tabs.into_iter()
        .find(|t| t.layoutstate == layout_oid)
        .map(|t| t.oid)
}

/// Resolve an "otype:oid" string to the corresponding wave object JSON.
fn get_object_by_oref(store: &WaveStore, oref_str: &str) -> Result<serde_json::Value, String> {
    let oref = crate::backend::ORef::parse(oref_str).map_err(|e| e.to_string())?;

    // Validate otype is known
    match oref.otype.as_str() {
        OTYPE_CLIENT | OTYPE_WINDOW | OTYPE_WORKSPACE | OTYPE_TAB | OTYPE_LAYOUT | OTYPE_BLOCK => {}
        _ => return Err(format!("unknown otype: {}", oref.otype)),
    }

    // Use raw JSON read to avoid strict struct deserialization issues
    // (e.g. layout leaforder with embedded BlockDef objects).
    // This matches Go's generic map-based GetObject behavior.
    store
        .get_raw(&oref.otype, &oref.oid)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("not found: {}", oref_str))
}

/// Update a wave object by replacing it wholesale in the store.
/// The incoming value must have `otype` and `oid` fields.
/// Matches Go's ObjectService.UpdateObject behavior.
/// Returns (otype, oid, updated_value_with_new_version) on success.
fn update_object(
    store: &WaveStore,
    mut value: serde_json::Value,
) -> Result<(String, String, serde_json::Value), String> {
    let otype = value
        .get("otype")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "UpdateObject: missing otype field".to_string())?
        .to_string();
    let oid = value
        .get("oid")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "UpdateObject: missing oid field".to_string())?
        .to_string();

    // Validate the otype is known
    match otype.as_str() {
        OTYPE_CLIENT | OTYPE_WINDOW | OTYPE_WORKSPACE | OTYPE_TAB | OTYPE_LAYOUT | OTYPE_BLOCK => {}
        _ => return Err(format!("UpdateObject: unsupported otype: {}", otype)),
    }

    // Use raw JSON storage (matching Go's generic map-based UpdateObject).
    // The frontend sends the full replacement object; strict Rust struct deserialization
    // can fail on dynamic fields (e.g. layout rootnode with embedded BlockDefs).
    let new_version = store
        .update_raw(&otype, &oid, &value)
        .map_err(|e| format!("UpdateObject: {}", e))?;

    // Update version in the value for the returned update event
    if let Some(obj) = value.as_object_mut() {
        obj.insert("version".to_string(), serde_json::json!(new_version));
    }

    Ok((otype, oid, value))
}

/// Update object meta by oref string. Merges meta into existing object.
pub(crate) fn update_object_meta(
    store: &WaveStore,
    oref_str: &str,
    meta_update: &MetaMapType,
) -> Result<(), String> {
    let oref = crate::backend::ORef::parse(oref_str).map_err(|e| e.to_string())?;
    match oref.otype.as_str() {
        OTYPE_CLIENT => {
            let mut obj = store.must_get::<Client>(&oref.oid).map_err(|e| e.to_string())?;
            obj.meta = merge_meta(&obj.meta, meta_update, true);
            store.update(&mut obj).map_err(|e| e.to_string())?;
        }
        OTYPE_WINDOW => {
            let mut obj = store.must_get::<Window>(&oref.oid).map_err(|e| e.to_string())?;
            obj.meta = merge_meta(&obj.meta, meta_update, true);
            store.update(&mut obj).map_err(|e| e.to_string())?;
        }
        OTYPE_WORKSPACE => {
            let mut obj = store
                .must_get::<Workspace>(&oref.oid)
                .map_err(|e| e.to_string())?;
            obj.meta = merge_meta(&obj.meta, meta_update, true);
            store.update(&mut obj).map_err(|e| e.to_string())?;
        }
        OTYPE_TAB => {
            let mut obj = store.must_get::<Tab>(&oref.oid).map_err(|e| e.to_string())?;
            obj.meta = merge_meta(&obj.meta, meta_update, true);
            store.update(&mut obj).map_err(|e| e.to_string())?;
        }
        OTYPE_BLOCK => {
            let mut obj = store.must_get::<Block>(&oref.oid).map_err(|e| e.to_string())?;
            obj.meta = merge_meta(&obj.meta, meta_update, true);
            store.update(&mut obj).map_err(|e| e.to_string())?;
        }
        _ => return Err(format!("cannot update meta for otype: {}", oref.otype)),
    }
    Ok(())
}


// ---- Phase E.2c.2 reducer-dispatch helpers ----

/// Dispatch a command into the srv reducer and return the emitted
/// events. Locks the reducer mutex briefly; the lock is released
/// before any I/O (caller is responsible for publishing the events
/// to the broadcast bus).
pub(crate) async fn dispatch_to_reducer(
    state: &AppState,
    cmd: agentmux_common::ipc::Command,
) -> Vec<agentmux_common::ipc::Event> {
    let now = chrono::Utc::now().to_rfc3339();
    let mut s = state.srv_state.lock().await;
    let ctx = crate::reducer::Ctx {
        now_rfc3339: now,
        // RPC-originated dispatch has no IPC connection — sentinel.
        conn_id: 0,
        registered_pid: None,
    };
    crate::reducer::update(&mut s, cmd, &ctx)
}

/// Publish each event on the srv broadcast bus. Failures (no
/// subscribers) are non-fatal.
pub(crate) fn publish_events(state: &AppState, events: &[agentmux_common::ipc::Event]) {
    for event in events {
        let _ = state.srv_events_tx.send(event.clone());
    }
}

/// Compensation helper: dispatch a command into the reducer and
/// apply its emitted events to wstore best-effort. Used when an
/// earlier sync apply partially wrote SQLite and we need to undo
/// the leaked rows. SQLite errors during compensation are logged
/// but ignored — the caller is already returning an error to the
/// client; throwing on the cleanup just hides the original cause.
/// (codex P1 + reagent P2 #616 — partial-write cleanup.)
async fn compensate_via_reducer(
    state: &AppState,
    cmd: agentmux_common::ipc::Command,
    store: &WaveStore,
) {
    let events = dispatch_to_reducer(state, cmd).await;
    for ev in &events {
        if let Err(e) = crate::persist_subscriber::apply_event_to_wstore(ev, store) {
            tracing::warn!(
                "compensation: SQLite cleanup failed for event {:?}: {}",
                std::mem::discriminant(ev),
                e
            );
        }
    }
}

/// Phase E.5.5 — set up the layout tree for a tab that just received
/// its first block via the TearOffBlock saga. Called from the
/// TearOffBlock RPC handler after the saga's reducer-state portion
/// (CreateTab + MoveBlock) completes. Mirrors the layout-rootnode
/// + leaforder construction that `wcore::tear_off_block` previously
/// embedded in its single function.
///
/// Layout state migration is E.4 — until then layout writes are
/// wcore-direct and not reducer-routed. Best-effort: a failure here
/// leaves the new tab with the moved block but a malformed layout;
/// the user-visible symptom is an empty render in the new window.
fn setup_torn_off_block_layout(
    store: &WaveStore,
    new_tab_id: &str,
    block_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let new_tab = store.must_get::<Tab>(new_tab_id)?;
    let mut layout = store.must_get::<LayoutState>(&new_tab.layoutstate)?;
    let node_id = uuid::Uuid::new_v4().to_string();
    layout.rootnode = Some(serde_json::json!({
        "id": node_id,
        "data": { "blockId": block_id },
        "flexDirection": "row",
        "size": 1
    }));
    layout.leaforder = Some(vec![LeafOrderEntry {
        nodeid: node_id,
        blockid: block_id.to_string(),
    }]);
    store.update(&mut layout)?;
    Ok(())
}

/// Phase E.5.5 — append a layout-delete action to the source tab's
/// `LayoutState.pendingbackendactions` so the source window's
/// frontend tears the moved block out of its layout tree on next
/// poll. Mirrors the action-queueing portion of
/// `wcore::tear_off_block`. Layout migration is E.4.
fn queue_source_layout_delete(
    store: &WaveStore,
    source_tab_id: &str,
    block_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let source_tab = store.must_get::<Tab>(source_tab_id)?;
    let mut source_layout = store.must_get::<LayoutState>(&source_tab.layoutstate)?;
    let mut actions = source_layout.pendingbackendactions.take().unwrap_or_default();
    actions.push(LayoutActionData {
        actiontype: "delete".to_string(),
        actionid: uuid::Uuid::new_v4().to_string(),
        blockid: block_id.to_string(),
        nodesize: None,
        indexarr: None,
        focused: false,
        magnified: false,
        ephemeral: false,
        targetblockid: String::new(),
        position: String::new(),
    });
    source_layout.pendingbackendactions = Some(actions);
    store.update(&mut source_layout)?;
    Ok(())
}

/// Existence check used by `DeleteWorkspace` to decide whether to
/// run the wcore delete path. Propagates `StoreError` so the caller
/// can surface real I/O / corruption failures instead of
/// misclassifying them as "not found" (codex P2 #615 carryover —
/// the prior `bool` return collapsed `Err(_)` into `false`, which
/// led to silent successes when SQLite was unhealthy: reducer would
/// delete its own copy and report success while the disk row was
/// never touched).
fn wstore_workspace_exists(
    store: &WaveStore,
    workspace_id: &str,
) -> Result<bool, crate::backend::storage::StoreError> {
    Ok(store.get::<Workspace>(workspace_id)?.is_some())
}

// `build_workspace_from_state` removed in E.2c.2. The reducer's
// WorkspaceRecord can't faithfully render a Workspace during the
// migration window (no pinnedtabids; tabids/activetabid go stale
// vs wcore-direct tab ops). It will be reintroduced in E.2c.3
// when tabs migrate into the reducer and pinned/active state is
// authoritative there. (reagent + codex P1 #615.)
