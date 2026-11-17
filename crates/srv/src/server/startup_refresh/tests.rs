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

/// An agent definition named `name` (also its slug) of `provider`, with the id `uid`.
fn add_agent(state: &AppState, uid: &str, name: &str, provider: &str) {
    let mut def = test_agent_def(uid, name, provider, "host", 1, "");
    def.slug = name.to_string();
    state.mstore.agent_def_insert(&mut def).unwrap();
}

/// A pane `pane` of the agent `uid` whose folder is `cwd` (as stored), launched under `launched_name` when given.
fn add_pane(state: &AppState, pane: &str, uid: &str, cwd: &str, launched_name: Option<&str>) {
    let mut block = Block {
        oid: pane.to_string(),
        parentoref: String::new(),
        version: 0,
        runtimeopts: None,
        stickers: None,
        meta: {
            let mut m = obj::MetaMapType::new();
            m.insert("view".to_string(), serde_json::json!("agent"));
            m.insert("agentId".to_string(), serde_json::json!(uid));
            m.insert("cmd:cwd".to_string(), serde_json::json!(cwd));
            if let Some(name) = launched_name {
                m.insert("agentName".to_string(), serde_json::json!(name));
            }
            m
        },
        subblockids: None,
    };
    state.mstore.insert(&mut block).unwrap();
}

/// A state with one agent of `provider` and a pane for it whose folder is `dir`.
fn state_with_a_pane(provider: &str, dir: &Path) -> AppState {
    let state = test_state();
    add_agent(&state, "uid-refresh", "refresh", provider);
    add_pane(&state, PANE, "uid-refresh", &dir.to_string_lossy(), None);
    state
}

/// The agent's own instructions, where `{{AGENT_DISPLAY}}` expands to the name the pane was launched under.
fn set_soul(state: &AppState, uid: &str, text: &str) {
    let content = crate::backend::storage::store::AgentContent { agent_id: uid.to_string(), content_type: "soul".to_string(), content: text.to_string(), updated_at: 0 };
    state.mstore.agent_content_set(&content).unwrap();
}

/// A startup file of AgentMux's holding `text` as the Global Memory.
fn write_startup_file(dir: &Path, text: &str) {
    std::fs::write(dir.join("CLAUDE.md"), format!("{CLAUDE_MD_MANAGED_MARKER}\n\n# Memory\n# [Workspace] Rules\n\n{text}\n")).unwrap();
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

/// `agent.open` stores a default folder as `~/.agentmux/agents/<slug>`; it is looked at where the launch wrote the
/// file, not under a folder named `~`.
#[tokio::test]
async fn a_home_relative_folder_is_expanded_before_the_ownership_check() {
    let _guard = crate::test_support::ISOLATED_AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().unwrap();
    let prior = std::env::var_os("AGENTMUX_HOME_OVERRIDE");
    std::env::set_var("AGENTMUX_HOME_OVERRIDE", home.path());

    let folder = home.path().join("agents").join("refresh");
    std::fs::create_dir_all(&folder).unwrap();
    let state = test_state();
    add_agent(&state, "uid-refresh", "refresh", "claude");
    add_pane(&state, PANE, "uid-refresh", "~/.agentmux/agents/refresh", None);
    global(&state, "workspace rules, second form");
    write_startup_file(&folder, "workspace rules, first form");
    let outcome = refresh_pane(&state, PANE);
    let panes_folder = pane_cwd(&state.mstore, PANE);

    match prior {
        Some(p) => std::env::set_var("AGENTMUX_HOME_OVERRIDE", p),
        None => std::env::remove_var("AGENTMUX_HOME_OVERRIDE"),
    }
    assert_eq!(outcome, Refresh::Written);
    assert_eq!(panes_folder.as_deref().map(Path::new), Some(folder.as_path()));
    let text = claude_md(&folder);
    assert!(text.contains("workspace rules, second form") && !text.contains("first form"), "{text}");
}

/// The whole file is replaced, so the name the pane was launched under must be written again, not the definition's.
#[tokio::test]
async fn a_pane_launched_under_its_own_name_keeps_it_in_the_refreshed_file() {
    let dir = tempfile::tempdir().unwrap();
    let state = test_state();
    add_agent(&state, "uid-refresh", "refresh", "claude");
    add_pane(&state, PANE, "uid-refresh", &dir.path().to_string_lossy(), Some("Custom Pane"));
    set_soul(&state, "uid-refresh", "I am {{AGENT_DISPLAY}}.");
    write_startup_file(dir.path(), "stale");

    assert_eq!(refresh_pane(&state, PANE), Refresh::Written);

    let text = claude_md(dir.path());
    assert!(text.contains("I am Custom Pane."), "{text}");
    assert!(!text.contains("I am refresh."),"the definition's name must not replace the launched one: {text}");
}

#[tokio::test]
async fn a_pane_launched_without_a_name_gets_the_definitions() {
    let dir = tempfile::tempdir().unwrap();
    let state = state_with_a_pane("claude", dir.path());
    set_soul(&state, "uid-refresh", "I am {{AGENT_DISPLAY}}.");
    write_startup_file(dir.path(), "stale");

    assert_eq!(refresh_pane(&state, PANE), Refresh::Written);
    assert!(claude_md(dir.path()).contains("I am refresh."), "{}", claude_md(dir.path()));
}

/// A pane of another provider that is visited first must not use the folder up: the Claude pane in the same folder
/// still has to be refreshed, whichever order the panes come in.
#[tokio::test]
async fn a_claude_pane_is_refreshed_whichever_pane_of_the_folder_comes_first() {
    for codex_first in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state();
        add_agent(&state, "uid-claude", "claude-agent", "claude");
        add_agent(&state, "uid-codex", "codex-agent", "codex");
        let cwd = dir.path().to_string_lossy().into_owned();
        add_pane(&state, "pane-claude", "uid-claude", &cwd, None);
        add_pane(&state, "pane-codex", "uid-codex", &cwd, None);
        global(&state, "workspace rules, second form");
        write_startup_file(dir.path(), "workspace rules, first form");

        let order: Vec<String> = if codex_first { vec!["pane-codex".into(), "pane-claude".into()] } else { vec!["pane-claude".into(), "pane-codex".into()] };
        assert_eq!(refresh_panes(&state, &order), 1, "codex_first={codex_first}");

        let text = claude_md(dir.path());
        assert!(text.contains("workspace rules, second form") && !text.contains("first form"), "codex_first={codex_first}: {text}");
    }
}

#[tokio::test]
async fn two_claude_panes_in_one_folder_write_its_file_once() {
    let dir = tempfile::tempdir().unwrap();
    let state = test_state();
    add_agent(&state, "uid-a", "agent-a", "claude");
    add_agent(&state, "uid-b", "agent-b", "claude");
    let cwd = dir.path().to_string_lossy().into_owned();
    add_pane(&state, "pane-a", "uid-a", &cwd, None);
    add_pane(&state, "pane-b", "uid-b", &cwd, None);
    global(&state, "workspace rules, second form");
    write_startup_file(dir.path(), "workspace rules, first form");

    assert_eq!(refresh_panes(&state, &["pane-a".to_string(), "pane-b".to_string()]), 1);
}

/// A pane whose definition is gone says nothing about its folder, so it does not use the folder up either.
#[tokio::test]
async fn a_pane_without_a_definition_does_not_use_the_folder_up() {
    let dir = tempfile::tempdir().unwrap();
    let state = test_state();
    add_agent(&state, "uid-claude", "claude-agent", "claude");
    let cwd = dir.path().to_string_lossy().into_owned();
    add_pane(&state, "pane-orphan", "uid-gone", &cwd, None);
    add_pane(&state, "pane-claude", "uid-claude", &cwd, None);
    global(&state, "workspace rules, second form");
    write_startup_file(dir.path(), "workspace rules, first form");

    assert_eq!(refresh_panes(&state, &["pane-orphan".to_string(), "pane-claude".to_string()]), 1);
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
