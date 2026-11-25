// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Orphan reaper — docs/specs/SPEC_AGENT_PANE_CLOSE_GRACEFUL_SHUTDOWN_2026_09_18.md §4.9.
//
// An agent block that is in a tab's `block_ids` but in no pane of that tab
// is a bug, not a state: its process keeps running and nothing on screen
// can reach it (the Manoz incident, spec §2). The close paths no longer
// produce one (#3402, #3414), but other ways remain — a create-then-place
// interrupted between its two steps, and (until windows stopped pushing
// trees) a stale whole-tree push
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
// - blocks of any other kind than an agent or a management view (a terminal,
//   an editor, a browser: they may hold running work or unsaved text). A
//   management view (the Vault, Settings, the Kanban, ...: `MANAGEMENT_VIEWS`)
//   shows only the app's own state, so it is put back in a pane (an interrupted
//   create-then-place is finished), or closed when its tab already shows a pane
//   of that view (the person's click opened one meanwhile: this one is a copy);
// - sub-blocks (their parent is a block, not a pane — they sit in a parent's
//   `subblockids`);
// - blocks already closing;
// - blocks in a tab whose layout tree is empty — "no tree yet" can't be told
//   apart from "not loaded yet" from here.
//
// Every sweep also checks each tab's pane invariants
// (SPEC_PANE_TABS_REDUCER_COMMANDS_2026_09_18.md §3.1) and logs each
// violation once, at error level, with full tab and block ids: a block of any
// kind that stays in no pane past `MIN_ORPHAN_AGE` (I1; only agents and
// management views are reaped), and whatever `validate_layout_invariants` finds in a stored tree.

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

#[cfg(any(windows, test))]
pub use crate::backend::sweep_stats::last_sweep_stats;

/// Start the periodic sweep. Called once from `main` after `AppState` exists.
pub fn install(state: &AppState) {
    let state = state.clone();
    tokio::spawn(async move {
        tokio::time::sleep(FIRST_SWEEP_DELAY).await;
        let mut watch = Watch::default();
        loop {
            let reaped = sweep(&state, &mut watch, Instant::now(), MIN_ORPHAN_AGE).await;
            if !reaped.is_empty() {
                tracing::warn!(count = reaped.len(), blocks = ?reaped, "orphan_reaper: closed unplaced agent blocks");
            }
            tokio::time::sleep(SWEEP_INTERVAL).await;
        }
    });
}

/// What sweeps carry between runs: each unplaced block's first sighting (a
/// block that got placed, or deleted, in between drops out), and the
/// violations already logged, so each is logged once while it lasts.
#[derive(Default)]
pub(crate) struct Watch {
    pub(crate) first_seen: HashMap<String, Instant>,
    reported: HashSet<String>,
}

/// One sweep. Returns the block ids it closed.
pub(crate) async fn sweep(state: &AppState, watch: &mut Watch, now: Instant, min_age: Duration) -> Vec<String> {
    let scan = scan(state).await;
    let mut still_violated = HashSet::new();
    for (tab_id, violation) in &scan.tree_violations {
        let key = format!("tree:{tab_id}:{violation}");
        if watch.reported.insert(key.clone()) {
            tracing::error!(tab_id = %tab_id, violation = %violation, "layout-doctor: stored layout tree violates an invariant");
        }
        still_violated.insert(key);
    }

    watch.first_seen.retain(|id, _| scan.unplaced.iter().any(|u| &u.block_id == id));
    let mut due = Vec::new();
    for u in &scan.unplaced {
        let seen = *watch.first_seen.entry(u.block_id.clone()).or_insert(now);
        let age = now.duration_since(seen);
        if age < min_age {
            continue;
        }
        let key = format!("unplaced:{}", u.block_id);
        still_violated.insert(key.clone());
        if watch.reported.insert(key) {
            tracing::error!(
                tab_id = %u.tab_id,
                block_id = %u.block_id,
                view = %u.view,
                unplaced_secs = age.as_secs(),
                reaped = u.is_repairable(),
                "layout-doctor: I1 violation: a block of this tab has been in no pane of it for too long"
            );
        }
        if u.is_repairable() {
            due.push((u.tab_id.clone(), u.block_id.clone(), u.view.clone(), u.is_management()));
        }
    }
    watch.reported.retain(|key| still_violated.contains(key));

    let mut reaped = Vec::new();
    // The (tab, view) pairs that have a pane: the tab's own, and each one this sweep put back.
    let mut shown = scan.shown_views;
    for (tab_id, id, view, management) in due {
        if management && shown.insert((tab_id.clone(), view.clone())) {
            tracing::warn!(tab_id = %tab_id, block_id = %id, view = %view, "layout-doctor: repair: a management view block has been in no pane for too long, putting it back in a pane");
            match crate::server::service::layout_intents::place_block(state, &tab_id, &id, crate::server::service::layout_intents::Placement::Append { focused: false }).await {
                Ok(()) => {
                    watch.first_seen.remove(&id);
                }
                Err(e) => {
                    shown.remove(&(tab_id, view));
                    tracing::warn!(block_id = %id, error = %e, "layout-doctor: repair: putting the block back failed; will retry next sweep");
                }
            }
            continue;
        }
        if management {
            tracing::warn!(tab_id = %tab_id, block_id = %id, view = %view, "layout-doctor: repair: a management view block in no pane is a copy of a pane its tab shows, closing it");
        } else {
            tracing::warn!(block_id = %id, "orphan_reaper: agent block has been in no pane for too long — closing it");
        }
        match super::close_pane::run(state, vec![id.clone()]).await {
            Ok(_) => {
                watch.first_seen.remove(&id);
                reaped.push(id);
            }
            Err(e) => tracing::warn!(block_id = %id, error = %e, "orphan_reaper: close failed; will retry next sweep"),
        }
    }

    crate::backend::sweep_stats::record(scan.unplaced.iter().filter(|u| u.is_agent()).count(), watch.first_seen.len(), reaped.len());
    reaped
}

/// A block in its tab's `block_ids` but in no pane of the tab's layout.
struct Unplaced {
    tab_id: String,
    block_id: String,
    view: String,
    /// Block meta `agentId` is set: a terminal-tier agent is a `term` block.
    has_agent_id: bool,
}

impl Unplaced {
    fn is_agent(&self) -> bool {
        is_agent_view(&self.view, self.has_agent_id)
    }

    /// One of the app's own management views, which holds nothing of the person's.
    fn is_management(&self) -> bool {
        MANAGEMENT_VIEWS.contains(&self.view.as_str())
    }

    /// What the doctor repairs once it has been in no pane for too long: an agent's block is closed, a management view's is
    /// put back in a pane (or closed when its tab shows one of that view already).
    fn is_repairable(&self) -> bool {
        self.is_agent() || self.is_management()
    }
}

/// The built-in management views (the widget bar's one-per-tab panes). They show the app's own state, so a block of one
/// that is in no pane holds no work: it is put back in a pane, or closed when it is a copy. Any other view (a terminal,
/// an editor with unsaved text, a browser) is only reported, never moved or closed.
const MANAGEMENT_VIEWS: &[&str] = &["atlas", "audit", "connectors", "drone", "help", "kanban", "memory", "settings", "swarm", "toolchain", "tower", "vault"];

/// An agent's block: the `agent` view, or a `term` block with an `agentId` in
/// its meta (a terminal-tier agent).
fn is_agent_view(view: &str, has_agent_id: bool) -> bool {
    view == "agent" || (view == "term" && has_agent_id)
}

/// Whether `block_id` is an agent's block, from its stored meta. Which panes
/// `Stop pane` refuses and which blocks `Stop agents` accepts
/// (docs/specs/SPEC_STOP_AN_AGENT_ONE_WAY_AND_SHUTDOWN_TOAST_2026_10_23.md §3.2).
pub(crate) fn is_agent_block(state: &AppState, block_id: &str) -> bool {
    let Some(block) = state.mstore.get::<Block>(block_id).ok().flatten() else {
        return false;
    };
    // A history reader names its agent but is not its pane: Stop pane may close it.
    if crate::backend::obj::is_history_reader(&block.meta) {
        return false;
    }
    let has_agent_id = !crate::backend::obj::meta_get_string(&block.meta, "agentId", "").is_empty();
    is_agent_view(&crate::backend::obj::meta_get_string(&block.meta, "view", ""), has_agent_id)
}

/// What one sweep finds: unplaced blocks (minus everything the module doc
/// lists as never reaped, whatever their kind), and each stored tree's
/// `validate_layout_invariants` findings by tab.
struct Scan {
    unplaced: Vec<Unplaced>,
    tree_violations: Vec<(String, String)>,
    /// The (tab, view) pairs of the blocks that are in a pane of their tab.
    shown_views: HashSet<(String, String)>,
}

async fn scan(state: &AppState) -> Scan {
    let (unplaced, tree_violations, placed) = {
        let s = state.srv_state.lock().await;
        let (mut unplaced, mut violations, mut placed) = (Vec::new(), Vec::new(), Vec::new());
        for tab in s.tabs.values().filter(|t| t.rootnode.is_some()) {
            let stray = crate::backend::layout::unplaced_blocks(&tab.rootnode, &tab.block_ids);
            placed.extend(tab.block_ids.iter().filter(|b| !stray.contains(b)).map(|b| (tab.tab_id.clone(), b.clone())));
            for block_id in stray {
                unplaced.push((tab.tab_id.clone(), block_id));
            }
            for v in crate::backend::layout::validate_layout_invariants(&tab.rootnode) {
                violations.push((tab.tab_id.clone(), v));
            }
        }
        (unplaced, violations, placed)
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
    let unplaced: Vec<(String, String)> = unplaced.into_iter().filter(|(_, b)| !pending.contains(b)).collect();
    if unplaced.is_empty() {
        return Scan { unplaced: Vec::new(), tree_violations, shown_views: HashSet::new() };
    }

    let blocks: Vec<Block> = state.mstore.get_all::<Block>().unwrap_or_default();
    let sub_blocks: HashSet<&String> = blocks
        .iter()
        .filter_map(|b| b.subblockids.as_ref())
        .flatten()
        .collect();
    let views: HashMap<&String, (String, bool)> = blocks
        .iter()
        .map(|b| {
            let view = crate::backend::obj::meta_get_string(&b.meta, "view", "");
            (&b.oid, (view, !crate::backend::obj::meta_get_string(&b.meta, "agentId", "").is_empty()))
        })
        .collect();
    let shown_views = placed
        .into_iter()
        .filter_map(|(tab_id, id)| views.get(&id).map(|(view, _)| (tab_id, view.clone())))
        .collect();
    let unplaced = unplaced
        .into_iter()
        .filter(|(_, id)| !sub_blocks.contains(id) && !blockcontroller::is_closing(id))
        .map(|(tab_id, block_id)| {
            let (view, has_agent_id) = views.get(&block_id).cloned().unwrap_or_default();
            Unplaced { tab_id, block_id, view, has_agent_id }
        })
        .collect();
    Scan { unplaced, tree_violations, shown_views }
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

    /// The blocks of the tab that are in no pane of its layout.
    async fn stray_blocks(state: &AppState, tab_id: &str) -> Vec<String> {
        let s = state.srv_state.lock().await;
        let tab = s.tabs.get(tab_id).unwrap();
        crate::backend::layout::unplaced_blocks(&tab.rootnode, &tab.block_ids)
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
        let mut seen = Watch::default();
        let age = Duration::from_secs(90);

        assert!(sweep(&state, &mut seen, t0, age).await.is_empty(), "first sighting: too young");
        assert!(state.srv_state.lock().await.blocks.contains_key(&orphan));

        let reaped = sweep(&state, &mut seen, t0 + age, age).await;
        assert_eq!(reaped, vec![orphan.clone()]);
        let s = state.srv_state.lock().await;
        assert!(!s.blocks.contains_key(&orphan), "orphan closed");
        assert!(s.blocks.contains_key(&placed), "placed block untouched");
    }

    /// A parked agent block is in no tab's `block_ids`, so the sweep never sees it, however long it
    /// has been unplaced (always-available agents).
    #[tokio::test]
    async fn a_parked_agent_block_is_never_reaped() {
        let state = test_state();
        let tab = seed_tab(&state).await;
        let placed = seed_block(&state, &tab, "agent").await;
        let parked = seed_block(&state, &tab, "agent").await;
        place(&state, &tab, &[&placed]).await;
        dispatch_apply(&state, Command::ParkBlock { tab_id: tab.clone(), block_id: parked.clone() }).await;

        let t0 = Instant::now();
        let mut seen = Watch::default();
        let age = Duration::from_secs(90);
        assert!(sweep(&state, &mut seen, t0, age).await.is_empty());
        assert!(sweep(&state, &mut seen, t0 + age * 10, age).await.is_empty());
        assert!(state.srv_state.lock().await.blocks.contains_key(&parked), "still there, parked");
    }

    /// `last_sweep_stats()`'s cumulative-reaped counter is process-wide
    /// (shared with every other test in this binary, since it's a plain
    /// static) and other tests in this module call `sweep` concurrently —
    /// so this only asserts the one property that's safe under that:
    /// monotonic, non-decreasing, and increases by at least what THIS
    /// sweep reaped. It deliberately does not assert an exact value for
    /// `candidates`/`first_seen`, which the last-writer-wins semantics of a
    /// shared static make inherently racy to pin down under parallel tests.
    #[tokio::test]
    async fn last_sweep_stats_reaped_cumulative_is_monotonic_across_a_real_reap() {
        let state = test_state();
        let tab = seed_tab(&state).await;
        // A placed block is required for the tab to have a real layout
        // tree at all — an empty tree is indistinguishable from "not
        // loaded yet" and the whole tab is skipped (see
        // `a_tab_with_no_layout_tree_is_skipped`), so an unplaced block on
        // its own would never even become a candidate.
        let placed = seed_block(&state, &tab, "agent").await;
        let orphan = seed_block(&state, &tab, "agent").await;
        place(&state, &tab, &[&placed]).await;

        let (_, _, before) = last_sweep_stats();
        let t0 = Instant::now();
        let mut seen = Watch::default();
        let age = Duration::from_secs(90);
        sweep(&state, &mut seen, t0, age).await; // first sighting, not yet due
        let reaped = sweep(&state, &mut seen, t0 + age, age).await;
        assert_eq!(reaped, vec![orphan]);

        let (_, _, after) = last_sweep_stats();
        assert!(
            after >= before + 1,
            "cumulative reaped count must have grown by at least the one block this sweep reaped: before={before}, after={after}"
        );
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
        let mut seen = Watch::default();
        let age = Duration::from_secs(90);
        sweep(&state, &mut seen, t0, age).await;
        assert!(seen.first_seen.contains_key(&fresh));

        place(&state, &tab, &[&a, &fresh]).await;
        assert!(sweep(&state, &mut seen, t0 + age, age).await.is_empty());
        assert!(!seen.first_seen.contains_key(&fresh), "placed → dropped from the watch list");
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
        let mut seen = Watch::default();
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
        let mut seen = Watch::default();
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
        let mut seen = Watch::default();
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
        let mut seen = Watch::default();
        let age = Duration::from_secs(90);
        sweep(&state, &mut seen, t0, age).await;
        assert!(sweep(&state, &mut seen, t0 + age, age).await.is_empty());
        assert!(state.srv_state.lock().await.blocks.contains_key(&background));
    }

    /// A non-agent block left in no pane is reported once (I1), never
    /// reaped, and forgotten once it is placed again.
    #[tokio::test]
    async fn an_unplaced_non_agent_block_is_reported_once_and_left_running() {
        let state = test_state();
        let tab = seed_tab(&state).await;
        let placed = seed_block(&state, &tab, "agent").await;
        let term = seed_block(&state, &tab, "term").await;
        place(&state, &tab, &[&placed]).await;

        let t0 = Instant::now();
        let mut seen = Watch::default();
        let age = Duration::from_secs(90);
        sweep(&state, &mut seen, t0, age).await;
        let key = format!("unplaced:{term}");
        assert!(!seen.reported.contains(&key), "too young to report");
        assert!(sweep(&state, &mut seen, t0 + age, age).await.is_empty());
        assert!(seen.reported.contains(&key));
        assert!(state.srv_state.lock().await.blocks.contains_key(&term), "only agents are reaped");

        place(&state, &tab, &[&placed, &term]).await;
        sweep(&state, &mut seen, t0 + age + age, age).await;
        assert!(!seen.reported.contains(&key), "placed again: no longer a violation");
    }

    /// A management view (the Vault, Settings, the Kanban, ...) holds nothing of the person's: left in no pane it is put
    /// back in one after the minimum age. A terminal or an editor in the same state is only reported: it may hold running
    /// work or unsaved text, and it is not moved.
    #[tokio::test]
    async fn an_unplaced_management_view_block_is_put_back_in_a_pane_after_the_minimum_age_and_work_is_left() {
        let state = test_state();
        let tab = seed_tab(&state).await;
        let placed = seed_block(&state, &tab, "agent").await;
        let vault = seed_block(&state, &tab, "vault").await;
        let settings = seed_block(&state, &tab, "settings").await;
        let term = seed_block(&state, &tab, "term").await;
        let editor = seed_block(&state, &tab, "editor").await;
        place(&state, &tab, &[&placed]).await;

        let t0 = Instant::now();
        let mut seen = Watch::default();
        let age = Duration::from_secs(90);
        assert!(sweep(&state, &mut seen, t0, age).await.is_empty(), "first sighting: too young");
        assert_eq!(stray_blocks(&state, &tab).await.len(), 4, "too young: nothing moved");
        assert!(sweep(&state, &mut seen, t0 + age, age).await.is_empty(), "nothing is closed");
        let mut stray = stray_blocks(&state, &tab).await;
        stray.sort();
        let mut expected = vec![term.clone(), editor.clone()];
        expected.sort();
        assert_eq!(stray, expected, "the two management views have a pane; work is left where it was");
        let s = state.srv_state.lock().await;
        assert!(s.blocks.contains_key(&vault) && s.blocks.contains_key(&settings) && s.blocks.contains_key(&term) && s.blocks.contains_key(&editor));
    }

    /// The person's click opened a new Vault pane while the old block was in no pane: the old block is a copy, so it is
    /// closed, and the tab keeps its one pane of the view. Two strays of one view leave one pane and one closed copy.
    #[tokio::test]
    async fn a_management_view_block_in_no_pane_is_closed_when_its_tab_already_shows_one_of_that_view() {
        let state = test_state();
        let tab = seed_tab(&state).await;
        let shown = seed_block(&state, &tab, "vault").await;
        let stray = seed_block(&state, &tab, "vault").await;
        let twin_a = seed_block(&state, &tab, "settings").await;
        let twin_b = seed_block(&state, &tab, "settings").await;
        place(&state, &tab, &[&shown]).await;

        let t0 = Instant::now();
        let mut seen = Watch::default();
        let age = Duration::from_secs(90);
        sweep(&state, &mut seen, t0, age).await;
        let closed = sweep(&state, &mut seen, t0 + age, age).await;
        assert_eq!(closed.len(), 2, "the Vault copy and one of the two Settings: {closed:?}");
        assert!(closed.contains(&stray));
        let s = state.srv_state.lock().await;
        assert!(s.blocks.contains_key(&shown));
        assert_eq!([&twin_a, &twin_b].iter().filter(|b| s.blocks.contains_key(**b)).count(), 1, "one Settings block is kept");
        drop(s);
        assert!(stray_blocks(&state, &tab).await.is_empty(), "the kept Settings block has its pane");
    }

    /// A stored tree that breaks an invariant (here I1's other half: one
    /// block in two panes) is reported with its tab, once while it lasts.
    #[tokio::test]
    async fn a_block_in_two_panes_is_reported_with_its_tab() {
        let state = test_state();
        let tab = seed_tab(&state).await;
        let a = seed_block(&state, &tab, "term").await;
        let pane = |id: &str| agentmux_common::LayoutNode {
            id: id.into(),
            size: 1.0,
            data: Some(agentmux_common::LayoutNodeData { block_id: a.clone(), ..Default::default() }),
            ..Default::default()
        };
        let tree = agentmux_common::LayoutNode { id: "root".into(), children: vec![pane("p1"), pane("p2")], ..Default::default() };
        let set = Command::LayoutSetTree { tab_id: tab.clone(), new_tree: Some(tree), correlation_id: String::new(), slices: None };
        dispatch_apply(&state, set).await;

        let mut seen = Watch::default();
        sweep(&state, &mut seen, Instant::now(), Duration::from_secs(90)).await;
        let prefix = format!("tree:{tab}:BLOCK_IN_TWO_PANES");
        assert!(seen.reported.iter().any(|k| k.starts_with(&prefix)), "{:?}", seen.reported);
    }

    #[test]
    fn only_the_management_views_are_repaired_besides_agents() {
        let u = |view: &str| super::Unplaced { tab_id: "t".into(), block_id: "b".into(), view: view.into(), has_agent_id: false };
        for view in ["vault", "settings", "connectors", "kanban", "memory", "swarm", "audit", "atlas", "drone", "help", "toolchain", "tower"] {
            assert!(u(view).is_repairable(), "{view}");
        }
        for view in ["term", "editor", "preview", "browser", "files", "agent", "sysinfo", ""] {
            assert!(!u(view).is_management(), "{view}");
        }
    }

    #[test]
    fn a_term_block_with_an_agent_is_reaped_like_an_agent() {
        let u = |view: &str, has_agent_id: bool| super::Unplaced {
            tab_id: "t".into(),
            block_id: "b".into(),
            view: view.into(),
            has_agent_id,
        };
        assert!(u("agent", false).is_agent());
        // A terminal-tier agent (Aider, Crush, Muse Code) is a `term` block.
        assert!(u("term", true).is_agent());
        assert!(!u("term", false).is_agent());
        assert!(!u("preview", true).is_agent());
    }
}
