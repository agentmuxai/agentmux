// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `window` service handler (GetWindow, CreateWindow, CloseWindow, …).

use crate::backend::obj::*;
use crate::backend::service::{self, WebCallType, WebReturnType};

use super::super::AppState;
use super::reducer_helpers::{dispatch_to_reducer, publish_events};

pub(super) async fn handle_window_service(state: &AppState, call: &WebCallType) -> WebReturnType {
    let store = &state.wstore;
    let args = &call.args;
    match call.method.as_str() {
        "GetWindow" => {
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
        "CreateWindow" => {
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
                                // Internal compensation path — not
                                // saga-driven (Step 5 PR 2 added the
                                // `force` flag for saga provenance).
                                force: false,
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
                    // Seed the default 3-pane launch layout (agent + sysinfo +
                    // swarm) into the fresh tab so "Open another window" matches
                    // first launch instead of opening blank. Only this
                    // fresh-workspace branch seeds; tear-off (existing workspace,
                    // the `else` arm) reattaches its populated workspace as-is.
                    // Non-fatal: a seed failure leaves an empty tab (the prior
                    // behaviour) rather than failing window creation.
                    // See docs/retro/retro-blank-new-window-2026-06-21.md.
                    //
                    // 2nd-window-tear-off desync fix (#1681): the seed blocks
                    // MUST be created through the reducer (CreateBlock command),
                    // NOT via the store-only `seed_default_layout`/`create_block`
                    // path. This handler runs AFTER bootstrap, so anything
                    // written straight to SQLite is invisible to the in-memory
                    // reducer `srv_state` until the next restart. A new window
                    // seeded store-only renders fine (frontend reads SQLite) but
                    // its blocks are absent from `srv_state`, so a later
                    // `TearOffBlock` from that window is rejected "block not
                    // found" (ws/tab exist — they went through the reducer — but
                    // the block did not). Dispatch CreateBlock per pane so the
                    // blocks land in BOTH srv_state and (via the subscriber)
                    // SQLite, then write the shared layout referencing them.
                    let mut block_seed_events: Vec<agentmux_common::ipc::Event> = Vec::new();
                    if let Some(new_tab_id) = tab_events.iter().find_map(|e| match e {
                        agentmux_common::ipc::Event::TabCreated { tab_id, .. } => {
                            Some(tab_id.clone())
                        }
                        _ => None,
                    }) {
                        // Dispatch the three seed blocks through the reducer.
                        let mut seeded_ids: Vec<String> = Vec::new();
                        for view in ["agent", "sysinfo", "swarm"] {
                            let evs = dispatch_to_reducer(
                                state,
                                agentmux_common::ipc::Command::CreateBlock {
                                    tab_id: new_tab_id.clone(),
                                    meta: serde_json::json!({ "view": view }),
                                },
                            )
                            .await;
                            if let Some(err_msg) = evs.iter().find_map(|e| match e {
                                agentmux_common::ipc::Event::Error { message, .. } => {
                                    Some(message.clone())
                                }
                                _ => None,
                            }) {
                                tracing::warn!(
                                    tab_id = %new_tab_id,
                                    view = %view,
                                    error = %err_msg,
                                    "CreateWindow: seed block create failed — opening blank tab"
                                );
                                break;
                            }
                            for ev in &evs {
                                if let Err(e) =
                                    crate::persist_subscriber::apply_event_to_wstore(ev, store)
                                {
                                    tracing::warn!(
                                        tab_id = %new_tab_id,
                                        error = %e,
                                        "CreateWindow: seed block SQLite write failed"
                                    );
                                }
                            }
                            if let Some(block_id) = evs.iter().find_map(|e| match e {
                                agentmux_common::ipc::Event::BlockCreated { block_id, .. } => {
                                    Some(block_id.clone())
                                }
                                _ => None,
                            }) {
                                seeded_ids.push(block_id);
                            }
                            block_seed_events.extend(evs);
                        }

                        if seeded_ids.len() == 3 {
                            // SPEC_864 Phase 3 — post-bootstrap seed routes
                            // through the reducer (single writer of
                            // db_layout); the tree shape is shared with the
                            // pre-bootstrap store-direct first-launch seed
                            // via `default_three_pane_tree`.
                            let (tree, focused, leaforder) =
                                crate::backend::wcore::default_three_pane_tree(
                                    &seeded_ids[0],
                                    &seeded_ids[1],
                                    &seeded_ids[2],
                                );
                            if let Err(e) = super::reducer_helpers::seed_layout_via_reducer(
                                state,
                                &new_tab_id,
                                tree,
                                focused,
                                leaforder,
                            )
                            .await
                            {
                                tracing::warn!(
                                    tab_id = %new_tab_id,
                                    error = %e,
                                    "CreateWindow: default layout write failed — opening blank tab"
                                );
                            }
                        }
                    }
                    let mut combined = ws_events;
                    combined.extend(tab_events);
                    combined.extend(block_seed_events);
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
                            // Internal compensation path (Step 5 PR 2).
                            force: false,
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
        "CloseWindow" => {
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
            //
            // Step 5 PR 2 — route the user-initiated cascade through
            // the `delete_workspace` saga instead of dispatching
            // `Command::DeleteWorkspace` inline. The saga records
            // lifecycle brackets in the durable saga log so a crash
            // mid-cascade is recoverable via `recovery::compensate_unresolved`.
            // The saga also takes a snapshot of the workspace's
            // tabs+blocks before issuing the cascade, so the durable
            // log captures what was deleted (provenance for
            // `--diag sagas`).
            if let Some(ws_id) = ws_id {
                let any_other_window = {
                    let s = state.srv_state.lock().await;
                    s.windows.values().any(|w| w.workspace_id == ws_id)
                };
                if !any_other_window {
                    if let Err(e) =
                        crate::sagas::delete_workspace::run(state, ws_id.clone()).await
                    {
                        tracing::warn!(
                            workspace_id = %ws_id,
                            "CloseWindow: delete_workspace saga failed: {}",
                            e,
                        );
                    }
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
        "SwitchWorkspace" => {
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
        "SetWindowPosAndSize" => {
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
        // SPEC_PILLAR1_STEP2 (Slice A, Phase 1) — durable mirror of the
        // host's per-window opacity so a crashed/restarted host can
        // restore it (Phase 2, not yet wired: nothing calls this arm
        // today). Direct store read-modify-write, same shape as
        // SetWindowPosAndSize just above — window opacity isn't
        // reducer-tracked state (see `state::WindowRecord`), so there's
        // no split-brain risk to route around the way #864's layout
        // tree had. `opacity: None` clears back to fully-opaque/unset —
        // that's a real, destructive state change (unlike pos/size's
        // `None` in SetWindowPosAndSize, which just means "skip this
        // field"), so unlike that handler a malformed (non-null,
        // non-f32) argument must surface as an error, not get silently
        // treated as an explicit clear. (reagent P1.)
        "SetWindowOpacity" => {
            let window_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let opacity: Option<f32> = match service::get_optional_arg(args, 1) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            if let Some(o) = opacity {
                if !(0.0..=1.0).contains(&o) {
                    return WebReturnType::error(format!(
                        "SetWindowOpacity: opacity must be in 0.0..=1.0, got {o}"
                    ));
                }
            }
            match store.must_get::<Window>(&window_id) {
                Ok(mut win) => {
                    win.opacity = opacity;
                    match store.update(&mut win) {
                        Ok(_) => WebReturnType::success_empty(),
                        Err(e) => WebReturnType::error(e.to_string()),
                    }
                }
                Err(e) => WebReturnType::error(e.to_string()),
            }
        }
        // SPEC_PILLAR1_STEP3 — durable mirror of a window's kind + parent
        // linkage, so a future reproject can tell which native-window
        // creation path to drive for each window in `Client.windowids`.
        // Direct store read-modify-write, same shape as SetWindowOpacity
        // just above — kind/parent aren't reducer-tracked state (see
        // `state::WindowRecord`), so there's no split-brain risk to route
        // around.
        "SetWindowTopology" => {
            let window_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let kind: Option<String> = match service::get_optional_arg(args, 1) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let parent_window_id: Option<String> = match service::get_optional_arg(args, 2) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            if let Some(k) = &kind {
                if k != "full_instance" && k != "subwindow" {
                    return WebReturnType::error(format!(
                        "SetWindowTopology: kind must be 'full_instance' or 'subwindow', got '{k}'"
                    ));
                }
                if k == "subwindow" && parent_window_id.is_none() {
                    return WebReturnType::error(
                        "SetWindowTopology: kind='subwindow' requires a non-null parent_window_id"
                            .to_string(),
                    );
                }
            }
            match store.must_get::<Window>(&window_id) {
                Ok(mut win) => {
                    win.kind = kind;
                    win.parent_window_id = parent_window_id;
                    match store.update(&mut win) {
                        Ok(_) => WebReturnType::success_empty(),
                        Err(e) => WebReturnType::error(e.to_string()),
                    }
                }
                Err(e) => WebReturnType::error(e.to_string()),
            }
        }
        _ => WebReturnType::error(format!("unknown window method: {}", call.method)),
    }
}

#[cfg(test)]
mod create_window_seed_tests {
    use super::handle_window_service;
    use crate::backend::service::WebCallType;
    use crate::server::tests::test_state;

    fn create_window_call(workspace_id: &str) -> WebCallType {
        WebCallType {
            service: "window".to_string(),
            method: "CreateWindow".to_string(),
            uicontext: None,
            // arg0 is ignored by the handler; arg1 is the (optional) workspace
            // to reattach. Empty → fresh-workspace seed path.
            args: vec![
                serde_json::Value::Null,
                serde_json::Value::String(workspace_id.to_string()),
            ],
        }
    }

    /// Regression for the 2nd-window-tear-off desync (#1681).
    ///
    /// "Open another window" used to seed its three default blocks straight
    /// into SQLite (`seed_default_layout` → `create_block`), bypassing the
    /// reducer. The handler runs after bootstrap, so those blocks never
    /// reached the in-memory `srv_state`. The frontend rendered them (it reads
    /// SQLite) but a subsequent `TearOffBlock` from that window was rejected
    /// "block not found" — the workspace/tab existed (they went through the
    /// reducer) but the block did not. This asserts the seed blocks now land in
    /// `srv_state` AND that a tear-off from the new window succeeds end-to-end.
    #[tokio::test]
    async fn new_window_seed_blocks_are_in_reducer_state_and_tear_off_succeeds() {
        let state = test_state();
        let blocks_before = state.srv_state.lock().await.blocks.len();

        let ret = handle_window_service(&state, &create_window_call("")).await;
        assert!(ret.success, "CreateWindow failed: {:?}", ret.error);

        let win = ret.data.expect("CreateWindow returns the Window");
        let workspace_id = win
            .get("workspaceid")
            .and_then(|v| v.as_str())
            .expect("window has a workspaceid")
            .to_string();

        // The fix: the three seed blocks are present in the in-memory reducer
        // state, attached to the new window's tab.
        let (tab_id, block_id) = {
            let s = state.srv_state.lock().await;
            assert_eq!(
                s.blocks.len(),
                blocks_before + 3,
                "the 3 seed blocks must be tracked in srv_state, not only SQLite"
            );
            let ws = s
                .workspaces
                .get(&workspace_id)
                .expect("new workspace is in the reducer");
            let tab_id = ws.tab_ids.first().expect("new workspace has a tab").clone();
            let tab = s.tabs.get(&tab_id).expect("new tab is in the reducer");
            assert_eq!(
                tab.block_ids.len(),
                3,
                "the new window's tab must hold its 3 seed blocks in the reducer"
            );
            (tab_id, tab.block_ids[0].clone())
        };

        // End-to-end: tearing a block off the freshly-created window no longer
        // hits the "block not found" pre-condition.
        let result =
            crate::sagas::tear_off_block::run(&state, block_id, tab_id, workspace_id).await;
        assert!(
            result.is_ok(),
            "tear-off from a freshly-created window must succeed, got: {:?}",
            result.err()
        );
    }
}

/// SPEC_PILLAR1_STEP2 Slice A Phase 1 — `SetWindowOpacity` unit tests.
#[cfg(test)]
mod set_window_opacity_tests {
    use super::handle_window_service;
    use crate::backend::obj::Window;
    use crate::backend::service::WebCallType;
    use crate::server::tests::test_state;

    fn call(window_id: &str, opacity: Option<f32>) -> WebCallType {
        WebCallType {
            service: "window".to_string(),
            method: "SetWindowOpacity".to_string(),
            uicontext: None,
            args: vec![
                serde_json::Value::String(window_id.to_string()),
                opacity
                    .map(|o| serde_json::json!(o))
                    .unwrap_or(serde_json::Value::Null),
            ],
        }
    }

    async fn seeded_window_id(state: &crate::server::AppState) -> String {
        let ret = handle_window_service(
            state,
            &WebCallType {
                service: "window".to_string(),
                method: "CreateWindow".to_string(),
                uicontext: None,
                args: vec![
                    serde_json::Value::Null,
                    serde_json::Value::String(String::new()),
                ],
            },
        )
        .await;
        assert!(ret.success, "CreateWindow failed: {:?}", ret.error);
        ret.data
            .expect("CreateWindow returns the Window")
            .get("oid")
            .and_then(|v| v.as_str())
            .expect("window has an oid")
            .to_string()
    }

    #[tokio::test]
    async fn sets_and_persists_opacity() {
        let state = test_state();
        let window_id = seeded_window_id(&state).await;

        let ret = handle_window_service(&state, &call(&window_id, Some(0.85))).await;
        assert!(ret.success, "SetWindowOpacity failed: {:?}", ret.error);

        let win = state.wstore.must_get::<Window>(&window_id).unwrap();
        assert_eq!(win.opacity, Some(0.85));
    }

    #[tokio::test]
    async fn null_opacity_clears_it() {
        let state = test_state();
        let window_id = seeded_window_id(&state).await;

        handle_window_service(&state, &call(&window_id, Some(0.5)))
            .await;
        let ret = handle_window_service(&state, &call(&window_id, None)).await;
        assert!(ret.success, "SetWindowOpacity (clear) failed: {:?}", ret.error);

        let win = state.wstore.must_get::<Window>(&window_id).unwrap();
        assert_eq!(win.opacity, None, "None must clear back to unset/opaque");
    }

    #[tokio::test]
    async fn rejects_out_of_range_opacity() {
        let state = test_state();
        let window_id = seeded_window_id(&state).await;

        for bad in [-0.1_f32, 1.1_f32] {
            let ret = handle_window_service(&state, &call(&window_id, Some(bad))).await;
            assert!(!ret.success, "opacity {bad} must be rejected");
        }
        // Rejected — window's opacity must stay unset.
        let win = state.wstore.must_get::<Window>(&window_id).unwrap();
        assert_eq!(win.opacity, None);
    }

    #[tokio::test]
    async fn errors_on_unknown_window() {
        let state = test_state();
        let ret = handle_window_service(&state, &call("ghost-window", Some(0.5))).await;
        assert!(!ret.success, "unknown window must error");
    }

    /// reagent P1: a malformed (non-null, non-f32) opacity argument must
    /// surface as an error, not be silently swallowed into an explicit
    /// clear — `None` here means "wipe the persisted opacity," a real
    /// state change, so a parse failure must not be conflated with it.
    #[tokio::test]
    async fn malformed_opacity_errors_and_does_not_clear() {
        let state = test_state();
        let window_id = seeded_window_id(&state).await;

        let set_ret = handle_window_service(&state, &call(&window_id, Some(0.6))).await;
        assert!(set_ret.success);

        let bad_call = WebCallType {
            service: "window".to_string(),
            method: "SetWindowOpacity".to_string(),
            uicontext: None,
            args: vec![
                serde_json::Value::String(window_id.clone()),
                serde_json::Value::String("not-a-number".to_string()),
            ],
        };
        let ret = handle_window_service(&state, &bad_call).await;
        assert!(!ret.success, "malformed opacity must error, not be treated as clear");

        let win = state.wstore.must_get::<Window>(&window_id).unwrap();
        assert_eq!(
            win.opacity,
            Some(0.6),
            "a rejected malformed argument must not wipe the previously-set opacity"
        );
    }

    #[tokio::test]
    async fn boundary_values_are_accepted() {
        let state = test_state();
        let window_id = seeded_window_id(&state).await;

        for boundary in [0.0_f32, 1.0_f32] {
            let ret = handle_window_service(&state, &call(&window_id, Some(boundary))).await;
            assert!(ret.success, "boundary opacity {boundary} must be accepted");
            let win = state.wstore.must_get::<Window>(&window_id).unwrap();
            assert_eq!(win.opacity, Some(boundary));
        }
    }
}

/// SPEC_PILLAR1_STEP3 — `SetWindowTopology` unit tests.
#[cfg(test)]
mod set_window_topology_tests {
    use super::handle_window_service;
    use crate::backend::obj::Window;
    use crate::backend::service::WebCallType;
    use crate::server::tests::test_state;

    fn call(window_id: &str, kind: Option<&str>, parent_window_id: Option<&str>) -> WebCallType {
        WebCallType {
            service: "window".to_string(),
            method: "SetWindowTopology".to_string(),
            uicontext: None,
            args: vec![
                serde_json::Value::String(window_id.to_string()),
                kind.map(|k| serde_json::json!(k)).unwrap_or(serde_json::Value::Null),
                parent_window_id
                    .map(|p| serde_json::json!(p))
                    .unwrap_or(serde_json::Value::Null),
            ],
        }
    }

    async fn seeded_window_id(state: &crate::server::AppState) -> String {
        let ret = handle_window_service(
            state,
            &WebCallType {
                service: "window".to_string(),
                method: "CreateWindow".to_string(),
                uicontext: None,
                args: vec![
                    serde_json::Value::Null,
                    serde_json::Value::String(String::new()),
                ],
            },
        )
        .await;
        assert!(ret.success, "CreateWindow failed: {:?}", ret.error);
        ret.data
            .expect("CreateWindow returns the Window")
            .get("oid")
            .and_then(|v| v.as_str())
            .expect("window has an oid")
            .to_string()
    }

    #[tokio::test]
    async fn sets_and_persists_full_instance() {
        let state = test_state();
        let window_id = seeded_window_id(&state).await;

        let ret = handle_window_service(&state, &call(&window_id, Some("full_instance"), None)).await;
        assert!(ret.success, "SetWindowTopology failed: {:?}", ret.error);

        let win = state.wstore.must_get::<Window>(&window_id).unwrap();
        assert_eq!(win.kind, Some("full_instance".to_string()));
        assert_eq!(win.parent_window_id, None);
    }

    #[tokio::test]
    async fn sets_and_persists_subwindow_with_parent() {
        let state = test_state();
        let parent_id = seeded_window_id(&state).await;
        let window_id = seeded_window_id(&state).await;

        let ret =
            handle_window_service(&state, &call(&window_id, Some("subwindow"), Some(&parent_id))).await;
        assert!(ret.success, "SetWindowTopology failed: {:?}", ret.error);

        let win = state.wstore.must_get::<Window>(&window_id).unwrap();
        assert_eq!(win.kind, Some("subwindow".to_string()));
        assert_eq!(win.parent_window_id, Some(parent_id));
    }

    #[tokio::test]
    async fn rejects_subwindow_without_parent() {
        let state = test_state();
        let window_id = seeded_window_id(&state).await;

        let ret = handle_window_service(&state, &call(&window_id, Some("subwindow"), None)).await;
        assert!(!ret.success, "subwindow with no parent_window_id must be rejected");

        let win = state.wstore.must_get::<Window>(&window_id).unwrap();
        assert_eq!(win.kind, None, "rejected write must not persist");
    }

    #[tokio::test]
    async fn rejects_unknown_kind() {
        let state = test_state();
        let window_id = seeded_window_id(&state).await;

        let ret = handle_window_service(&state, &call(&window_id, Some("floating"), None)).await;
        assert!(!ret.success, "unknown kind must be rejected");
    }

    #[tokio::test]
    async fn null_kind_clears_it() {
        let state = test_state();
        let window_id = seeded_window_id(&state).await;

        handle_window_service(&state, &call(&window_id, Some("full_instance"), None)).await;
        let ret = handle_window_service(&state, &call(&window_id, None, None)).await;
        assert!(ret.success, "SetWindowTopology (clear) failed: {:?}", ret.error);

        let win = state.wstore.must_get::<Window>(&window_id).unwrap();
        assert_eq!(win.kind, None);
        assert_eq!(win.parent_window_id, None);
    }

    #[tokio::test]
    async fn errors_on_unknown_window() {
        let state = test_state();
        let ret = handle_window_service(&state, &call("ghost-window", Some("full_instance"), None)).await;
        assert!(!ret.success, "unknown window must error");
    }
}
