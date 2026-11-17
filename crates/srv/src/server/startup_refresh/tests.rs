// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The startup refresh: which files it rewrites and which it leaves alone, and when the watcher sees a change.

use super::*;
use crate::backend::storage::agents::test_agent_def;
use crate::server::tests::test_state;

const PANE: &str = "5d0e7a21-3b64-4c9f-9a18-6e2f7c1b8d03";

fn global(state: &AppState, text: &str) {
    let bundle: crate::backend::storage::store::Bundle = serde_json::from_value(serde_json::json!({
        "id": "g-rules", "name": "Rules", "is_global": true, "instructions": text,
    }))
    .unwrap();
    state.id_store.bundle_upsert(&bundle).unwrap();
}

/// A state with one agent of `provider` and a pane for it whose folder is `dir`.
fn state_with_a_pane(provider: &str, dir: &Path) -> AppState {
    let state = test_state();
    let mut def = test_agent_def("uid-refresh", "refresh", provider, "host", 1, "");
    def.slug = "refresh".to_string();
    state.mstore.agent_def_insert(&mut def).unwrap();
    let mut block = Block {
        oid: PANE.to_string(),
        parentoref: String::new(),
        version: 0,
        runtimeopts: None,
        stickers: None,
        meta: {
            let mut m = obj::MetaMapType::new();
            m.insert("view".to_string(), serde_json::json!("agent"));
            m.insert("agentId".to_string(), serde_json::json!("uid-refresh"));
            m.insert("cmd:cwd".to_string(), serde_json::json!(dir.to_string_lossy()));
            m
        },
        subblockids: None,
    };
    state.mstore.insert(&mut block).unwrap();
    state
}

fn claude_md(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("CLAUDE.md")).unwrap()
}

#[tokio::test]
async fn an_agentmux_owned_startup_file_gets_the_global_memory_of_today() {
    let dir = tempfile::tempdir().unwrap();
    let state = state_with_a_pane("claude", dir.path());
    global(&state, "workspace rules, first form");
    std::fs::write(dir.path().join("CLAUDE.md"), format!("{CLAUDE_MD_MANAGED_MARKER}\n\n# Memory\n# [Workspace] Rules\n\nworkspace rules, first form\n")).unwrap();

    global(&state, "workspace rules, second form");
    assert_eq!(refresh_pane(&state, PANE), Refresh::Written);

    let text = claude_md(dir.path());
    assert!(text.starts_with(CLAUDE_MD_MANAGED_MARKER), "{text}");
    assert!(text.contains("workspace rules, second form"), "{text}");
    assert!(!text.contains("first form"), "{text}");
}

#[tokio::test]
async fn a_users_own_claude_md_is_left_exactly_as_it_is() {
    let dir = tempfile::tempdir().unwrap();
    let state = state_with_a_pane("claude", dir.path());
    global(&state, "workspace rules");
    let theirs = "# My project\n\nUse tabs.\n";
    std::fs::write(dir.path().join("CLAUDE.md"), theirs).unwrap();

    assert_eq!(refresh_pane(&state, PANE), Refresh::NotOurs);
    assert_eq!(claude_md(dir.path()), theirs);
    assert!(!dir.path().join(AGENTMUX_MEMORY_FILENAME).exists(), "a refresh adds no file to a user's project");
}

#[tokio::test]
async fn a_users_claude_md_that_imports_the_memory_file_gets_that_file_refreshed() {
    let dir = tempfile::tempdir().unwrap();
    let state = state_with_a_pane("claude", dir.path());
    global(&state, "workspace rules, second form");
    let theirs = "# My project\n\n@.claude/AGENTMUX_MEMORY.md\n";
    std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
    std::fs::write(dir.path().join("CLAUDE.md"), theirs).unwrap();
    std::fs::write(dir.path().join(AGENTMUX_MEMORY_FILENAME), "# Memory\n# [Workspace] Rules\n\nworkspace rules, first form\n").unwrap();

    assert_eq!(refresh_pane(&state, PANE), Refresh::Written);

    let memory = std::fs::read_to_string(dir.path().join(AGENTMUX_MEMORY_FILENAME)).unwrap();
    assert!(memory.contains("workspace rules, second form") && !memory.contains("first form"), "{memory}");
    assert!(claude_md(dir.path()).starts_with("# My project"), "their file keeps its own text");
}

#[tokio::test]
async fn a_folder_without_a_startup_file_of_ours_is_not_given_one() {
    let dir = tempfile::tempdir().unwrap();
    let state = state_with_a_pane("claude", dir.path());
    global(&state, "workspace rules");

    assert_eq!(refresh_pane(&state, PANE), Refresh::NotOurs);
    assert!(!dir.path().join("CLAUDE.md").exists());
}

#[tokio::test]
async fn another_providers_startup_file_is_not_touched() {
    let dir = tempfile::tempdir().unwrap();
    let state = state_with_a_pane("codex", dir.path());
    global(&state, "workspace rules");
    std::fs::write(dir.path().join("AGENTS.md"), "their instructions\n").unwrap();

    assert_eq!(refresh_pane(&state, PANE), Refresh::NotClaude);
    assert_eq!(std::fs::read_to_string(dir.path().join("AGENTS.md")).unwrap(), "their instructions\n");
}

#[tokio::test]
async fn a_pane_with_no_folder_is_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let state = state_with_a_pane("claude", dir.path());
    let mut block = state.mstore.get::<Block>(PANE).unwrap().unwrap();
    block.meta.remove("cmd:cwd");
    state.mstore.update(&mut block).unwrap();
    assert_eq!(refresh_pane(&state, PANE), Refresh::NotOurs);
}

#[test]
fn the_watcher_records_a_baseline_then_reports_only_a_move() {
    let mut watch = Watch::default();
    assert!(!watch.changed(Some(1)), "the first look is the baseline");
    assert!(!watch.changed(Some(1)), "the same fingerprint is no change");
    assert!(!watch.changed(None), "a look that could not read Global Memory changes nothing");
    assert!(watch.changed(Some(2)), "a new fingerprint is a change");
    assert!(!watch.changed(Some(2)), "and is reported once");
}

#[tokio::test]
async fn the_fingerprint_follows_the_global_sections() {
    let state = test_state();
    global(&state, "one");
    let first = fingerprint(&state.id_store);
    assert_eq!(first, fingerprint(&state.id_store), "the same sections, the same fingerprint");
    global(&state, "two");
    assert_ne!(first, fingerprint(&state.id_store), "a changed section moves it");
}
