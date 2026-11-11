// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Heal a layout tree loaded from disk. A row written by an earlier build
//! can hold a leaf for a block the tab no longer owns (moved to another tab,
//! or deleted); the window used to prune those itself after loading. Now
//! srv does, once, at bootstrap. SPEC_STRONG_REDUCER_AUTHORITY_LAYOUT_2026_06_30.md §7, Phase 6.

use agentmux_common::ipc::Event;

use super::{clear_dangling_focus_magnify, report_layout_violations};
use crate::backend::layout::intent::migrate_legacy_minimize;
use crate::state::State;

/// Migrate the pre-display-mode minimize fields of `tab_id`'s tree, remove
/// the leaves whose block the tab doesn't own, then normalize the tree as a
/// delete intent leaves it (balanced) and clear a focus/magnify id left
/// pointing at nothing. A tree with nothing to migrate or remove is left
/// exactly as loaded. The window no longer migrates a tree itself: every tree
/// it is given has been through here or through an intent.
///
/// Returns the events that persist the change (empty when nothing changed),
/// with versions taken the way an intent's are.
pub(crate) fn heal_loaded_layout(state: &mut State, tab_id: &str) -> Vec<Event> {
    let Some(tab) = state.tabs.get_mut(tab_id) else { return Vec::new() };
    let owned: std::collections::HashSet<String> = tab.block_ids.iter().cloned().collect();
    let migrated = tab.rootnode.as_mut().map_or(0, migrate_legacy_minimize);
    let pruned = crate::backend::layout::prune_dangling_block_refs(&mut tab.rootnode, &owned);
    if migrated == 0 && pruned == 0 {
        return Vec::new();
    }
    if let Some(root) = tab.rootnode.as_mut() {
        if let Err(e) = crate::backend::layout::balance_node(root) {
            tracing::warn!(tab_id, error = %e, "layout heal: balance failed; keeping the healed tree");
        }
    }
    let (was_focused, was_magnified) = clear_dangling_focus_magnify(tab);
    tracing::warn!(tab_id, migrated, pruned, "layout heal: migrated legacy minimize fields, removed leaves for blocks the tab doesn't own");
    report_layout_violations(tab_id, "bootstrap heal", &tab.rootnode);
    let new_tree = tab.rootnode.clone();

    let mut events = Vec::new();
    let v = state.bump_version();
    events.push(Event::LayoutTreeReplaced {
        tab_id: tab_id.to_string(),
        new_tree,
        correlation_id: String::new(),
        slices: None,
        version: v,
    });
    if was_focused {
        let v = state.bump_version();
        events.push(Event::FocusedNodeChanged { tab_id: tab_id.to_string(), node_id: String::new(), version: v });
    }
    if was_magnified {
        let v = state.bump_version();
        events.push(Event::MagnifiedNodeChanged { tab_id: tab_id.to_string(), node_id: String::new(), version: v });
    }
    events
}
