// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Where a pane an agent opens goes when the agent does not say: a tab of its own pane group.
//!
//! An agent that opens a pane (an Editor, Files, Browser or widget pane) and names no placement gets a new tab in the
//! pane group of the pane it opened last, or in its own pane's group when it has none open there. It gets a split only
//! when it asks for one (`split`), a floating window only when it asks for one (`floating`), and a tab of the person's
//! other panes or of another window tab never: the group is looked for in the agent's own tab only.

use crate::backend::obj::{LayoutState, Tab};
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
