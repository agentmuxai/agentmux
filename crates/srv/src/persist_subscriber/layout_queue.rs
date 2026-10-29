// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The backend-to-frontend layout action queue of a layout row.

use crate::backend::obj::{LayoutActionData, LayoutState as PersistedLayoutState, Tab};
use crate::backend::storage::store::Store;

/// How many queued layout actions a row keeps: more than a window ever has
/// left to mirror at once.
pub(super) const QUEUE_LIMIT: usize = 32;

/// How long a queued action stays: a window mirrors it within seconds of the change, and one
/// that was asleep longer shows srv's stored tree (it compares that tree with its own whenever
/// it is quiet), so an older action is only dead weight on every layout update.
pub(super) const QUEUE_MAX_AGE_MS: i64 = 10 * 60 * 1000;

/// Drop the actions that are older than `QUEUE_MAX_AGE_MS` at `now_ms`, and any with no queue
/// time (queued by an older srv, so of unknown age). Returns whether it dropped any.
pub(super) fn prune_stale(queue: &mut Vec<LayoutActionData>, now_ms: i64) -> bool {
    let before = queue.len();
    queue.retain(|a| a.queuedat.is_some_and(|at| now_ms.saturating_sub(at) <= QUEUE_MAX_AGE_MS));
    queue.len() != before
}

/// SPEC_864 Phase 4 — apply a `LayoutBackendActionsQueued` event:
/// APPEND the reducer-validated actions to
/// `LayoutState.pendingbackendactions`, keeping the latest `QUEUE_LIMIT`.
/// The queue only tells a window what srv already did to the tree (every
/// queued action is applied in srv first), so the oldest can go: a window that
/// missed one shows srv's stored tree anyway. Each action is stamped when it is queued and
/// `prune_stale` drops the old ones, here and whenever a layout edit rewrites the row.
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
    let now = agentmux_common::time::now_ms();
    let pruned = prune_stale(&mut pending, now);
    let before = pending.len();
    for mut action in incoming {
        if !pending.iter().any(|p| p.actionid == action.actionid) {
            action.queuedat = Some(now);
            pending.push(action);
        }
    }
    if pending.len() == before && !pruned {
        return Ok(()); // a double-apply: no version churn
    }
    let excess = pending.len().saturating_sub(QUEUE_LIMIT);
    pending.drain(..excess);
    layout.pendingbackendactions = Some(pending).filter(|queue| !queue.is_empty());
    mstore.update(&mut layout)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use agentmux_common::ipc::Event;

    use super::*;
    use crate::persist_subscriber::apply_event_to_mstore;

    fn action(id: &str, queuedat: Option<i64>) -> LayoutActionData {
        LayoutActionData { actiontype: "delete".into(), actionid: id.into(), blockid: "b".into(), queuedat, ..Default::default() }
    }

    #[test]
    fn prune_stale_drops_old_and_undated_actions_and_keeps_fresh_ones() {
        let now = 10_000_000;
        let mut queue = vec![
            action("old", Some(now - QUEUE_MAX_AGE_MS - 1)),
            action("undated", None),
            action("edge", Some(now - QUEUE_MAX_AGE_MS)),
            action("fresh", Some(now - 1000)),
        ];
        assert!(prune_stale(&mut queue, now));
        let ids: Vec<&str> = queue.iter().map(|a| a.actionid.as_str()).collect();
        assert_eq!(ids, ["edge", "fresh"]);
        assert!(!prune_stale(&mut queue, now), "nothing left to drop");
    }

    #[test]
    fn a_queued_action_is_stamped_and_the_legacy_ones_go() {
        let s = Store::open_in_memory().expect("in-memory mstore");
        apply_event_to_mstore(&Event::WorkspaceCreated { workspace_id: "ws".into(), name: "W".into(), version: 1 }, &s).unwrap();
        apply_event_to_mstore(&Event::TabCreated { workspace_id: "ws".into(), tab_id: "tab".into(), name: "T".into(), version: 2 }, &s).unwrap();
        // A queue an older srv left: undated, and never drained.
        let tab = s.get::<Tab>("tab").unwrap().unwrap();
        let mut layout = s.get::<PersistedLayoutState>(&tab.layoutstate).unwrap().unwrap();
        layout.pendingbackendactions = Some((0..QUEUE_LIMIT).map(|i| action(&format!("legacy{i}"), None)).collect());
        s.update(&mut layout).unwrap();

        let ev = Event::LayoutBackendActionsQueued {
            tab_id: "tab".into(),
            actions: serde_json::to_value(vec![action("new", None)]).unwrap(),
            correlation_id: String::new(),
            version: 3,
        };
        apply_event_to_mstore(&ev, &s).unwrap();
        let queue = s.get::<PersistedLayoutState>(&tab.layoutstate).unwrap().unwrap().pendingbackendactions.unwrap();
        assert_eq!(queue.len(), 1, "the undated legacy queue is gone");
        assert_eq!(queue[0].actionid, "new");
        assert!(queue[0].queuedat.is_some(), "srv stamps the action when it queues it");
    }

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
