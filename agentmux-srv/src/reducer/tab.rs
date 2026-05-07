// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use agentmux_common::ipc::{Command, ErrorCode, Event};

use crate::state::State;

use super::Ctx;

use crate::state::TabRecord;

/// Phase E.2b — create a tab inside a workspace. Validates the
/// parent exists; otherwise emits `Event::Error` (non-fatal). On
/// success: assigns a UUID, appends to the workspace's `tab_ids`,
/// inserts into `state.tabs`, emits `Event::TabCreated`. If the
/// workspace had no active tab, the new tab also becomes active
/// and an `Event::ActiveTabChanged` is emitted alongside.
///
/// NOT idempotent on retry (same UUID-assignment caveat as
/// `handle_create_workspace`).
pub(super) fn handle_create_tab(state: &mut State, workspace_id: String, name: String) -> Vec<Event> {
    let Some(workspace_record) = state.workspaces.get(&workspace_id) else {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!("CreateTab: workspace not found: {}", workspace_id),
            fatal: false,
            version: v,
        }];
    };
    // codex P2 #622: auto-generate `tabN` when name is empty,
    // matching `wcore::create_tab`'s default-naming behaviour. The
    // counter uses the reducer's tab_ids length + 1 (matching the
    // old SQLite-side count: tabids.len() + pinnedtabids.len() + 1
    // — pinnedtabids stays at zero in production since pinning
    // was removed in E.2c.3b, so reducer-only counting matches).
    let resolved_name = if name.is_empty() {
        format!("tab{}", workspace_record.tab_ids.len() + 1)
    } else {
        name
    };
    let tab_id = uuid::Uuid::new_v4().to_string();
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

/// Phase E.2b — delete a tab from a workspace. Idempotent: deleting
/// a missing tab is a silent no-op. If the deleted tab was the
/// active tab, the workspace's active tab becomes the next tab in
/// `tab_ids` (or the previous one if the deleted was last; or None
/// if the workspace is now empty), and an `Event::ActiveTabChanged`
/// is emitted alongside `Event::TabDeleted`.
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
    // Last-tab guard with `force` bypass (round 4 of PR #633).
    // History:
    //   * Round 1: saga pre-check → reagent flagged TOCTOU race.
    //   * Round 2: moved guard to reducer (atomic) → broke CreateTab
    //     compensation (codex P1 round 2) and Cmd+W keyboard flow
    //     (codex P1 round 1).
    //   * Round 3: removed guard entirely; saga keeps soft pre-check
    //     → codex P2 round 4 re-flagged the TOCTOU race.
    //   * Round 4 (this): atomic guard with `force: bool` bypass.
    //     User-facing flows (CloseTab RPC → DeleteTab saga) pass
    //     `force: false`; compensation paths (`CreateTab` rollback,
    //     `PromoteBlockToTab.ctx.compensate`) pass `force: true`.
    //     Frontend keyboard handler `simpleCloseStaticTab` already
    //     gates pre-RPC, so the reducer rejection is a defense-in-
    //     depth backstop that catches automation/race paths.
    if !force && workspace.tab_ids.len() <= 1 {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!(
                "DeleteTab: refusing to delete the last tab in workspace {} (would leave empty workspace; pass force=true for compensation paths)",
                workspace_id,
            ),
            fatal: false,
            version: v,
        }];
    }
    workspace.tab_ids.remove(pos);
    let active_changed = if workspace.active_tab_id.as_deref() == Some(tab_id.as_str()) {
        let new_active = workspace
            .tab_ids
            .get(pos)
            .or_else(|| pos.checked_sub(1).and_then(|i| workspace.tab_ids.get(i)))
            .cloned();
        workspace.active_tab_id = new_active.clone();
        Some(new_active)
    } else {
        None
    };
    let removed_tab = state.tabs.remove(&tab_id);
    // Phase E.3 — cascade to blocks. Subscribers observing TabDeleted
    // are expected to drop dependent block state (no per-block
    // BlockDeleted events emitted; mirrors workspace→tabs cascade
    // semantics).
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
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!("SetActiveTab: workspace not found: {}", workspace_id),
            fatal: false,
            version: v,
        }];
    };
    if !workspace.tab_ids.iter().any(|t| t == &tab_id) {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!(
                "SetActiveTab: tab {} not in workspace {}",
                tab_id, workspace_id
            ),
            fatal: false,
            version: v,
        }];
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
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!("ReorderTab: workspace not found: {}", workspace_id),
            fatal: false,
            version: v,
        }];
    };
    let Some(current_pos) = workspace.tab_ids.iter().position(|t| t == &tab_id) else {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!(
                "ReorderTab: tab {} not in workspace {}",
                tab_id, workspace_id
            ),
            fatal: false,
            version: v,
        }];
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
    // codex P1 #620 carryover: relax membership validation until tab
    // moves are migrated through the reducer. `MoveTabToWorkspace`
    // and `PromoteBlockToTab` (planned for PR 4) still write through
    // wcore without dispatching reducer commands, so the reducer's
    // view of `workspace.tab_ids` can be stale relative to SQLite.
    // A subsequent `UpdateTabIds` (now routed through this command)
    // must not refuse the canonical order just because the reducer
    // hasn't seen the upstream move yet — that would be a
    // user-visible regression vs. the prior wcore-direct path.
    //
    // Treat the caller's `tab_ids` as authoritative. The remaining
    // checks are basic sanity: the workspace must exist in the
    // reducer, and `tab_ids` must not contain duplicates (which would
    // produce a corrupt persisted ordering with no way for the
    // subscriber to recover). Length / set comparison against the
    // reducer's stale view is dropped here; PR 4 reinstates strict
    // validation once tab moves go through the reducer.
    if !state.workspaces.contains_key(&workspace_id) {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!("ReorderTabsBulk: workspace not found: {}", workspace_id),
            fatal: false,
            version: v,
        }];
    }
    {
        let mut seen: std::collections::HashSet<&String> =
            std::collections::HashSet::with_capacity(tab_ids.len());
        for id in &tab_ids {
            if !seen.insert(id) {
                let v = state.bump_version();
                return vec![Event::Error {
                    code: ErrorCode::InvalidCommand,
                    message: format!(
                        "ReorderTabsBulk: tab_ids contains duplicate entry: {}",
                        id
                    ),
                    fatal: false,
                    version: v,
                }];
            }
        }
    }
    if state.workspaces.get(&workspace_id).expect("checked").tab_ids == tab_ids {
        return Vec::new();
    }
    state.workspaces.get_mut(&workspace_id).expect("checked").tab_ids = tab_ids.clone();
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
/// `active_tab_id`, src's active reverts to its first remaining
/// tab (or `None` when empty).
///
/// Errors when:
/// * source / dest workspace not found,
/// * tab not found,
/// * `tab.workspace_id != src_workspace_id` (caller-side bug),
/// * `src_workspace_id == dst_workspace_id` (use `ReorderTab` for
///   intra-workspace reorders — same-workspace moves through this
///   path would create ambiguity around `dst_index` semantics).
pub(super) fn handle_move_tab(
    state: &mut State,
    tab_id: String,
    src_workspace_id: String,
    dst_workspace_id: String,
    dst_index: u32,
) -> Vec<Event> {
    if src_workspace_id == dst_workspace_id {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: "MoveTab: src and dst workspaces are identical; use ReorderTab".into(),
            fatal: false,
            version: v,
        }];
    }
    // Strict validation (Phase E.4 strict-mode flip): the
    // migration-tolerant lazy-import fallback (codex P1 round-2
    // #621) was removed once the soak window closed with no
    // `lazy-import` warnings observed in production. All reducer-
    // routed paths now keep `state.tabs` and
    // `state.workspaces[*].tab_ids` consistent with SQLite, so we
    // can reject unknown tabs and workspace_id mismatches outright.
    if !state.workspaces.contains_key(&src_workspace_id) {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!("MoveTab: src workspace not found: {}", src_workspace_id),
            fatal: false,
            version: v,
        }];
    }
    if !state.workspaces.contains_key(&dst_workspace_id) {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!("MoveTab: dst workspace not found: {}", dst_workspace_id),
            fatal: false,
            version: v,
        }];
    }
    let tab_workspace_id: Option<String> =
        state.tabs.get(&tab_id).map(|t| t.workspace_id.clone());
    match tab_workspace_id {
        None => {
            let v = state.bump_version();
            return vec![Event::Error {
                code: ErrorCode::InvalidCommand,
                message: format!("MoveTab: tab not found in state: {}", tab_id),
                fatal: false,
                version: v,
            }];
        }
        Some(actual_ws) if actual_ws != src_workspace_id => {
            let v = state.bump_version();
            return vec![Event::Error {
                code: ErrorCode::InvalidCommand,
                message: format!(
                    "MoveTab: workspace_id mismatch — tab {} belongs to {}, not {}",
                    tab_id, actual_ws, src_workspace_id
                ),
                fatal: false,
                version: v,
            }];
        }
        Some(_) => {}
    }

    // Remove from src.
    let new_src_active_tab_id: Option<String> = {
        let src = state.workspaces.get_mut(&src_workspace_id).expect("checked");
        src.tab_ids.retain(|id| id != &tab_id);
        if src.active_tab_id.as_deref() == Some(tab_id.as_str()) {
            src.active_tab_id = src.tab_ids.first().cloned();
        }
        src.active_tab_id.clone()
    };

    // Insert into dst at clamped index. Set the moved tab as dst's
    // new active tab — mirrors wcore::move_tab_to_workspace
    // behaviour and addresses codex P2 #621 (dst.active_tab_id was
    // previously left untouched, so a saga-driven tear-off could
    // produce a destination workspace with no active tab selected).
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
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!("RenameTab: tab not found: {}", tab_id),
            fatal: false,
            version: v,
        }];
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
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!("UpdateTabMeta: tab not found: {}", tab_id),
            fatal: false,
            version: v,
        }];
    }
    let v = state.bump_version();
    vec![Event::TabMetaUpdated {
        tab_id,
        meta_patch,
        version: v,
    }]
}
