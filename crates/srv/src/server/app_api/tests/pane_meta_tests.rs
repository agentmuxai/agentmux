// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What `pane.open` builds as a new pane's metadata (`build_pane_meta`), one view at a time: the keys each view
//! writes, which of them only an explicit value writes, the arguments a view refuses to open without, and the
//! connection and title steps after the view's own. The files, line and viewer paths are in
//! pane_open_reducer_tests.rs; these pin the rest before the function is split by view.

use serde_json::json;

use super::pane::build_pane_meta;
use crate::backend::rpc_types::CommandPaneOpenData;

fn cmd(view: &str) -> CommandPaneOpenData {
    CommandPaneOpenData { view: view.into(), ..Default::default() }
}

fn keys(meta: &crate::backend::obj::MetaMapType) -> Vec<&str> {
    let mut k: Vec<&str> = meta.keys().map(String::as_str).collect();
    k.sort_unstable();
    k
}

#[test]
fn an_editor_needs_a_file_and_writes_only_the_view_and_the_file_for_it() {
    let mut c = cmd("editor");
    assert_eq!(build_pane_meta(&c).unwrap_err(), "MISSING_ARG: view=editor requires 'file'");
    c.file = Some(String::new());
    assert_eq!(build_pane_meta(&c).unwrap_err(), "MISSING_ARG: view=editor requires 'file'");

    c.file = Some("/work/a.rs".into());
    let meta = build_pane_meta(&c).unwrap();
    assert_eq!(keys(&meta), ["file", "view"]);
    assert_eq!(meta["view"], "editor");
    assert_eq!(meta["file"], "/work/a.rs");
}

#[test]
fn an_editor_writes_the_tree_and_the_follow_flag_only_when_the_caller_gave_them() {
    let mut c = cmd("editor");
    c.file = Some("/work/a.rs".into());

    c.tree_expanded = Some(true);
    assert_eq!(build_pane_meta(&c).unwrap()["editor:tree_expanded"], json!(true));
    c.tree_expanded = Some(false);
    assert_eq!(build_pane_meta(&c).unwrap()["editor:tree_expanded"], json!(false), "false is a request too");
    c.tree_expanded = None;
    assert!(!build_pane_meta(&c).unwrap().contains_key("editor:tree_expanded"));

    c.follow_folder = Some(true);
    assert_eq!(build_pane_meta(&c).unwrap()["editor:follow_folder"], json!(true));
    c.follow_folder = Some(false);
    assert!(!build_pane_meta(&c).unwrap().contains_key("editor:follow_folder"), "only true follows");
    c.follow_folder = None;
    assert!(!build_pane_meta(&c).unwrap().contains_key("editor:follow_folder"));
}

#[test]
fn an_editor_with_everything_set_writes_each_key_once() {
    let mut c = cmd("editor");
    c.file = Some("/work/a.rs".into());
    c.tree_expanded = Some(false);
    c.line = Some(7);
    c.follow_folder = Some(true);
    c.viewer = Some("text".into());
    c.title = Some("A".into());
    let meta = build_pane_meta(&c).unwrap();
    assert_eq!(
        keys(&meta),
        ["editor:follow_folder", "editor:line", "editor:tree_expanded", "editor:viewer", "file", "frame:title", "view"]
    );
}

#[test]
fn a_terminal_is_a_shell_and_takes_a_folder_only_when_it_names_one() {
    let mut c = cmd("term");
    let meta = build_pane_meta(&c).unwrap();
    assert_eq!(keys(&meta), ["controller", "view"]);
    assert_eq!(meta["view"], "term");
    assert_eq!(meta["controller"], "shell");

    c.cwd = Some(String::new());
    assert!(!build_pane_meta(&c).unwrap().contains_key("cmd:cwd"), "an empty folder is none");
    c.cwd = Some("/work/repo".into());
    assert_eq!(build_pane_meta(&c).unwrap()["cmd:cwd"], "/work/repo");
}

#[test]
fn a_browser_needs_an_address() {
    let mut c = cmd("browser");
    assert_eq!(build_pane_meta(&c).unwrap_err(), "MISSING_ARG: view=browser requires 'url'");
    c.url = Some(String::new());
    assert_eq!(build_pane_meta(&c).unwrap_err(), "MISSING_ARG: view=browser requires 'url'");

    c.url = Some("https://example.org/a".into());
    let meta = build_pane_meta(&c).unwrap();
    assert_eq!(keys(&meta), ["url", "view"]);
    assert_eq!(meta["view"], "browser");
    assert_eq!(meta["url"], "https://example.org/a");
}

#[test]
fn the_views_with_nothing_to_set_write_only_their_name() {
    for view in ["sysinfo", "help", "tower"] {
        let mut c = cmd(view);
        // What other views read is no part of these.
        c.file = Some("/ignored".into());
        c.url = Some("https://ignored.example".into());
        c.cwd = Some("/ignored".into());
        let meta = build_pane_meta(&c).unwrap();
        assert_eq!(keys(&meta), ["view"], "{view}");
        assert_eq!(meta["view"], view);
    }
}

#[test]
fn media_opens_as_an_editor_that_follows_the_folder() {
    let mut c = cmd("media");
    assert_eq!(build_pane_meta(&c).unwrap_err(), "MISSING_ARG: view=media requires 'file'");

    c.file = Some("/shots/a.png".into());
    let meta = build_pane_meta(&c).unwrap();
    assert_eq!(keys(&meta), ["editor:follow_folder", "file", "view"]);
    assert_eq!(meta["view"], "editor");
    assert_eq!(meta["file"], "/shots/a.png");
    assert_eq!(meta["editor:follow_folder"], json!(true));
}

#[test]
fn an_unknown_view_is_refused_naming_the_supported_ones() {
    assert_eq!(
        build_pane_meta(&cmd("nope")).unwrap_err(),
        "INVALID_VIEW: unsupported view 'nope' (expected editor/term/browser/sysinfo/help/media/files/tower)"
    );
}

#[test]
fn the_title_is_kept_and_a_browser_pane_also_records_its_address() {
    let mut c = cmd("term");
    c.title = Some("Build".into());
    let meta = build_pane_meta(&c).unwrap();
    assert_eq!(meta["frame:title"], "Build");
    assert!(!meta.contains_key("frame:title_url"), "only a browser records an address");

    c.title = Some(String::new());
    assert!(!build_pane_meta(&c).unwrap().contains_key("frame:title"), "an empty title is none");

    let mut b = cmd("browser");
    b.url = Some("https://example.org/a".into());
    b.title = Some("Docs".into());
    let meta = build_pane_meta(&b).unwrap();
    assert_eq!(meta["frame:title"], "Docs");
    assert_eq!(meta["frame:title_url"], "https://example.org/a");
}

#[test]
fn the_views_that_work_on_a_connection_record_it_trimmed() {
    for view in ["editor", "term", "files"] {
        let mut c = cmd(view);
        c.file = Some("/work/a".into());
        c.connection = Some("  dev@host.example  ".into());
        assert_eq!(build_pane_meta(&c).unwrap()["connection"], "dev@host.example", "{view}");
    }
}

#[test]
fn local_or_no_connection_records_nothing_and_the_other_views_have_none() {
    for conn in [None, Some(""), Some("   "), Some("local"), Some(" local ")] {
        let mut c = cmd("editor");
        c.file = Some("/work/a".into());
        c.connection = conn.map(str::to_string);
        assert!(!build_pane_meta(&c).unwrap().contains_key("connection"), "{conn:?}");
    }
    for view in ["sysinfo", "help", "tower"] {
        let mut c = cmd(view);
        c.connection = Some("dev@host.example".into());
        assert!(!build_pane_meta(&c).unwrap().contains_key("connection"), "{view}");
    }
    let mut b = cmd("browser");
    b.url = Some("https://example.org".into());
    b.connection = Some("dev@host.example".into());
    assert!(!build_pane_meta(&b).unwrap().contains_key("connection"), "browser");
}

#[test]
fn media_reads_this_computers_files_only_so_an_ssh_connection_is_refused() {
    let mut c = cmd("media");
    c.file = Some("/shots/a.png".into());

    c.connection = Some("dev@host.example".into());
    assert_eq!(
        build_pane_meta(&c).unwrap_err(),
        "INVALID_ARG: media files on dev@host.example can't be shown yet"
    );

    // A WSL distribution or local is no SSH host: the request goes through, with no connection recorded.
    for conn in ["wsl://Ubuntu", "local"] {
        c.connection = Some(conn.into());
        let meta = build_pane_meta(&c).unwrap();
        assert!(!meta.contains_key("connection"), "{conn}");
    }
}
