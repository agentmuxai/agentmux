// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Apply a layout intent and store its result: the write path of every layout
//! edit a window makes (`layout.applyIntent`), and of srv's own placements
//! (`place_block`). SPEC_STRONG_REDUCER_AUTHORITY_LAYOUT_2026_06_30.md §4.

use agentmux_common::ipc::{Command, Event};
use agentmux_common::{FlexDirection, LayoutClientSlices, LayoutIntent, LayoutNode, LayoutNodeData, LayoutViewState, SplitPosition};

use crate::backend::obj::{LayoutActionData, LayoutState, Tab};

use super::super::AppState;

/// What became of an intent.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct IntentOutcome {
    /// False when the reducer refused it; `error` says why.
    pub applied: bool,
    pub error: Option<String>,
    /// The layout row's version afterwards.
    pub version: u64,
}

/// Dispatch `LayoutApplyIntent` and write its events to the layout row in one
/// hold of the reducer lock, so rows are written in the order intents are
/// applied. A failed write puts the tab back as the row has it. Publishes on
/// success.
pub(crate) async fn apply_layout_intent(
    state: &AppState,
    tab_id: &str,
    intent: LayoutIntent,
    action_id: &str,
    view: Option<LayoutViewState>,
) -> Result<IntentOutcome, String> {
    let store = &state.mstore;
    let layout_oid = store
        .must_get::<Tab>(tab_id)
        .map_err(|e| format!("layout intent: tab {tab_id}: {e}"))?
        .layoutstate;
    let row_version = || layout_row_version(store, tab_id);
    let cmd = Command::LayoutApplyIntent {
        tab_id: tab_id.to_string(),
        intent,
        action_id: action_id.to_string(),
        view,
    };
    let (events, outcome) = {
        let mut s = state.srv_state.lock().await;
        let old_row = store.get::<LayoutState>(&layout_oid).map_err(|e| format!("layout intent: {e}"))?;
        let ctx = crate::reducer::Ctx::new(None);
        let events = crate::reducer::update(&mut s, cmd, &ctx);
        if let Some(message) = events.iter().find_map(|e| match e {
            Event::Error { message, .. } => Some(message.clone()),
            _ => None,
        }) {
            tracing::info!(tab_id, action_id, "layout intent refused: {message}");
            let outcome = IntentOutcome { applied: false, error: Some(message), version: row_version() };
            (Vec::new(), outcome)
        } else if let Some(err) = events
            .iter()
            .find_map(|ev| crate::persist_subscriber::apply_event_to_mstore(ev, store).err())
        {
            roll_back(&mut s, store, tab_id, action_id, old_row);
            return Err(format!("layout intent: SQLite write failed: {err}"));
        } else {
            (events, IntentOutcome { applied: true, error: None, version: row_version() })
        }
    };
    super::publish_events(state, &events);
    Ok(outcome)
}

/// The version of `tab_id`'s layout row (0 when it has none).
pub(crate) fn layout_row_version(store: &crate::backend::storage::store::Store, tab_id: &str) -> u64 {
    store
        .get::<Tab>(tab_id)
        .ok()
        .flatten()
        .and_then(|tab| store.get::<LayoutState>(&tab.layoutstate).ok().flatten())
        .map_or(0, |row| row.version as u64)
}

/// Put the tab back as `old_row` stores it, and forget the action id so a
/// resend applies.
fn roll_back(
    s: &mut crate::state::State,
    store: &crate::backend::storage::store::Store,
    tab_id: &str,
    action_id: &str,
    old_row: Option<LayoutState>,
) {
    let Some(old) = old_row else { return };
    let rollback = Command::LayoutSetTree {
        tab_id: tab_id.to_string(),
        new_tree: old.rootnode.clone(),
        correlation_id: String::new(),
        slices: Some(LayoutClientSlices {
            leaforder: old.leaforder.as_ref().and_then(|v| serde_json::to_value(v).ok()),
            focused_node_id: old.focusednodeid.clone(),
            magnified_node_id: old.magnifiednodeid.clone(),
            pending_backend_actions: old.pendingbackendactions.as_ref().and_then(|v| serde_json::to_value(v).ok()),
        }),
    };
    for ev in crate::reducer::update(s, rollback, &crate::reducer::Ctx::new(None)) {
        if let Err(e) = crate::persist_subscriber::apply_event_to_mstore(&ev, store) {
            tracing::warn!(error = %e, "layout intent: rollback SQLite mirror failed");
        }
    }
    if let Some(tab) = s.tabs.get_mut(tab_id) {
        tab.recent_intent_ids.retain(|id| id != action_id);
    }
}

/// Where `place_block` puts a block.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Placement {
    /// The first free slot (`insert`).
    Append { focused: bool },
    /// Beside the pane holding `target_block_id`, carving `size_fraction` of
    /// it (`None`: the new pane's default size). Falls back to `Append` when
    /// that pane is gone or minimized.
    Split {
        target_block_id: String,
        horizontal: bool,
        position: SplitPosition,
        size_fraction: Option<f64>,
        focused: bool,
    },
}

/// Place `block_id` in its tab's layout: srv applies the placement to its own
/// tree, then queues the matching action for the window showing the tab,
/// under the same action id and with the leaf's node id, so the window's
/// mirror builds the tree srv stored.
pub(crate) async fn place_block(state: &AppState, tab_id: &str, block_id: &str, placement: Placement) -> Result<(), String> {
    let node_id = uuid::Uuid::new_v4().to_string();
    let action_id = uuid::Uuid::new_v4().to_string();
    let leaf = LayoutNode {
        id: node_id.clone(),
        flex_direction: FlexDirection::Row,
        size: crate::backend::layout::DEFAULT_NODE_SIZE,
        data: Some(LayoutNodeData { block_id: block_id.to_string(), ..Default::default() }),
        ..Default::default()
    };
    let mut placement = placement;
    if let Placement::Split { target_block_id, horizontal, position, size_fraction, focused } = &placement {
        let target_node_id = state.srv_state.lock().await.tabs.get(tab_id).and_then(|t| t.rootnode.as_ref()).and_then(|root| {
            crate::backend::layout::find_leaf_containing_block(root, target_block_id).map(|n| n.id.clone())
        });
        if let Some(target_node_id) = target_node_id {
            let new_node = leaf.clone();
            let (position, size_fraction, focused) = (*position, *size_fraction, *focused);
            let intent = if *horizontal {
                LayoutIntent::SplitHorizontal { target_node_id, new_node, position, size_fraction, focused }
            } else {
                LayoutIntent::SplitVertical { target_node_id, new_node, position, size_fraction, focused }
            };
            if apply_layout_intent(state, tab_id, intent, &action_id, None).await?.applied {
                let action = LayoutActionData {
                    actiontype: if *horizontal { "splithorizontal" } else { "splitvertical" }.to_string(),
                    actionid: action_id,
                    blockid: block_id.to_string(),
                    nodesizefraction: size_fraction,
                    focused,
                    targetblockid: target_block_id.clone(),
                    position: match position {
                        SplitPosition::Before => "before",
                        SplitPosition::After => "after",
                    }
                    .to_string(),
                    nodeid: Some(node_id),
                    ..Default::default()
                };
                return super::reducer_helpers::queue_layout_actions_via_reducer(state, tab_id, vec![action]).await;
            }
        }
        placement = Placement::Append { focused: *focused };
    }
    let Placement::Append { focused } = placement else { unreachable!("a split fell back above") };
    let outcome =
        apply_layout_intent(state, tab_id, LayoutIntent::Insert { node: leaf, magnified: false, focused }, &action_id, None)
            .await?;
    if !outcome.applied {
        return Err(format!("placing block {block_id}: {}", outcome.error.unwrap_or_default()));
    }
    let action = LayoutActionData {
        actiontype: "insert".to_string(),
        actionid: action_id,
        blockid: block_id.to_string(),
        focused,
        nodeid: Some(node_id),
        ..Default::default()
    };
    super::reducer_helpers::queue_layout_actions_via_reducer(state, tab_id, vec![action]).await
}

/// Take `block_id` out of its tab's layout in srv (one member of a stack, or
/// its pane): `LayoutDeleteNodeByBlock`, stored and published. A block in no
/// pane is a no-op.
pub(crate) async fn remove_block_from_layout(state: &AppState, tab_id: &str, block_id: &str) -> Result<(), String> {
    let cmd = Command::LayoutDeleteNodeByBlock {
        tab_id: tab_id.to_string(),
        block_id: block_id.to_string(),
        correlation_id: String::new(),
    };
    let events = {
        let mut s = state.srv_state.lock().await;
        let events = crate::reducer::update(&mut s, cmd, &crate::reducer::Ctx::new(None));
        if let Some(message) = events.iter().find_map(|e| match e {
            Event::Error { message, .. } => Some(message.clone()),
            _ => None,
        }) {
            return Err(message);
        }
        for ev in &events {
            crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore).map_err(|e| e.to_string())?;
        }
        events
    };
    super::publish_events(state, &events);
    Ok(())
}
