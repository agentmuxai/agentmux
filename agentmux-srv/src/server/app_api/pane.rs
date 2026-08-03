use super::*;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    register_pane_open(engine, state);
}

fn register_pane_open(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let state = state.clone();
    engine.register_handler(
        COMMAND_PANE_OPEN,
        Box::new(move |data, _ctx| {
            let state = state.clone();
            Box::pin(async move {
                let cmd: CommandPaneOpenData = serde_json::from_value(data)
                    .map_err(|e| format!("pane.open: {e}"))?;
                let result = open_pane(&state, cmd).await?;
                Ok(Some(serde_json::to_value(&result).unwrap()))
            })
        }),
    );
}

/// Floating-pane branch of `open_pane`. The block already exists in
/// `source_tab_id`'s blockids (created by the caller, with no layout node).
/// This moves it into a fresh floating workspace via the `tear_off_block`
/// saga, sets up the new tab's layout, broadcasts the new WaveObjs, and asks
/// the source window's frontend to materialize the chromeless floating OS
/// window via the host `open_floating_pane_window` command (srv cannot open
/// windows itself). See docs/specs/SPEC_OPENEDITOR_FLOATING_AND_COLLAPSED_TREE_2026_06_16.md.
pub(super) async fn open_pane_floating(
    state: &AppState,
    wstore: &Store,
    event_bus: &crate::backend::eventbus::EventBus,
    view: String,
    source_tab_id: String,
    meta: MetaMapType,
) -> Result<PaneOpenResult, String> {
    use agentmux_common::ipc::{Command, Event};

    // Source workspace from the reducer's canonical tab→workspace map.
    let source_ws_id = {
        let s = state.srv_state.lock().await;
        s.tabs
            .get(&source_tab_id)
            .map(|t| t.workspace_id.clone())
            .ok_or_else(|| format!("pane.open: floating: tab {source_tab_id} not in reducer state"))?
    };

    // Create the block through the reducer (NOT wcore-direct) so it lands in
    // `state.blocks` — the `tear_off_block` saga's pre-condition checks the
    // reducer-canonical block map. The `BlockCreated` event also carries the
    // meta, which `persist_subscriber::apply_block_created` writes into the
    // wstore Block so the editor renders with its file + tree state. We skip
    // layout placement, so the block never renders docked before the saga
    // moves it into the floating workspace (no flash).
    let meta_val = serde_json::to_value(&meta)
        .map_err(|e| format!("pane.open: floating: meta serialize: {e}"))?;
    let create_events = crate::server::service::dispatch_to_reducer(
        state,
        Command::CreateBlock {
            tab_id: source_tab_id.clone(),
            meta: meta_val,
        },
    )
    .await;
    if let Some(msg) = create_events.iter().find_map(|e| match e {
        Event::Error { message, .. } => Some(message.clone()),
        _ => None,
    }) {
        return Err(format!("pane.open: floating: CreateBlock: {msg}"));
    }
    let block_id = create_events
        .iter()
        .find_map(|e| match e {
            Event::BlockCreated { block_id, .. } => Some(block_id.clone()),
            _ => None,
        })
        .ok_or_else(|| "pane.open: floating: CreateBlock emitted no BlockCreated".to_string())?;
    for ev in &create_events {
        if let Err(e) = crate::persist_subscriber::apply_event_to_wstore(ev, wstore) {
            tracing::warn!("pane.open: floating: CreateBlock wstore apply failed: {e}");
        }
    }
    crate::server::service::publish_events(state, &create_events);

    // Tear the block off into a fresh floating workspace + tab (reuses the
    // exact saga the drag tear-off uses: CreateWorkspace → CreateTab → MoveBlock).
    let saga_val = crate::sagas::tear_off_block::run(
        state,
        block_id.clone(),
        source_tab_id.clone(),
        source_ws_id.clone(),
    )
    .await?;
    let new_ws_id = saga_val
        .get("new_workspace_id")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let new_tab_id = saga_val
        .get("new_tab_id")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    if new_ws_id.is_empty() || new_tab_id.is_empty() {
        return Err("pane.open: floating: tear_off_block returned empty ids".to_string());
    }

    // Make the moved block the new tab's single root node so it renders.
    if let Err(e) =
        crate::server::service::setup_torn_off_block_layout(state, &new_tab_id, &block_id).await
    {
        tracing::warn!(
            new_tab = %new_tab_id,
            "pane.open: floating: layout setup failed: {e} (block moved but layout malformed)"
        );
    }

    // Broadcast the new workspace + layout + tab + block so any frontend syncs
    // its WaveObj cache (mirrors the docked path + the tear-off DnD handler).
    {
        let mut updates: Vec<obj::WaveObjUpdate> = Vec::new();
        if let Ok(ws) = wstore.must_get::<Workspace>(&new_ws_id) {
            updates.push(obj::WaveObjUpdate {
                updatetype: "update".into(),
                otype: "workspace".into(),
                oid: new_ws_id.clone(),
                obj: Some(obj::wave_obj_to_value(&ws)),
            });
        }
        if let Ok(t) = wstore.must_get::<Tab>(&new_tab_id) {
            if let Ok(layout) = wstore.must_get::<obj::LayoutState>(&t.layoutstate) {
                updates.push(obj::WaveObjUpdate {
                    updatetype: "update".into(),
                    otype: "layout".into(),
                    oid: t.layoutstate.clone(),
                    obj: Some(obj::wave_obj_to_value(&layout)),
                });
            }
            updates.push(obj::WaveObjUpdate {
                updatetype: "update".into(),
                otype: "tab".into(),
                oid: new_tab_id.clone(),
                obj: Some(obj::wave_obj_to_value(&t)),
            });
        }
        if let Ok(b) = wstore.must_get::<Block>(&block_id) {
            updates.push(obj::WaveObjUpdate {
                updatetype: "update".into(),
                otype: "block".into(),
                oid: block_id.clone(),
                obj: Some(obj::wave_obj_to_value(&b)),
            });
        }
        for update in &updates {
            let oref = format!("{}:{}", update.otype, update.oid);
            if let Ok(data) = serde_json::to_value(update) {
                event_bus.broadcast_event(&crate::backend::eventbus::WSEventType {
                    eventtype: "waveobj:update".to_string(),
                    oref,
                    data: Some(data),
                });
            }
        }
    }

    // Ask the source window's frontend to open the floating OS window — scoped
    // to that window (mirrors the window-scoped `userinput` event) so exactly
    // one window acts. The frontend handler calls the host
    // `open_floating_pane_window` command.
    let window_id = {
        let s = state.srv_state.lock().await;
        s.windows
            .iter()
            .find(|(_, w)| w.workspace_id == source_ws_id)
            .map(|(id, _)| id.clone())
    };
    match window_id {
        Some(win) => {
            state.broker.publish(crate::backend::wps::WaveEvent {
                event: "openfloatingpane".to_string(),
                scopes: vec![win],
                sender: String::new(),
                persist: 0,
                data: Some(json!({
                    "block_id": block_id,
                    "workspace_id": new_ws_id,
                })),
            });
        }
        None => {
            tracing::warn!(
                source_ws = %source_ws_id,
                "pane.open: floating: no window mapped to source workspace — floater not opened"
            );
        }
    }

    Ok(PaneOpenResult {
        block_id,
        tab_id: new_tab_id,
        view,
        created: true,
    })
}

/// Build the metadata map for a pane.open request, validating required args.
pub(super) fn build_pane_meta(cmd: &CommandPaneOpenData) -> Result<MetaMapType, String> {
    let mut meta = MetaMapType::new();

    match cmd.view.as_str() {
        "editor" => {
            let file = cmd.file.as_deref().filter(|s| !s.is_empty())
                .ok_or_else(|| "MISSING_ARG: view=editor requires 'file'".to_string())?;
            meta.insert("view".to_string(), json!("editor"));
            meta.insert("file".to_string(), json!(file));

            let is_markdown = file.to_ascii_lowercase().ends_with(".md");

            // Tree state: explicit caller value wins; for markdown default to
            // collapsed so the rendered preview gets full horizontal width.
            if let Some(expanded) = cmd.tree_expanded {
                meta.insert("editor:tree_expanded".to_string(), json!(expanded));
            } else if is_markdown {
                meta.insert("editor:tree_expanded".to_string(), json!(false));
            }

            // Markdown files default to preview-only (source editor hidden).
            // Agents open .md files to surface documentation, not to edit.
            if is_markdown {
                meta.insert("editor:source_hidden".to_string(), json!(true));
            }
        }
        "term" => {
            meta.insert("view".to_string(), json!("term"));
            meta.insert("controller".to_string(), json!("shell"));
            if let Some(cwd) = cmd.cwd.as_deref().filter(|s| !s.is_empty()) {
                meta.insert("cmd:cwd".to_string(), json!(cwd));
            }
        }
        "browser" => {
            let url = cmd.url.as_deref().filter(|s| !s.is_empty())
                .ok_or_else(|| "MISSING_ARG: view=browser requires 'url'".to_string())?;
            meta.insert("view".to_string(), json!("browser"));
            meta.insert("url".to_string(), json!(url));
        }
        "sysinfo" => {
            meta.insert("view".to_string(), json!("sysinfo"));
        }
        "help" => {
            meta.insert("view".to_string(), json!("help"));
        }
        "media" => {
            let file = cmd.file.as_deref().filter(|s| !s.is_empty())
                .ok_or_else(|| "MISSING_ARG: view=media requires 'file'".to_string())?;
            meta.insert("view".to_string(), json!("media"));
            meta.insert("media:path".to_string(), json!(file));
        }
        other => {
            return Err(format!(
                "INVALID_VIEW: unsupported view '{other}' (expected editor/term/browser/sysinfo/help/media)"
            ));
        }
    }

    if let Some(title) = cmd.title.as_deref().filter(|s| !s.is_empty()) {
        meta.insert("frame:title".to_string(), json!(title));
    }

    Ok(meta)
}

/// WPS event asking an already-mounted Editor pane to open an additional
/// file as a new tab. Payload is `{ path }` — the frontend's `EditorViewModel`
/// calls its own existing `openFile(path)` on receipt, so pin-if-existing/
/// language-detection/RPC-load all apply unchanged. Scoped `block:<id>`,
/// matching `EVENT_EDITOR_FILE_CHANGED`'s existing shape exactly.
/// See SPEC_EDITOR_MCP_OPEN_BLANK_PREVIEW_AND_PANE_REUSE_2026_08_03.md Part 2.
pub(super) const EVENT_EDITOR_OPEN_FILE_REQUEST: &str = "editor:open_file_request";

/// Persist a handful of open-file requests per block (`Broker::subscribe`'s
/// replay-on-subscribe, `agentmux-srv/src/backend/wps.rs:206-239`) rather
/// than firing with `persist: 0`. Closes a real race (codex P1 on PR #2404):
/// a just-created Editor block may not have finished mounting its
/// `EditorViewModel` (and installing this event's subscription) by the time
/// a second back-to-back `OpenEditor` call reuses it — with `persist: 0` that
/// second call's request would be published to zero subscribers and silently
/// lost. A small positive count also survives *several* such back-to-back
/// calls arriving before the pane ever mounts, not just one.
const OPEN_FILE_REQUEST_PERSIST_COUNT: usize = 20;

/// If the calling agent (identified by its own block id, `caller_block_id`)
/// already has an Editor pane open in its own tab, push `file` into that
/// pane as a new tab instead of creating another Editor pane. Returns
/// `Ok(None)` when there's no existing Editor pane to reuse (or the caller's
/// tab can't be resolved) — the normal create-new-block path handles that
/// case unchanged.
///
/// `focus` mirrors `open_pane`'s own `cmd.focus.unwrap_or(true)` default
/// (reagent P1 on PR #2404: the reuse path returns before the create path's
/// `focused` layout-action logic ever runs, so without this the reused
/// pane's tab would silently never become the layout's focused node — every
/// other `pane.open` caller gets `focus` honored, reuse must too).
pub(super) async fn maybe_reuse_editor_pane(
    state: &AppState,
    caller_block_id: &str,
    file: &str,
    focus: Option<bool>,
) -> Result<Option<PaneOpenResult>, String> {
    let wstore = &state.wstore;
    let tab_id = match super::resolve_tab_id_for_block(wstore, caller_block_id) {
        Ok(id) => id,
        Err(_) => return Ok(None), // caller's own block isn't in any known tab — fall through
    };

    let existing = match super::find_editor_block(wstore, &tab_id)? {
        Some(block) => block,
        None => return Ok(None),
    };

    state.broker.publish(crate::backend::wps::WaveEvent {
        event: EVENT_EDITOR_OPEN_FILE_REQUEST.to_string(),
        scopes: vec![format!("block:{}", existing.oid)],
        sender: String::new(),
        persist: OPEN_FILE_REQUEST_PERSIST_COUNT,
        data: Some(json!({ "path": file })),
    });

    // Bring the reused pane's tab into layout focus, same default as the
    // create-new-block path. Best-effort, matching this file's other
    // reducer-dispatch error handling (a focus failure shouldn't fail the
    // whole reuse — the file still opens as a tab either way).
    if focus.unwrap_or(true) {
        let focus_events = crate::server::service::dispatch_to_reducer(
            state,
            agentmux_common::ipc::Command::SetFocusedNode {
                tab_id: tab_id.clone(),
                node_id: existing.oid.clone(),
            },
        )
        .await;
        for ev in &focus_events {
            if let Err(e) = crate::persist_subscriber::apply_event_to_wstore(ev, wstore) {
                tracing::warn!("pane.open: reuse: SetFocusedNode wstore apply failed: {e}");
            }
        }
        crate::server::service::publish_events(state, &focus_events);
    }

    tracing::info!(
        block_id = %existing.oid,
        tab_id = %tab_id,
        "pane.open: reused existing editor pane instead of creating a new one"
    );

    Ok(Some(PaneOpenResult {
        block_id: existing.oid,
        tab_id,
        view: "editor".to_string(),
        created: false,
    }))
}

/// Translate `split_direction` + `split_reference_block_id` into the backend
/// layout action triple. Returns `(actiontype, targetblockid, position)`.
/// Falls back to a plain `insert` if direction/reference are missing.
pub(super) fn resolve_placement(
    direction: Option<&str>,
    reference: Option<&str>,
) -> (String, String, String) {
    let reference = match reference.filter(|s| !s.is_empty()) {
        Some(r) => r,
        None => return ("insert".to_string(), String::new(), String::new()),
    };

    let (actiontype, position) = match direction {
        Some("right") => (crate::backend::wcore::LAYOUT_ACTION_SPLIT_HORIZONTAL, "after"),
        Some("left") => (crate::backend::wcore::LAYOUT_ACTION_SPLIT_HORIZONTAL, "before"),
        Some("down") | Some("below") => (crate::backend::wcore::LAYOUT_ACTION_SPLIT_VERTICAL, "after"),
        Some("up") | Some("above") => (crate::backend::wcore::LAYOUT_ACTION_SPLIT_VERTICAL, "before"),
        _ => return ("insert".to_string(), String::new(), String::new()),
    };

    (actiontype.to_string(), reference.to_string(), position.to_string())
}
