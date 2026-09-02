// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use agentmux_common::ipc::{ErrorCode, Event};

use crate::state::State;


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
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!("ReorderTabsBulk: workspace not found: {}", workspace_id),
            fatal: false,
            version: v,
        }];
    };
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
    if current.len() != tab_ids.len()
        || !current.iter().all(|id| tab_ids.contains(id))
    {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!(
                "ReorderTabsBulk: tab_ids must be a permutation of the workspace's current tabs (got {:?}, expected a permutation of {:?})",
                tab_ids, current
            ),
            fatal: false,
            version: v,
        }];
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reducer::test_support::*;
    use crate::reducer::update;
    use agentmux_common::ipc::Command;

    #[test]
    fn create_tab_validates_workspace_exists() {
        let mut state = State::default();
        let events = update(
            &mut state,
            Command::CreateTab {
                workspace_id: "no-such-ws".into(),
                name: "t1".into(),
            },
            &ctx(1),
        );
        assert!(matches!(&events[0], Event::Error { .. }));
        assert!(state.tabs.is_empty());
    }

    #[test]
    fn create_tab_first_tab_becomes_active() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let events = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: "t1".into(),
            },
            &ctx(1),
        );
        // First event: TabCreated; second: ActiveTabChanged.
        assert!(matches!(&events[0], Event::TabCreated { .. }));
        assert!(matches!(&events[1], Event::ActiveTabChanged { .. }));
        let workspace = &state.workspaces[&ws_id];
        assert_eq!(workspace.tab_ids.len(), 1);
        assert_eq!(workspace.active_tab_id, Some(workspace.tab_ids[0].clone()));
        assert_eq!(state.tabs.len(), 1);
    }

    #[test]
    fn create_tab_second_tab_does_not_steal_active() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let _ = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: "t1".into(),
            },
            &ctx(1),
        );
        let first_active = state.workspaces[&ws_id].active_tab_id.clone();
        let events = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: "t2".into(),
            },
            &ctx(2),
        );
        // Only TabCreated — second tab does not become active.
        assert!(matches!(&events[0], Event::TabCreated { .. }));
        assert_eq!(events.len(), 1);
        assert_eq!(state.workspaces[&ws_id].active_tab_id, first_active);
        assert_eq!(state.workspaces[&ws_id].tab_ids.len(), 2);
    }

    #[test]
    fn delete_tab_removes_from_state_and_workspace_list() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let _ = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: "t1".into(),
            },
            &ctx(1),
        );
        let _ = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: "t2".into(),
            },
            &ctx(2),
        );
        let tab2_id = state.workspaces[&ws_id].tab_ids[1].clone();
        let events = update(
            &mut state,
            Command::DeleteTab {
                workspace_id: ws_id.clone(),
                tab_id: tab2_id.clone(),
                force: false,
            },
            &ctx(3),
        );
        assert!(matches!(&events[0], Event::TabDeleted { .. }));
        // tab2 wasn't active, so no ActiveTabChanged.
        assert_eq!(events.len(), 1);
        assert!(!state.tabs.contains_key(&tab2_id));
        assert_eq!(state.workspaces[&ws_id].tab_ids.len(), 1);
    }

    #[test]
    fn delete_active_tab_promotes_neighbor() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let _ = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: "t1".into(),
            },
            &ctx(1),
        );
        let _ = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: "t2".into(),
            },
            &ctx(2),
        );
        let tab1_id = state.workspaces[&ws_id].tab_ids[0].clone();
        let tab2_id = state.workspaces[&ws_id].tab_ids[1].clone();
        // tab1 was created first → it's active. Delete it.
        let events = update(
            &mut state,
            Command::DeleteTab {
                workspace_id: ws_id.clone(),
                tab_id: tab1_id.clone(),
                force: false,
            },
            &ctx(3),
        );
        assert!(matches!(&events[0], Event::TabDeleted { .. }));
        assert!(matches!(
            &events[1],
            Event::ActiveTabChanged { tab_id: Some(_), .. }
        ));
        // tab2 should now be active.
        assert_eq!(state.workspaces[&ws_id].active_tab_id, Some(tab2_id));
    }

    #[test]
    fn delete_last_tab_clears_active_to_none() {
        // Reducer accepts last-tab delete (round 2 of PR #633 walked
        // back the guard). User-facing flows gate at the call site
        // (close button + keymodel both check `tab_ids.len() <= 1`);
        // internal compensation paths rely on this acceptance to
        // roll back failed CreateTab persists.
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let _ = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: "t1".into(),
            },
            &ctx(1),
        );
        let tab1_id = state.workspaces[&ws_id].tab_ids[0].clone();
        let events = update(
            &mut state,
            Command::DeleteTab {
                workspace_id: ws_id.clone(),
                tab_id: tab1_id,
                // Last-tab delete needs force=true post-round-4
                // (codex P2 #633). Test asserts the
                // ActiveTabChanged-to-None behavior still works
                // when compensation paths force a last-tab delete.
                force: true,
            },
            &ctx(2),
        );
        assert!(matches!(&events[0], Event::TabDeleted { .. }));
        assert!(matches!(
            &events[1],
            Event::ActiveTabChanged { tab_id: None, .. }
        ));
        assert_eq!(state.workspaces[&ws_id].active_tab_id, None);
        assert!(state.tabs.is_empty());
    }

    #[test]
    fn delete_unknown_tab_silent_no_op() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let events = update(
            &mut state,
            Command::DeleteTab {
                workspace_id: ws_id,
                tab_id: "ghost".into(),
                force: false,
            },
            &ctx(1),
        );
        assert!(events.is_empty());
    }

    #[test]
    fn set_active_tab_validates_workspace_and_tab() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let _ = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: "t1".into(),
            },
            &ctx(1),
        );
        // Wrong workspace.
        let events = update(
            &mut state,
            Command::SetActiveTab {
                workspace_id: "no-such".into(),
                tab_id: "x".into(),
            },
            &ctx(2),
        );
        assert!(matches!(&events[0], Event::Error { .. }));
        // Right workspace, wrong tab.
        let events = update(
            &mut state,
            Command::SetActiveTab {
                workspace_id: ws_id,
                tab_id: "ghost".into(),
            },
            &ctx(3),
        );
        assert!(matches!(&events[0], Event::Error { .. }));
    }

    #[test]
    fn set_active_tab_idempotent_no_event_when_already_active() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let _ = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: "t1".into(),
            },
            &ctx(1),
        );
        let tab_id = state.workspaces[&ws_id].tab_ids[0].clone();
        // Already active (auto-activated on first tab create).
        let events = update(
            &mut state,
            Command::SetActiveTab {
                workspace_id: ws_id,
                tab_id,
            },
            &ctx(2),
        );
        assert!(events.is_empty());
    }

    #[test]
    fn reorder_tab_moves_to_new_index() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        for i in 1..=3 {
            let _ = update(
                &mut state,
                Command::CreateTab {
                    workspace_id: ws_id.clone(),
                    name: format!("t{}", i),
                },
                &ctx(i),
            );
        }
        let original = state.workspaces[&ws_id].tab_ids.clone();
        // Move first tab to index 2 (last).
        let events = update(
            &mut state,
            Command::ReorderTab {
                workspace_id: ws_id.clone(),
                tab_id: original[0].clone(),
                new_index: 2,
            },
            &ctx(10),
        );
        assert!(matches!(&events[0], Event::TabReordered { .. }));
        let after = &state.workspaces[&ws_id].tab_ids;
        assert_eq!(after[0], original[1]);
        assert_eq!(after[1], original[2]);
        assert_eq!(after[2], original[0]);
    }

    #[test]
    fn reorder_tab_clamps_to_last_index() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let _ = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: "t1".into(),
            },
            &ctx(1),
        );
        let _ = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: "t2".into(),
            },
            &ctx(2),
        );
        let original = state.workspaces[&ws_id].tab_ids.clone();
        // Asking for index 99 should clamp to 1 (len-1).
        let events = update(
            &mut state,
            Command::ReorderTab {
                workspace_id: ws_id.clone(),
                tab_id: original[0].clone(),
                new_index: 99,
            },
            &ctx(3),
        );
        if let Event::TabReordered { new_index, .. } = &events[0] {
            assert_eq!(*new_index, 1);
        } else {
            panic!("expected TabReordered");
        }
    }

    #[test]
    fn reorder_tab_already_at_position_no_op() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let _ = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: "t1".into(),
            },
            &ctx(1),
        );
        let tab_id = state.workspaces[&ws_id].tab_ids[0].clone();
        let events = update(
            &mut state,
            Command::ReorderTab {
                workspace_id: ws_id,
                tab_id,
                new_index: 0,
            },
            &ctx(2),
        );
        assert!(events.is_empty());
    }

    #[test]
    fn reorder_tab_validates_workspace_and_tab() {
        let mut state = State::default();
        let events = update(
            &mut state,
            Command::ReorderTab {
                workspace_id: "no-such-ws".into(),
                tab_id: "x".into(),
                new_index: 0,
            },
            &ctx(1),
        );
        assert!(matches!(&events[0], Event::Error { .. }));
        let ws_id = create_workspace(&mut state, "w");
        let events = update(
            &mut state,
            Command::ReorderTab {
                workspace_id: ws_id,
                tab_id: "ghost".into(),
                new_index: 0,
            },
            &ctx(2),
        );
        assert!(matches!(&events[0], Event::Error { .. }));
    }

    /// SPEC_864 Phase 5 — strict validation reinstated: `MoveTabToWorkspace`,
    /// `PromoteBlockToTab`, `TearOffBlock`, and `TearOffTab` are all
    /// reducer-routed now, so `workspace.tab_ids` can't legitimately
    /// diverge from the caller's view. A `tab_ids` list containing an id
    /// the reducer doesn't know about for this workspace must be rejected,
    /// not silently accepted as the codex P1 #620 migration-window
    /// relaxation used to do.
    #[test]
    fn reorder_tabs_bulk_rejects_unknown_ids() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let known = create_tab(&mut state, &ws_id, "known");
        let unknown = "unknown-tab".to_string();
        let events = update(
            &mut state,
            Command::ReorderTabsBulk {
                workspace_id: ws_id.clone(),
                tab_ids: vec![unknown.clone(), known.clone()],
            },
            &ctx(99),
        );
        match &events[0] {
            Event::Error { code, message, .. } => {
                assert_eq!(*code, ErrorCode::InvalidCommand);
                assert!(
                    message.contains("permutation"),
                    "error should mention permutation, got: {}",
                    message
                );
            }
            other => panic!("expected Error event, got {:?}", other),
        }
        // Rejected — workspace's tab_ids must be untouched.
        let ws = state.workspaces.get(&ws_id).expect("ws still present");
        assert_eq!(ws.tab_ids, vec![known]);
    }

    /// The permutation itself (same set, new order) must still succeed —
    /// that's the whole point of the command.
    #[test]
    fn reorder_tabs_bulk_accepts_permutation_of_known_ids() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let t1 = create_tab(&mut state, &ws_id, "t1");
        let t2 = create_tab(&mut state, &ws_id, "t2");
        let events = update(
            &mut state,
            Command::ReorderTabsBulk {
                workspace_id: ws_id.clone(),
                tab_ids: vec![t2.clone(), t1.clone()],
            },
            &ctx(99),
        );
        assert!(
            matches!(&events[0], Event::TabsReorderedBulk { .. }),
            "expected TabsReorderedBulk, got {:?}",
            events.first()
        );
        let ws = state.workspaces.get(&ws_id).expect("ws still present");
        assert_eq!(ws.tab_ids, vec![t2, t1]);
    }

    /// A `tab_ids` list missing a tab the reducer knows about (short
    /// permutation) must also be rejected, not silently drop the tab.
    #[test]
    fn reorder_tabs_bulk_rejects_missing_known_id() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let t1 = create_tab(&mut state, &ws_id, "t1");
        let _t2 = create_tab(&mut state, &ws_id, "t2");
        let events = update(
            &mut state,
            Command::ReorderTabsBulk {
                workspace_id: ws_id,
                tab_ids: vec![t1],
            },
            &ctx(99),
        );
        assert!(matches!(&events[0], Event::Error { .. }));
    }

    /// codex P1 #620 carryover: a duplicate tab_id in the new list
    /// is still rejected — that would corrupt the persisted ordering
    /// in a way the subscriber can't recover from.
    #[test]
    fn reorder_tabs_bulk_rejects_duplicates() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let t1 = create_tab(&mut state, &ws_id, "t1");
        let events = update(
            &mut state,
            Command::ReorderTabsBulk {
                workspace_id: ws_id,
                tab_ids: vec![t1.clone(), t1.clone()],
            },
            &ctx(2),
        );
        match &events[0] {
            Event::Error { code, message, .. } => {
                assert_eq!(*code, ErrorCode::InvalidCommand);
                assert!(
                    message.contains("duplicate"),
                    "error should mention duplicate, got: {}",
                    message
                );
            }
            other => panic!("expected Error event, got {:?}", other),
        }
    }

    #[test]
    fn reorder_tabs_bulk_validates_workspace() {
        let mut state = State::default();
        let events = update(
            &mut state,
            Command::ReorderTabsBulk {
                workspace_id: "no-such-ws".into(),
                tab_ids: vec!["a".into()],
            },
            &ctx(1),
        );
        assert!(matches!(&events[0], Event::Error { .. }));
    }

    /// issue #2218 (B.4): TabDeleted/WorkspaceDeleted never emitted a
    /// per-block BlockDeleted, so the host had no signal to tear down a
    /// browser-pane renderer whose tab/workspace was deleted while it was
    /// never live/loaded in a window. `block_ids` on these events is that
    /// signal — this test locks the cascade actually carries them.
    #[test]
    fn delete_tab_emits_cascaded_block_ids() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let _ = update(
            &mut state,
            Command::CreateTab { workspace_id: ws_id.clone(), name: "t1".into() },
            &ctx(1),
        );
        let tab1_id = state.workspaces[&ws_id].tab_ids[0].clone();
        // A second tab so the delete isn't a last-tab delete (needs force).
        let _ = update(
            &mut state,
            Command::CreateTab { workspace_id: ws_id.clone(), name: "t2".into() },
            &ctx(2),
        );
        let block1 = match &update(
            &mut state,
            Command::CreateBlock { tab_id: tab1_id.clone(), meta: serde_json::Value::Null },
            &ctx(3),
        )[0] {
            Event::BlockCreated { block_id, .. } => block_id.clone(),
            other => panic!("expected BlockCreated, got {:?}", other),
        };
        let block2 = match &update(
            &mut state,
            Command::CreateBlock { tab_id: tab1_id.clone(), meta: serde_json::Value::Null },
            &ctx(4),
        )[0] {
            Event::BlockCreated { block_id, .. } => block_id.clone(),
            other => panic!("expected BlockCreated, got {:?}", other),
        };
        let events = update(
            &mut state,
            Command::DeleteTab { workspace_id: ws_id, tab_id: tab1_id, force: false },
            &ctx(5),
        );
        match &events[0] {
            Event::TabDeleted { block_ids, .. } => {
                assert_eq!(block_ids.len(), 2);
                assert!(block_ids.contains(&block1));
                assert!(block_ids.contains(&block2));
            }
            other => panic!("expected TabDeleted, got {:?}", other),
        }
    }

    #[test]
    fn delete_tab_with_zero_blocks_emits_empty_block_ids() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let _ = update(
            &mut state,
            Command::CreateTab { workspace_id: ws_id.clone(), name: "t1".into() },
            &ctx(1),
        );
        let tab1_id = state.workspaces[&ws_id].tab_ids[0].clone();
        let _ = update(
            &mut state,
            Command::CreateTab { workspace_id: ws_id.clone(), name: "t2".into() },
            &ctx(2),
        );
        let events = update(
            &mut state,
            Command::DeleteTab { workspace_id: ws_id, tab_id: tab1_id, force: false },
            &ctx(3),
        );
        match &events[0] {
            Event::TabDeleted { block_ids, .. } => assert!(block_ids.is_empty()),
            other => panic!("expected TabDeleted, got {:?}", other),
        }
    }

    /// codex P2 #622: empty name auto-generates `tabN`, mirroring
    /// `wcore::create_tab`'s default-naming behaviour. Without this,
    /// CreateWindow's "fresh workspace" path + TearOffBlock's new tab
    /// would land with blank titles — a user-visible regression.
    #[test]
    fn create_tab_auto_generates_tabN_when_name_empty() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let events = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: String::new(),
            },
            &ctx(2),
        );
        match &events[0] {
            Event::TabCreated { name, tab_id, .. } => {
                assert_eq!(name, "Tab 1", "first tab in fresh workspace");
                assert_eq!(state.tabs[tab_id].name, "Tab 1");
            }
            other => panic!("expected TabCreated, got {:?}", other),
        }
        // Second empty-name CreateTab → "Tab 2".
        let events = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: String::new(),
            },
            &ctx(3),
        );
        match &events[0] {
            Event::TabCreated { name, .. } => assert_eq!(name, "Tab 2"),
            other => panic!("expected TabCreated, got {:?}", other),
        }
        // Explicit non-empty name passes through verbatim.
        let events = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id,
                name: "my custom tab".into(),
            },
            &ctx(4),
        );
        match &events[0] {
            Event::TabCreated { name, .. } => assert_eq!(name, "my custom tab"),
            other => panic!("expected TabCreated, got {:?}", other),
        }
    }

    #[test]
    fn delete_tab_cascades_blocks() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let tab_id = create_tab(&mut state, &ws_id, "t");
        let _ = update(
            &mut state,
            Command::CreateBlock { tab_id: tab_id.clone(), meta: serde_json::Value::Null },
            &ctx(2),
        );
        let _ = update(
            &mut state,
            Command::CreateBlock { tab_id: tab_id.clone(), meta: serde_json::Value::Null },
            &ctx(3),
        );
        assert_eq!(state.blocks.len(), 2);
        let _ = update(
            &mut state,
            Command::DeleteTab {
                workspace_id: ws_id,
                tab_id,
                // Single-tab workspace; force=true bypasses last-tab
                // guard so we can test the block cascade.
                force: true,
            },
            &ctx(4),
        );
        assert!(state.blocks.is_empty());
    }

    // ---- Phase E.5.5 — MoveTab tests ----

    #[test]
    fn move_tab_cross_workspace_updates_lists_and_parent() {
        let mut state = State::default();
        let src = create_workspace(&mut state, "src");
        let dst = create_workspace(&mut state, "dst");
        let t1 = create_tab(&mut state, &src, "t1");
        let t2 = create_tab(&mut state, &src, "t2");
        let dst_existing = create_tab(&mut state, &dst, "existing");
        let events = update(
            &mut state,
            Command::MoveTab {
                tab_id: t1.clone(),
                src_workspace_id: src.clone(),
                dst_workspace_id: dst.clone(),
                dst_index: 0,
            },
            &ctx(99),
        );
        match &events[0] {
            Event::TabMoved {
                tab_id,
                src_workspace_id,
                dst_workspace_id,
                dst_index,
                new_src_active_tab_id,
                ..
            } => {
                assert_eq!(tab_id, &t1);
                assert_eq!(src_workspace_id, &src);
                assert_eq!(dst_workspace_id, &dst);
                assert_eq!(*dst_index, 0);
                assert_eq!(new_src_active_tab_id, &Some(t2.clone()));
            }
            other => panic!("expected TabMoved, got {:?}", other),
        }
        assert_eq!(state.workspaces[&src].tab_ids, vec![t2.clone()]);
        assert_eq!(state.workspaces[&dst].tab_ids, vec![t1.clone(), dst_existing]);
        assert_eq!(state.tabs[&t1].workspace_id, dst);
        assert_eq!(state.workspaces[&src].active_tab_id, Some(t2));
    }

    #[test]
    fn move_tab_clamps_dst_index_to_dst_length() {
        let mut state = State::default();
        let src = create_workspace(&mut state, "src");
        let dst = create_workspace(&mut state, "dst");
        let t1 = create_tab(&mut state, &src, "t1");
        let _ = create_tab(&mut state, &src, "filler");
        let events = update(
            &mut state,
            Command::MoveTab {
                tab_id: t1.clone(),
                src_workspace_id: src,
                dst_workspace_id: dst.clone(),
                dst_index: 999,
            },
            &ctx(2),
        );
        match &events[0] {
            Event::TabMoved { dst_index, .. } => assert_eq!(*dst_index, 0),
            other => panic!("expected TabMoved, got {:?}", other),
        }
        assert_eq!(state.workspaces[&dst].tab_ids, vec![t1]);
    }

    #[test]
    fn move_tab_src_active_clears_when_workspace_empties() {
        let mut state = State::default();
        let src = create_workspace(&mut state, "src");
        let dst = create_workspace(&mut state, "dst");
        let only_tab = create_tab(&mut state, &src, "only");
        let events = update(
            &mut state,
            Command::MoveTab {
                tab_id: only_tab,
                src_workspace_id: src.clone(),
                dst_workspace_id: dst,
                dst_index: 0,
            },
            &ctx(2),
        );
        match &events[0] {
            Event::TabMoved {
                new_src_active_tab_id,
                ..
            } => assert_eq!(new_src_active_tab_id, &None),
            other => panic!("expected TabMoved, got {:?}", other),
        }
        assert_eq!(state.workspaces[&src].active_tab_id, None);
        assert!(state.workspaces[&src].tab_ids.is_empty());
    }

    #[test]
    fn move_tab_rejects_same_workspace() {
        let mut state = State::default();
        let ws = create_workspace(&mut state, "w");
        let t1 = create_tab(&mut state, &ws, "t1");
        let events = update(
            &mut state,
            Command::MoveTab {
                tab_id: t1,
                src_workspace_id: ws.clone(),
                dst_workspace_id: ws,
                dst_index: 0,
            },
            &ctx(2),
        );
        assert!(matches!(&events[0], Event::Error { .. }));
    }

    #[test]
    fn move_tab_rejects_unknown_src_or_dst_or_tab() {
        let mut state = State::default();
        let src = create_workspace(&mut state, "src");
        let dst = create_workspace(&mut state, "dst");
        let t1 = create_tab(&mut state, &src, "t1");

        let events = update(
            &mut state,
            Command::MoveTab {
                tab_id: t1.clone(),
                src_workspace_id: "no-such-src".into(),
                dst_workspace_id: dst.clone(),
                dst_index: 0,
            },
            &ctx(2),
        );
        assert!(matches!(&events[0], Event::Error { .. }));

        let events = update(
            &mut state,
            Command::MoveTab {
                tab_id: t1.clone(),
                src_workspace_id: src.clone(),
                dst_workspace_id: "no-such-dst".into(),
                dst_index: 0,
            },
            &ctx(3),
        );
        assert!(matches!(&events[0], Event::Error { .. }));

        // Phase E.4 strict-mode flip: unknown tabs are now REJECTED.
        // The migration-tolerant lazy-import fallback was removed once
        // the soak window closed without `lazy-import` warnings being
        // observed in production. See `move_tab_unknown_tab_rejects`
        // for the dedicated test.
        let events = update(
            &mut state,
            Command::MoveTab {
                tab_id: "ghost-tab".into(),
                src_workspace_id: src,
                dst_workspace_id: dst,
                dst_index: 0,
            },
            &ctx(4),
        );
        assert!(matches!(&events[0], Event::Error { .. }));
    }

    /// Phase E.4 strict-mode flip: an unknown tab id (not present in
    /// `state.tabs`) is rejected with a clear "tab not found" error
    /// rather than being lazy-imported. Replaces the migration-window
    /// `move_tab_lazy_imports_unknown_tab` test.
    #[test]
    fn move_tab_unknown_tab_rejects() {
        let mut state = State::default();
        let src = create_workspace(&mut state, "src");
        let dst = create_workspace(&mut state, "dst");
        let unknown_id = "unknown-tab-xyz".to_string();
        assert!(!state.tabs.contains_key(&unknown_id));
        let events = update(
            &mut state,
            Command::MoveTab {
                tab_id: unknown_id.clone(),
                src_workspace_id: src,
                dst_workspace_id: dst,
                dst_index: 0,
            },
            &ctx(2),
        );
        match &events[0] {
            Event::Error { code, message, .. } => {
                assert_eq!(*code, ErrorCode::InvalidCommand);
                assert!(
                    message.contains("tab not found"),
                    "error should mention `tab not found`, got: {}",
                    message
                );
            }
            other => panic!("expected Error event, got {:?}", other),
        }
        // No lazy import side-effect.
        assert!(!state.tabs.contains_key(&unknown_id));
    }

    /// Phase E.4 strict-mode flip: a known tab whose reducer-state
    /// `workspace_id` doesn't match `src_workspace_id` is rejected.
    /// Replaces the migration-window
    /// `move_tab_tolerates_workspace_id_mismatch_during_migration`
    /// test.
    #[test]
    fn move_tab_wrong_workspace_rejects() {
        let mut state = State::default();
        let real_src = create_workspace(&mut state, "real_src");
        let dst = create_workspace(&mut state, "dst");
        let other = create_workspace(&mut state, "other");
        let t1 = create_tab(&mut state, &real_src, "t1");
        let filler = create_tab(&mut state, &other, "filler");
        // Claim the tab lives in `other` even though it actually
        // belongs to `real_src` per reducer state.
        let events = update(
            &mut state,
            Command::MoveTab {
                tab_id: t1.clone(),
                src_workspace_id: other.clone(),
                dst_workspace_id: dst.clone(),
                dst_index: 0,
            },
            &ctx(2),
        );
        match &events[0] {
            Event::Error { code, message, .. } => {
                assert_eq!(*code, ErrorCode::InvalidCommand);
                assert!(
                    message.contains("workspace_id mismatch"),
                    "error should mention `workspace_id mismatch`, got: {}",
                    message
                );
            }
            other => panic!("expected Error event, got {:?}", other),
        }
        // Reducer state untouched: t1 still in real_src, filler still
        // in other, dst empty.
        assert_eq!(state.tabs[&t1].workspace_id, real_src);
        assert_eq!(state.workspaces[&real_src].tab_ids, vec![t1]);
        assert_eq!(state.workspaces[&other].tab_ids, vec![filler]);
        assert!(state.workspaces[&dst].tab_ids.is_empty());
    }
}
