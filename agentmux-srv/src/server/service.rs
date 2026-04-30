
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
            match wcore::create_block(store, &tab_id, block_def.meta) {
                Ok(block) => {
                    let block_oid = block.oid.clone();
                    let block_update = WaveObjUpdate {
                        updatetype: "update".into(),
                        otype: OTYPE_BLOCK.to_string(),
                        oid: block.oid.clone(),
                        obj: Some(wave_obj_to_value(&block)),
                    };
                    let updates = match store.must_get::<Tab>(&tab_id) {
                        Ok(tab) => {
                            let tab_update = WaveObjUpdate {
                                updatetype: "update".into(),
                                otype: OTYPE_TAB.to_string(),
                                oid: tab_id.clone(),
                                obj: Some(wave_obj_to_value(&tab)),
                            };
                            vec![block_update, tab_update]
                        }
                        Err(_) => vec![block_update],
                    };
                    WebReturnType::success_data_updates(serde_json::json!(block_oid), updates)
                }
                Err(e) => WebReturnType::error(e.to_string()),
            }
        }
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
            match wcore::delete_block(store, &tab_id, &block_id) {
                Ok(()) => WebReturnType::success_empty(),
                Err(e) => WebReturnType::error(e.to_string()),
            }
        }
        ("object", "UpdateObject") => {
            let wave_obj_value: serde_json::Value = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            match update_object(store, wave_obj_value) {
                Ok((otype, oid, obj_val)) => {
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
        ("object", "UpdateObjectMeta") => {
            // args[0] = oref string, args[1] = meta map
            // (Go dispatcher strips UIContext from args; TS sends [oref, meta])
            let oref_str: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let meta_update: MetaMapType = match service::get_arg(args, 1) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            match update_object_meta(store, &oref_str, &meta_update) {
                Ok(()) => {
                    // Return the updated object so the frontend WOS cache stays in sync.
                    // (Without this, atoms like cmd:cwd never update after OSC 7 fires.)
                    let oref = match crate::backend::ORef::parse(&oref_str) {
                        Ok(v) => v,
                        Err(e) => return WebReturnType::error(e.to_string()),
                    };
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
                Err(e) => WebReturnType::error(e),
            }
        }
        ("object", "UpdateTabName") => {
            let tab_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let name: String = match service::get_arg(args, 1) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            match store.must_get::<Tab>(&tab_id) {
                Ok(mut tab) => {
                    tab.name = name;
                    match store.update(&mut tab) {
                        Ok(_) => {
                            // Return updated tab so frontend WOS cache stays in sync
                            if let Ok(updated_tab) = store.must_get::<Tab>(&tab_id) {
                                let update = WaveObjUpdate {
                                    updatetype: "update".into(),
                                    otype: OTYPE_TAB.to_string(),
                                    oid: tab_id.clone(),
                                    obj: Some(wave_obj_to_value(&updated_tab)),
                                };
                                WebReturnType::success_with_updates(vec![update])
                            } else {
                                WebReturnType::success_empty()
                            }
                        }
                        Err(e) => WebReturnType::error(e.to_string()),
                    }
                }
                Err(e) => WebReturnType::error(e.to_string()),
            }
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
        ("window", "CreateWindow") => {
            let ws_id: String = match service::get_arg(args, 1) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            match wcore::create_window_full(store, &ws_id) {
                Ok(win) => {
                    WebReturnType::success(serde_json::to_value(&win).unwrap_or_default())
                }
                Err(e) => WebReturnType::error(e.to_string()),
            }
        }
        ("window", "CloseWindow") => {
            let window_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            match wcore::close_window(store, &window_id) {
                Ok(()) => WebReturnType::success_empty(),
                Err(e) => WebReturnType::error(e.to_string()),
            }
        }
        ("window", "SwitchWorkspace") => {
            let window_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let ws_id: String = match service::get_arg(args, 1) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            match wcore::switch_workspace(store, &window_id, &ws_id) {
                Ok(()) => WebReturnType::success_empty(),
                Err(e) => WebReturnType::error(e.to_string()),
            }
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
        // Phase E.2c.3 — CreateTab routes through the reducer for
        // regular (unpinned) tabs. Pinned tabs (`pinned=true` in
        // args[3]) stay on the wcore-direct path until E.2c.3b adds
        // pinned-tab support to the reducer. The split is necessary
        // because the reducer's WorkspaceRecord doesn't yet track
        // `pinned_tab_ids` separately — adding that is structural
        // (state shape + bootstrap split + subscriber routing) and
        // doesn't fit cleanly into this PR.
        ("workspace", "CreateTab") => {
            let ws_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let tab_name: String = service::get_arg(args, 1).unwrap_or_default();
            let activate: bool = service::get_arg(args, 2).unwrap_or(true);
            let pinned: bool = service::get_arg(args, 3).unwrap_or(false);
            if pinned {
                // Pinned-tab path: wcore-direct (unchanged from E.2c.2).
                match wcore::create_tab_with_opts(store, &ws_id, &tab_name, true) {
                    Ok(tab) => {
                        if activate {
                            let _ = wcore::set_active_tab(store, &ws_id, &tab.oid);
                        }
                        let tab_oid = tab.oid.clone();
                        let mut updates = vec![WaveObjUpdate {
                            updatetype: "update".into(),
                            otype: OTYPE_TAB.to_string(),
                            oid: tab.oid.clone(),
                            obj: Some(wave_obj_to_value(&tab)),
                        }];
                        if let Ok(ws) = store.must_get::<Workspace>(&ws_id) {
                            updates.push(WaveObjUpdate {
                                updatetype: "update".into(),
                                otype: OTYPE_WORKSPACE.to_string(),
                                oid: ws_id.clone(),
                                obj: Some(wave_obj_to_value(&ws)),
                            });
                        }
                        return WebReturnType::success_data_updates(
                            serde_json::to_value(&tab_oid).unwrap_or_default(),
                            updates,
                        );
                    }
                    Err(e) => return WebReturnType::error(e.to_string()),
                }
            }
            // Regular tab: dispatch through reducer. Auto-generate
            // a `tab{N}` name when the caller passed empty so the
            // reducer path matches the wcore behaviour
            // (wcore::create_tab_with_opts auto-names empties from
            // `ws.tabids.len() + ws.pinnedtabids.len()`). Without
            // this, regular reducer-routed tabs got blank titles
            // while pinned wcore-routed tabs auto-named — UX
            // inconsistency. (reagent + codex P1 #616.)
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
            let _ = wcore::heal_layout(store, &tab_id);

            // Pinned tabs aren't tracked in the reducer yet — dispatch
            // would emit Error("tab not in workspace's tab list").
            // Detect and fall through to wcore for those.
            let is_pinned = match store.get::<Workspace>(&ws_id) {
                Ok(Some(ws)) => ws.pinnedtabids.iter().any(|t| t == &tab_id),
                Ok(None) => false,
                Err(e) => {
                    // Surface SQLite errors instead of falling through
                    // into the reducer path with a misleading "tab
                    // not in workspace" reducer error. (codex P2 #616.)
                    return WebReturnType::error(format!(
                        "SetActiveTab: SQLite read failed: {}",
                        e
                    ));
                }
            };
            if is_pinned {
                return match wcore::set_active_tab(store, &ws_id, &tab_id) {
                    Ok(()) => {
                        // Pinned tabs aren't in the reducer's
                        // WorkspaceRecord.tab_ids (E.2c.3 doesn't yet
                        // model them). Direct-mutate
                        // `active_tab_id = Some(pinned_tab_id)` so
                        // both bugs from the prior iterations are
                        // avoided:
                        //   * Setting `None` would cause the next
                        //     CreateTab(activate=false) to be auto-
                        //     activated (handle_create_tab auto-
                        //     activates when active_tab_id is None).
                        //     codex P1 #616 round-3.
                        //   * Leaving the previous regular tab as
                        //     active would make the next
                        //     SetActiveTab(same regular) a no-op
                        //     (reducer thinks it's already active),
                        //     so user bounces pinned→regular would
                        //     be lost. codex P1 #616 round-2.
                        // Storing the pinned id here is "external"
                        // to the reducer's tab_ids set, but the
                        // reducer is a session-only projection
                        // during the migration window — having a
                        // value the reducer wouldn't validate via
                        // its own command path is fine. E.2c.3b
                        // adds proper pinned support and unifies.
                        {
                            let mut s = state.srv_state.lock().await;
                            if let Some(ws_record) = s.workspaces.get_mut(&ws_id) {
                                ws_record.active_tab_id = Some(tab_id.clone());
                            }
                        }
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
                    Err(e) => WebReturnType::error(e.to_string()),
                };
            }

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
        ("workspace", "CloseTab") => {
            let ws_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let tab_id: String = match service::get_arg(args, 1) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            match wcore::delete_tab(store, &ws_id, &tab_id) {
                Ok(()) => {
                    let rtn = CloseTabRtnType {
                        closewindow: false,
                        newactivetabid: String::new(),
                    };
                    let mut updates = vec![];
                    // Include deleted tab update so frontend removes it from cache
                    updates.push(WaveObjUpdate {
                        updatetype: "delete".into(),
                        otype: OTYPE_TAB.to_string(),
                        oid: tab_id.clone(),
                        obj: None,
                    });
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
                Err(e) => WebReturnType::error(e.to_string()),
            }
        }
        ("workspace", "UpdateWorkspace") => {
            let ws_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let name: Option<String> = service::get_optional_arg(args, 1).unwrap_or(None);
            match store.must_get::<Workspace>(&ws_id) {
                Ok(mut ws) => {
                    if let Some(n) = name {
                        ws.name = n;
                    }
                    match store.update(&mut ws) {
                        Ok(_) => WebReturnType::success_empty(),
                        Err(e) => WebReturnType::error(e.to_string()),
                    }
                }
                Err(e) => WebReturnType::error(e.to_string()),
            }
        }
        ("workspace", "UpdateTabIds") => {
            let ws_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let tab_ids: Vec<String> = match service::get_arg(args, 1) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let pinned_tab_ids: Vec<String> = service::get_arg(args, 2).unwrap_or_default();
            match store.must_get::<Workspace>(&ws_id) {
                Ok(mut ws) => {
                    ws.tabids = tab_ids;
                    ws.pinnedtabids = pinned_tab_ids;
                    match store.update(&mut ws) {
                        Ok(_) => {
                            if let Ok(updated_ws) = store.must_get::<Workspace>(&ws_id) {
                                let update = WaveObjUpdate {
                                    updatetype: "update".into(),
                                    otype: OTYPE_WORKSPACE.to_string(),
                                    oid: ws_id.clone(),
                                    obj: Some(wave_obj_to_value(&updated_ws)),
                                };
                                WebReturnType::success_with_updates(vec![update])
                            } else {
                                WebReturnType::success_empty()
                            }
                        }
                        Err(e) => WebReturnType::error(e.to_string()),
                    }
                }
                Err(e) => WebReturnType::error(e.to_string()),
            }
        }
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
            tracing::info!(ws_id = %ws_id, block_id = %block_id, source_tab = %source_tab_id, dest_tab = %dest_tab_id, "[dnd:svc] MoveBlockToTab");
            match wcore::move_block_to_tab(store, &block_id, &source_tab_id, &dest_tab_id, &ws_id, auto_close) {
                Ok(()) => {
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
                Err(e) => WebReturnType::error(e.to_string()),
            }
        }
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
            tracing::info!(ws_id = %ws_id, block_id = %block_id, source_tab = %source_tab_id, "[dnd:svc] PromoteBlockToTab");
            match wcore::promote_block_to_tab(store, &block_id, &source_tab_id, &ws_id, auto_close) {
                Ok(new_tab) => {
                    let new_tab_oid = new_tab.oid.clone();
                    let mut updates = vec![];
                    updates.push(WaveObjUpdate {
                        updatetype: "update".into(),
                        otype: OTYPE_TAB.to_string(),
                        oid: new_tab.oid.clone(),
                        obj: Some(wave_obj_to_value(&new_tab)),
                    });
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
                Err(e) => WebReturnType::error(e.to_string()),
            }
        }
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
            match wcore::reorder_tab(store, &ws_id, &tab_id, new_index) {
                Ok(()) => {
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
                Err(e) => WebReturnType::error(e.to_string()),
            }
        }
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
            let insert_index: Option<usize> = service::get_arg(args, 3).ok();
            tracing::info!(tab_id = %tab_id, source_ws = %source_ws_id, dest_ws = %dest_ws_id, insert_index = ?insert_index, "[dnd:svc] MoveTabToWorkspace");
            match wcore::move_tab_to_workspace(store, &tab_id, &source_ws_id, &dest_ws_id, insert_index) {
                Ok(()) => {
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
                Err(e) => WebReturnType::error(e.to_string()),
            }
        }
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
            let insert_index: Option<usize> = service::get_arg(args, 3).ok();
            // arg 4 is `wasPinned: bool`. Default false — older clients
            // and the merge path (which always restores into the target's
            // tabids regardless of source pin status) don't need to pass
            // it. Pinned-status preservation is a cancel-back-only feature.
            let was_pinned: bool = service::get_arg(args, 4).unwrap_or(false);
            tracing::info!(tab_id = %tab_id, source_ws = %source_ws_id, dest_ws = %dest_ws_id, insert_index = ?insert_index, was_pinned = %was_pinned, "[dnd:svc] RestoreTornOffTab");
            match wcore::restore_torn_off_tab(store, &tab_id, &source_ws_id, &dest_ws_id, insert_index, was_pinned) {
                Ok(()) => {
                    let mut updates = Vec::new();
                    // Source workspace: emit a delete update if we deleted
                    // it; otherwise an update with current state. Frontends
                    // listening on the dragged window's workspace can react
                    // to either.
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
                Err(e) => WebReturnType::error(e.to_string()),
            }
        }
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
            tracing::info!(block_id = %block_id, source_tab = %source_tab_id, source_ws = %source_ws_id, "[dnd:svc] TearOffBlock");
            match wcore::tear_off_block(store, &block_id, &source_tab_id, &source_ws_id, auto_close) {
                Ok(new_ws) => {
                    let new_ws_oid = new_ws.oid.clone();
                    let mut updates = Vec::new();
                    // Source tab update
                    if let Ok(src_tab) = store.must_get::<Tab>(&source_tab_id) {
                        updates.push(WaveObjUpdate {
                            updatetype: "update".into(),
                            otype: OTYPE_TAB.to_string(),
                            oid: source_tab_id.clone(),
                            obj: Some(wave_obj_to_value(&src_tab)),
                        });
                    }
                    // Source workspace update
                    if let Ok(src_ws) = store.must_get::<Workspace>(&source_ws_id) {
                        updates.push(WaveObjUpdate {
                            updatetype: "update".into(),
                            otype: OTYPE_WORKSPACE.to_string(),
                            oid: source_ws_id.clone(),
                            obj: Some(wave_obj_to_value(&src_ws)),
                        });
                    }
                    // New workspace update
                    updates.push(WaveObjUpdate {
                        updatetype: "update".into(),
                        otype: OTYPE_WORKSPACE.to_string(),
                        oid: new_ws_oid.clone(),
                        obj: Some(wave_obj_to_value(&new_ws)),
                    });
                    WebReturnType::success_data_updates(
                        serde_json::to_value(&new_ws_oid).unwrap_or_default(),
                        updates,
                    )
                }
                Err(e) => WebReturnType::error(e.to_string()),
            }
        }
        ("workspace", "TearOffTab") => {
            let tab_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let source_ws_id: String = match service::get_arg(args, 1) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            tracing::info!(tab_id = %tab_id, source_ws = %source_ws_id, "[dnd:svc] TearOffTab");
            match wcore::tear_off_tab(store, &tab_id, &source_ws_id) {
                Ok(new_ws) => {
                    let new_ws_oid = new_ws.oid.clone();
                    let mut updates = Vec::new();
                    // Source workspace update
                    if let Ok(src_ws) = store.must_get::<Workspace>(&source_ws_id) {
                        updates.push(WaveObjUpdate {
                            updatetype: "update".into(),
                            otype: OTYPE_WORKSPACE.to_string(),
                            oid: source_ws_id.clone(),
                            obj: Some(wave_obj_to_value(&src_ws)),
                        });
                    }
                    // New workspace update
                    updates.push(WaveObjUpdate {
                        updatetype: "update".into(),
                        otype: OTYPE_WORKSPACE.to_string(),
                        oid: new_ws_oid.clone(),
                        obj: Some(wave_obj_to_value(&new_ws)),
                    });
                    WebReturnType::success_data_updates(
                        serde_json::to_value(&new_ws_oid).unwrap_or_default(),
                        updates,
                    )
                }
                Err(e) => WebReturnType::error(e.to_string()),
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
async fn dispatch_to_reducer(
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
fn publish_events(state: &AppState, events: &[agentmux_common::ipc::Event]) {
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
