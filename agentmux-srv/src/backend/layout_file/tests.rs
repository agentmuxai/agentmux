// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! SPEC_LAYOUT_FILES_2026_09_25.md §6.2.

use serde_json::json;

use super::*;
use agentmux_common::layout_types::{FlexDirection, LayoutNodeData};

fn meta(pairs: Value) -> MetaMapType {
    pairs.as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.clone())).collect()
}

fn leaf(id: &str, block: &str, size: f32) -> LayoutNode {
    LayoutNode {
        id: id.into(),
        size,
        data: Some(LayoutNodeData { block_id: block.into(), ..Default::default() }),
        ..Default::default()
    }
}

fn block(store: &Store, oid: &str, m: Value) {
    let mut b = Block { oid: oid.into(), meta: meta(m), ..Default::default() };
    store.insert(&mut b).unwrap();
}

const HOME: &str = if cfg!(windows) { "C:\\Users\\alice" } else { "/home/alice" };

fn under_home(rel: &str) -> String {
    if cfg!(windows) {
        format!("{HOME}\\{}", rel.replace('/', "\\"))
    } else {
        format!("{HOME}/{rel}")
    }
}

fn ctx(include_resume: bool) -> ExportContext {
    ExportContext {
        home: Some(PathBuf::from(HOME)),
        app_version: "0.57.3".into(),
        instance: Some("gr2q7gf5lh6pzfdnurnkvputhm".into()),
        saved_at: "2026-09-25T18:04:11Z".into(),
        include_resume,
    }
}

/// A window whose single workspace has two tabs:
/// - "main": row split 3:1 — an agent pane (with a terminal as a second pane
///   tab) | a column split of browser and editor, focused on the browser;
/// - "misc": one sysinfo pane.
///
/// Every block carries the meta keys the file must never contain.
fn seeded_store() -> Store {
    let store = Store::open_in_memory().unwrap();
    let mut def = crate::backend::storage::agents::test_agent_def("def-uuid-1", "Reviewer", "claude", "agent", 1, "");
    def.slug = "reviewer".into();
    store.agent_def_insert(&mut def).unwrap();

    block(&store, "b-agent", json!({
        "view": "agent",
        "agentId": "def-uuid-1",
        "agentInstanceId": "inst-row-7",
        "agent:sessionid": "sess-abc",
        "session:tokens": 1234,
        "subagent:id": "sub-1",
        "agentCliPath": "C:/secret/bin/claude.exe",
        "agent:last_failure": { "kind": "x" },
    }));
    block(&store, "b-term", json!({
        "view": "term",
        "cmd:cwd": under_home("src/agentmux"),
        "cmd:env": { "GITHUB_TOKEN": "ghp_supersecret" },
        "term:fontsize": 12,
    }));
    block(&store, "b-web", json!({
        "view": "browser",
        "url": "https://example.com/docs/page?token=abc123&x=1#frag",
    }));
    block(&store, "b-edit", json!({ "view": "editor", "file": under_home("notes/todo.md") }));
    block(&store, "b-sys", json!({ "view": "sysinfo", "sysinfo:type": "CPU" }));

    let main_tree = LayoutNode {
        id: "n-root".into(),
        flex_direction: FlexDirection::Row,
        children: vec![
            LayoutNode {
                id: "n-agent".into(),
                size: 3.0,
                data: Some(LayoutNodeData {
                    block_id: "b-agent".into(),
                    block_stack: vec!["b-agent".into(), "b-term".into()],
                    active_block_id: "b-agent".into(),
                    ..Default::default()
                }),
                ..Default::default()
            },
            LayoutNode {
                id: "n-right".into(),
                size: 1.0,
                flex_direction: FlexDirection::Column,
                children: vec![leaf("n-web", "b-web", 1.0), leaf("n-edit", "b-edit", 1.0)],
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let mut main_layout = LayoutState {
        oid: "l-main".into(),
        rootnode: Some(main_tree),
        focusednodeid: "n-web".into(),
        ..Default::default()
    };
    store.insert(&mut main_layout).unwrap();
    let mut misc_layout = LayoutState {
        oid: "l-misc".into(),
        rootnode: Some(leaf("n-sys", "b-sys", 1.0)),
        ..Default::default()
    };
    store.insert(&mut misc_layout).unwrap();

    let mut main = Tab {
        oid: "t-main".into(),
        name: "main".into(),
        layoutstate: "l-main".into(),
        blockids: vec!["b-agent".into(), "b-term".into(), "b-web".into(), "b-edit".into()],
        ..Default::default()
    };
    store.insert(&mut main).unwrap();
    let mut misc = Tab {
        oid: "t-misc".into(),
        name: "misc".into(),
        layoutstate: "l-misc".into(),
        blockids: vec!["b-sys".into()],
        ..Default::default()
    };
    store.insert(&mut misc).unwrap();
    let mut ws = Workspace {
        oid: "ws-1".into(),
        tabids: vec!["t-main".into(), "t-misc".into()],
        activetabid: "t-misc".into(),
        ..Default::default()
    };
    store.insert(&mut ws).unwrap();
    let mut win = Window { oid: "w-1".into(), workspaceid: "ws-1".into(), ..Default::default() };
    store.insert(&mut win).unwrap();
    store
}

fn export(include_resume: bool) -> Exported {
    export_window(&seeded_store(), "w-1", "Review setup", &ctx(include_resume)).unwrap()
}

#[test]
fn the_envelope_names_format_version_and_the_saving_install() {
    let doc = export(false).doc;
    assert_eq!(doc.format, "agentmux.layout");
    assert_eq!(doc.version, 1);
    assert_eq!(doc.name, "Review setup");
    assert_eq!(doc.scope, "window");
    assert_eq!(doc.saved_by.app_version, "0.57.3");
    assert_eq!(doc.saved_by.instance.as_deref(), Some("gr2q7gf5lh6pzfdnurnkvputhm"));
    assert_eq!(doc.windows.len(), 1);
    assert_eq!(doc.windows[0].active_tab, Some(1), "misc was the active tab");
    assert_eq!(doc.windows[0].tabs.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["main", "misc"]);
}

#[test]
fn the_tree_is_ratios_with_file_local_pane_ids_and_pane_tabs() {
    let doc = export(false).doc;
    let main = &doc.windows[0].tabs[0];
    let Some(TreeNode::Split { split, children, .. }) = &main.root else { panic!("root should be a split") };
    assert_eq!(split, "row");
    assert_eq!(children.iter().map(|c| c.ratio).collect::<Vec<_>>(), [0.75, 0.25]);
    let TreeNode::Leaf { pane, .. } = &children[0].node else { panic!() };
    assert_eq!(pane, "p1");
    let TreeNode::Split { split, children: right, .. } = &children[1].node else { panic!() };
    assert_eq!(split, "column");
    assert_eq!(right.iter().map(|c| c.ratio).collect::<Vec<_>>(), [0.5, 0.5]);

    let agent_pane = &main.panes["p1"];
    assert_eq!(agent_pane.views.iter().map(|v| v.view_type.as_str()).collect::<Vec<_>>(), ["agent", "term"]);
    assert_eq!(agent_pane.active, 0);
    assert_eq!(main.focused.as_deref(), Some("p2"), "focus maps from the node id to the file's pane id");
    assert_eq!(main.magnified, None);
}

#[test]
fn every_split_sums_to_exactly_one() {
    for sizes in [vec![1.0, 1.0, 1.0], vec![3.0, 1.0], vec![0.0, 0.0], vec![7.0, 11.0, 13.0, 17.0]] {
        let r = ratios(&sizes);
        assert!((r.iter().sum::<f64>() - 1.0).abs() < 1e-9, "{sizes:?} → {r:?}");
    }
    assert_eq!(ratios(&[0.0, 0.0]), [0.5, 0.5], "non-positive sizes share equally");
}

#[test]
fn views_are_written_from_the_allowlist_only() {
    let doc = export(false).doc;
    let main = &doc.windows[0].tabs[0];
    let agent = &main.panes["p1"].views[0];
    assert_eq!(
        agent.config,
        json!({ "agent": "reviewer", "name": "Reviewer", "provider": "claude" }).as_object().unwrap().clone()
    );
    assert_eq!(agent.resume, None, "resume is opt-in");
    let term = &main.panes["p1"].views[1];
    assert_eq!(term.config, json!({ "cwd": "~/src/agentmux" }).as_object().unwrap().clone());
    let web = &main.panes["p2"].views[0];
    assert_eq!(web.config["url"], "https://example.com/docs/page");
    let edit = &main.panes["p3"].views[0];
    assert_eq!(edit.config["file"], "~/notes/todo.md");
    let sys = &doc.windows[0].tabs[1].panes["p1"].views[0];
    assert_eq!(sys.config["sysinfo_type"], "CPU");
}

#[test]
fn nothing_instance_specific_or_secret_reaches_the_file() {
    let json = serde_json::to_string(&export(false).doc).unwrap();
    for forbidden in [
        "def-uuid-1", "inst-row-7", "sess-abc", "sub-1", "secret", "ghp_supersecret", "GITHUB_TOKEN",
        "token=abc123", "#frag", "last_failure", "session:tokens", "fontsize",
        // OIDs and node ids
        "w-1", "ws-1", "t-main", "t-misc", "l-main", "b-agent", "b-term", "n-root", "n-web",
        // the absolute home path
        HOME,
    ] {
        assert!(!json.contains(forbidden), "{forbidden:?} leaked into the file: {json}");
    }
}

#[test]
fn resume_references_are_written_only_when_asked_for() {
    let doc = export(true).doc;
    let agent = &doc.windows[0].tabs[0].panes["p1"].views[0];
    assert_eq!(agent.resume.as_ref().unwrap()["session_id"], "sess-abc");
}

#[test]
fn an_unknown_view_type_is_written_as_its_type_alone() {
    let store = Store::open_in_memory().unwrap();
    let v = view_from_block(
        &store,
        &meta(json!({ "view": "hologram", "hologram:secret": "x", "url": "https://x" })),
        &ctx(false),
        &mut Vec::new(),
    );
    assert_eq!(v.view_type, "hologram");
    assert!(v.config.is_empty() && v.resume.is_none() && v.extra.is_empty());
}

#[test]
fn a_command_that_looks_sensitive_is_kept_but_warned_about() {
    let store = Store::open_in_memory().unwrap();
    let mut warnings = Vec::new();
    let v = view_from_block(
        &store,
        &meta(json!({ "view": "term", "cmd": "rm", "cmd:args": ["-rf", "/tmp/x"] })),
        &ctx(false),
        &mut warnings,
    );
    assert_eq!(v.config["command"], "rm");
    assert_eq!(warnings.len(), 1, "{warnings:?}");
}

#[test]
fn a_pane_whose_blocks_are_gone_is_dropped_and_its_split_collapses() {
    let store = seeded_store();
    // Delete the editor's block: the column split keeps only the browser,
    // so it collapses into a plain leaf.
    let mut tab = store.get::<Tab>("t-main").unwrap().unwrap();
    tab.blockids.retain(|b| b != "b-edit");
    store.update(&mut tab).unwrap();
    let doc = export_window(&store, "w-1", "x", &ctx(false)).unwrap().doc;
    let main = &doc.windows[0].tabs[0];
    let Some(TreeNode::Split { children, .. }) = &main.root else { panic!() };
    assert!(matches!(children[1].node, TreeNode::Leaf { .. }), "{:?}", children[1].node);
    assert_eq!(main.panes.len(), 2);
}

#[test]
fn home_relative_paths() {
    assert_eq!(home_relative(&under_home("a/b"), Some(Path::new(HOME))), "~/a/b");
    assert_eq!(home_relative(HOME, Some(Path::new(HOME))), "~");
    // A sibling that merely shares the prefix is not under home.
    assert_eq!(home_relative(&format!("{HOME}2/x"), Some(Path::new(HOME))), format!("{HOME}2/x"));
    assert_eq!(home_relative("/opt/x", Some(Path::new(HOME))), "/opt/x");
    assert_eq!(home_relative("/opt/x", None), "/opt/x");
}

#[test]
fn urls_lose_their_query_and_fragment() {
    assert_eq!(strip_query("https://a.b/c?d=e#f"), "https://a.b/c");
    assert_eq!(strip_query("https://a.b/c#f"), "https://a.b/c");
    assert_eq!(strip_query("https://a.b/c"), "https://a.b/c");
}

#[test]
fn unknown_fields_survive_a_round_trip() {
    let mut doc = export(false).doc;
    doc.extra.insert("future_top".into(), json!({ "x": 1 }));
    doc.windows[0].tabs[0].panes.get_mut("p1").unwrap().views[0]
        .extra
        .insert("future_view_field".into(), json!(true));
    let json = serde_json::to_string(&doc).unwrap();
    let back = parse_layout(&json).unwrap();
    assert_eq!(back, doc);
}

#[test]
fn another_format_or_a_newer_version_is_refused() {
    let mut doc = export(false).doc;
    doc.version = 2;
    let err = parse_layout(&serde_json::to_string(&doc).unwrap()).unwrap_err();
    assert!(err.contains("newer AgentMux"), "{err}");
    doc.version = 1;
    doc.format = "something.else".into();
    assert!(parse_layout(&serde_json::to_string(&doc).unwrap()).unwrap_err().contains("not an AgentMux layout"));
}

#[test]
fn the_writer_accepts_only_absolute_layout_paths_and_writes_atomically() {
    let dir = tempfile::tempdir().unwrap();
    let doc = export(false).doc;
    let good = dir.path().join("nested").join("Review setup.agentmux-layout.json");
    write_layout_file(&good, &doc).unwrap();
    let written = std::fs::read_to_string(&good).unwrap();
    assert!(written.ends_with('\n'));
    assert_eq!(parse_layout(&written).unwrap(), doc);
    let leftovers: Vec<_> = std::fs::read_dir(good.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(leftovers, ["Review setup.agentmux-layout.json"], "no temp file left behind");

    for bad in [
        dir.path().join("x.json"),
        dir.path().join(".agentmux-layout.json"),
        PathBuf::from("relative.agentmux-layout.json"),
    ] {
        assert!(write_layout_file(&bad, &doc).is_err(), "{}", bad.display());
    }
}

/// The published schema and the writer agree on the envelope: every key the
/// schema requires at the top level is one the writer emits, and the
/// `format` constant matches. (A full JSON Schema validator isn't a
/// dependency; this pins the parts a reader keys on.)
#[test]
fn the_published_schema_matches_the_envelope() {
    let schema: Value = serde_json::from_str(include_str!("../../../../schema/agentmux-layout.v1.schema.json")).unwrap();
    assert_eq!(schema["properties"]["format"]["const"], LAYOUT_FORMAT);
    let written = serde_json::to_value(export(false).doc).unwrap();
    for key in schema["required"].as_array().unwrap() {
        assert!(written.get(key.as_str().unwrap()).is_some(), "writer omits required {key}");
    }
}

#[test]
fn an_unknown_window_or_an_empty_one_is_an_error() {
    let store = seeded_store();
    assert!(export_window(&store, "no-such-window", "x", &ctx(false)).is_err());
    let mut empty_ws = Workspace { oid: "ws-empty".into(), ..Default::default() };
    store.insert(&mut empty_ws).unwrap();
    let mut empty_win = Window { oid: "w-empty".into(), workspaceid: "ws-empty".into(), ..Default::default() };
    store.insert(&mut empty_win).unwrap();
    assert!(export_window(&store, "w-empty", "x", &ctx(false)).unwrap_err().contains("no tabs"));
}
