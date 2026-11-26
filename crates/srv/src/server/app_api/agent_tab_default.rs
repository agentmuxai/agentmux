// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Where a pane an agent opens goes when the agent does not say: a tab of its own pane group.
//!
//! An agent that opens a pane (an Editor, Files or widget pane) and names no placement gets a new tab in the pane group
//! of the pane it opened last, or in its own pane's group when it has none open there. A Browser is different: the first
//! one an agent drives gets a pane of its own (a split below the agent's pane), and each later one is a new tab of the
//! newest browser pane the agent opened that is still open in its own window tab. An agent gets a split only when it
//! asks for one (`split`), a floating window only when it asks for one (`floating`), and a tab of the person's other
//! panes, of another agent's browser or of another window tab never: the group is looked for in the agent's own tab
//! only, among panes the agent opened.

use crate::backend::obj::{Block, LayoutState, Tab};
use crate::backend::storage::store::Store;
use crate::backend::rpc_types::CommandPaneOpenData;
use crate::server::AppState;

/// Whether `cmd` says where the pane goes: a split, a floating window, a pane to stack onto, no placement at all, or a
/// tab of its own.
fn names_where(cmd: &CommandPaneOpenData) -> bool {
    cmd.floating == Some(true)
        || cmd.skip_placement == Some(true)
        || cmd.stack_onto_block_id.as_deref().is_some_and(|b| !b.is_empty())
        || cmd.split_direction.as_deref().is_some_and(|d| !d.is_empty())
        || cmd.tab_id.as_deref().is_some_and(|t| !t.is_empty())
        || super::floater_place::names_placement(cmd)
}

/// The panes the layout of tab `tab_id` holds: a block made with no placement is in the tab but in no pane.
fn laid_out(mstore: &Store, tab_id: &str) -> Vec<String> {
    let layout = mstore.get::<Tab>(tab_id).ok().flatten().and_then(|tab| mstore.get::<LayoutState>(&tab.layoutstate).ok().flatten());
    layout.and_then(|l| l.rootnode).map(|root| super::member_block_ids(&root)).unwrap_or_default()
}

/// Where an agent's first browser goes: below the agent's own pane, which gives up half of its space (`down:half`, see
/// `resolve_placement`).
pub(crate) const BROWSER_FIRST_SPLIT: &str = "down:half";

/// Whether `block_id` is a browser pane.
fn is_browser(mstore: &Store, block_id: &str) -> bool {
    mstore.get::<Block>(block_id).ok().flatten().is_some_and(|b| crate::backend::obj::meta_get_string(&b.meta, "view", "") == "browser")
}

/// The browser pane `agent`'s next browser stacks onto: the newest browser pane it opened that is still in a pane of
/// the tab of its own pane `own`. `None` for the agent's first browser, and after its browsers were closed or moved
/// to another tab: the next one is a first again. `opened` is the agent's docked panes, newest first.
pub(crate) fn browser_target(mstore: &Store, opened: &[String], own: &str) -> Option<String> {
    let own_tab = super::resolve_tab_id_for_block(mstore, own).ok()?;
    let held = laid_out(mstore, &own_tab);
    opened.iter().find(|pane| held.contains(pane) && is_browser(mstore, pane)).cloned()
}

/// The pane `agent` stacks its next pane onto: the newest pane it opened that is still there, docked, in a pane of the
/// tab of its own pane `own`; else `own`. `opened` is the agent's docked panes, newest first.
pub(crate) fn target_for(mstore: &Store, opened: &[String], own: &str) -> Option<String> {
    let own_tab = super::resolve_tab_id_for_block(mstore, own).ok()?;
    let held = laid_out(mstore, &own_tab);
    let earlier = opened.iter().find(|pane| held.contains(pane));
    Some(earlier.map_or_else(|| own.to_string(), Clone::clone))
}

/// Make `cmd` open as a tab of the agent's own pane group when it names no place, for an agent's open (`by_agent`; a
/// person's own shell keeps its splits). `agent` is the opener, known by its
/// signed identity (none for a container's own route, which has no history to look at); the agent's own pane is the one
/// the request names as its reference.
pub(crate) fn apply(state: &AppState, by_agent: bool, agent: Option<&str>, cmd: &mut CommandPaneOpenData) {
    if !by_agent || names_where(cmd) {
        return;
    }
    let Some(own) = cmd.split_reference_block_id.clone().filter(|b| !b.is_empty()) else { return };
    let opened = agent.map(crate::server::pane_openers::docked_of).unwrap_or_default();
    if cmd.view == "browser" {
        match browser_target(&state.mstore, &opened, &own) {
            // Shown in its pane, so the person sees what the agent drives: switching the tab of a pane of the agent's
            // own browsers takes nobody's focus or caret.
            Some(browser) => (cmd.stack_onto_block_id, cmd.focus) = (Some(browser), Some(true)),
            None => cmd.split_direction = Some(BROWSER_FIRST_SPLIT.to_string()),
        }
        return;
    }
    if let Some(target) = target_for(&state.mstore, &opened, &own) {
        cmd.stack_onto_block_id = Some(target);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cmd(v: serde_json::Value) -> CommandPaneOpenData {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn a_request_that_says_where_is_left_as_it_is() {
        for said in [
            json!({ "view": "editor", "split_direction": "right" }),
            json!({ "view": "editor", "floating": true }),
            json!({ "view": "editor", "skip_placement": true }),
            json!({ "view": "editor", "stack_onto_block_id": "b9" }),
            json!({ "view": "editor", "tab_id": "t1" }),
            json!({ "view": "editor", "floating": true, "width": 800 }),
        ] {
            assert!(names_where(&cmd(said.clone())), "{said}");
        }
        for open in [json!({ "view": "editor" }), json!({ "view": "browser", "split_reference_block_id": "own" }), json!({ "view": "editor", "stack_onto_block_id": "" })] {
            assert!(!names_where(&cmd(open.clone())), "{open}");
        }
    }
}
