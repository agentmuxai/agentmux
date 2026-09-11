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
        // One batched frame so the renderer applies all of them in a single
        // reactive flush — see EventBus::broadcast_wave_obj_updates.
        event_bus.broadcast_wave_obj_updates(&updates);
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

/// Block-meta key carrying an ARRAY of files the reused pane should open —
/// the sole delivery path (see below for why an earlier version's second,
/// "live WPS event" path was removed). Drained (all entries, in order)
/// reactively by `EditorViewModel`'s `createEffect` over its own block meta,
/// then cleared immediately after — covers both "not yet mounted when this
/// was written" and "already mounted, reacts as soon as the write lands"
/// uniformly through the same WaveObj sync path the pane already depends on
/// for everything else.
///
/// **Array, not a single scalar** (codex P1 on PR #2404): if 2+ `OpenEditor`
/// reuse calls arrive before the target pane ever mounts, a single-value key
/// would have each call overwrite the last, silently losing every request
/// but the final one. Appending to an array and draining all of them at
/// once fixes that.
///
/// **Sole delivery path — no separate live WPS event** (codex P1 on PR
/// #2404, found twice): an earlier version ALSO fired a direct WPS event
/// (`persist: 0`) alongside this meta write, for immediate delivery when the
/// pane was already mounted. First finding: the frontend's live handler
/// didn't clear its own entry from this array, so it could be reprocessed
/// on a later, unrelated remount. Second, deeper finding after fixing that:
/// the WPS event is a direct WS push and arrives essentially synchronously,
/// while THIS meta write only reaches the frontend's `blockAtom` after an
/// async WaveObj DB-refetch — so the live handler's own dequeue attempt
/// could run and read stale data (this exact write not yet reflected)
/// before it landed, no-op, and strand the entry anyway. Removing the
/// separate live path entirely (rather than patching a second-order race in
/// its own race-fix) leaves one delivery mechanism and one reactive
/// consumer — nothing to race. Trades a small amount of latency for the
/// already-mounted case (a real WaveObj round-trip instead of a direct
/// push) for not being racy.
///
/// **Superseded relying on WPS `persist > 0` for durability** (codex P1 on
/// PR #2404, earliest finding on this function): a just-created Editor
/// block may not have finished mounting its `EditorViewModel` by the time a
/// second back-to-back `OpenEditor` call reuses it. `persist: N` closes that
/// race but opens a *worse* one: `Broker::unsubscribe_all` clears a route's
/// replay marker on disconnect (`agentmux-srv/src/backend/wps.rs:312-323`),
/// so any later, unrelated reconnect would replay the *entire* persisted
/// history again — reopening files the user has since closed. The broker
/// has no ack/consume concept, so nothing marks a persisted event "already
/// delivered." Block meta does have exactly that shape (write once, drain
/// once, clear after reading) — reusing it avoids inventing new broker
/// machinery.
const META_PENDING_OPEN_FILES: &str = "editor:pending_open_files";

/// If the calling agent (identified by its own block id, `caller_block_id`)
/// already has an Editor pane open in its own tab, push `file` into that
/// pane as a new tab instead of creating another Editor pane. Returns
/// `Ok(None)` when there's no existing Editor pane to reuse (or the caller's
/// tab can't be resolved) — the normal create-new-block path handles that
/// case unchanged.
///
/// **Known, accepted limitation: does not apply layout focus.** An earlier
/// version of this function resolved the reused block's layout leaf id and
/// dispatched `Command::SetFocusedNode` — reagent (PR #2404) confirmed the
/// leaf-id resolution itself was correct, then found a deeper problem:
/// the frontend's `onBackendUpdate` (`frontend/layout/lib/layoutPersistence.ts:59-82`)
/// only re-derives `focusedNodeId` at initial model construction or via a
/// `pendingBackendActions`-driven tree action — a bare `focusednodeid`
/// WaveObj push to an ALREADY-MOUNTED `LayoutModel` (confirmed by reading
/// the function directly: no branch reads `waveObj.focusednodeid` outside
/// those two triggers) is silently never applied to the live `treeState`.
/// Making that work would mean changing `onBackendUpdate`'s reactivity —
/// shared code this file's own comments show has deliberately been kept
/// narrow/non-reactive before (see the dangling-leaf-prune history right
/// above it) — a real, separate piece of work, not a one-line fix. Shipping
/// a backend dispatch that compiles and passes a reducer-internal test but
/// has no visible frontend effect would be worse than not attempting it: it
/// would look done without being done. The reused pane's new tab still
/// becomes its *own* active tab via `openFile()`; only the cross-pane
/// layout-focus indicator is unaffected, same as before this feature
/// existed.
pub(super) async fn maybe_reuse_editor_pane(
    state: &AppState,
    caller_block_id: &str,
    file: &str,
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

    // Durable delivery path: append to any already-pending queue (read then
    // write — a small race window under truly concurrent reuse calls is
    // accepted, matching this codebase's general best-effort meta-patch
    // posture elsewhere) so a not-yet-mounted (or remounting)
    // EditorViewModel drains every pending file once at construction, then
    // clears the queue — see META_PENDING_OPEN_FILES's doc comment for why
    // this replaces relying on WPS persist/replay for correctness.
    let mut pending: Vec<String> = existing
        .meta
        .get(META_PENDING_OPEN_FILES)
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    pending.push(file.to_string());

    let meta_events = crate::server::service::dispatch_to_reducer(
        state,
        agentmux_common::ipc::Command::UpdateBlockMeta {
            block_id: existing.oid.clone(),
            meta_patch: json!({ META_PENDING_OPEN_FILES: pending }),
        },
    )
    .await;
    for ev in &meta_events {
        if let Err(e) = crate::persist_subscriber::apply_event_to_wstore(ev, wstore) {
            tracing::warn!("pane.open: reuse: UpdateBlockMeta wstore apply failed: {e}");
        }
    }
    crate::server::service::publish_events(state, &meta_events);

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

/// `POST /api/v1/agent/pane/close` — backs the `ClosePane` MCP tool.
/// See docs/specs/SPEC_AGENT_PANE_LIFECYCLE_CONTROL_2026_09_10.md.
///
/// Identity is always verified first via the same `verified_block_id`
/// mechanism `UIClick`/`UIQuery`/`UIScreenshot` use (§5.0 of that spec) —
/// this both resolves the caller's OWN pane when `req.block_id` is absent,
/// and supplies a real, checked `source_agent` for the audit log when
/// `req.block_id` targets another agent's pane (fleet tier, §5.1/§5.2): no
/// ownership check on the TARGET (closing another agent's stuck pane without
/// needing its cooperation is this spec's whole motivation), but the CALLER
/// is never anonymous the way `FleetBulkStop`'s existing calls are today.
pub(crate) async fn handle_close_pane(
    axum::extract::State(state): axum::extract::State<AppState>,
    axum::Json(req): axum::Json<agentmux_common::api_types::ClosePaneRequest>,
) -> impl axum::response::IntoResponse {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use axum::Json;

    let caller_agent_id = req.auth.agent_id.clone();
    let verified_own_block_id = match crate::server::ui_handlers::verified_block_id(&state, &req.auth) {
        Ok(b) => b,
        Err(e) => return (StatusCode::UNAUTHORIZED, Json(json!({ "error": e }))).into_response(),
    };

    let target_block_id = req.block_id.clone().unwrap_or(verified_own_block_id);
    let is_cross_pane = req.block_id.is_some();

    // codex P2 on PR #3193: resolve the target's agent name BEFORE deleting
    // the block, not after. `delete_block::run` stops the block's
    // controller, and a running agent's own exit-triggered
    // `unregister_block` can race ahead of this lookup once it does — a
    // post-delete lookup would then fall back to the bare block UUID
    // instead of the real agent name, making attribution timing-dependent.
    let target_agent = state
        .reactive_handler
        .get_agent_by_block(&target_block_id)
        .map(|a| a.agent_id)
        .unwrap_or_else(|| target_block_id.clone());

    let tab_id = {
        let s = state.srv_state.lock().await;
        match s.blocks.get(&target_block_id) {
            Some(b) => b.tab_id.clone(),
            None => {
                let err = format!("block not found: {target_block_id}");
                if is_cross_pane {
                    let request_id = uuid::Uuid::new_v4().to_string();
                    state.reactive_handler.log_fleet_action_audit(
                        Some(&caller_agent_id), &target_agent, &target_block_id,
                        "pane.close", false, Some(&err), &request_id, req.reason.as_deref(),
                    );
                }
                return (StatusCode::NOT_FOUND, Json(json!({ "error": err }))).into_response();
            }
        }
    };

    let result = crate::sagas::delete_block::run(&state, tab_id, target_block_id.clone()).await;

    if is_cross_pane {
        let request_id = uuid::Uuid::new_v4().to_string();
        match &result {
            Ok(_) => state.reactive_handler.log_fleet_action_audit(
                Some(&caller_agent_id), &target_agent, &target_block_id,
                "pane.close", true, None, &request_id, req.reason.as_deref(),
            ),
            Err(e) => state.reactive_handler.log_fleet_action_audit(
                Some(&caller_agent_id), &target_agent, &target_block_id,
                "pane.close", false, Some(e), &request_id, req.reason.as_deref(),
            ),
        }
    }

    match result {
        Ok(v) => (StatusCode::OK, Json(v)).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response(),
    }
}

#[cfg(test)]
mod close_pane_tests {
    use super::*;
    use axum::response::IntoResponse;
    use crate::server::tests::test_state;
    use agentmux_common::api_types::{ClosePaneRequest, UiAutomationAuth};
    use agentmux_common::ipc::{Command, Event};

    async fn dispatch_apply(state: &AppState, cmd: Command) -> Vec<Event> {
        let evs = crate::server::service::dispatch_to_reducer(state, cmd).await;
        for ev in &evs {
            crate::persist_subscriber::apply_event_to_wstore(ev, &state.wstore).unwrap();
        }
        evs
    }

    /// Seed a workspace + tab + block, same shape as
    /// `sagas::delete_block::tests::seed` — duplicated rather than shared
    /// across modules (both are small, private `#[cfg(test)]` helpers).
    async fn seed(state: &AppState) -> (String, String) {
        let ws_evs = dispatch_apply(state, Command::CreateWorkspace { name: "w".into() }).await;
        let ws_id = ws_evs
            .iter()
            .find_map(|e| match e {
                Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
                _ => None,
            })
            .unwrap();
        let tab_evs = dispatch_apply(
            state,
            Command::CreateTab { workspace_id: ws_id, name: "t".into() },
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
            state,
            Command::CreateBlock { tab_id: tab_id.clone(), meta: serde_json::Value::Null },
        )
        .await;
        let block_id = blk_evs
            .iter()
            .find_map(|e| match e {
                Event::BlockCreated { block_id, .. } => Some(block_id.clone()),
                _ => None,
            })
            .unwrap();
        (tab_id, block_id)
    }

    /// Mint a real signing key for `agent_id` (same store call
    /// `agent_open`/spawn use) and sign a fresh `UiAutomationAuth` for it —
    /// the exact mechanism `agentmux-mcp`'s `sign_ui_automation_auth` uses,
    /// reproduced here since this is a different crate.
    fn sign_auth(state: &AppState, agent_id: &str) -> UiAutomationAuth {
        let key = state.wstore.agent_jekt_key_ensure(agent_id).unwrap();
        let ts_secs = agentmux_common::time::now_secs();
        let sig = agentmux_common::jekt_sign::sign_jekt(
            &key, "ui-automation-identity", agent_id, "__srv__", ts_secs, "",
        );
        UiAutomationAuth { agent_id: agent_id.to_string(), ts_secs, sig }
    }

    #[tokio::test]
    async fn own_pane_close_removes_the_caller_s_own_block() {
        let state = test_state();
        let (tab_id, block_id) = seed(&state).await;
        let agent_id = format!("close-own-{}", uuid::Uuid::new_v4());
        state.reactive_handler.register_agent(&agent_id, &block_id, Some(&tab_id)).unwrap();
        let auth = sign_auth(&state, &agent_id);

        let resp = handle_close_pane(
            axum::extract::State(state.clone()),
            axum::Json(ClosePaneRequest { auth, block_id: None, reason: None }),
        )
        .await
        .into_response();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);

        let s = state.srv_state.lock().await;
        assert!(!s.blocks.contains_key(&block_id), "own pane must be gone after close");
    }

    #[tokio::test]
    async fn cross_pane_close_removes_the_target_and_logs_the_verified_caller() {
        let state = test_state();
        // Caller: registered with its OWN pane, but acts on someone else's.
        let (caller_tab, caller_block) = seed(&state).await;
        let caller_id = format!("close-caller-{}", uuid::Uuid::new_v4());
        state.reactive_handler.register_agent(&caller_id, &caller_block, Some(&caller_tab)).unwrap();
        let auth = sign_auth(&state, &caller_id);

        // Target: a different agent's pane, entirely unrelated to the caller.
        let (target_tab, target_block) = seed(&state).await;
        let target_id = format!("close-target-{}", uuid::Uuid::new_v4());
        state.reactive_handler.register_agent(&target_id, &target_block, Some(&target_tab)).unwrap();

        let resp = handle_close_pane(
            axum::extract::State(state.clone()),
            axum::Json(ClosePaneRequest {
                auth,
                block_id: Some(target_block.clone()),
                reason: Some("verifying the fix".to_string()),
            }),
        )
        .await
        .into_response();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);

        // Target pane gone; caller's own pane untouched.
        {
            let s = state.srv_state.lock().await;
            assert!(!s.blocks.contains_key(&target_block), "target pane must be closed");
            assert!(s.blocks.contains_key(&caller_block), "caller's own pane must be untouched");
        }

        // Audit entry carries the CALLER's verified identity as source_agent —
        // not None, the gap this spec's §5.2 fixed (FleetBulkStop still has
        // it; this new call site must not repeat it).
        // get_audit_log returns most-recent-first (its own doc comment) — no
        // extra .rev() needed; the closed-block entry is naturally first.
        let entries = state.reactive_handler.get_audit_log(50);
        let entry = entries
            .iter()
            .find(|e| e.block_id == target_block)
            .expect("expected an audit entry for the closed target block");
        assert_eq!(entry.source_agent.as_deref(), Some(caller_id.as_str()));
        assert_eq!(entry.target_agent, target_id);
        assert!(entry.success);
        assert_eq!(entry.reason.as_deref(), Some("verifying the fix"));
    }

    #[tokio::test]
    async fn rejects_an_invalid_signature() {
        let state = test_state();
        let (tab_id, block_id) = seed(&state).await;
        let agent_id = format!("close-bad-sig-{}", uuid::Uuid::new_v4());
        state.reactive_handler.register_agent(&agent_id, &block_id, Some(&tab_id)).unwrap();
        // A signature signed with the WRONG key — this agent's real key was
        // never used, simulating a forged/corrupted signature.
        let bogus_key = vec![0u8; 32];
        let ts_secs = agentmux_common::time::now_secs();
        let sig = agentmux_common::jekt_sign::sign_jekt(
            &bogus_key, "ui-automation-identity", &agent_id, "__srv__", ts_secs, "",
        );
        let auth = UiAutomationAuth { agent_id, ts_secs, sig };

        let resp = handle_close_pane(
            axum::extract::State(state.clone()),
            axum::Json(ClosePaneRequest { auth, block_id: None, reason: None }),
        )
        .await
        .into_response();
        assert_eq!(resp.status(), axum::http::StatusCode::UNAUTHORIZED);

        // Nothing was touched.
        let s = state.srv_state.lock().await;
        assert!(s.blocks.contains_key(&block_id));
    }
}
