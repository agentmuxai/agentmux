// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Orphan reaper — docs/specs/SPEC_AGENT_PANE_CLOSE_GRACEFUL_SHUTDOWN_2026_09_18.md §4.9.
//
// An agent block that is in a tab's `block_ids` but in no pane of that tab
// is a bug, not a state: its process keeps running and nothing on screen
// can reach it (the Manoz incident, spec §2). The close paths no longer
// produce one (#3402, #3414), but other ways remain — a create-then-place
// interrupted between its two steps, a stale whole-tree push
// (SPEC_PANE_TABS_REDUCER_COMMANDS_2026_09_18.md §2) — and orphans from
// before those fixes are still on disk. This is the backstop, not the fix.
//
// Every sweep finds agent blocks that are unplaced in a tab with a loaded
// layout. A block is reaped (the ordinary graceful close, `close_pane`) only
// once it has stayed unplaced for `MIN_ORPHAN_AGE` across sweeps — the
// window in which a block is legitimately unplaced (created with
// `pane.open { skip_placement }` and not yet pushed onto a stack, or
// mid-drag) is well under a second.
//
// Never reaped:
// - non-agent blocks (terminals etc. — narrower scope first; widen once this
//   has run in the field);
// - sub-blocks (their parent is a block, not a pane — they sit in a parent's
//   `subblockids`);
// - blocks already closing;
// - blocks in a tab whose layout tree is empty — "no tree yet" can't be told
//   apart from "not loaded yet" from here.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use crate::backend::blockcontroller;
use crate::backend::obj::Block;
use crate::server::AppState;

/// First sweep waits for the frontend to load and push its layouts.
const FIRST_SWEEP_DELAY: Duration = Duration::from_secs(120);
const SWEEP_INTERVAL: Duration = Duration::from_secs(60);
/// How long a block must stay unplaced before it is reaped.
pub(crate) const MIN_ORPHAN_AGE: Duration = Duration::from_secs(90);

/// Start the periodic sweep. Called once from `main` after `AppState` exists.
pub fn install(state: &AppState) {
    let state = state.clone();
    tokio::spawn(async move {
        tokio::time::sleep(FIRST_SWEEP_DELAY).await;
        let mut first_seen: HashMap<String, Instant> = HashMap::new();
        loop {
            let reaped = sweep(&state, &mut first_seen, Instant::now(), MIN_ORPHAN_AGE).await;
            if !reaped.is_empty() {
                tracing::warn!(count = reaped.len(), blocks = ?reaped, "orphan_reaper: closed unplaced agent blocks");
            }
            tokio::time::sleep(SWEEP_INTERVAL).await;
        }
    });
}

/// One sweep. `first_seen` carries each candidate's first sighting across
/// sweeps; a block that got placed (or deleted) in between drops out of it.
/// Returns the block ids it closed.
pub(crate) async fn sweep(
    state: &AppState,
    first_seen: &mut HashMap<String, Instant>,
    now: Instant,
    min_age: Duration,
) -> Vec<String> {
    let candidates = unplaced_agent_blocks(state).await;
    first_seen.retain(|id, _| candidates.contains(id));
    let mut due = Vec::new();
    for id in candidates {
        let seen = *first_seen.entry(id.clone()).or_insert(now);
        if now.duration_since(seen) >= min_age {
            due.push(id);
        }
    }

    let mut reaped = Vec::new();
    for id in due {
        tracing::warn!(block_id = %id, "orphan_reaper: agent block has been in no pane for too long — closing it");
        match super::close_pane::run(state, vec![id.clone()]).await {
            Ok(_) => {
                first_seen.remove(&id);
                reaped.push(id);
            }
            Err(e) => tracing::warn!(block_id = %id, error = %e, "orphan_reaper: close failed; will retry next sweep"),
        }
    }
    reaped
}

/// Agent blocks that are in their tab's `block_ids` but in no leaf of the
/// tab's layout, minus everything the module doc lists as never reaped.
async fn unplaced_agent_blocks(state: &AppState) -> HashSet<String> {
    let unplaced: Vec<(String, String)> = {
        let s = state.srv_state.lock().await;
        let mut out = Vec::new();
        for tab in s.tabs.values() {
            let Some(root) = tab.rootnode.as_ref() else { continue };
            let mut placed = HashSet::new();
            collect_members(root, &mut placed);
            out.extend(
                tab.block_ids
                    .iter()
                    .filter(|b| !placed.contains(*b))
                    .map(|b| (tab.tab_id.clone(), b.clone())),
            );
        }
        out
    };
    // A block the backend has queued for placement (`agent.open`, docked
    // `pane.open`, redock) is in no leaf until a frontend applies the action
    // and pushes its tree back — indefinitely, if no window is showing that
    // tab. That is pending placement, not an orphan.
    let pending: HashSet<String> = unplaced
        .iter()
        .map(|(tab_id, _)| tab_id)
        .collect::<HashSet<_>>()
        .into_iter()
        .flat_map(|tab_id| pending_placement_blocks(state, tab_id))
        .collect();
    let unplaced: Vec<String> = unplaced
        .into_iter()
        .map(|(_, b)| b)
        .filter(|b| !pending.contains(b))
        .collect();
    if unplaced.is_empty() {
        return HashSet::new();
    }

    let blocks: Vec<Block> = state.mstore.get_all::<Block>().unwrap_or_default();
    let sub_blocks: HashSet<&String> = blocks
        .iter()
        .filter_map(|b| b.subblockids.as_ref())
        .flatten()
        .collect();
    let agent_blocks: HashSet<&String> = blocks
        .iter()
        .filter(|b| crate::backend::obj::meta_get_string(&b.meta, "view", "") == "agent")
        .map(|b| &b.oid)
        .collect();

    unplaced
        .into_iter()
        .filter(|id| agent_blocks.contains(id) && !sub_blocks.contains(id) && !blockcontroller::is_closing(id))
        .collect()
}

/// Blocks named by a queued, not-yet-applied placement action
/// (`pendingbackendactions`) in this tab's layout row.
fn pending_placement_blocks(state: &AppState, tab_id: &str) -> Vec<String> {
    let Ok(tab) = state.mstore.must_get::<crate::backend::obj::Tab>(tab_id) else {
        return Vec::new();
    };
    let Ok(layout) = state.mstore.must_get::<crate::backend::obj::LayoutState>(&tab.layoutstate) else {
        return Vec::new();
    };
    layout
        .pendingbackendactions
        .unwrap_or_default()
        .into_iter()
        .filter(|a| a.actiontype != "delete")
        .map(|a| a.blockid)
        .collect()
}

fn collect_members(node: &agentmux_common::LayoutNode, out: &mut HashSet<String>) {
    if let Some(data) = node.data.as_ref() {
        out.extend(crate::backend::layout::leaf_members(data));
    }
    for child in &node.children {
        collect_members(child, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::tests::test_state;
    use agentmux_common::ipc::{Command, Event};

    async fn dispatch_apply(state: &AppState, cmd: Command) -> Vec<Event> {
        let events = crate::server::service::dispatch_to_reducer(state, cmd).await;
        for ev in &events {
            crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore).unwrap();
        }
        events
    }

    async fn seed_tab(state: &AppState) -> String {
        let ws = dispatch_apply(state, Command::CreateWorkspace { name: "w".into() })
            .await
            .iter()
            .find_map(|e| match e {
                Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
                _ => None,
            })
            .unwrap();
        dispatch_apply(state, Command::CreateTab { workspace_id: ws, name: "t".into() })
            .await
            .iter()
            .find_map(|e| match e {
                Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
                _ => None,
            })
            .unwrap()
    }

    async fn seed_block(state: &AppState, tab_id: &str, view: &str) -> String {
        dispatch_apply(
            state,
            Command::CreateBlock {
                tab_id: tab_id.to_string(),
                meta: serde_json::json!({ "view": view }),
            },
        )
        .await
        .iter()
        .find_map(|e| match e {
            Event::BlockCreated { block_id, .. } => Some(block_id.clone()),
            _ => None,
        })
        .unwrap()
    }

    async fn place(state: &AppState, tab_id: &str, block_ids: &[&String]) {
        let children = block_ids
            .iter()
            .map(|b| agentmux_common::LayoutNode {
                id: format!("n-{b}"),
                data: Some(agentmux_common::LayoutNodeData {
                    block_id: (*b).clone(),
                    ..Default::default()
                }),
                ..Default::default()
            })
            .collect();
        dispatch_apply(
            state,
            Command::LayoutSetTree {
                tab_id: tab_id.to_string(),
                new_tree: Some(agentmux_common::LayoutNode {
                    id: "root".into(),
                    children,
                    ..Default::default()
                }),
                correlation_id: String::new(),
                slices: None,
            },
        )
        .await;
    }

    /// The incident shape: an agent block in the tab, in no pane. It is left
    /// alone until it has been unplaced for the minimum age, then closed.
    #[tokio::test]
    async fn an_unplaced_agent_block_is_reaped_only_after_the_minimum_age() {
        let state = test_state();
        let tab = seed_tab(&state).await;
        let placed = seed_block(&state, &tab, "agent").await;
        let orphan = seed_block(&state, &tab, "agent").await;
        place(&state, &tab, &[&placed]).await;

        let t0 = Instant::now();
        let mut seen = HashMap::new();
        let age = Duration::from_secs(90);

        assert!(sweep(&state, &mut seen, t0, age).await.is_empty(), "first sighting: too young");
        assert!(state.srv_state.lock().await.blocks.contains_key(&orphan));

        let reaped = sweep(&state, &mut seen, t0 + age, age).await;
        assert_eq!(reaped, vec![orphan.clone()]);
        let s = state.srv_state.lock().await;
        assert!(!s.blocks.contains_key(&orphan), "orphan closed");
        assert!(s.blocks.contains_key(&placed), "placed block untouched");
    }

    /// A block that gets placed between sweeps (skip_placement then a stack
    /// push, or a drag) starts over — never reaped.
    #[tokio::test]
    async fn a_block_placed_between_sweeps_is_forgotten() {
        let state = test_state();
        let tab = seed_tab(&state).await;
        let a = seed_block(&state, &tab, "agent").await;
        let fresh = seed_block(&state, &tab, "agent").await;
        place(&state, &tab, &[&a]).await;

        let t0 = Instant::now();
        let mut seen = HashMap::new();
        let age = Duration::from_secs(90);
        sweep(&state, &mut seen, t0, age).await;
        assert!(seen.contains_key(&fresh));

        place(&state, &tab, &[&a, &fresh]).await;
        assert!(sweep(&state, &mut seen, t0 + age, age).await.is_empty());
        assert!(!seen.contains_key(&fresh), "placed → dropped from the watch list");
        assert!(state.srv_state.lock().await.blocks.contains_key(&fresh));
    }

    #[tokio::test]
    async fn non_agent_blocks_and_sub_blocks_are_never_reaped() {
        let state = test_state();
        let tab = seed_tab(&state).await;
        let placed = seed_block(&state, &tab, "agent").await;
        let term = seed_block(&state, &tab, "term").await;
        let sub = seed_block(&state, &tab, "agent").await;
        place(&state, &tab, &[&placed]).await;
        // Make `sub` a sub-block of `placed`.
        let mut parent = state.mstore.must_get::<Block>(&placed).unwrap();
        parent.subblockids = Some(vec![sub.clone()]);
        state.mstore.update(&mut parent).unwrap();

        let t0 = Instant::now();
        let mut seen = HashMap::new();
        let age = Duration::from_secs(90);
        sweep(&state, &mut seen, t0, age).await;
        assert!(sweep(&state, &mut seen, t0 + age, age).await.is_empty());
        let s = state.srv_state.lock().await;
        assert!(s.blocks.contains_key(&term) && s.blocks.contains_key(&sub));
    }

    /// `agent.open` queues the new block's placement for the frontend; with
    /// no window showing the tab it stays queued. Not an orphan.
    #[tokio::test]
    async fn a_block_with_a_queued_placement_is_not_reaped() {
        let state = test_state();
        let tab = seed_tab(&state).await;
        let placed = seed_block(&state, &tab, "agent").await;
        let opening = seed_block(&state, &tab, "agent").await;
        place(&state, &tab, &[&placed]).await;
        crate::server::service::layout_helpers::queue_target_layout_insert(&state, &tab, &opening)
            .await
            .unwrap();

        let t0 = Instant::now();
        let mut seen = HashMap::new();
        let age = Duration::from_secs(90);
        sweep(&state, &mut seen, t0, age).await;
        assert!(sweep(&state, &mut seen, t0 + age, age).await.is_empty());
        assert!(state.srv_state.lock().await.blocks.contains_key(&opening));
    }

    /// No tree at all can't be told apart from "layout not loaded yet".
    #[tokio::test]
    async fn a_tab_with_no_layout_tree_is_skipped() {
        let state = test_state();
        let tab = seed_tab(&state).await;
        let a = seed_block(&state, &tab, "agent").await;

        let t0 = Instant::now();
        let mut seen = HashMap::new();
        let age = Duration::from_secs(90);
        sweep(&state, &mut seen, t0, age).await;
        assert!(sweep(&state, &mut seen, t0 + age, age).await.is_empty());
        assert!(state.srv_state.lock().await.blocks.contains_key(&a));
    }

    #[tokio::test]
    async fn a_background_stack_member_counts_as_placed() {
        let state = test_state();
        let tab = seed_tab(&state).await;
        let visible = seed_block(&state, &tab, "agent").await;
        let background = seed_block(&state, &tab, "agent").await;
        dispatch_apply(
            &state,
            Command::LayoutSetTree {
                tab_id: tab.clone(),
                new_tree: Some(agentmux_common::LayoutNode {
                    id: "pane".into(),
                    data: Some(agentmux_common::LayoutNodeData {
                        block_id: visible.clone(),
                        block_stack: vec![visible.clone(), background.clone()],
                        active_block_id: visible.clone(),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                correlation_id: String::new(),
                slices: None,
            },
        )
        .await;

        let t0 = Instant::now();
        let mut seen = HashMap::new();
        let age = Duration::from_secs(90);
        sweep(&state, &mut seen, t0, age).await;
        assert!(sweep(&state, &mut seen, t0 + age, age).await.is_empty());
        assert!(state.srv_state.lock().await.blocks.contains_key(&background));
    }
}
