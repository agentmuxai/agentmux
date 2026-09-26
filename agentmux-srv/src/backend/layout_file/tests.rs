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

/// ReAgent P1 on #3787: a multi-byte character straddling the home dir's
/// byte length must not panic — the path is simply not under home.
#[test]
fn a_multibyte_character_at_the_home_boundary_does_not_panic() {
    let home = Path::new(HOME);
    // Replace home's last character with a 2-byte one, so byte `HOME.len()`
    // falls inside it.
    let straddling = format!("{}é{}", &HOME[..HOME.len() - 1], if cfg!(windows) { "\\x" } else { "/x" });
    assert!(!straddling.is_char_boundary(HOME.len()), "the fixture must straddle the boundary");
    assert_eq!(home_relative(&straddling, Some(home)), straddling);
    // Shorter than home, and non-ASCII: also fine.
    assert_eq!(home_relative("é", Some(home)), "é");
    assert_eq!(home_relative(&under_home("café/menu.md"), Some(home)), "~/café/menu.md");
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

// ── Apply plan (spec §3.5, Phase 2) ──

fn doc_with_tab(panes: Value, root: Value) -> LayoutDoc {
    parse_layout(
        &json!({
            "format": "agentmux.layout",
            "version": 1,
            "name": "t",
            "windows": [{ "active_tab": 0, "tabs": [{ "name": "one", "root": root, "panes": panes, "focused": "p2" }] }]
        })
        .to_string(),
    )
    .unwrap()
}

fn one_pane(view: Value) -> LayoutDoc {
    doc_with_tab(json!({ "p1": { "views": [view] } }), json!({ "pane": "p1" }))
}

fn opts(home: &Path, run_commands: bool) -> PlanOptions {
    PlanOptions { home: Some(home.to_path_buf()), run_commands }
}

fn block_meta(plan: &ApplyPlan, tab: usize, i: usize) -> &Map<String, Value> {
    plan.tabs[tab].blocks[i].as_object().unwrap()
}

#[test]
fn an_exported_window_plans_back_into_the_same_tree() {
    let store = seeded_store();
    let doc = export_window(&store, "w-1", "x", &ctx(false)).unwrap().doc;
    let home = tempfile::tempdir().unwrap();
    let plan = plan_from_doc(&store, &doc, &opts(home.path(), false));
    assert_eq!(plan.tabs.len(), 2);
    assert_eq!(plan.active_tab, Some(1));
    let main = &plan.tabs[0];
    assert_eq!(main.blocks.len(), 4, "agent + term (one pane) + browser + editor");
    let root = main.rootnode.as_ref().unwrap();
    assert_eq!(root.children.len(), 2);
    assert_eq!(root.children[0].size, 75.0);
    assert_eq!(root.children[1].size, 25.0);
    assert_eq!(root.children[1].flex_direction, agentmux_common::layout_types::FlexDirection::Column);
    let agent_leaf = root.children[0].data.as_ref().unwrap();
    assert_eq!(agent_leaf.block_stack.len(), 2, "the agent pane keeps its two pane tabs");
    assert_eq!(agent_leaf.block_id, agent_leaf.active_block_id);
    // The agent resolves to the local definition by its slug.
    assert_eq!(block_meta(&plan, 0, 0)[META_LAYOUT_AGENT], "def-uuid-1");
    assert_eq!(block_meta(&plan, 0, 0)["view"], "agent");
    // Focus follows the file's pane to the new node id.
    let web_node = &root.children[1].children[0];
    assert_eq!(main.focused, web_node.id);
    // Node ids are fresh, never the store's.
    assert!(!root.id.is_empty() && root.id != "n-root");
}

#[test]
fn agents_resolve_by_slug_then_name_only_when_unambiguous_and_never_to_a_template() {
    let store = Store::open_in_memory().unwrap();
    let mut template = crate::backend::storage::agents::test_agent_def("tmpl", "Reviewer", "claude", "agent", 1, "");
    template.slug = "reviewer".into();
    template.is_seeded = 1;
    store.agent_def_insert(&mut template).unwrap();
    for (id, name) in [("a1", "Twin"), ("a2", "twin")] {
        let mut d = crate::backend::storage::agents::test_agent_def(id, name, "claude", "agent", 1, "");
        d.slug = id.into();
        store.agent_def_insert(&mut d).unwrap();
    }
    let home = tempfile::tempdir().unwrap();

    // Only the template matches: not launchable.
    let plan = plan_from_doc(
        &store,
        &one_pane(json!({ "type": "agent", "config": { "agent": "reviewer", "name": "Reviewer" } })),
        &opts(home.path(), false),
    );
    assert!(block_meta(&plan, 0, 0).get(META_LAYOUT_AGENT).is_none());
    assert!(plan.notes.iter().any(|n| n.contains("agent picker")), "{:?}", plan.notes);

    // Two user agents share the name: ambiguous, picker.
    let plan = plan_from_doc(&store, &one_pane(json!({ "type": "agent", "config": { "name": "TWIN" } })), &opts(home.path(), false));
    assert!(block_meta(&plan, 0, 0).get(META_LAYOUT_AGENT).is_none());

    // The slug is unique: resolved.
    let plan = plan_from_doc(
        &store,
        &one_pane(json!({ "type": "agent", "config": { "agent": "A2", "name": "twin" } })),
        &opts(home.path(), false),
    );
    assert_eq!(block_meta(&plan, 0, 0)[META_LAYOUT_AGENT], "a2");
}

#[test]
fn terminal_commands_run_only_when_allowed_and_are_always_listed() {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join("src")).unwrap();
    let store = Store::open_in_memory().unwrap();
    let doc = one_pane(json!({ "type": "term", "config": { "cwd": "~/src", "command": "npm", "args": ["test"] } }));

    let held = plan_from_doc(&store, &doc, &opts(home.path(), false));
    let m = block_meta(&held, 0, 0);
    assert_eq!(m["controller"], "shell");
    assert!(m.get("cmd").is_none(), "an untrusted command does not run");
    assert_eq!(m["cmd:cwd"], home.path().join("src").to_string_lossy().as_ref());
    assert_eq!(held.commands, ["npm test"]);
    assert!(held.tabs[0].summary[0].contains("command not run"));

    let run = plan_from_doc(&store, &doc, &opts(home.path(), true));
    let m = block_meta(&run, 0, 0);
    assert_eq!(m["controller"], "cmd");
    assert_eq!(m["cmd"], "npm");
    assert_eq!(m["cmd:args"], json!(["test"]));
}

#[test]
fn a_missing_folder_is_dropped_with_a_note() {
    let home = tempfile::tempdir().unwrap();
    let plan = plan_from_doc(
        &Store::open_in_memory().unwrap(),
        &one_pane(json!({ "type": "term", "config": { "cwd": "~/nope" } })),
        &opts(home.path(), false),
    );
    assert!(block_meta(&plan, 0, 0).get("cmd:cwd").is_none());
    assert!(plan.notes.iter().any(|n| n.contains("doesn't exist")));
}

#[test]
fn only_http_urls_reach_a_browser_pane() {
    let home = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    for (url, kept) in [
        ("https://a.b/c", true),
        ("http://a.b", true),
        ("javascript:alert(1)", false),
        ("file:///etc/passwd", false),
    ] {
        let plan = plan_from_doc(&store, &one_pane(json!({ "type": "browser", "config": { "url": url } })), &opts(home.path(), false));
        assert_eq!(block_meta(&plan, 0, 0).contains_key("url"), kept, "{url}");
    }
}

#[test]
fn unknown_types_open_as_empty_panes_with_a_sanitized_name() {
    let home = tempfile::tempdir().unwrap();
    let plan = plan_from_doc(
        &Store::open_in_memory().unwrap(),
        &one_pane(json!({ "type": "holo<script>gram", "config": { "x": 1 } })),
        &opts(home.path(), false),
    );
    assert_eq!(block_meta(&plan, 0, 0)["view"], "holoscriptgram");
    assert_eq!(block_meta(&plan, 0, 0).len(), 1, "nothing from an unknown type's config is carried");
    assert!(plan.notes.iter().any(|n| n.contains("isn't known")));
}

#[test]
fn editor_like_views_open_as_the_editor() {
    let home = tempfile::tempdir().unwrap();
    std::fs::write(home.path().join("a.md"), "x").unwrap();
    let store = Store::open_in_memory().unwrap();
    for kind in ["editor", "codeeditor", "preview"] {
        let plan = plan_from_doc(&store, &one_pane(json!({ "type": kind, "config": { "file": "~/a.md" } })), &opts(home.path(), false));
        let m = block_meta(&plan, 0, 0);
        assert_eq!(m["view"], "editor", "{kind}");
        assert_eq!(m["file"], home.path().join("a.md").to_string_lossy().as_ref());
        assert!(plan.notes.is_empty(), "{:?}", plan.notes);
    }
}

#[test]
fn panes_missing_from_the_map_are_skipped_and_a_tab_with_none_left_is_noted() {
    let home = tempfile::tempdir().unwrap();
    let doc = doc_with_tab(json!({}), json!({ "pane": "ghost" }));
    let plan = plan_from_doc(&Store::open_in_memory().unwrap(), &doc, &opts(home.path(), false));
    assert!(plan.tabs.is_empty());
    assert!(plan.notes.iter().any(|n| n.contains("skipped")));
}

#[test]
fn a_dangling_or_empty_pane_is_dropped_with_a_note_naming_it() {
    let home = tempfile::tempdir().unwrap();
    let doc = doc_with_tab(
        json!({ "p1": { "views": [{ "type": "sysinfo" }] }, "hollow": { "views": [] } }),
        json!({ "split": "row", "children": [
            { "ratio": 0.4, "node": { "pane": "p1" } },
            { "ratio": 0.3, "node": { "pane": "ghost" } },
            { "ratio": 0.3, "node": { "pane": "hollow" } }
        ] }),
    );
    let plan = plan_from_doc(&Store::open_in_memory().unwrap(), &doc, &opts(home.path(), false));
    assert_eq!(plan.tabs[0].blocks.len(), 1);
    assert!(plan.notes.iter().any(|n| n.contains("“ghost”")), "{:?}", plan.notes);
    assert!(plan.notes.iter().any(|n| n.contains("“hollow”")), "{:?}", plan.notes);
}

#[test]
fn panes_past_the_per_tab_limit_are_dropped_with_one_note() {
    let home = tempfile::tempdir().unwrap();
    let count = MAX_PANES_PER_TAB + 2;
    let panes: Map<String, Value> = (0..count).map(|i| (format!("p{i}"), json!({ "views": [{ "type": "sysinfo" }] }))).collect();
    let children: Vec<Value> = (0..count).map(|i| json!({ "ratio": 1.0 / count as f64, "node": { "pane": format!("p{i}") } })).collect();
    let doc = doc_with_tab(Value::Object(panes), json!({ "split": "row", "children": children }));
    let plan = plan_from_doc(&Store::open_in_memory().unwrap(), &doc, &opts(home.path(), false));
    assert_eq!(plan.tabs[0].blocks.len(), MAX_PANES_PER_TAB);
    let capped: Vec<_> = plan.notes.iter().filter(|n| n.contains(&MAX_PANES_PER_TAB.to_string())).collect();
    assert_eq!(capped.len(), 1, "{:?}", plan.notes);
    assert!(capped[0].contains("one"), "names the tab: {}", capped[0]);
}

#[test]
fn a_tree_nested_past_the_limit_is_cut_with_one_note() {
    let home = tempfile::tempdir().unwrap();
    // Every level holds a pane beside the next level, so several branches
    // cross the depth limit.
    let mut root = json!({ "pane": "p0" });
    for _ in 0..MAX_TREE_DEPTH + 3 {
        root = json!({ "split": "row", "children": [{ "ratio": 0.5, "node": { "pane": "p0" } }, { "ratio": 0.5, "node": root }] });
    }
    let doc = doc_with_tab(json!({ "p0": { "views": [{ "type": "sysinfo" }] } }), root);
    let plan = plan_from_doc(&Store::open_in_memory().unwrap(), &doc, &opts(home.path(), false));
    assert!(!plan.tabs[0].blocks.is_empty());
    assert_eq!(plan.notes.iter().filter(|n| n.contains("nests too deeply")).count(), 1, "{:?}", plan.notes);
}

#[test]
fn trust_needs_both_this_install_and_the_layouts_folder() {
    let mut doc = export(false).doc;
    let dir = PathBuf::from(if cfg!(windows) { "C:\\l" } else { "/l" });
    let inside = dir.join("x.agentmux-layout.json");
    let outside = PathBuf::from(if cfg!(windows) { "C:\\elsewhere\\x.agentmux-layout.json" } else { "/elsewhere/x.agentmux-layout.json" });
    let own = "gr2q7gf5lh6pzfdnurnkvputhm";
    assert!(is_trusted(&doc, &inside, Some(own), Some(&dir)));
    assert!(!is_trusted(&doc, &outside, Some(own), Some(&dir)), "a file saved here but moved elsewhere");
    assert!(!is_trusted(&doc, &inside, Some("aaaaaaaaaaaaaaaaaaaaaaaaaa"), Some(&dir)), "another install's file");
    assert!(!is_trusted(&doc, &inside, None, Some(&dir)), "no wan.db: can't tell, so not trusted");
    doc.saved_by.instance = None;
    assert!(!is_trusted(&doc, &inside, Some(own), Some(&dir)));
}

#[test]
fn a_file_too_large_to_be_a_layout_is_refused_unread() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("big.agentmux-layout.json");
    std::fs::write(&path, vec![b' '; (MAX_LAYOUT_FILE_BYTES + 1) as usize]).unwrap();
    assert!(read_layout_file(&path).unwrap_err().contains("too large"));
}
