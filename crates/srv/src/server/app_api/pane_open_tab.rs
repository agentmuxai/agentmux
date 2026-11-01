// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//! Which tab a `pane.open` lands in (`REPORT_ACTION_RESULTS_ALWAYS_VISIBLE_ACROSS_WINDOWS_2026_10_31.md` §4, step 2).
//!
//! The order is: the `tab_id` the caller names, the tab of the `split_reference_block_id` it names, the
//! `origin_tab_id` (the tab of the window the person acted in), and only then "the first workspace's active
//! tab". A window that sends none of them is refused, so a click can't open its pane in another window.

use super::*;

/// The tab a `pane.open` with these fields opens in.
pub(super) fn resolve_open_tab(
    mstore: &Store,
    tab_id: Option<&str>,
    split_reference_block_id: Option<&str>,
    origin_tab_id: Option<&str>,
) -> Result<String, String> {
    if let Some(explicit) = tab_id {
        return Ok(explicit.to_string());
    }
    if let Some(derived) = split_reference_block_id.and_then(|id| resolve_tab_id_for_block(mstore, id).ok()) {
        return Ok(derived);
    }
    if let Some(origin) = origin_tab_id.filter(|t| !t.is_empty()) {
        // A tab that is gone is an error, not a reason to open the pane in whichever tab is first.
        return match mstore.get::<Tab>(origin) {
            Ok(Some(_)) => Ok(origin.to_string()),
            _ => Err(format!("TAB_NOT_FOUND: the tab {origin} this was opened from is gone")),
        };
    }
    resolve_tab_id(mstore, None)
}

/// A window's `pane.open` must say where it was asked from. An agent may leave it out (srv then picks its
/// own tab); a window may not, because "the first workspace" need not be the window the person clicked in.
pub(super) fn window_open_names_a_tab(cmd: &CommandPaneOpenData) -> Result<(), String> {
    let names = |t: &Option<String>| t.as_deref().is_some_and(|t| !t.is_empty());
    if names(&cmd.tab_id) || names(&cmd.split_reference_block_id) || names(&cmd.origin_tab_id) || names(&cmd.stack_onto_block_id) {
        return Ok(());
    }
    Err("INVALID_ARG: pane.open from a window needs origin_tab_id (the tab the person acted in), tab_id or split_reference_block_id".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::tests::test_state;
    use agentmux_common::ipc::{Command, Event};

    async fn new_tab(state: &AppState, name: &str) -> String {
        let mut tab = None;
        for cmd in [Command::CreateWorkspace { name: name.into() }] {
            let evs = crate::server::service::dispatch_to_reducer(state, cmd).await;
            for ev in &evs {
                crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore).unwrap();
            }
            let ws = evs.iter().find_map(|e| match e {
                Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
                _ => None,
            });
            let evs = crate::server::service::dispatch_to_reducer(state, Command::CreateTab { workspace_id: ws.unwrap(), name: name.into() }).await;
            for ev in &evs {
                crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore).unwrap();
            }
            tab = evs.iter().find_map(|e| match e {
                Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
                _ => None,
            });
        }
        tab.unwrap()
    }

    fn open(origin: Option<&str>, tab: Option<&str>) -> CommandPaneOpenData {
        CommandPaneOpenData {
            view: "sysinfo".into(),
            origin_tab_id: origin.map(str::to_string),
            tab_id: tab.map(str::to_string),
            ..Default::default()
        }
    }

    fn holds(state: &AppState, tab_id: &str, block_id: &str) -> bool {
        state.mstore.must_get::<Tab>(tab_id).unwrap().blockids.iter().any(|b| b == block_id)
    }

    #[tokio::test]
    async fn the_tab_a_window_acted_in_gets_the_pane_not_the_first_workspaces() {
        let state = test_state();
        let seeded = resolve_tab_id(&state.mstore, None).expect("the seeded workspace has a tab");
        let acted_in = new_tab(&state, "second").await;
        assert_ne!(seeded, acted_in);
        let res = open_pane(&state, open(Some(&acted_in), None)).await.expect("open from the second window");
        assert!(holds(&state, &acted_in, &res.block_id), "the pane is in the tab the person acted in");
        assert!(!holds(&state, &seeded, &res.block_id), "and not in the first workspace's tab");
    }

    #[tokio::test]
    async fn a_named_tab_still_beats_the_origin_and_a_gone_origin_is_an_error() {
        let state = test_state();
        let (named, origin) = (new_tab(&state, "named").await, new_tab(&state, "origin").await);
        let res = open_pane(&state, open(Some(&origin), Some(&named))).await.expect("open with both");
        assert!(holds(&state, &named, &res.block_id), "tab_id is the more exact answer");
        let err = open_pane(&state, open(Some("no-such-tab"), None)).await.unwrap_err();
        assert!(err.starts_with("TAB_NOT_FOUND"), "{err}");
    }

    /// `resolve_tab_id` falls back to the first workspace's active tab when it is given none, which may be
    /// another window's. Only these files may call it: `pane.open`'s tab choice (after the caller's own tab
    /// and the origin), and the agent-open paths, where the caller is an agent with no window of its own.
    /// A new caller has to come through `resolve_open_tab`, or be added here on purpose.
    #[test]
    fn nothing_else_asks_for_the_first_workspaces_tab() {
        const ALLOWED: [&str; 4] = ["pane_open_tab.rs", "agent_block.rs", "agent_attach.rs", "mod.rs"];
        fn walk(dir: &std::path::Path, hits: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, hits);
                    continue;
                }
                let name = path.file_name().unwrap().to_string_lossy().to_string();
                let in_tests = name.ends_with("_tests.rs") || path.components().any(|c| c.as_os_str() == "tests");
                if !name.ends_with(".rs") || in_tests || ALLOWED.contains(&name.as_str()) {
                    continue;
                }
                // Whole text with comments removed, so a call split over lines or with any argument counts.
                let code: String = std::fs::read_to_string(&path)
                    .unwrap()
                    .lines()
                    .map(|l| l.split("//").next().unwrap_or(""))
                    .collect::<Vec<_>>()
                    .join("
");
                let mut at = 0;
                while let Some(i) = code[at..].find("resolve_tab_id") {
                    let start = at + i;
                    let rest = code[start + "resolve_tab_id".len()..].trim_start();
                    let before = code[..start].trim_end();
                    let is_call = rest.starts_with('(') && !before.ends_with("fn");
                    let own_ident = code[..start].chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '_');
                    if is_call && !own_ident {
                        hits.push(format!("{} (byte {start})", path.display()));
                    }
                    at = start + "resolve_tab_id".len();
                }
            }
        }
        let mut hits = Vec::new();
        walk(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut hits);
        assert!(
            hits.is_empty(),
            "resolve_tab_id picks the first workspace's tab when it is given none, which may be another window's. Use pane_open_tab::resolve_open_tab, or name the tab. Found: {hits:?}"
        );
    }

    #[test]
    fn a_window_open_must_say_where_it_was_asked_from() {
        assert!(window_open_names_a_tab(&open(None, None)).is_err(), "nothing named: refused");
        assert!(window_open_names_a_tab(&open(Some(""), None)).is_err(), "an empty id names nothing");
        assert!(window_open_names_a_tab(&open(Some("t1"), None)).is_ok());
        assert!(window_open_names_a_tab(&open(None, Some("t1"))).is_ok());
        let beside = CommandPaneOpenData { split_reference_block_id: Some("b1".into()), ..open(None, None) };
        assert!(window_open_names_a_tab(&beside).is_ok());
        let stacked = CommandPaneOpenData { stack_onto_block_id: Some("b1".into()), ..open(None, None) };
        assert!(window_open_names_a_tab(&stacked).is_ok());
    }
}
