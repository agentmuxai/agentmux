// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `layout.save` — write one window as an `agentmux.layout` file
//! (docs/specs/SPEC_LAYOUT_FILES_2026_09_25.md §6.1). The walk and the
//! per-view allowlist live in `backend::layout_file`; this is the RPC shell.
//!
//! The destination normally comes from the host's Save dialog. The handler
//! still refuses anything but an absolute `*.agentmux-layout.json` path: a
//! caller holding the auth key could otherwise use this to overwrite any
//! file the user can write.

use super::*;
use crate::backend::layout_file;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let mstore = state.mstore.clone();
    engine.register_typed(COMMAND_LAYOUT_SAVE, move |req: CommandLayoutSaveData, _ctx| {
        let mstore = mstore.clone();
        async move {
            // Store reads and a file write: off the async worker.
            tokio::task::spawn_blocking(move || layout_save_impl(&mstore, req))
                .await
                .map_err(|e| format!("layout.save: task: {e}"))?
        }
    });

    let mstore = state.mstore.clone();
    engine.register_typed(COMMAND_LAYOUT_PREVIEW, move |req: CommandLayoutPreviewData, _ctx| {
        let mstore = mstore.clone();
        async move {
            tokio::task::spawn_blocking(move || layout_preview_impl(&mstore, &req.path, &live_trust_inputs()))
                .await
                .map_err(|e| format!("layout.preview: task: {e}"))?
        }
    });

    let open_state = state.clone();
    engine.register_typed(COMMAND_LAYOUT_OPEN, move |req: CommandLayoutOpenData, _ctx| {
        let state = open_state.clone();
        async move { layout_open_impl(&state, req, &live_trust_inputs()).await }
    });
}

/// What decides whether a file is trusted (spec §3.5), injected for tests.
pub(super) struct TrustInputs {
    pub own_instance: Option<String>,
    pub layouts_dir: Option<std::path::PathBuf>,
    pub home: Option<std::path::PathBuf>,
}

fn live_trust_inputs() -> TrustInputs {
    TrustInputs {
        own_instance: crate::backend::storage::wan_identity::global()
            .and_then(|w| w.instance_ensure(&crate::backend::reactive::registry::local_host_label()).ok())
            .map(|i| i.instance_id),
        layouts_dir: agentmux_common::DataPaths::from_env().map(|p| p.shared_dir.join("layouts")),
        home: dirs::home_dir(),
    }
}

/// Parse the file and build its plan. `run_commands` = `None` means "as
/// trusted".
fn load_plan(
    store: &Store,
    path: &str,
    trust: &TrustInputs,
    run_commands: Option<bool>,
) -> Result<(layout_file::ApplyPlan, bool), String> {
    let path = std::path::PathBuf::from(path);
    if !layout_file::is_layout_path(&path) {
        return Err(format!("the path must be absolute and end in {}", layout_file::LAYOUT_EXTENSION));
    }
    let doc = layout_file::read_layout_file(&path)?;
    let trusted = layout_file::is_trusted(&doc, &path, trust.own_instance.as_deref(), trust.layouts_dir.as_deref());
    let plan = layout_file::plan_from_doc(
        store,
        &doc,
        &layout_file::PlanOptions { home: trust.home.clone(), run_commands: run_commands.unwrap_or(trusted) },
    );
    Ok((plan, trusted))
}

pub(super) fn layout_preview_impl(store: &Store, path: &str, trust: &TrustInputs) -> Result<LayoutPreviewResult, String> {
    let (plan, trusted) = load_plan(store, path, trust, None).map_err(|e| format!("layout.preview: {e}"))?;
    Ok(LayoutPreviewResult {
        name: plan.name,
        trusted,
        tabs: plan
            .tabs
            .into_iter()
            .map(|t| LayoutPreviewTab { name: t.name, panes: t.summary })
            .collect(),
        commands: plan.commands,
        notes: plan.notes,
    })
}

/// Add the file's tabs to `window_id`'s workspace — or, with `new_window`,
/// to a new workspace for the caller to open a window onto — then launch
/// each agent into its placeholder through `agent.open` (spec §3.5). Nothing
/// already open is touched. An agent that can't start (not installed, live
/// elsewhere, …) leaves its pane as the agent picker and adds a note.
pub(super) async fn layout_open_impl(
    state: &AppState,
    req: CommandLayoutOpenData,
    trust: &TrustInputs,
) -> Result<LayoutOpenResult, String> {
    let store = state.mstore.clone();
    let path = req.path.clone();
    let run = req.run_commands;
    let trust_copy =
        TrustInputs { own_instance: trust.own_instance.clone(), layouts_dir: trust.layouts_dir.clone(), home: trust.home.clone() };
    let (plan, _trusted) = tokio::task::spawn_blocking(move || load_plan(&store, &path, &trust_copy, run))
        .await
        .map_err(|e| format!("layout.open: task: {e}"))?
        .map_err(|e| format!("layout.open: {e}"))?;
    if plan.tabs.is_empty() {
        return Err("layout.open: the layout has no tabs that can be opened".to_string());
    }
    let new_window = req.new_window.unwrap_or(false);
    let ws_id = if new_window {
        create_empty_workspace(state, &plan.name).await?
    } else {
        state
            .mstore
            .get::<obj::Window>(&req.window_id)
            .map_err(|e| format!("layout.open: {e}"))?
            .ok_or_else(|| format!("layout.open: no such window: {}", req.window_id))?
            .workspaceid
    };

    let replay: Vec<crate::server::service::session_restore::ReplayTab> = plan
        .tabs
        .iter()
        .map(|t| crate::server::service::session_restore::ReplayTab {
            name: t.name.clone(),
            blocks: t.blocks.clone(),
            rootnode: t.rootnode.clone(),
            focused: t.focused.clone(),
            magnified: t.magnified.clone(),
        })
        .collect();
    let (events, replayed) =
        crate::server::service::session_restore::replay_tabs(state, &ws_id, &replay, "layout.open").await;
    crate::server::service::publish_events(state, &events);

    let mut notes = plan.notes;
    let tab_ids: Vec<String> = replayed.iter().flatten().map(|r| r.tab_id.clone()).collect();
    if new_window && tab_ids.is_empty() {
        // Nothing for the new window to show: don't leave an empty workspace.
        let events = crate::server::service::dispatch_to_reducer(
            state,
            agentmux_common::ipc::Command::DeleteWorkspace { workspace_id: ws_id, force: false },
        )
        .await;
        for ev in &events {
            let _ = crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore);
        }
        crate::server::service::publish_events(state, &events);
        return Err("layout.open: none of the layout's tabs could be created".to_string());
    }
    if tab_ids.len() < plan.tabs.len() {
        notes.push(format!("{} of {} tabs couldn't be created.", plan.tabs.len() - tab_ids.len(), plan.tabs.len()));
    }

    // The layout's active tab becomes the workspace's active tab.
    if let Some(active) = plan.active_tab.and_then(|i| replayed.get(i)).and_then(|r| r.as_ref()) {
        let events = crate::server::service::dispatch_to_reducer(
            state,
            agentmux_common::ipc::Command::SetActiveTab { workspace_id: ws_id.clone(), tab_id: active.tab_id.clone() },
        )
        .await;
        for ev in &events {
            let _ = crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore);
        }
        crate::server::service::publish_events(state, &events);
    }

    // Launch each agent into its placeholder pane.
    for tab in replayed.iter().flatten() {
        for block_id in &tab.block_ids {
            let Some(block) = state.mstore.get::<obj::Block>(block_id).ok().flatten() else { continue };
            let def_id = obj::meta_get_string(&block.meta, layout_file::META_LAYOUT_AGENT, "");
            if def_id.is_empty() {
                continue;
            }
            if let Err(e) = super::agent_open::open_agent_into_block(state, &def_id, &tab.tab_id, block_id).await {
                let name = obj::meta_get_string(&block.meta, "agentName", "");
                let who = if name.is_empty() { def_id.clone() } else { name };
                notes.push(format!("An agent ({who}) didn't start, so its pane shows the agent picker: {e}"));
            }
        }
    }

    Ok(LayoutOpenResult { workspace_id: ws_id, tab_ids, notes })
}

/// A workspace with no tabs yet, for "Open in a new window": unlike
/// `CreateWindow`'s fresh workspace it gets no default tab, so the new window
/// shows only the layout. The caller opens the window onto it afterwards
/// (the host's `open_new_window { workspace_id }`, the tear-off attach path).
async fn create_empty_workspace(state: &AppState, name: &str) -> Result<String, String> {
    use agentmux_common::ipc::{Command, Event};
    let events = crate::server::service::dispatch_to_reducer(state, Command::CreateWorkspace { name: name.to_string() }).await;
    if let Some(message) = events.iter().find_map(|e| match e {
        Event::Error { message, .. } => Some(message.clone()),
        _ => None,
    }) {
        return Err(format!("layout.open: {message}"));
    }
    for ev in &events {
        crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore).map_err(|e| format!("layout.open: {e}"))?;
    }
    crate::server::service::publish_events(state, &events);
    events
        .iter()
        .find_map(|e| match e {
            Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
            _ => None,
        })
        .ok_or_else(|| "layout.open: no workspace was created".to_string())
}

pub(super) fn layout_save_impl(store: &Store, req: CommandLayoutSaveData) -> Result<LayoutSaveResult, String> {
    let path = std::path::PathBuf::from(&req.path);
    if !layout_file::is_layout_path(&path) {
        return Err(format!(
            "layout.save: the path must be absolute and end in {}",
            layout_file::LAYOUT_EXTENSION
        ));
    }
    let name = req.name.trim();
    let name = if name.is_empty() { "Layout" } else { name };
    let ctx = layout_file::ExportContext::live(req.include_resume.unwrap_or(false));
    let exported =
        layout_file::export_window(store, &req.window_id, name, &ctx).map_err(|e| format!("layout.save: {e}"))?;
    layout_file::write_layout_file(&path, &exported.doc).map_err(|e| format!("layout.save: {e}"))?;
    Ok(LayoutSaveResult { path: path.to_string_lossy().to_string(), warnings: exported.warnings })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::obj::{Block, LayoutState, Tab, Window, Workspace};
    use agentmux_common::layout_types::{LayoutNode, LayoutNodeData};

    fn one_pane_window(store: &Store) {
        let mut b = Block { oid: "b1".into(), ..Default::default() };
        b.meta.insert("view".into(), serde_json::json!("sysinfo"));
        store.insert(&mut b).unwrap();
        let mut l = LayoutState {
            oid: "l1".into(),
            rootnode: Some(LayoutNode {
                id: "n1".into(),
                data: Some(LayoutNodeData { block_id: "b1".into(), ..Default::default() }),
                ..Default::default()
            }),
            ..Default::default()
        };
        store.insert(&mut l).unwrap();
        let mut t = Tab { oid: "t1".into(), name: "T".into(), layoutstate: "l1".into(), blockids: vec!["b1".into()], ..Default::default() };
        store.insert(&mut t).unwrap();
        let mut ws = Workspace { oid: "ws1".into(), tabids: vec!["t1".into()], activetabid: "t1".into(), ..Default::default() };
        store.insert(&mut ws).unwrap();
        let mut w = Window { oid: "w1".into(), workspaceid: "ws1".into(), ..Default::default() };
        store.insert(&mut w).unwrap();
    }

    fn req(path: &std::path::Path, name: &str) -> CommandLayoutSaveData {
        CommandLayoutSaveData {
            window_id: "w1".into(),
            name: name.into(),
            path: path.to_string_lossy().to_string(),
            include_resume: None,
        }
    }

    #[test]
    fn saves_the_window_to_the_requested_file() {
        let store = Store::open_in_memory().unwrap();
        one_pane_window(&store);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mine.agentmux-layout.json");
        let out = layout_save_impl(&store, req(&path, "  Mine  ")).unwrap();
        assert!(out.warnings.is_empty());
        let doc = layout_file::parse_layout(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(doc.name, "Mine", "the name is trimmed");
        assert_eq!(doc.windows[0].tabs[0].name, "T");
    }

    #[test]
    fn a_blank_name_becomes_layout() {
        let store = Store::open_in_memory().unwrap();
        one_pane_window(&store);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.agentmux-layout.json");
        layout_save_impl(&store, req(&path, "  ")).unwrap();
        assert_eq!(layout_file::parse_layout(&std::fs::read_to_string(&path).unwrap()).unwrap().name, "Layout");
    }

    #[test]
    fn refuses_any_other_destination_and_writes_nothing() {
        let store = Store::open_in_memory().unwrap();
        one_pane_window(&store);
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("settings.json");
        std::fs::write(&target, "keep me").unwrap();
        assert!(layout_save_impl(&store, req(&target, "x")).is_err());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep me");
    }

    // ── layout.preview / layout.open (spec §3.5, Phase 2) ──

    use crate::server::tests::test_state;
    use agentmux_common::ipc::{Command, Event};

    const OWN: &str = "gr2q7gf5lh6pzfdnurnkvputhm";

    /// A layout file with one tab: an agent pane (whose definition uses a
    /// provider that doesn't exist — `agent.open` refuses it before anything
    /// is spawned, even inside a real AgentMux) beside a terminal with a
    /// command, and a sysinfo pane as a second pane tab of the terminal.
    fn write_layout(dir: &std::path::Path, instance: &str) -> std::path::PathBuf {
        let path = dir.join("work.agentmux-layout.json");
        let doc = serde_json::json!({
            "format": "agentmux.layout", "version": 1, "name": "Work",
            "saved_by": { "app": "agentmux", "app_version": "x", "instance": instance },
            "windows": [{ "active_tab": 0, "tabs": [{
                "name": "Work",
                "focused": "p2",
                "root": { "split": "row", "children": [
                    { "ratio": 0.5, "node": { "pane": "p1" } },
                    { "ratio": 0.5, "node": { "pane": "p2" } }
                ]},
                "panes": {
                    "p1": { "views": [{ "type": "agent", "config": { "agent": "rev", "name": "Rev" } }] },
                    "p2": { "views": [
                        { "type": "term", "config": { "command": "echo", "args": ["hi"] } },
                        { "type": "sysinfo" }
                    ], "active": 0 }
                }
            }]}]
        });
        std::fs::write(&path, doc.to_string()).unwrap();
        path
    }

    async fn window_with_workspace(state: &AppState) -> (String, String) {
        let events = crate::server::service::dispatch_to_reducer(state, Command::CreateWorkspace { name: "w".into() }).await;
        for ev in &events {
            crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore).unwrap();
        }
        let ws_id = events
            .iter()
            .find_map(|e| match e {
                Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
                _ => None,
            })
            .unwrap();
        let mut w = Window { oid: "win-layout".into(), workspaceid: ws_id.clone(), ..Default::default() };
        state.mstore.insert(&mut w).unwrap();
        (w.oid, ws_id)
    }

    fn with_agent(state: &AppState) {
        let mut d = crate::backend::storage::agents::test_agent_def("def-rev", "Rev", "no-such-provider", "agent", 1, "");
        d.slug = "rev".into();
        state.mstore.agent_def_insert(&mut d).unwrap();
    }

    fn trust(dir: &std::path::Path, own: Option<&str>) -> TrustInputs {
        TrustInputs { own_instance: own.map(str::to_string), layouts_dir: Some(dir.to_path_buf()), home: Some(dir.to_path_buf()) }
    }

    #[tokio::test]
    async fn preview_describes_without_opening() {
        let state = test_state();
        with_agent(&state);
        let dir = tempfile::tempdir().unwrap();
        let path = write_layout(dir.path(), "someone-else");
        let before = state.mstore.get_all::<Tab>().unwrap().len();
        let p = layout_preview_impl(&state.mstore, &path.to_string_lossy(), &trust(dir.path(), Some(OWN))).unwrap();
        assert_eq!(p.name, "Work");
        assert!(!p.trusted, "saved by another install");
        assert_eq!(p.commands, ["echo hi"]);
        assert_eq!(p.tabs.len(), 1);
        assert_eq!(p.tabs[0].panes, ["Agent: Rev", "Terminal (command not run: `echo hi`)", "System info"]);
        assert_eq!(state.mstore.get_all::<Tab>().unwrap().len(), before, "a preview creates nothing");
    }

    #[tokio::test]
    async fn open_adds_the_tabs_to_the_window_and_holds_an_untrusted_command() {
        let state = test_state();
        with_agent(&state);
        let (window_id, ws_id) = window_with_workspace(&state).await;
        let dir = tempfile::tempdir().unwrap();
        let path = write_layout(dir.path(), "someone-else");
        let out = layout_open_impl(
            &state,
            CommandLayoutOpenData { path: path.to_string_lossy().to_string(), window_id, run_commands: None, new_window: None },
            &trust(dir.path(), Some(OWN)),
        )
        .await
        .unwrap();

        assert_eq!(out.tab_ids.len(), 1);
        assert_eq!(out.workspace_id, ws_id);
        let ws = state.mstore.get::<Workspace>(&ws_id).unwrap().unwrap();
        assert!(ws.tabids.contains(&out.tab_ids[0]), "the tab lands in the window's own workspace");
        let tab = state.mstore.get::<Tab>(&out.tab_ids[0]).unwrap().unwrap();
        assert_eq!(tab.name, "Work");
        assert_eq!(tab.blockids.len(), 3);

        let blocks: Vec<Block> = tab.blockids.iter().map(|b| state.mstore.get::<Block>(b).unwrap().unwrap()).collect();
        let view = |b: &Block| obj::meta_get_string(&b.meta, "view", "");
        let term = blocks.iter().find(|b| view(b) == "term").unwrap();
        assert_eq!(obj::meta_get_string(&term.meta, "controller", ""), "shell", "untrusted: the command is held");
        assert!(!term.meta.contains_key("cmd"));
        // The agent couldn't start (no such provider): its pane stays the picker.
        let agent = blocks.iter().find(|b| view(b) == "agent").unwrap();
        assert!(obj::meta_get_string(&agent.meta, "agentId", "").is_empty());
        assert!(out.notes.iter().any(|n| n.contains("didn't start")), "{:?}", out.notes);

        // The saved tree came back with real block ids.
        let layout = state.mstore.get::<LayoutState>(&tab.layoutstate).unwrap().unwrap();
        let json = serde_json::to_string(&layout.rootnode).unwrap();
        assert!(!json.contains("__snap_block_"), "every placeholder resolved: {json}");
        for b in &tab.blockids {
            assert!(json.contains(b.as_str()), "block {b} is in the tree");
        }
        assert_eq!(layout.rootnode.as_ref().unwrap().children.len(), 2);
    }

    #[tokio::test]
    async fn a_trusted_file_runs_its_command_and_an_explicit_choice_wins() {
        let state = test_state();
        let (window_id, _) = window_with_workspace(&state).await;
        let dir = tempfile::tempdir().unwrap();
        let path = write_layout(dir.path(), OWN);
        let term_controller = |state: &AppState, tab_id: &str| {
            let tab = state.mstore.get::<Tab>(tab_id).unwrap().unwrap();
            tab.blockids
                .iter()
                .map(|b| state.mstore.get::<Block>(b).unwrap().unwrap())
                .find(|b| obj::meta_get_string(&b.meta, "view", "") == "term")
                .map(|b| obj::meta_get_string(&b.meta, "controller", ""))
                .unwrap()
        };

        let trusted = layout_open_impl(
            &state,
            CommandLayoutOpenData { path: path.to_string_lossy().to_string(), window_id: window_id.clone(), run_commands: None, new_window: None },
            &trust(dir.path(), Some(OWN)),
        )
        .await
        .unwrap();
        assert_eq!(term_controller(&state, &trusted.tab_ids[0]), "cmd");

        let declined = layout_open_impl(
            &state,
            CommandLayoutOpenData { path: path.to_string_lossy().to_string(), window_id, run_commands: Some(false), new_window: None },
            &trust(dir.path(), Some(OWN)),
        )
        .await
        .unwrap();
        assert_eq!(term_controller(&state, &declined.tab_ids[0]), "shell");
    }

    #[tokio::test]
    async fn open_in_a_new_window_builds_its_own_workspace_and_leaves_this_one_alone() {
        let state = test_state();
        with_agent(&state);
        let (_, ws_id) = window_with_workspace(&state).await;
        let before = state.mstore.get::<Workspace>(&ws_id).unwrap().unwrap().tabids;
        let dir = tempfile::tempdir().unwrap();
        let path = write_layout(dir.path(), "someone-else");
        let out = layout_open_impl(
            &state,
            // No window is needed: the new one is opened afterwards.
            CommandLayoutOpenData {
                path: path.to_string_lossy().to_string(),
                window_id: String::new(),
                run_commands: None,
                new_window: Some(true),
            },
            &trust(dir.path(), Some(OWN)),
        )
        .await
        .unwrap();

        assert_ne!(out.workspace_id, ws_id);
        assert_eq!(state.mstore.get::<Workspace>(&ws_id).unwrap().unwrap().tabids, before, "the current window is untouched");
        let ws = state.mstore.get::<Workspace>(&out.workspace_id).unwrap().unwrap();
        assert_eq!(ws.name, "Work", "named after the layout");
        assert_eq!(ws.tabids, out.tab_ids, "only the layout's tabs, no default tab");
        assert_eq!(ws.activetabid, out.tab_ids[0]);
        assert_eq!(
            state.srv_state.lock().await.workspaces.get(&out.workspace_id).map(|w| w.tab_ids.clone()),
            Some(out.tab_ids.clone()),
            "the reducer knows the workspace, so a window can attach to it"
        );
        assert!(
            state.mstore.get_all::<Window>().unwrap().iter().all(|w| w.workspaceid != out.workspace_id),
            "no window yet; the caller opens one"
        );

        // The same rules as adding tabs: the untrusted command is held.
        let tab = state.mstore.get::<Tab>(&out.tab_ids[0]).unwrap().unwrap();
        let term = tab
            .blockids
            .iter()
            .map(|b| state.mstore.get::<Block>(b).unwrap().unwrap())
            .find(|b| obj::meta_get_string(&b.meta, "view", "") == "term")
            .unwrap();
        assert_eq!(obj::meta_get_string(&term.meta, "controller", ""), "shell");
    }

    #[tokio::test]
    async fn open_refuses_a_non_layout_path_and_an_unknown_window() {
        let state = test_state();
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("settings.json");
        std::fs::write(&bad, "{}").unwrap();
        assert!(layout_preview_impl(&state.mstore, &bad.to_string_lossy(), &trust(dir.path(), None)).is_err());
        let path = write_layout(dir.path(), OWN);
        let err = layout_open_impl(
            &state,
            CommandLayoutOpenData { path: path.to_string_lossy().to_string(), window_id: "nope".into(), run_commands: None, new_window: None },
            &trust(dir.path(), None),
        )
        .await
        .unwrap_err();
        assert!(err.contains("no such window"), "{err}");
    }

    #[test]
    fn deserializes_the_documented_request_shape() {
        let r: CommandLayoutSaveData = serde_json::from_value(serde_json::json!({
            "window_id": "w1", "name": "n", "path": "/tmp/a.agentmux-layout.json"
        }))
        .unwrap();
        assert_eq!(r.include_resume, None);
    }
}
