// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The backend-to-frontend layout action queue of a layout row.

use crate::backend::obj::{LayoutActionData, LayoutState as PersistedLayoutState, Tab};
use crate::backend::storage::store::Store;

/// How many queued layout actions a row keeps: more than a window ever has
/// left to mirror at once.
pub(super) const QUEUE_LIMIT: usize = 32;

/// SPEC_864 Phase 4 — apply a `LayoutBackendActionsQueued` event:
/// APPEND the reducer-validated actions to
/// `LayoutState.pendingbackendactions`, keeping the latest `QUEUE_LIMIT`.
/// The queue only tells a window what srv already did to the tree (every
/// queued action is applied in srv first), so the oldest can go: a window that
/// missed one shows srv's stored tree anyway.
///
/// Idempotency: the event is applied twice on the happy path (sync in
/// `queue_layout_actions_via_reducer`, again by the subscriber task
/// consuming the broadcast), so actions whose `actionid` is already in
/// the queue are skipped. Every writer stamps a fresh UUID per action;
/// an (out-of-contract) empty `actionid` dedupes against other empties,
/// which is the safe direction — skip rather than duplicate.
pub(super) fn apply_layout_backend_actions_queued(
    mstore: &Store,
    tab_id: &str,
    actions: &serde_json::Value,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(tab) = mstore.get::<Tab>(tab_id)? else {
        return Ok(());
    };
    if tab.layoutstate.is_empty() {
        return Ok(());
    }
    let Some(mut layout) = mstore.get::<PersistedLayoutState>(&tab.layoutstate)? else {
        return Ok(());
    };
    let incoming: Vec<LayoutActionData> = serde_json::from_value(actions.clone())?;
    let mut pending = layout.pendingbackendactions.take().unwrap_or_default();
    let before = pending.len();
    for action in incoming {
        if !pending.iter().any(|p| p.actionid == action.actionid) {
            pending.push(action);
        }
    }
    if pending.len() == before {
        return Ok(()); // a double-apply: no version churn
    }
    let excess = pending.len().saturating_sub(QUEUE_LIMIT);
    pending.drain(..excess);
    layout.pendingbackendactions = Some(pending);
    mstore.update(&mut layout)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use agentmux_common::ipc::Event;

    use super::*;
    use crate::persist_subscriber::apply_event_to_mstore;

    #[test]
    fn the_queue_keeps_the_latest_actions() {
        let s = Store::open_in_memory().expect("in-memory mstore");
        apply_event_to_mstore(&Event::WorkspaceCreated { workspace_id: "ws".into(), name: "W".into(), version: 1 }, &s).unwrap();
        let created = Event::TabCreated { workspace_id: "ws".into(), tab_id: "tab".into(), name: "T".into(), version: 2 };
        apply_event_to_mstore(&created, &s).unwrap();
        for i in 0..QUEUE_LIMIT + 8 {
            let action = LayoutActionData { actiontype: "delete".into(), actionid: format!("a{i}"), blockid: "b".into(), ..Default::default() };
            let ev = Event::LayoutBackendActionsQueued {
                tab_id: "tab".into(),
                actions: serde_json::to_value(vec![action]).unwrap(),
                correlation_id: String::new(),
                version: 3 + i as u64,
            };
            apply_event_to_mstore(&ev, &s).unwrap();
        }
        let tab = s.get::<Tab>("tab").unwrap().unwrap();
        let queue = s.get::<PersistedLayoutState>(&tab.layoutstate).unwrap().unwrap().pendingbackendactions.unwrap();
        assert_eq!(queue.len(), QUEUE_LIMIT);
        assert_eq!(queue.first().map(|a| a.actionid.as_str()), Some("a8"));
        assert_eq!(queue.last().map(|a| a.actionid.as_str()), Some(format!("a{}", QUEUE_LIMIT + 7).as_str()));
    }
}
