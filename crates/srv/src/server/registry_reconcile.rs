// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Puts a running agent back in the reactive registry when it has fallen out of it (task #2093).
//!
//! An agent is addressable (a message reaches it, `Agents conversations` and `Agents transcript` find it)
//! only while the registry holds it, and the registry is written when its process spawns. If the entry
//! goes while the process lives (a closing tool shell once removed the pane's own entry, #6931), nothing
//! wrote it back until the next spawn, so a working agent answered "not found".
//!
//! Every [`LOOK_EVERY`] this looks at the agents that have a pane and registers each one that is missing,
//! the way a start does (`jekt_start::make_addressable`). It only adds, and only for an agent whose
//! controller holds a live process and carries that agent's own uid, so a stopped, parked or
//! look-alike pane is never registered. An agent registered twice is the same registration.

use std::time::Duration;

use crate::backend::blockcontroller::{self, STATUS_RUNNING};
use crate::backend::obj::Block;

use super::AppState;

/// How often the registry is checked against the panes.
const LOOK_EVERY: Duration = Duration::from_secs(60);

/// Whether an agent that is missing from the registry should be put back: its controller holds a live
/// process and was spawned as this uid, and its block is not parked (a parked block is on its way out).
fn should_register(uid: &str, parked: bool, controller_uid: Option<&str>, process_status: &str) -> bool {
    !parked && controller_uid == Some(uid) && process_status == STATUS_RUNNING
}

/// Registers every agent with a live process that the registry does not hold. Returns the uids it put back.
/// Blocking: reads the store.
pub(crate) fn reconcile(state: &AppState) -> Vec<String> {
    let mut registered = Vec::new();
    for (uid, block_id) in super::agent_runtime::pane_blocks_by_agent(&state.mstore) {
        if state.reactive_handler.has_uid_registration(&uid) {
            continue;
        }
        let Some(controller) = blockcontroller::get_controller(&block_id) else {
            continue;
        };
        let parked = state
            .mstore
            .get::<Block>(&block_id)
            .ok()
            .flatten()
            .is_some_and(|b| crate::backend::obj::meta_get_bool(&b.meta, crate::persist_subscriber::parked::PARKED_META_KEY, false));
        let status = controller.get_runtime_status().shellprocstatus;
        if should_register(&uid, parked, controller.stable_agent_uid().as_deref(), &status)
            && super::jekt_start::make_addressable(state, &uid, &block_id)
        {
            tracing::warn!(agent_uid = %uid, block_id = %block_id, "registry: a running agent was missing from it — registered again");
            registered.push(uid);
        }
    }
    registered
}

/// Runs [`reconcile`] on an interval for the life of the server.
pub(crate) fn spawn(state: AppState) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(LOOK_EVERY).await;
            let looked = state.clone();
            if let Err(e) = tokio::task::spawn_blocking(move || reconcile(&looked)).await {
                tracing::warn!("registry reconcile: {e}");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_running_agent_whose_controller_carries_its_uid_is_put_back() {
        assert!(should_register("u1", false, Some("u1"), STATUS_RUNNING));
    }

    #[test]
    fn a_parked_stopped_or_look_alike_pane_is_left_alone() {
        assert!(!should_register("u1", true, Some("u1"), STATUS_RUNNING), "parked");
        assert!(!should_register("u1", false, Some("u1"), "done"), "its process ended");
        assert!(!should_register("u1", false, Some("u1"), ""), "no process yet");
        assert!(!should_register("u1", false, Some("u2"), STATUS_RUNNING), "another agent's controller");
        assert!(!should_register("u1", false, None, STATUS_RUNNING), "a controller with no agent uid");
    }

    #[tokio::test]
    async fn an_agent_with_no_pane_is_not_registered() {
        let state = super::super::tests::test_state();
        assert!(reconcile(&state).is_empty());
    }
}
