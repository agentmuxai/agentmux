// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// `pane_open_reducer_tests`, moved out of server/app_api/mod.rs unchanged
// (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4, item 9).

use super::*;
use crate::backend::rpc_types::CommandPaneOpenData;
use crate::server::tests::test_state;
use agentmux_common::ipc::{Command, Event};

async fn dispatch_apply(state: &AppState, cmd: Command) -> Vec<Event> {
    let evs = crate::server::service::dispatch_to_reducer(state, cmd).await;
    for ev in &evs {
        crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore).unwrap();
    }
    evs
}

/// Regression for #1681: the docked `pane.open` path created its block
/// store-only (`wcore::create_block`), so the block was absent from the
/// reducer's `state.blocks` and a later TearOffBlock / RedockFloatingPane
/// was rejected "block not found". Assert the block now lands in `srv_state`
/// and a tear-off of the freshly-opened pane succeeds end-to-end.
#[tokio::test]
async fn docked_pane_open_block_is_in_reducer_and_tears_off() {
    let state = test_state();

    // Workspace + tab through the reducer (→ srv_state AND, via apply, mstore).
    let ws_evs = dispatch_apply(&state, Command::CreateWorkspace { name: "w".into() }).await;
    let ws_id = ws_evs
        .iter()
        .find_map(|e| match e {
            Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
            _ => None,
        })
        .unwrap();
    let tab_evs = dispatch_apply(
        &state,
        Command::CreateTab { workspace_id: ws_id.clone(), name: "t".into() },
    )
    .await;
    let tab_id = tab_evs
        .iter()
        .find_map(|e| match e {
            Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
            _ => None,
        })
        .unwrap();

    // Open a docked sysinfo pane (no required args).
    let cmd = CommandPaneOpenData {
        view: "sysinfo".into(),
        file: None,
        url: None,
        cwd: None,
        title: None,
        tab_id: Some(tab_id.clone()),
        split_direction: None,
        split_reference_block_id: None,
        focus: None,
        tree_expanded: None,
        floating: None,
        meta: None,
        skip_placement: None,
        stack_onto_block_id: None,
        reuse_editor_pane: None,
        select: None,
        line: None,
        connection: None,
        auth: None,
    };
    let res = open_pane(&state, cmd).await.expect("open_pane docked");

    // The block is now visible to the reducer (was the bug: store-only).
    {
        let s = state.srv_state.lock().await;
        assert!(
            s.blocks.contains_key(&res.block_id),
            "docked pane.open block must be tracked in srv_state"
        );
    }

    // And tearing it off no longer hits "block not found".
    let r = crate::sagas::tear_off_block::run(
        &state,
        res.block_id.clone(),
        tab_id.clone(),
        ws_id.clone(),
    )
    .await;
    assert!(r.is_ok(), "tear-off of an opened pane must succeed, got: {:?}", r.err());
}

/// In-pane tabs (SPEC_PANE_TAB_STRIP_AGENT_TERMINAL_2026_07_20.md §4.2):
/// `skip_placement` must create a reducer-tracked block (same as the
/// docked path) but leave the tab's layout tree completely untouched —
/// the caller is about to attach it to an existing leaf's block-stack,
/// not give it its own tile.
#[tokio::test]
async fn stack_onto_block_id_creates_the_block_as_a_tab_of_that_pane() {
    // SPEC_PANE_TABS_REDUCER_COMMANDS_2026_09_18.md §3.3: created and
    // placed in one step, plus a queued `stackpush` for the frontend.
    let state = test_state();
    let ws_id = dispatch_apply(&state, Command::CreateWorkspace { name: "w".into() })
        .await
        .iter()
        .find_map(|e| match e {
            Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
            _ => None,
        })
        .unwrap();
    let tab_id = dispatch_apply(&state, Command::CreateTab { workspace_id: ws_id, name: "t".into() })
        .await
        .iter()
        .find_map(|e| match e {
            Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
            _ => None,
        })
        .unwrap();
    let anchor = dispatch_apply(
        &state,
        Command::CreateBlock { tab_id: tab_id.clone(), meta: serde_json::json!({ "view": "agent" }) },
    )
    .await
    .iter()
    .find_map(|e| match e {
        Event::BlockCreated { block_id, .. } => Some(block_id.clone()),
        _ => None,
    })
    .unwrap();
    dispatch_apply(
        &state,
        Command::LayoutSetTree {
            tab_id: tab_id.clone(),
            new_tree: Some(agentmux_common::LayoutNode {
                id: "pane".into(),
                data: Some(agentmux_common::LayoutNodeData { block_id: anchor.clone(), ..Default::default() }),
                ..Default::default()
            }),
            correlation_id: String::new(),
            slices: None,
        },
    )
    .await;

    let cmd = CommandPaneOpenData {
        view: "agent".into(),
        file: None,
        url: None,
        cwd: None,
        title: None,
        tab_id: None,
        split_direction: None,
        split_reference_block_id: None,
        focus: None,
        tree_expanded: None,
        floating: None,
        meta: Some({
            let mut m = MetaMapType::new();
            m.insert("view".to_string(), serde_json::json!("agent"));
            m
        }),
        skip_placement: None,
        stack_onto_block_id: Some(anchor.clone()),
        reuse_editor_pane: None,
        select: None,
        line: None,
        connection: None,
        auth: None,
    };
    let res = open_pane(&state, cmd).await.expect("open_pane stack_onto_block_id");
    assert!(res.created);
    assert_eq!(res.tab_id, tab_id, "tab derived from the target block");

    {
        let s = state.srv_state.lock().await;
        assert!(s.tabs[&tab_id].block_ids.contains(&res.block_id));
        let data = s.tabs[&tab_id].rootnode.as_ref().unwrap().data.clone().unwrap();
        assert_eq!(data.block_stack, vec![anchor.clone(), res.block_id.clone()]);
        assert_eq!(data.block_id, res.block_id, "the new tab is visible");
    }
    let tab = state.mstore.must_get::<crate::backend::obj::Tab>(&tab_id).unwrap();
    let layout = state.mstore.must_get::<crate::backend::obj::LayoutState>(&tab.layoutstate).unwrap();
    let actions = layout.pendingbackendactions.unwrap_or_default();
    assert!(
        actions.iter().any(|a| a.actiontype == "stackpush" && a.blockid == res.block_id && a.targetblockid == anchor),
        "frontend told via a queued stackpush: {actions:?}"
    );
    // The persisted tree agrees with the reducer (single writer).
    let persisted = layout.rootnode.expect("tree persisted");
    assert_eq!(persisted.data.unwrap().block_stack, vec![anchor, res.block_id]);
}

// ── pane.moveTab (SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md §4.1, Phase 3) ──

async fn seed_stacked_pane(state: &AppState, stack: &[&str]) -> (String, Vec<String>) {
    let ws_id = dispatch_apply(state, Command::CreateWorkspace { name: "w".into() })
        .await
        .iter()
        .find_map(|e| match e {
            Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
            _ => None,
        })
        .unwrap();
    let tab_id = dispatch_apply(state, Command::CreateTab { workspace_id: ws_id, name: "t".into() })
        .await
        .iter()
        .find_map(|e| match e {
            Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
            _ => None,
        })
        .unwrap();
    let mut block_ids = Vec::new();
    for _ in stack {
        let id = dispatch_apply(state, Command::CreateBlock { tab_id: tab_id.clone(), meta: serde_json::Value::Null })
            .await
            .iter()
            .find_map(|e| match e {
                Event::BlockCreated { block_id, .. } => Some(block_id.clone()),
                _ => None,
            })
            .unwrap();
        block_ids.push(id);
    }
    dispatch_apply(
        state,
        Command::LayoutSetTree {
            tab_id: tab_id.clone(),
            new_tree: Some(agentmux_common::LayoutNode {
                id: "pane".into(),
                data: Some(agentmux_common::LayoutNodeData {
                    block_id: block_ids[0].clone(),
                    block_stack: block_ids.clone(),
                    active_block_id: block_ids[0].clone(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            correlation_id: String::new(),
            slices: None,
        },
    )
    .await;
    (tab_id, block_ids)
}

#[tokio::test]
async fn move_tab_reorders_within_the_same_pane_and_queues_a_stackmove() {
    let state = test_state();
    let (tab_id, blocks) = seed_stacked_pane(&state, &["a", "b", "c"]).await;
    let (a, b, c) = (blocks[0].clone(), blocks[1].clone(), blocks[2].clone());

    move_tab(
        &state,
        CommandPaneMoveTabData {
            block_id: c.clone(),
            target_block_id: b.clone(),
            position: "before".into(),
            activate: false,
        },
    )
    .await
    .expect("move_tab reorder");

    let s = state.srv_state.lock().await;
    let data = s.tabs[&tab_id].rootnode.as_ref().unwrap().data.clone().unwrap();
    assert_eq!(data.block_stack, vec![a, c.clone(), b]);
    drop(s);

    let tab = state.mstore.must_get::<crate::backend::obj::Tab>(&tab_id).unwrap();
    let layout = state.mstore.must_get::<crate::backend::obj::LayoutState>(&tab.layoutstate).unwrap();
    let actions = layout.pendingbackendactions.unwrap_or_default();
    assert!(
        actions.iter().any(|a| a.actiontype == "stackmove"
            && a.blockid == c
            && a.targetblockid == blocks[1]
            && a.position == "before"
            && !a.focused), // activate: false rides on `focused` — see queue_target_stack_move's doc comment
        "frontend told via a queued stackmove: {actions:?}"
    );
}

// Phase 4 (SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md §3.4): a cross-pane
// target is now accepted — Codex P2 on PR #3444 had this rejected
// through Phase 3, until the frontend's pending-action handler could
// correctly mirror a cross-leaf "stackmove" to other windows/tabs
// (layoutPersistence.ts, via moveMemberAcrossStacks). That gap is
// closed; this test replaces the old rejection test.
#[tokio::test]
async fn move_tab_moves_a_tab_across_panes_and_queues_a_cross_pane_stackmove() {
    let state = test_state();
    let ws_id = dispatch_apply(&state, Command::CreateWorkspace { name: "w".into() })
        .await
        .iter()
        .find_map(|e| match e {
            Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
            _ => None,
        })
        .unwrap();
    let tab_id = dispatch_apply(&state, Command::CreateTab { workspace_id: ws_id, name: "t".into() })
        .await
        .iter()
        .find_map(|e| match e {
            Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
            _ => None,
        })
        .unwrap();
    let mut ids = Vec::new();
    for _ in 0..3 {
        let id = dispatch_apply(&state, Command::CreateBlock { tab_id: tab_id.clone(), meta: serde_json::Value::Null })
            .await
            .iter()
            .find_map(|e| match e {
                Event::BlockCreated { block_id, .. } => Some(block_id.clone()),
                _ => None,
            })
            .unwrap();
        ids.push(id);
    }
    // Pane A is a real 2-member stack (so the source leaf survives the
    // move — a solo-member source, whose pane is removed, is covered by
    // the dedicated only-tab test below).
    let (a1, a2, b) = (ids[0].clone(), ids[1].clone(), ids[2].clone());
    dispatch_apply(
        &state,
        Command::LayoutSetTree {
            tab_id: tab_id.clone(),
            new_tree: Some(agentmux_common::LayoutNode {
                id: "root".into(),
                children: vec![
                    agentmux_common::LayoutNode {
                        id: "pane-a".into(),
                        data: Some(agentmux_common::LayoutNodeData {
                            block_id: a1.clone(),
                            block_stack: vec![a1.clone(), a2.clone()],
                            active_block_id: a1.clone(),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    agentmux_common::LayoutNode {
                        id: "pane-b".into(),
                        data: Some(agentmux_common::LayoutNodeData { block_id: b.clone(), ..Default::default() }),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }),
            correlation_id: String::new(),
            slices: None,
        },
    )
    .await;

    move_tab(
        &state,
        CommandPaneMoveTabData {
            block_id: a2.clone(),
            target_block_id: b.clone(),
            position: "end".into(),
            activate: true,
        },
    )
    .await
    .expect("cross-pane move_tab");

    let tree = state.srv_state.lock().await.tabs[&tab_id].rootnode.clone().unwrap();
    let pane_a = tree.children[0].data.as_ref().unwrap();
    assert_eq!(pane_a.block_stack, vec![a1], "removed from the source pane");
    let pane_b = tree.children[1].data.as_ref().unwrap();
    assert_eq!(pane_b.block_stack, vec![b.clone(), a2.clone()]);
    assert_eq!((pane_b.block_id.as_str(), pane_b.active_block_id.as_str()), (a2.as_str(), a2.as_str()));

    let tab = state.mstore.must_get::<crate::backend::obj::Tab>(&tab_id).unwrap();
    let layout = state.mstore.must_get::<crate::backend::obj::LayoutState>(&tab.layoutstate).unwrap();
    let actions = layout.pendingbackendactions.unwrap_or_default();
    assert!(
        actions.iter().any(|a| a.actiontype == "stackmove" && a.blockid == a2 && a.targetblockid == b && a.focused),
        "frontend told via a queued cross-pane stackmove: {actions:?}"
    );
}

#[tokio::test]
async fn move_tab_of_a_panes_only_tab_moves_it_and_removes_the_emptied_pane() {
    // Both panes live in the SAME tab (LayoutStackMove operates within
    // one tab's tree) — a solo-block leaf and a two-member stacked leaf
    // as siblings under one root. Moving the solo pane's only tab empties
    // it, so the pane goes and the block lives on in the destination.
    // SPEC_PANE_TAB_DRAG_LANDING_FLASH_AND_LAST_TAB_CLOSE_2026_09_24.md §4.
    let state = test_state();
    let ws_id = dispatch_apply(&state, Command::CreateWorkspace { name: "w".into() })
        .await
        .iter()
        .find_map(|e| match e {
            Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
            _ => None,
        })
        .unwrap();
    let tab_id = dispatch_apply(&state, Command::CreateTab { workspace_id: ws_id, name: "t".into() })
        .await
        .iter()
        .find_map(|e| match e {
            Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
            _ => None,
        })
        .unwrap();
    let mut ids = Vec::new();
    for _ in 0..3 {
        let id = dispatch_apply(&state, Command::CreateBlock { tab_id: tab_id.clone(), meta: serde_json::Value::Null })
            .await
            .iter()
            .find_map(|e| match e {
                Event::BlockCreated { block_id, .. } => Some(block_id.clone()),
                _ => None,
            })
            .unwrap();
        ids.push(id);
    }
    let (solo, x, y) = (ids[0].clone(), ids[1].clone(), ids[2].clone());
    dispatch_apply(
        &state,
        Command::LayoutSetTree {
            tab_id: tab_id.clone(),
            new_tree: Some(agentmux_common::LayoutNode {
                id: "root".into(),
                children: vec![
                    agentmux_common::LayoutNode {
                        id: "solo-pane".into(),
                        data: Some(agentmux_common::LayoutNodeData { block_id: solo.clone(), ..Default::default() }),
                        ..Default::default()
                    },
                    agentmux_common::LayoutNode {
                        id: "dst-pane".into(),
                        data: Some(agentmux_common::LayoutNodeData {
                            block_id: x.clone(),
                            block_stack: vec![x.clone(), y.clone()],
                            active_block_id: x.clone(),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }),
            correlation_id: String::new(),
            slices: None,
        },
    )
    .await;

    move_tab(
        &state,
        CommandPaneMoveTabData {
            block_id: solo.clone(),
            target_block_id: y.clone(),
            position: "end".into(),
            activate: true,
        },
    )
    .await
    .expect("moving a pane's only tab into another pane succeeds");

    let (tree, block_survives) = {
        let srv = state.srv_state.lock().await;
        (srv.tabs[&tab_id].rootnode.clone().unwrap(), srv.blocks.contains_key(&solo))
    };
    // The emptied pane is gone and the root collapsed onto the only
    // remaining leaf (delete_node's single-child collapse).
    assert!(tree.children.is_empty(), "one pane left: {tree:?}");
    let dst = tree.data.as_ref().expect("root is now the destination leaf");
    assert_eq!(dst.block_stack, vec![x.clone(), y.clone(), solo.clone()]);
    assert_eq!((dst.block_id.as_str(), dst.active_block_id.as_str()), (solo.as_str(), solo.as_str()));
    assert!(block_survives, "a move, not a close: the block itself must not be deleted");

    let tab = state.mstore.must_get::<crate::backend::obj::Tab>(&tab_id).unwrap();
    let layout = state.mstore.must_get::<crate::backend::obj::LayoutState>(&tab.layoutstate).unwrap();
    assert_eq!(layout.rootnode.as_ref(), Some(&tree), "db_layout persisted the post-move tree");
    let actions = layout.pendingbackendactions.unwrap_or_default();
    assert!(
        actions.iter().any(|a| a.actiontype == "stackmove" && a.blockid == solo && a.targetblockid == y),
        "other windows told via a queued stackmove: {actions:?}"
    );
}

#[tokio::test]
async fn skip_placement_creates_block_without_touching_the_layout_tree() {
    let state = test_state();

    let ws_evs = dispatch_apply(&state, Command::CreateWorkspace { name: "w".into() }).await;
    let ws_id = ws_evs
        .iter()
        .find_map(|e| match e {
            Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
            _ => None,
        })
        .unwrap();
    let tab_evs = dispatch_apply(
        &state,
        Command::CreateTab { workspace_id: ws_id.clone(), name: "t".into() },
    )
    .await;
    let tab_id = tab_evs
        .iter()
        .find_map(|e| match e {
            Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
            _ => None,
        })
        .unwrap();

    // A fresh tab has no layout tree yet — confirm that baseline before
    // asserting skip_placement doesn't add one.
    {
        let s = state.srv_state.lock().await;
        assert!(s.tabs.get(&tab_id).unwrap().rootnode.is_none());
    }

    let cmd = CommandPaneOpenData {
        view: "agent".into(),
        file: None,
        url: None,
        cwd: None,
        title: None,
        tab_id: Some(tab_id.clone()),
        split_direction: None,
        split_reference_block_id: None,
        focus: None,
        tree_expanded: None,
        floating: None,
        meta: Some({
            let mut m = MetaMapType::new();
            m.insert("view".to_string(), serde_json::json!("agent"));
            m
        }),
        skip_placement: Some(true),
        stack_onto_block_id: None,
        reuse_editor_pane: None,
        select: None,
        line: None,
        connection: None,
        auth: None,
    };
    let res = open_pane(&state, cmd).await.expect("open_pane skip_placement");
    assert!(res.created);

    let s = state.srv_state.lock().await;
    assert!(
        s.blocks.contains_key(&res.block_id),
        "skip_placement block must still be tracked in srv_state"
    );
    assert!(
        s.tabs.get(&tab_id).unwrap().rootnode.is_none(),
        "skip_placement must not place the block into the tab's layout tree"
    );
}

fn editor_open_cmd(
    tab_id: Option<String>,
    file: &str,
    split_reference_block_id: Option<String>,
    reuse_editor_pane: Option<bool>,
) -> CommandPaneOpenData {
    CommandPaneOpenData {
        view: "editor".into(),
        file: Some(file.to_string()),
        url: None,
        cwd: None,
        title: None,
        tab_id,
        split_direction: None,
        split_reference_block_id,
        focus: None,
        tree_expanded: None,
        floating: None,
        meta: None,
        skip_placement: None,
        stack_onto_block_id: None,
        reuse_editor_pane,
        select: None,
        line: None,
        connection: None,
        auth: None,
    }
}

/// No existing Editor pane in the caller's tab → today's unchanged
/// behavior: a new block is created (SPEC_EDITOR_MCP_OPEN_BLANK_PREVIEW_AND_PANE_REUSE_2026_08_03.md
/// Part 2's fall-through path).
#[tokio::test]
async fn open_editor_creates_new_pane_when_none_exists_in_tab() {
    let state = test_state();

    let ws_evs = dispatch_apply(&state, Command::CreateWorkspace { name: "w".into() }).await;
    let ws_id = ws_evs
        .iter()
        .find_map(|e| match e {
            Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
            _ => None,
        })
        .unwrap();
    let tab_evs = dispatch_apply(
        &state,
        Command::CreateTab { workspace_id: ws_id.clone(), name: "t".into() },
    )
    .await;
    let tab_id = tab_evs
        .iter()
        .find_map(|e| match e {
            Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
            _ => None,
        })
        .unwrap();

    // A "caller" pane in the tab (stands in for the calling agent's own
    // block) — but no Editor pane exists yet, so reuse must not trigger.
    let caller = open_pane(&state, {
        let mut cmd = editor_open_cmd(Some(tab_id.clone()), "/tmp/unused.txt", None, None);
        cmd.view = "sysinfo".into();
        cmd.file = None;
        cmd
    })
    .await
    .expect("open_pane caller");

    // tab_id passed explicitly (matching this file's other tests) —
    // resolve_tab_id's separate "first workspace" fallback is exercised
    // by test_state()'s own seeded default workspace/tab and isn't part
    // of what this test is checking.
    let cmd = editor_open_cmd(Some(tab_id), "/tmp/a.md", Some(caller.block_id.clone()), Some(true));
    let res = open_pane(&state, cmd).await.expect("open_pane editor");
    assert!(res.created, "must create a new Editor pane when none exists in the tab");
    assert_ne!(res.block_id, caller.block_id);
}

/// Regression for reagent P2 on PR #2404: an explicit `collapse_tree`
/// request (`tree_expanded: Some(false)`) has no live mechanism to apply
/// to an already-mounted pane's tree state — bypass reuse for it
/// entirely rather than silently ignoring the request.
#[tokio::test]
async fn open_editor_bypasses_reuse_when_tree_expanded_requested() {
    let state = test_state();

    let ws_evs = dispatch_apply(&state, Command::CreateWorkspace { name: "w".into() }).await;
    let ws_id = ws_evs
        .iter()
        .find_map(|e| match e {
            Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
            _ => None,
        })
        .unwrap();
    let tab_evs = dispatch_apply(
        &state,
        Command::CreateTab { workspace_id: ws_id.clone(), name: "t".into() },
    )
    .await;
    let tab_id = tab_evs
        .iter()
        .find_map(|e| match e {
            Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
            _ => None,
        })
        .unwrap();

    let caller = open_pane(&state, {
        let mut cmd = editor_open_cmd(Some(tab_id.clone()), "/tmp/unused.txt", None, None);
        cmd.view = "sysinfo".into();
        cmd.file = None;
        cmd
    })
    .await
    .expect("open_pane caller");

    let existing_editor = open_pane(
        &state,
        editor_open_cmd(Some(tab_id.clone()), "/tmp/existing.md", None, None),
    )
    .await
    .expect("open_pane existing editor");

    let mut cmd = editor_open_cmd(None, "/tmp/collapsed.md", Some(caller.block_id.clone()), Some(true));
    cmd.tree_expanded = Some(false);
    let res = open_pane(&state, cmd).await.expect("open_pane collapse_tree request");
    assert_ne!(
        res.block_id, existing_editor.block_id,
        "an explicit tree_expanded request must bypass reuse and create its own pane"
    );
}

/// An Editor pane already open in the caller's own tab → reused (file
/// appended to META_PENDING_OPEN_FILES for the pane to drain) instead of
/// spawning a second Editor pane.
#[tokio::test]
async fn open_editor_reuses_existing_pane_in_callers_tab() {
    let state = test_state();

    let ws_evs = dispatch_apply(&state, Command::CreateWorkspace { name: "w".into() }).await;
    let ws_id = ws_evs
        .iter()
        .find_map(|e| match e {
            Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
            _ => None,
        })
        .unwrap();
    let tab_evs = dispatch_apply(
        &state,
        Command::CreateTab { workspace_id: ws_id.clone(), name: "t".into() },
    )
    .await;
    let tab_id = tab_evs
        .iter()
        .find_map(|e| match e {
            Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
            _ => None,
        })
        .unwrap();

    // Caller's own pane (e.g. the agent pane that will call OpenEditor).
    let caller = open_pane(&state, {
        let mut cmd = editor_open_cmd(Some(tab_id.clone()), "/tmp/unused.txt", None, None);
        cmd.view = "sysinfo".into();
        cmd.file = None;
        cmd
    })
    .await
    .expect("open_pane caller");

    // An Editor pane already open in the same tab.
    let first_editor = open_pane(
        &state,
        editor_open_cmd(Some(tab_id.clone()), "/tmp/first.md", None, None),
    )
    .await
    .expect("open_pane first editor");
    assert!(first_editor.created);

    // A second OpenEditor call from the caller, same tab, different file
    // — must reuse the existing Editor pane, not create a second one.
    let reused = open_pane(
        &state,
        editor_open_cmd(None, "/tmp/second.md", Some(caller.block_id.clone()), Some(true)),
    )
    .await
    .expect("open_pane reused editor");
    assert!(!reused.created, "must reuse the existing Editor pane, not create a second one");
    assert_eq!(reused.block_id, first_editor.block_id);
    assert_eq!(reused.tab_id, tab_id);
}

/// An Editor on an SSH host is never reused for a file on this computer (or
/// the other way): its reads and saves go to its host, so the file would be
/// read there (#4294). The same host is reused as before.
#[tokio::test]
async fn open_editor_reuses_only_an_editor_on_the_same_connection() {
    let state = test_state();
    let ws_id = dispatch_apply(&state, Command::CreateWorkspace { name: "w".into() })
        .await
        .iter()
        .find_map(|e| match e {
            Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
            _ => None,
        })
        .unwrap();
    let tab_id = dispatch_apply(&state, Command::CreateTab { workspace_id: ws_id, name: "t".into() })
        .await
        .iter()
        .find_map(|e| match e {
            Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
            _ => None,
        })
        .unwrap();
    let caller = open_pane(&state, {
        let mut cmd = editor_open_cmd(Some(tab_id.clone()), "/tmp/unused.txt", None, None);
        cmd.view = "sysinfo".into();
        cmd.file = None;
        cmd
    })
    .await
    .expect("open_pane caller");
    let on_host = open_pane(&state, {
        let mut cmd = editor_open_cmd(Some(tab_id.clone()), "/home/u/a.md", None, None);
        cmd.connection = Some("user@box".into());
        cmd
    })
    .await
    .expect("open_pane host editor");

    // A local file: a new Editor, not the host's.
    let local = open_pane(
        &state,
        editor_open_cmd(None, "/tmp/local.md", Some(caller.block_id.clone()), Some(true)),
    )
    .await
    .expect("open_pane local editor");
    assert!(local.created, "a host's editor must not take a local file");
    assert_ne!(local.block_id, on_host.block_id);

    // Another file on the same host: that editor, however the host is spelled.
    let same_host = open_pane(&state, {
        let mut cmd = editor_open_cmd(None, "/home/u/b.md", Some(caller.block_id.clone()), Some(true));
        cmd.connection = Some(" user@box ".into());
        cmd
    })
    .await
    .expect("open_pane same host");
    assert!(!same_host.created);
    assert_eq!(same_host.block_id, on_host.block_id);
}

/// Regression for codex P1 on PR #2404: 2+ reuse calls before the target
/// pane's frontend ever drains its pending-files meta must all survive,
/// not overwrite each other down to just the last one.
#[tokio::test]
async fn open_editor_reuse_queues_multiple_pending_files() {
    let state = test_state();

    let ws_evs = dispatch_apply(&state, Command::CreateWorkspace { name: "w".into() }).await;
    let ws_id = ws_evs
        .iter()
        .find_map(|e| match e {
            Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
            _ => None,
        })
        .unwrap();
    let tab_evs = dispatch_apply(
        &state,
        Command::CreateTab { workspace_id: ws_id.clone(), name: "t".into() },
    )
    .await;
    let tab_id = tab_evs
        .iter()
        .find_map(|e| match e {
            Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
            _ => None,
        })
        .unwrap();

    let caller = open_pane(&state, {
        let mut cmd = editor_open_cmd(Some(tab_id.clone()), "/tmp/unused.txt", None, None);
        cmd.view = "sysinfo".into();
        cmd.file = None;
        cmd
    })
    .await
    .expect("open_pane caller");

    let first_editor = open_pane(
        &state,
        editor_open_cmd(Some(tab_id.clone()), "/tmp/first.md", None, None),
    )
    .await
    .expect("open_pane first editor");

    // Three back-to-back reuse calls, none of which drain the queue
    // (no frontend attached in this test) — all three must still be
    // present afterward, not just the last one.
    for path in ["/tmp/a.md", "/tmp/b.md", "/tmp/c.md"] {
        let res = open_pane(
            &state,
            editor_open_cmd(None, path, Some(caller.block_id.clone()), Some(true)),
        )
        .await
        .expect("open_pane reuse");
        assert!(!res.created);
        assert_eq!(res.block_id, first_editor.block_id);
    }

    let block: Block = state.mstore.must_get(&first_editor.block_id).unwrap();
    let pending = block
        .meta
        .get("editor:pending_open_files")
        .and_then(|v| v.as_array())
        .expect("pending_open_files must be an array");
    let paths: Vec<&str> = pending.iter().filter_map(|v| v.as_str()).collect();
    assert_eq!(
        paths,
        vec!["/tmp/a.md", "/tmp/b.md", "/tmp/c.md"],
        "all three stacked reuse requests must survive, in order, not just the last one"
    );
}

/// Regression for reagent P1 on PR #2404: `EditorViewModel.openToTheSide`/
/// `openInTerminal` (in `frontend/app/view/editor/editor-model.ts`)
/// call the same generic `pane.open` RPC with `split_reference_block_id`
/// set to their OWN block id, for split placement only — never setting
/// `reuse_editor_pane`. Reuse must not trigger for them, or "Open to the
/// Side" would silently redirect into the calling pane itself instead of
/// creating the requested second pane.
#[tokio::test]
async fn open_editor_does_not_reuse_without_explicit_opt_in() {
    let state = test_state();

    let ws_evs = dispatch_apply(&state, Command::CreateWorkspace { name: "w".into() }).await;
    let ws_id = ws_evs
        .iter()
        .find_map(|e| match e {
            Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
            _ => None,
        })
        .unwrap();
    let tab_evs = dispatch_apply(
        &state,
        Command::CreateTab { workspace_id: ws_id.clone(), name: "t".into() },
    )
    .await;
    let tab_id = tab_evs
        .iter()
        .find_map(|e| match e {
            Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
            _ => None,
        })
        .unwrap();

    // The "current" Editor pane, standing in for the one openToTheSide
    // is called from.
    let current_editor = open_pane(
        &state,
        editor_open_cmd(Some(tab_id.clone()), "/tmp/current.md", None, None),
    )
    .await
    .expect("open_pane current editor");

    // openToTheSide's exact shape: split_reference_block_id = its own
    // block id, no reuse_editor_pane set.
    let side = open_pane(
        &state,
        editor_open_cmd(
            Some(tab_id.clone()),
            "/tmp/side.md",
            Some(current_editor.block_id.clone()),
            None,
        ),
    )
    .await
    .expect("open_pane openToTheSide");
    assert!(
        side.created,
        "openToTheSide must always create its own new pane, never reuse the calling pane"
    );
    assert_ne!(side.block_id, current_editor.block_id);
}

/// Regression for codex P1 on PR #2404: a `floating: true` OpenEditor
/// call must always get its own new floating window, even when an
/// Editor pane already exists (with reuse opted in) in the caller's tab
/// — reuse must not silently swallow it into the existing docked pane.
#[tokio::test]
async fn open_editor_floating_request_bypasses_reuse() {
    let state = test_state();

    let ws_evs = dispatch_apply(&state, Command::CreateWorkspace { name: "w".into() }).await;
    let ws_id = ws_evs
        .iter()
        .find_map(|e| match e {
            Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
            _ => None,
        })
        .unwrap();
    let tab_evs = dispatch_apply(
        &state,
        Command::CreateTab { workspace_id: ws_id.clone(), name: "t".into() },
    )
    .await;
    let tab_id = tab_evs
        .iter()
        .find_map(|e| match e {
            Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
            _ => None,
        })
        .unwrap();

    let caller = open_pane(&state, {
        let mut cmd = editor_open_cmd(Some(tab_id.clone()), "/tmp/unused.txt", None, None);
        cmd.view = "sysinfo".into();
        cmd.file = None;
        cmd
    })
    .await
    .expect("open_pane caller");

    let existing_editor = open_pane(
        &state,
        editor_open_cmd(Some(tab_id.clone()), "/tmp/existing.md", None, None),
    )
    .await
    .expect("open_pane existing editor");

    let mut floating_cmd = editor_open_cmd(
        None,
        "/tmp/floating.md",
        Some(caller.block_id.clone()),
        Some(true),
    );
    floating_cmd.floating = Some(true);
    let floating = open_pane(&state, floating_cmd)
        .await
        .expect("open_pane floating editor");
    assert_ne!(
        floating.block_id, existing_editor.block_id,
        "a floating OpenEditor request must never be swallowed into an existing docked pane"
    );
}

/// `view: "files"` (the Files pane, SPEC_FILE_BROWSER_PANE_2026_10_01.md
/// §8.1): the folder comes from `file`, else `cwd`, else home, and `select`
/// becomes the one-shot `files:select` only when it names something.
#[test]
fn files_view_meta_takes_the_folder_and_the_selection() {
    let mut cmd = editor_open_cmd(None, "/work/repo", None, None);
    cmd.view = "files".into();
    cmd.select = Some(vec!["src".into(), "README.md".into()]);
    let meta = pane::build_pane_meta(&cmd).unwrap();
    assert_eq!(meta["view"], "files");
    assert_eq!(meta["files:path"], "/work/repo");
    assert_eq!(meta["files:select"], serde_json::json!(["src", "README.md"]));

    cmd.file = None;
    cmd.cwd = Some("/from/cwd".into());
    cmd.select = Some(Vec::new());
    let meta = pane::build_pane_meta(&cmd).unwrap();
    assert_eq!(meta["files:path"], "/from/cwd");
    assert!(!meta.contains_key("files:select"), "an empty selection is no request");

    cmd.cwd = None;
    cmd.select = None;
    let meta = pane::build_pane_meta(&cmd).unwrap();
    let home = dirs::home_dir().unwrap().to_string_lossy().into_owned();
    assert_eq!(meta["files:path"], home.as_str(), "defaults to home");
}

/// `line` on an editor open (Remotes' "Edit in ssh config",
/// SPEC_REMOTES_PANE_2026_10_05.md §4.3) becomes the one-shot `editor:line`;
/// line 0 or no line is no request.
#[test]
fn editor_meta_carries_the_line_to_open_at() {
    let mut cmd = editor_open_cmd(None, "/home/u/.ssh/config", None, None);
    cmd.line = Some(12);
    let meta = pane::build_pane_meta(&cmd).unwrap();
    assert_eq!(meta["editor:line"], 12);

    cmd.line = Some(0);
    assert!(!pane::build_pane_meta(&cmd).unwrap().contains_key("editor:line"));
    cmd.line = None;
    assert!(!pane::build_pane_meta(&cmd).unwrap().contains_key("editor:line"));
}

#[test]
fn an_unknown_view_names_files_among_the_supported_ones() {
    let mut cmd = editor_open_cmd(None, "/x", None, None);
    cmd.view = "nope".into();
    let err = pane::build_pane_meta(&cmd).unwrap_err();
    assert!(err.starts_with("INVALID_VIEW") && err.contains("files"), "{err}");
}

// ── OpenBrowser and agent-owned browser panes ───────────────────────────────
// SPEC_AGENT_DRIVEN_BROWSER_PANES_2026_10_07.md §3, end to end through the
// real HTTP routes. No CEF host is registered in tests, and every Browser*
// handler checks ownership BEFORE contacting the host, so the status says
// what the ownership check decided: 503 (host not registered) means the
// caller got through; 403 and 404 mean it was stopped.

/// A fake agent registered on `block_id`, with a valid signed identity.
fn signed_agent_on(state: &AppState, block_id: &str) -> (String, serde_json::Value) {
    let agent_id = format!("browser-owner-agent-{}", uuid::Uuid::new_v4());
    let key = state.mstore.agent_jekt_key_ensure(&agent_id).unwrap();
    crate::backend::reactive::handler::get_global_handler()
        .register_agent(&agent_id, block_id, None)
        .unwrap();
    let ts_secs = agentmux_common::time::now_secs();
    let sig = agentmux_common::jekt_sign::sign_jekt(&key, "ui-automation-identity", &agent_id, "__srv__", ts_secs, "");
    (agent_id.clone(), serde_json::json!({ "agent_id": agent_id, "ts_secs": ts_secs, "sig": sig }))
}

async fn post_json(app: &axum::Router, uri: &str, body: serde_json::Value) -> (axum::http::StatusCode, serde_json::Value) {
    use tower::ServiceExt;
    let req = axum::http::Request::builder()
        .uri(uri)
        .method("POST")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(axum::body::Body::from(body.to_string()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null))
}

fn merged(auth: &serde_json::Value, extra: serde_json::Value) -> serde_json::Value {
    let mut b = auth.clone();
    for (k, v) in extra.as_object().unwrap() {
        b[k] = v.clone();
    }
    b
}

async fn stand_in_pane(state: &AppState, tab_id: &str) -> String {
    let mut cmd = editor_open_cmd(Some(tab_id.to_string()), "/tmp/unused.txt", None, None);
    cmd.view = "sysinfo".into();
    cmd.file = None;
    open_pane(state, cmd).await.expect("open a stand-in pane").block_id
}

#[tokio::test]
async fn open_browser_gives_the_agent_a_pane_only_it_can_drive_until_the_user_takes_over() {
    use axum::http::StatusCode;
    let state = test_state();
    let ws_id = dispatch_apply(&state, Command::CreateWorkspace { name: "w".into() })
        .await
        .iter()
        .find_map(|e| match e {
            Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
            _ => None,
        })
        .unwrap();
    let tab_id = dispatch_apply(&state, Command::CreateTab { workspace_id: ws_id, name: "t".into() })
        .await
        .iter()
        .find_map(|e| match e {
            Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
            _ => None,
        })
        .unwrap();
    let own = stand_in_pane(&state, &tab_id).await;
    let (agent, auth) = signed_agent_on(&state, &own);
    let app = crate::server::build_router(state.clone());

    // OpenBrowser refuses anything but an http(s) URL.
    let (s, _) = post_json(&app, "/api/v1/ui/browser/open", merged(&auth, serde_json::json!({ "url": "file:///etc/passwd" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // OpenBrowser opens a browser pane owned by the caller.
    let (s, body) = post_json(&app, "/api/v1/ui/browser/open", merged(&auth, serde_json::json!({ "url": "https://example.com/" }))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let pane = body["data"]["pane"].as_str().expect("pane id").to_string();
    let block = state.mstore.must_get::<crate::backend::obj::Block>(&pane).unwrap();
    assert_eq!(block.meta.get("view").and_then(|v| v.as_str()), Some("browser"));
    assert_eq!(block.meta.get("browser:owner_agent").and_then(|v| v.as_str()), Some(agent.as_str()));

    let eval = |auth: &serde_json::Value, pane: &str| merged(auth, serde_json::json!({ "pane": pane, "script": "1" }));

    // The owner gets past the ownership check (and stops at the missing host).
    let (s, _) = post_json(&app, "/api/v1/ui/browser/focus_info", eval(&auth, &pane)).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);

    // Even the owner can't use the routes that could submit a form without
    // the approval banner (spec §5.4); typing a letter is still fine.
    let (s, body) = post_json(&app, "/api/v1/ui/browser/eval", eval(&auth, &pane)).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{body}");
    assert!(body["error"].as_str().unwrap().contains("approval"), "{body}");
    let (s, _) = post_json(&app, "/api/v1/ui/click", merged(&auth, serde_json::json!({ "pane": pane, "selector": "button" }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let key = |k: &str| merged(&auth, serde_json::json!({ "pane": pane, "key": k }));
    let (s, _) = post_json(&app, "/api/v1/ui/browser/dispatch_key", key("Enter")).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = post_json(&app, "/api/v1/ui/browser/dispatch_key", merged(&auth, serde_json::json!({ "pane": pane, "text": "a\n" }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = post_json(&app, "/api/v1/ui/browser/dispatch_key", key("Tab")).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);

    // Another agent may not drive it.
    let other_own = stand_in_pane(&state, &tab_id).await;
    let (_, other) = signed_agent_on(&state, &other_own);
    let (s, body) = post_json(&app, "/api/v1/ui/browser/focus_info", eval(&other, &pane)).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{body}");

    // A pane that isn't a browser pane is refused, even a real one.
    let (s, _) = post_json(&app, "/api/v1/ui/browser/focus_info", eval(&auth, &other_own)).await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // A client can't make itself the owner by writing the meta key.
    let oref = format!("block:{pane}");
    let mut forged = crate::backend::obj::MetaMapType::new();
    forged.insert("browser:owner_agent".into(), serde_json::json!(agent));
    assert!(crate::server::browser_owner::guard_client_meta_write(&oref, &forged).is_err());

    // The user's Take over (a client clearing the key) ends ownership...
    let mut clear = crate::backend::obj::MetaMapType::new();
    clear.insert("browser:owner_agent".into(), serde_json::Value::Null);
    crate::server::browser_owner::guard_client_meta_write(&oref, &clear).unwrap();
    crate::server::service::update_object_meta(&state.mstore, &oref, &clear).unwrap();
    let (s, body) = post_json(&app, "/api/v1/ui/browser/focus_info", eval(&auth, &pane)).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{body}");

    // ...for good: even with the key written straight back (bypassing the
    // client guard), the owner map no longer agrees.
    crate::server::service::update_object_meta(&state.mstore, &oref, &forged).unwrap();
    let (s, _) = post_json(&app, "/api/v1/ui/browser/focus_info", eval(&auth, &pane)).await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // A closed pane is 404.
    crate::backend::wcore::delete_block(&state.mstore, &tab_id, &pane).unwrap();
    let (s, _) = post_json(&app, "/api/v1/ui/browser/focus_info", eval(&auth, &pane)).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // Without `pane`, the tools still act on the caller's own pane, as before.
    let (s, _) = post_json(&app, "/api/v1/ui/browser/eval", merged(&auth, serde_json::json!({ "script": "1" }))).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
}

async fn post_json_headers(
    app: &axum::Router,
    uri: &str,
    body: serde_json::Value,
    extra: &[(&str, &str)],
) -> (axum::http::StatusCode, serde_json::Value) {
    use tower::ServiceExt;
    let mut b = axum::http::Request::builder()
        .uri(uri)
        .method("POST")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json");
    for (k, v) in extra {
        b = b.header(*k, *v);
    }
    let resp = app.clone().oneshot(b.body(axum::body::Body::from(body.to_string())).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null))
}

// SPEC_AGENT_DRIVEN_BROWSER_PANES_2026_10_07.md §5.2: a hand-off shows a
// banner, pauses the agent's tools on the pane, and only the host (the
// user's click) can answer it; an agent with the instance auth key can't.
#[tokio::test]
async fn a_handoff_waits_for_the_users_answer_through_the_host_only() {
    use axum::http::StatusCode;
    let state = test_state();
    let ws_id = dispatch_apply(&state, Command::CreateWorkspace { name: "w".into() })
        .await
        .iter()
        .find_map(|e| match e {
            Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
            _ => None,
        })
        .unwrap();
    let tab_id = dispatch_apply(&state, Command::CreateTab { workspace_id: ws_id, name: "t".into() })
        .await
        .iter()
        .find_map(|e| match e {
            Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
            _ => None,
        })
        .unwrap();
    let own = stand_in_pane(&state, &tab_id).await;
    let (_agent, auth) = signed_agent_on(&state, &own);
    *state.host_ipc.lock().await = Some(crate::server::state::HostIpc { port: 1, token: "host-ipc-token".into() });
    let app = crate::server::build_router(state.clone());

    let (s, body) = post_json(&app, "/api/v1/ui/browser/open", merged(&auth, serde_json::json!({ "url": "https://example.com/" }))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let pane = body["data"]["pane"].as_str().unwrap().to_string();

    // A hand-off on the agent's own (non-browser) pane is refused: nothing
    // would show the banner, and its tools would stay locked.
    let (s, _) = post_json(&app, "/api/v1/ui/browser/handoff", merged(&auth, serde_json::json!({ "reason": "x" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(crate::server::browser_attention::waiting_on_user(&own), None);

    // The agent hands the pane to the user and waits.
    let waiting = {
        let app = app.clone();
        let body = merged(&auth, serde_json::json!({ "pane": pane, "reason": "Sign in to your Microsoft account" }));
        tokio::spawn(async move { post_json(&app, "/api/v1/ui/browser/handoff", body).await })
    };
    let mut banner = serde_json::Value::Null;
    for _ in 0..100 {
        let block = state.mstore.must_get::<crate::backend::obj::Block>(&pane).unwrap();
        if let Some(b) = block.meta.get("browser:attention").filter(|v| !v.is_null()) {
            banner = b.clone();
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(banner["kind"], "handoff", "{banner}");
    assert_eq!(banner["reason"], "Sign in to your Microsoft account");
    let id = banner["id"].as_str().unwrap().to_string();

    // While the user works, the agent's tools on that pane are paused.
    let (s, _) = post_json(&app, "/api/v1/ui/browser/eval", merged(&auth, serde_json::json!({ "pane": pane, "script": "1" }))).await;
    assert_eq!(s, StatusCode::CONFLICT);

    // The agent can't answer its own request with the instance auth key...
    let answer = serde_json::json!({ "block_id": pane, "id": id, "decision": "done" });
    let (s, _) = post_json_headers(&app, "/api/v1/host/browser_attention", answer.clone(), &[]).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = post_json_headers(&app, "/api/v1/host/browser_attention", answer.clone(), &[("X-Host-Token", "guess")]).await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // ...only the host can, with its registered token.
    let (s, body) = post_json_headers(&app, "/api/v1/host/browser_attention", answer.clone(), &[("X-Host-Token", "host-ipc-token")]).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let (s, body) = waiting.await.unwrap();
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["answer"], "done");

    // The banner is gone, the pane is the agent's again, and the answer was single use.
    let block = state.mstore.must_get::<crate::backend::obj::Block>(&pane).unwrap();
    assert!(block.meta.get("browser:attention").is_none_or(|v| v.is_null()));
    assert_eq!(crate::server::browser_attention::waiting_on_user(&pane), None);
    let (s, _) = post_json_headers(&app, "/api/v1/host/browser_attention", answer, &[("X-Host-Token", "host-ipc-token")]).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // An agent that gives up waiting (its request is dropped mid-wait)
    // doesn't leave the pane locked or the banner up.
    let abandoned = {
        let app = app.clone();
        let body = merged(&auth, serde_json::json!({ "pane": pane, "reason": "Solve the CAPTCHA" }));
        tokio::spawn(async move { post_json(&app, "/api/v1/ui/browser/handoff", body).await })
    };
    for _ in 0..100 {
        if crate::server::browser_attention::waiting_on_user(&pane).is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(crate::server::browser_attention::waiting_on_user(&pane).is_some());
    abandoned.abort();
    let _ = abandoned.await;
    assert_eq!(crate::server::browser_attention::waiting_on_user(&pane), None);
    let block = state.mstore.must_get::<crate::backend::obj::Block>(&pane).unwrap();
    assert!(block.meta.get("browser:attention").is_none_or(|v| v.is_null()));

    // A banner left over from before an srv restart (the meta persisted, the
    // request didn't): the user's answer gets a 404 and srv takes it down.
    let mut stale = crate::backend::obj::MetaMapType::new();
    stale.insert("browser:attention".into(), serde_json::json!({ "id": "stale-1", "kind": "approval" }));
    crate::server::service::update_object_meta(&state.mstore, &format!("block:{pane}"), &stale).unwrap();
    let answer = serde_json::json!({ "block_id": pane, "id": "stale-1", "decision": "approve" });
    let (s, _) = post_json_headers(&app, "/api/v1/host/browser_attention", answer, &[("X-Host-Token", "host-ipc-token")]).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let block = state.mstore.must_get::<crate::backend::obj::Block>(&pane).unwrap();
    assert!(block.meta.get("browser:attention").is_none_or(|v| v.is_null()));

    // Take over while the agent waits answers its request with Cancelled.
    let waiting = {
        let app = app.clone();
        let body = merged(&auth, serde_json::json!({ "pane": pane, "reason": "Pick a seat" }));
        tokio::spawn(async move { post_json(&app, "/api/v1/ui/browser/handoff", body).await })
    };
    for _ in 0..100 {
        if crate::server::browser_attention::waiting_on_user(&pane).is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let mut clear = crate::backend::obj::MetaMapType::new();
    clear.insert("browser:owner_agent".into(), serde_json::Value::Null);
    crate::server::browser_owner::guard_client_meta_write(&format!("block:{pane}"), &clear).unwrap();
    let (s, body) = waiting.await.unwrap();
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["answer"], "cancelled");
    assert_eq!(crate::server::browser_attention::waiting_on_user(&pane), None);
}
