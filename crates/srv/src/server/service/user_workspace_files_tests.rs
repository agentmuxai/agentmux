// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Export… and Import… (SPEC_WORKSPACES_2026_10_23.md §3.2, §4.2): an export
//! holds every window with its state, and imports back as a new workspace; an
//! older layout file imports as one too.

use serde_json::json;

use super::*;
use crate::backend::obj::Window;
use crate::server::service::user_workspace_switch::tests::{
    add_pane, close, closed_list, fresh_state, open_window, shown_by, views, window_rpc, workspace_rpc,
};
use crate::server::service::user_workspace_switch::wait_for_switch;

fn ctx() -> layout_file::ExportContext {
    layout_file::ExportContext {
        home: None,
        app_version: "0.59.40".into(),
        instance: Some("gr2q7gf5lh6pzfdnurnkvputhm".into()),
        saved_at: "2026-10-23T12:00:00Z".into(),
        include_resume: false,
    }
}

fn trust(dir: &Path) -> ImportTrust {
    ImportTrust { own_instance: None, dirs: vec![dir.to_path_buf()], home: Some(dir.to_path_buf()) }
}

/// Default Workspace with a main window (a terminal added, named "Main") and
/// a closed window holding a files pane. Returns the main window.
async fn workspace_with_a_closed_window(state: &AppState) -> Window {
    let main = open_window(state, "main").await;
    add_pane(state, &main.workspaceid, "term").await;
    let mut row = state.mstore.get::<Window>(&main.oid).unwrap().unwrap();
    row.meta.insert("window:displayname".into(), json!("Main"));
    state.mstore.update(&mut row).unwrap();
    let other = open_window(state, "window-2").await;
    add_pane(state, &other.workspaceid, "files").await;
    close(state, &other.oid).await;
    main
}

#[tokio::test]
async fn export_writes_every_window_with_its_name_and_state() {
    let state = fresh_state().await;
    workspace_with_a_closed_window(&state).await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Mine.agentmux-workspace.json");

    let out = export(&state.mstore, &path.to_string_lossy(), &ctx()).unwrap();

    assert_eq!(out["path"], json!(path.to_string_lossy()));
    let doc = layout_file::read_layout_file(&path).unwrap();
    assert_eq!(doc.scope, "workspace");
    assert_eq!(doc.name, "Default Workspace");
    assert_eq!(doc.version, 1, "optional fields: still version 1");
    let states: Vec<(Option<&str>, Option<&str>)> =
        doc.windows.iter().map(|w| (w.name.as_deref(), w.state.as_deref())).collect();
    assert_eq!(states, [(Some("Main"), Some("open")), (None, Some("closed"))]);
    let types = |i: usize| -> Vec<String> {
        doc.windows[i].tabs.iter().flat_map(|t| t.panes.values()).flat_map(|p| p.views.iter()).map(|v| v.view_type.clone()).collect()
    };
    assert!(types(0).contains(&"term".to_string()));
    assert!(types(1).contains(&"files".to_string()));
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains("\"resume\""), "no machine-local references unless asked");
}

#[tokio::test]
async fn export_refuses_any_other_file() {
    let state = fresh_state().await;
    workspace_with_a_closed_window(&state).await;
    let dir = tempfile::tempdir().unwrap();
    for bad in ["x.json", "x.agentmux-layout.json"] {
        assert!(export(&state.mstore, &dir.path().join(bad).to_string_lossy(), &ctx()).is_err(), "{bad}");
    }
    assert!(export(&state.mstore, "relative.agentmux-workspace.json", &ctx()).is_err());
}

#[tokio::test]
async fn an_export_imports_back_as_a_new_workspace_and_opens_it() {
    let state = fresh_state().await;
    let main = workspace_with_a_closed_window(&state).await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Mine.agentmux-workspace.json");
    export(&state.mstore, &path.to_string_lossy(), &ctx()).unwrap();

    let (switched, notes) = import(&state, &path.to_string_lossy(), None, &main.oid, &trust(dir.path())).await.unwrap();
    wait_for_switch().await;

    assert_eq!(switched.name, "Default Workspace 2", "a name no workspace has");
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(current_id(&state.mstore).as_deref(), Some(switched.id.as_str()));
    let shown = shown_by(&state, &main.oid);
    assert_ne!(shown, main.workspaceid, "a new copy, not the exported contents");
    assert_eq!(views(&state, &shown), ["agent", "swarm", "sysinfo", "term"]);
    let named = state.mstore.get::<Window>(&main.oid).unwrap().unwrap();
    assert_eq!(named.meta.get("window:displayname"), Some(&json!("Main")));
    let closed = closed_list(&state);
    assert_eq!(closed.len(), 1, "the closed window stays closed");
    assert!(views(&state, &closed[0]).contains(&"files".to_string()));
    let listed = workspace_rpc(&state, "ListUserWorkspaces", vec![]).await.data.unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn an_old_layout_file_imports_as_a_new_workspace_with_fresh_panes() {
    let state = fresh_state().await;
    let main = open_window(&state, "main").await;
    add_pane(&state, &main.workspaceid, "files").await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Review setup.agentmux-layout.json");
    let doc = json!({
        "format": "agentmux.layout", "version": 1, "name": "",
        "saved_by": { "app": "agentmux", "app_version": "0.58.0", "instance": "someoneelseaaaaaaaaaaaaaaa" },
        "scope": "window",
        "windows": [{ "active_tab": 0, "tabs": [{
            "name": "Work",
            "root": { "split": "row", "children": [
                { "ratio": 0.5, "node": { "pane": "p1" } },
                { "ratio": 0.5, "node": { "pane": "p2" } }
            ]},
            "panes": {
                "p1": { "views": [{ "type": "term", "config": { "command": "echo", "args": ["hi"] } }] },
                "p2": { "views": [{ "type": "sysinfo" }] }
            }
        }]}]
    });
    std::fs::write(&path, doc.to_string()).unwrap();

    let p = preview(&state.mstore, &path.to_string_lossy(), &trust(dir.path())).unwrap();
    assert_eq!(p["name"], "Review setup", "named after the file");
    assert_eq!(p["trusted"], false);
    assert_eq!((p["windows"].as_u64(), p["closed"].as_u64()), (Some(1), Some(0)));
    assert_eq!(p["commands"], json!(["echo hi"]));

    let (switched, _) = import(&state, &path.to_string_lossy(), None, &main.oid, &trust(dir.path())).await.unwrap();
    wait_for_switch().await;

    assert_eq!(switched.name, "Review setup");
    let shown = shown_by(&state, &main.oid);
    assert_eq!(views(&state, &shown), ["sysinfo", "term"]);
    let ws = state.mstore.get::<crate::backend::obj::Workspace>(&shown).unwrap().unwrap();
    let tab = state.mstore.get::<Tab>(&ws.tabids[0]).unwrap().unwrap();
    assert_eq!(tab.name, "Work");
    let term = tab
        .blockids
        .iter()
        .filter_map(|b| state.mstore.get::<Block>(b).unwrap())
        .find(|b| obj::meta_get_string(&b.meta, "view", "") == "term")
        .unwrap();
    assert_eq!(obj::meta_get_string(&term.meta, "controller", ""), "shell", "an untrusted command is held");
    assert!(closed_list(&state).is_empty());
    // The workspace it left keeps its window.
    assert!(views(&state, &main.workspaceid).contains(&"files".to_string()));
}

#[tokio::test]
async fn import_refuses_a_file_it_cant_read_and_leaves_no_workspace() {
    let state = fresh_state().await;
    let main = open_window(&state, "main").await;
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("settings.json");
    std::fs::write(&bad, "{}").unwrap();
    assert!(import(&state, &bad.to_string_lossy(), None, &main.oid, &trust(dir.path())).await.is_err());
    let empty = dir.path().join("empty.agentmux-workspace.json");
    std::fs::write(&empty, json!({ "format": "agentmux.layout", "version": 1, "windows": [] }).to_string()).unwrap();
    let err = import(&state, &empty.to_string_lossy(), None, &main.oid, &trust(dir.path())).await.unwrap_err();
    assert!(err.contains("no windows"), "{err}");
    assert_eq!(state.mstore.user_workspace_list().unwrap().len(), 1);
    assert!(window_rpc(&state, "TakeRestoredWindows", vec![]).await.data.unwrap().as_array().unwrap().is_empty());
}

#[test]
fn a_file_with_only_closed_windows_opens_its_first() {
    let tab = layout_file::PlanTab {
        name: "t".into(),
        blocks: vec![json!({ "view": "term" })],
        rootnode: None,
        focused: String::new(),
        magnified: String::new(),
        summary: Vec::new(),
    };
    let window = |closed| layout_file::WindowPlan { name: None, closed, tabs: vec![tab.clone()], active_tab: Some(0) };
    let plan = WorkspacePlan { windows: vec![window(true), window(true)], ..Default::default() };
    let record = import_record(&plan).unwrap();
    let states: Vec<&str> = record["windows"].as_array().unwrap().iter().map(|w| w["state"].as_str().unwrap()).collect();
    assert_eq!(states, ["open", "closed"]);
    assert!(import_record(&WorkspacePlan::default()).is_none());
}

#[test]
fn names_come_from_the_file_and_never_repeat() {
    let doc = |name: &str| layout_file::parse_layout(&json!({ "format": "agentmux.layout", "version": 1, "name": name }).to_string()).unwrap();
    assert_eq!(import_name(&doc(" Ops "), Path::new("/x/a.agentmux-workspace.json")), "Ops");
    assert_eq!(import_name(&doc(""), Path::new("/x/Review.AgentMux-Layout.json")), "Review");
    assert_eq!(import_name(&doc(""), Path::new("/x/.agentmux-workspace.json")), "Imported workspace");
    let store = Store::open_in_memory().unwrap();
    super::super::user_workspace_switch::insert_workspace(&store, "Ops").unwrap();
    super::super::user_workspace_switch::insert_workspace(&store, "ops 2").unwrap();
    assert_eq!(unique_name(&store, "Ops"), "Ops 3");
    assert_eq!(unique_name(&store, "Fresh"), "Fresh");
}
