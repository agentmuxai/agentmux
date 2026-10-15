// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The `LayoutApplyIntent` arm: srv computes every layout edit the frontend
//! makes. SPEC_STRONG_REDUCER_AUTHORITY_LAYOUT_2026_06_30.md §4.

use agentmux_common::ipc::Event;
use agentmux_common::{LayoutIntent, LayoutViewState};

use super::{clear_dangling_focus_magnify, op_error, report_layout_violations, unknown_tab};
use crate::backend::layout::intent::{apply_intent, migrate_legacy_minimize, NodeIds};
use crate::state::State;

/// How many applied intent ids a tab remembers for de-duplication: more than
/// a frontend ever has in flight at once.
pub(crate) const RECENT_INTENTS: usize = 64;

/// Apply `intent` to a copy of the tab's tree, normalize it the way every
/// layout write is (balance, prune references to deleted blocks, legacy
/// minimize locks, the invariant report), and commit it. A refused intent
/// leaves the tab untouched and answers with an `Error` event.
///
/// Emits `LayoutTreeReplaced` (tree only) when the tree changed, and
/// `FocusedNodeChanged` / `MagnifiedNodeChanged` when those did. A repeat of
/// a recent `action_id` emits nothing.
pub(crate) fn handle_layout_apply_intent(
    state: &mut State,
    tab_id: String,
    intent: LayoutIntent,
    action_id: String,
    view: Option<LayoutViewState>,
) -> Vec<Event> {
    let live_blocks: std::collections::HashSet<String> = state.blocks.keys().cloned().collect();
    let Some(tab) = state.tabs.get(&tab_id) else {
        return unknown_tab(state, "LayoutApplyIntent", &tab_id);
    };
    if !action_id.is_empty() && tab.recent_intent_ids.contains(&action_id) {
        return Vec::new();
    }
    let name = intent.name();
    let mut tree = tab.rootnode.clone();
    if let Some(root) = tree.as_mut() {
        let migrated = migrate_legacy_minimize(root);
        if migrated > 0 {
            tracing::info!(tab_id = %tab_id, migrated, "layout intent: migrated legacy minimize fields");
        }
    }
    let mut ids = NodeIds::new(&action_id);
    let effect = match apply_intent(&mut tree, intent, &mut ids) {
        Ok(effect) => effect,
        Err(e) => return op_error(state, format!("LayoutApplyIntent {name}: {e} (tab {tab_id})")),
    };
    let pruned = crate::backend::layout::prune_dangling_block_refs(&mut tree, &live_blocks);
    if pruned > 0 {
        tracing::warn!(tab_id = %tab_id, pruned, "layout intent: pruned leaves of deleted blocks");
    }
    crate::backend::layout::enforce_minimized_locks(&mut tree);

    let tab = state.tabs.get_mut(&tab_id).expect("looked up above");
    let (focused_before, magnified_before) = (tab.focused_node_id.clone(), tab.magnified_node_id.clone());
    let tree_changed = tab.rootnode != tree;
    tab.rootnode = tree;
    if !action_id.is_empty() {
        tab.recent_intent_ids.push(action_id);
        if tab.recent_intent_ids.len() > RECENT_INTENTS {
            tab.recent_intent_ids.remove(0);
        }
    }
    match view {
        Some(view) => {
            tab.focused_node_id = view.focused_node_id;
            tab.magnified_node_id = view.magnified_node_id;
        }
        None => {
            if effect.clear {
                tab.focused_node_id.clear();
                tab.magnified_node_id.clear();
            }
            if effect.unfocus.as_deref() == Some(tab.focused_node_id.as_str()) {
                tab.focused_node_id.clear();
            }
            if let Some(id) = effect.magnify {
                tab.magnified_node_id = id;
            }
            if let Some(id) = effect.focus {
                tab.focused_node_id = id;
            }
        }
    }
    clear_dangling_focus_magnify(tab);
    let new_tree = tab.rootnode.clone();
    let (focused, magnified) = (tab.focused_node_id.clone(), tab.magnified_node_id.clone());

    let mut events = Vec::new();
    if tree_changed {
        report_layout_violations(&tab_id, "LayoutApplyIntent", &new_tree);
        let v = state.bump_version();
        events.push(Event::LayoutTreeReplaced {
            tab_id: tab_id.clone(),
            new_tree,
            correlation_id: String::new(),
            slices: None,
            version: v,
        });
    }
    if focused != focused_before {
        let v = state.bump_version();
        events.push(Event::FocusedNodeChanged { tab_id: tab_id.clone(), node_id: focused, version: v });
    }
    if magnified != magnified_before {
        let v = state.bump_version();
        events.push(Event::MagnifiedNodeChanged { tab_id, node_id: magnified, version: v });
    }
    events
}

#[cfg(test)]
mod tests;
