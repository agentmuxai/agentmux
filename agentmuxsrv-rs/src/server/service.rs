
use axum::{extract::State, response::Json};
use serde_json::json;

use crate::backend::blockcontroller;
use crate::backend::service::{self, CloseTabRtnType, WebCallType, WebReturnType};
use crate::backend::storage::wstore::WaveStore;
use crate::backend::waveobj::*;
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
    let result = dispatch_service(&state, &call);
    let elapsed = service_start.elapsed();
    tracing::info!(
        "[http-perf] {}.{}: {:.2}ms",
        call.service,
        call.method,
        elapsed.as_secs_f64() * 1000.0,
    );
    Json(result)
}

fn dispatch_service(state: &AppState, call: &WebCallType) -> WebReturnType {
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
            let tab_id = match call
                .uicontext
                .as_ref()
                .map(|ctx| ctx.active_tab_id.clone())
            {
                Some(id) if !id.is_empty() => id,
                _ => return WebReturnType::error("missing uicontext.activetabid"),
            };
            let block_def: BlockDef = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
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
        ("workspace", "CreateWorkspace") => {
            let name: String = service::get_arg(args, 0).unwrap_or_default();
            let icon: String = service::get_arg(args, 1).unwrap_or_default();
            let color: String = service::get_arg(args, 2).unwrap_or_default();
            match wcore::create_workspace(store, &name, &icon, &color) {
                Ok(ws) => WebReturnType::success(serde_json::to_value(&ws).unwrap_or_default()),
                Err(e) => WebReturnType::error(e.to_string()),
            }
        }
        ("workspace", "GetWorkspace") => {
            let ws_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
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
            match wcore::delete_workspace(store, &ws_id) {
                Ok(()) => WebReturnType::success_empty(),
                Err(e) => WebReturnType::error(e.to_string()),
            }
        }
        ("workspace", "ListWorkspaces") => match wcore::list_workspaces(store) {
            Ok(list) => WebReturnType::success(serde_json::to_value(&list).unwrap_or_default()),
            Err(e) => WebReturnType::error(e.to_string()),
        },
        ("workspace", "CreateTab") => {
            let ws_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let tab_name: String = service::get_arg(args, 1).unwrap_or_default();
            let activate: bool = service::get_arg(args, 2).unwrap_or(true);
            let pinned: bool = service::get_arg(args, 3).unwrap_or(false);
            match wcore::create_tab_with_opts(store, &ws_id, &tab_name, pinned) {
                Ok(tab) => {
                    // If activate requested, set active tab
                    if activate {
                        let _ = wcore::set_active_tab(store, &ws_id, &tab.oid);
                    }
                    let tab_oid = tab.oid.clone();
                    let tab_update = WaveObjUpdate {
                        updatetype: "update".into(),
                        otype: OTYPE_TAB.to_string(),
                        oid: tab.oid.clone(),
                        obj: Some(wave_obj_to_value(&tab)),
                    };
                    let mut updates = vec![tab_update];
                    if let Ok(ws) = store.must_get::<Workspace>(&ws_id) {
                        updates.push(WaveObjUpdate {
                            updatetype: "update".into(),
                            otype: OTYPE_WORKSPACE.to_string(),
                            oid: ws_id.clone(),
                            obj: Some(wave_obj_to_value(&ws)),
                        });
                    }
                    WebReturnType::success_data_updates(
                        serde_json::to_value(&tab_oid).unwrap_or_default(),
                        updates,
                    )
                }
                Err(e) => WebReturnType::error(e.to_string()),
            }
        }
        ("workspace", "SetActiveTab") => {
            let ws_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let tab_id: String = match service::get_arg(args, 1) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            match wcore::set_active_tab(store, &ws_id, &tab_id) {
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
        ("workspace", "GetColors") => {
            WebReturnType::success(json!(wcore::WORKSPACE_COLORS))
        }
        ("workspace", "GetIcons") => {
            WebReturnType::success(json!(wcore::WORKSPACE_ICONS))
        }
        ("workspace", "UpdateWorkspace") => {
            let ws_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let name: Option<String> = service::get_optional_arg(args, 1).unwrap_or(None);
            let icon: Option<String> = service::get_optional_arg(args, 2).unwrap_or(None);
            let color: Option<String> = service::get_optional_arg(args, 3).unwrap_or(None);
            match store.must_get::<Workspace>(&ws_id) {
                Ok(mut ws) => {
                    if let Some(n) = name {
                        ws.name = n;
                    }
                    if let Some(i) = icon {
                        ws.icon = i;
                    }
                    if let Some(c) = color {
                        ws.color = c;
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
        ("workspace", "ChangeTabPinning") => {
            let ws_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let tab_id: String = match service::get_arg(args, 1) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let pinned: bool = match service::get_arg(args, 2) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            match store.must_get::<Workspace>(&ws_id) {
                Ok(mut ws) => {
                    ws.pinnedtabids.retain(|id| id != &tab_id);
                    if pinned {
                        ws.pinnedtabids.push(tab_id);
                    }
                    match store.update(&mut ws) {
                        Ok(_) => WebReturnType::success_empty(),
                        Err(e) => WebReturnType::error(e.to_string()),
                    }
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
