// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use agentmux_common::ipc::Event;

use crate::state::State;


use crate::state::TabRecord;

use super::invalid_command;

/// Phase E.2b — create a tab with the caller-minted `tab_id`
/// (`Ctx::new_id`) inside a workspace. Refuses (non-fatal `Event::Error`)
/// a missing parent or an id already in use. On success: appends to the
/// workspace's `tab_ids`, inserts into `state.tabs`, emits
/// `Event::TabCreated`. If the workspace had no active tab, the new tab
/// also becomes active and an `Event::ActiveTabChanged` is emitted
/// alongside.
pub(super) fn handle_create_tab(
    state: &mut State,
    workspace_id: String,
    tab_id: String,
    name: String,
) -> Vec<Event> {
    if state.tabs.contains_key(&tab_id) {
        return invalid_command(state, format!("CreateTab: id already in use: {}", tab_id));
    }
    let Some(workspace_record) = state.workspaces.get(&workspace_id) else {
        return invalid_command(state, format!("CreateTab: workspace not found: {}", workspace_id));
    };
    // codex P2 #622: auto-generate `Tab N` when name is empty,
    // matching `wcore::create_tab`'s default-naming behaviour. The
    // counter uses the reducer's tab_ids length + 1 (matching the
    // old SQLite-side count: tabids.len() + pinnedtabids.len() + 1
    // — pinnedtabids stays at zero in production since pinning
    // was removed in E.2c.3b, so reducer-only counting matches).
    let resolved_name = if name.is_empty() {
        format!("Tab {}", workspace_record.tab_ids.len() + 1)
    } else {
        name
    };
    state.tabs.insert(
        tab_id.clone(),
        TabRecord {
            tab_id: tab_id.clone(),
            workspace_id: workspace_id.clone(),
            name: resolved_name.clone(),
            block_ids: Vec::new(),
            focused_node_id: String::new(),
            magnified_node_id: String::new(),
            rootnode: None,
            recent_intent_ids: Vec::new(),
        },
    );
    let workspace = state.workspaces.get_mut(&workspace_id).expect("checked");
    workspace.tab_ids.push(tab_id.clone());
    let activated = if workspace.active_tab_id.is_none() {
        workspace.active_tab_id = Some(tab_id.clone());
        true
    } else {
        false
    };
    let mut events = Vec::with_capacity(2);
    let v = state.bump_version();
    events.push(Event::TabCreated {
        workspace_id: workspace_id.clone(),
        tab_id: tab_id.clone(),
        name: resolved_name,
        version: v,
    });
    if activated {
        let v2 = state.bump_version();
        events.push(Event::ActiveTabChanged {
            workspace_id,
            tab_id: Some(tab_id),
            version: v2,
        });
    }
    events
}

/// The tab that takes over as active when the active tab leaves
/// `tab_ids` from `removed_pos`: the next tab, or the previous one if it
/// was last, or `None` if none is left. `tab_ids` is the list after the
/// removal. Deleting and moving a tab out both use it.
fn neighbour_tab(tab_ids: &[String], removed_pos: usize) -> Option<String> {
    tab_ids
        .get(removed_pos)
        .or_else(|| removed_pos.checked_sub(1).and_then(|i| tab_ids.get(i)))
        .cloned()
}

/// Phase E.2b — delete a tab from a workspace. Idempotent: deleting
/// a missing tab is a silent no-op. If the deleted tab was the
/// active tab, the workspace's active tab becomes its neighbour
/// (`neighbour_tab`), and an `Event::ActiveTabChanged` is emitted
/// alongside `Event::TabDeleted`.
pub(super) fn handle_delete_tab(
    state: &mut State,
    workspace_id: String,
    tab_id: String,
    force: bool,
) -> Vec<Event> {
    let Some(workspace) = state.workspaces.get_mut(&workspace_id) else {
        return Vec::new();
    };
    let Some(pos) = workspace.tab_ids.iter().position(|t| t == &tab_id) else {
        return Vec::new();
    };
    // Last-tab guard, here rather than in the caller so two concurrent
    // closes can't both pass a stale count (#633). User flows (CloseTab →
    // DeleteTab saga) pass `force: false`; compensation paths (`CreateTab`
    // rollback, `PromoteBlockToTab` compensate) pass `force: true`.
    if !force && workspace.tab_ids.len() <= 1 {
        return invalid_command(
            state,
            format!(
                "DeleteTab: refusing to delete the last tab in workspace {} (would leave empty workspace; pass force=true for compensation paths)",
                workspace_id,
            ),
        );
    }
    workspace.tab_ids.remove(pos);
    let active_changed = if workspace.active_tab_id.as_deref() == Some(tab_id.as_str()) {
        let new_active = neighbour_tab(&workspace.tab_ids, pos);
        workspace.active_tab_id = new_active.clone();
        Some(new_active)
    } else {
        None
    };
    let removed_tab = state.tabs.remove(&tab_id);
    // Phase E.3 — cascade to blocks. Subscribers observing TabDeleted
    // are expected to drop dependent block state (no per-block
    // BlockDeleted events emitted; mirrors workspace→tabs cascade
    // semantics). `block_ids` on the event below carries the cascaded ids
    // anyway (issue #2218, B.4) — the host uses it to tear down any
    // browser-pane renderer that was never live/loaded in a window and so
    // never got a chance to reach the renderer-mediated close path.
    let cascaded_block_ids: Vec<String> = removed_tab
        .as_ref()
        .map(|t| t.block_ids.clone())
        .unwrap_or_default();
    if let Some(tab) = &removed_tab {
        for block_id in &tab.block_ids {
            state.blocks.remove(block_id);
        }
    }
    let mut events = Vec::with_capacity(2);
    let v = state.bump_version();
    events.push(Event::TabDeleted {
        workspace_id: workspace_id.clone(),
        tab_id,
        block_ids: cascaded_block_ids,
        version: v,
    });
    if let Some(new_active) = active_changed {
        let v2 = state.bump_version();
        events.push(Event::ActiveTabChanged {
            workspace_id,
            tab_id: new_active,
            version: v2,
        });
    }
    events
}

/// Phase E.2b — set a workspace's active tab. No-op if already
/// active. Errors (non-fatal) if the workspace doesn't exist or the
/// tab isn't in that workspace's tab list.
pub(super) fn handle_set_active_tab(
    state: &mut State,
    workspace_id: String,
    tab_id: String,
) -> Vec<Event> {
    let Some(workspace) = state.workspaces.get_mut(&workspace_id) else {
        return invalid_command(state, format!("SetActiveTab: workspace not found: {}", workspace_id));
    };
    if !workspace.tab_ids.iter().any(|t| t == &tab_id) {
        return invalid_command(state, format!("SetActiveTab: tab {} not in workspace {}", tab_id, workspace_id));
    }
    if workspace.active_tab_id.as_deref() == Some(tab_id.as_str()) {
        return Vec::new();
    }
    workspace.active_tab_id = Some(tab_id.clone());
    let v = state.bump_version();
    vec![Event::ActiveTabChanged {
        workspace_id,
        tab_id: Some(tab_id),
        version: v,
    }]
}

/// Phase E.2c.3b — reorder a tab within its workspace's
/// `tab_ids`. `new_index` is clamped to `tab_ids.len()`. No-op
/// if the tab is already at that position. Errors (non-fatal) if
/// the workspace doesn't exist or the tab isn't in its tab list.
pub(super) fn handle_reorder_tab(
    state: &mut State,
    workspace_id: String,
    tab_id: String,
    new_index: u32,
) -> Vec<Event> {
    let Some(workspace) = state.workspaces.get_mut(&workspace_id) else {
        return invalid_command(state, format!("ReorderTab: workspace not found: {}", workspace_id));
    };
    let Some(current_pos) = workspace.tab_ids.iter().position(|t| t == &tab_id) else {
        return invalid_command(state, format!("ReorderTab: tab {} not in workspace {}", tab_id, workspace_id));
    };
    let len = workspace.tab_ids.len();
    let target = (new_index as usize).min(len.saturating_sub(1));
    if current_pos == target {
        return Vec::new();
    }
    let id = workspace.tab_ids.remove(current_pos);
    workspace.tab_ids.insert(target, id);
    let v = state.bump_version();
    vec![Event::TabReordered {
        workspace_id,
        tab_id,
        new_index: target as u32,
        version: v,
    }]
}

/// Phase E.5.3 — replace a workspace's `tab_ids` with the given
/// list. Validates the workspace exists and the new list is a
/// permutation of the current set (same elements, possibly
/// different order). No-op if identical.
pub(super) fn handle_reorder_tabs_bulk(
    state: &mut State,
    workspace_id: String,
    tab_ids: Vec<String>,
) -> Vec<Event> {
    // SPEC_864 Phase 5 — strict validation reinstated. The codex P1 #620
    // membership relaxation existed because `MoveTabToWorkspace` and
    // `PromoteBlockToTab` used to write through wcore without dispatching
    // reducer commands, leaving `workspace.tab_ids` stale here. Both are
    // now reducer-routed (`MoveTabToWorkspace` dispatches `MoveTab`
    // directly; `PromoteBlockToTab`, `TearOffBlock`, and `TearOffTab` run
    // through sagas whose steps dispatch `CreateTab`/`MoveTab`/`MoveBlock`)
    // — every path that changes a workspace's tab set now updates the
    // reducer in the same step, so `tab_ids` can't legitimately drift
    // from SQLite anymore. `tab_ids` must be a permutation of the
    // reducer's own set, not a caller-asserted replacement.
    let Some(current) = state.workspaces.get(&workspace_id).map(|w| w.tab_ids.clone()) else {
        return invalid_command(state, format!("ReorderTabsBulk: workspace not found: {}", workspace_id));
    };
    {
        let mut seen: std::collections::HashSet<&String> =
            std::collections::HashSet::with_capacity(tab_ids.len());
        for id in &tab_ids {
            if !seen.insert(id) {
                return invalid_command(state, format!("ReorderTabsBulk: tab_ids contains duplicate entry: {}", id));
            }
        }
    }
    if current.len() != tab_ids.len()
        || !current.iter().all(|id| tab_ids.contains(id))
    {
        return invalid_command(
            state,
            format!(
                "ReorderTabsBulk: tab_ids must be a permutation of the workspace's current tabs (got {:?}, expected a permutation of {:?})",
                tab_ids, current
            ),
        );
    }
    if current == tab_ids {
        return Vec::new();
    }
    state.workspaces.get_mut(&workspace_id).expect("checked above").tab_ids = tab_ids.clone();
    let v = state.bump_version();
    vec![Event::TabsReorderedBulk {
        workspace_id,
        tab_ids,
        version: v,
    }]
}

/// Phase E.5.5 — move a tab from `src_workspace_id` to
/// `dst_workspace_id`, inserting at `dst_index` (clamped to dst's
/// length). Updates the tab's `workspace_id`, removes it from src's
/// `tab_ids`, inserts into dst's `tab_ids`. If the tab was src's
/// `active_tab_id`, src's active becomes its neighbour, as when the
/// tab is deleted (`neighbour_tab`).
///
/// Errors when:
/// * source / dest workspace not found,
/// * tab not found,
/// * `tab.workspace_id != src_workspace_id` (caller-side bug),
/// * `src_workspace_id == dst_workspace_id` (use `ReorderTab` for
///   intra-workspace reorders — same-workspace moves through this
///   path would create ambiguity around `dst_index` semantics),
/// * the tab is the source's last tab and `allow_empty_source` is
///   false. The guard lives here, not in callers, so two concurrent
///   moves out of a two-tab workspace can't both pass a stale count.
pub(super) fn handle_move_tab(
    state: &mut State,
    tab_id: String,
    src_workspace_id: String,
    dst_workspace_id: String,
    dst_index: u32,
    allow_empty_source: bool,
) -> Vec<Event> {
    if src_workspace_id == dst_workspace_id {
        return invalid_command(state, "MoveTab: src and dst workspaces are identical; use ReorderTab");
    }
    // Strict validation (Phase E.4 strict-mode flip): the
    // migration-tolerant lazy-import fallback (codex P1 round-2
    // #621) was removed once the soak window closed with no
    // `lazy-import` warnings observed in production. All reducer-
    // routed paths now keep `state.tabs` and
    // `state.workspaces[*].tab_ids` consistent with SQLite, so we
    // can reject unknown tabs and workspace_id mismatches outright.
    if !state.workspaces.contains_key(&src_workspace_id) {
        return invalid_command(state, format!("MoveTab: src workspace not found: {}", src_workspace_id));
    }
    if !state.workspaces.contains_key(&dst_workspace_id) {
        return invalid_command(state, format!("MoveTab: dst workspace not found: {}", dst_workspace_id));
    }
    let tab_workspace_id: Option<String> =
        state.tabs.get(&tab_id).map(|t| t.workspace_id.clone());
    match tab_workspace_id {
        None => {
            return invalid_command(state, format!("MoveTab: tab not found in state: {}", tab_id));
        }
        Some(actual_ws) if actual_ws != src_workspace_id => {
            return invalid_command(
                state,
                format!(
                    "MoveTab: workspace_id mismatch — tab {} belongs to {}, not {}",
                    tab_id, actual_ws, src_workspace_id
                ),
            );
        }
        Some(_) => {}
    }
    if !allow_empty_source && state.workspaces[&src_workspace_id].tab_ids.len() <= 1 {
        return invalid_command(
            state,
            format!(
                "MoveTab: {} {} (pass allow_empty_source=true for draining flows)",
                crate::reducer::MOVE_TAB_LAST_TAB_ERROR,
                src_workspace_id,
            ),
        );
    }

    // Remove from src.
    let new_src_active_tab_id: Option<String> = {
        let src = state.workspaces.get_mut(&src_workspace_id).expect("checked");
        if let Some(pos) = src.tab_ids.iter().position(|id| id == &tab_id) {
            src.tab_ids.remove(pos);
            if src.active_tab_id.as_deref() == Some(tab_id.as_str()) {
                src.active_tab_id = neighbour_tab(&src.tab_ids, pos);
            }
        }
        src.active_tab_id.clone()
    };

    // Insert into dst at the clamped index and make the moved tab dst's
    // active tab, so a tear-off never leaves the destination with no
    // active tab (#621).
    let final_dst_index: u32 = {
        let dst = state.workspaces.get_mut(&dst_workspace_id).expect("checked");
        let clamped = (dst_index as usize).min(dst.tab_ids.len());
        dst.tab_ids.insert(clamped, tab_id.clone());
        dst.active_tab_id = Some(tab_id.clone());
        clamped as u32
    };

    // Update the tab's parent.
    state
        .tabs
        .get_mut(&tab_id)
        .expect("checked")
        .workspace_id = dst_workspace_id.clone();

    let v = state.bump_version();
    vec![Event::TabMoved {
        tab_id: tab_id.clone(),
        src_workspace_id,
        dst_workspace_id,
        dst_index: final_dst_index,
        new_src_active_tab_id,
        new_dst_active_tab_id: Some(tab_id),
        version: v,
    }]
}

/// Phase E.5.3 — rename a tab. Errors if missing; no-op if the
/// name is unchanged.
pub(super) fn handle_rename_tab(state: &mut State, tab_id: String, name: String) -> Vec<Event> {
    let Some(tab) = state.tabs.get_mut(&tab_id) else {
        return invalid_command(state, format!("RenameTab: tab not found: {}", tab_id));
    };
    if tab.name == name {
        return Vec::new();
    }
    tab.name = name.clone();
    let v = state.bump_version();
    vec![Event::TabRenamed {
        tab_id,
        name,
        version: v,
    }]
}

/// Phase E.5.3 — pass-through for tab meta updates. Same shape as
/// `handle_update_workspace_meta`.
pub(super) fn handle_update_tab_meta(
    state: &mut State,
    tab_id: String,
    meta_patch: serde_json::Value,
) -> Vec<Event> {
    if !state.tabs.contains_key(&tab_id) {
        return invalid_command(state, format!("UpdateTabMeta: tab not found: {}", tab_id));
    }
    let v = state.bump_version();
    vec![Event::TabMetaUpdated {
        tab_id,
        meta_patch,
        version: v,
    }]
}

#[cfg(test)]
mod tests;
