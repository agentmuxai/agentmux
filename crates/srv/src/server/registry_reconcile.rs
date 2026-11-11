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
//!
//! The process's own exit removes only the entry it wrote (compare-and-remove on its spawn nonce), so
//! each entry made here carries a nonce of its own, kept in [`Reconciled`], and the next look removes it
//! once the process has ended. A respawn that registered meanwhile has another nonce and is left alone.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::backend::blockcontroller::{self, STATUS_RUNNING};
use crate::backend::obj::Block;

use super::AppState;

/// How often the registry is checked against the panes.
const LOOK_EVERY: Duration = Duration::from_secs(60);

/// The entries this module made: block id to the nonce it registered under.
type Reconciled = HashMap<String, u64>;

/// A nonce for an entry made here. Counted down from the top, so it never meets a controller's, which
/// count up from 1.
fn next_nonce() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(u64::MAX);
    NEXT.fetch_sub(1, Ordering::Relaxed)
}

/// Whether an agent that is missing from the registry should be put back: its controller holds a live
/// process and was spawned as this uid, and its block is not parked (a parked block is on its way out).
fn should_register(uid: &str, parked: bool, controller_uid: Option<&str>, process_status: &str) -> bool {
    !parked && controller_uid == Some(uid) && process_status == STATUS_RUNNING
}

/// Whether the block's controller holds a live process.
fn process_running(block_id: &str) -> bool {
    blockcontroller::get_controller(block_id).is_some_and(|c| c.get_runtime_status().shellprocstatus == STATUS_RUNNING)
}

/// Removes the entries made here whose process has ended. Returns the blocks it cleared.
fn drop_ended(state: &AppState, made: &mut Reconciled, running: impl Fn(&str) -> bool) -> Vec<String> {
    let ended: Vec<String> = made.keys().filter(|b| !running(b)).cloned().collect();
    for block_id in &ended {
        if let Some(nonce) = made.remove(block_id) {
            state.reactive_handler.unregister_block_if_nonce(block_id, nonce);
        }
    }
    ended
}

/// Registers every agent with a live process that the registry does not hold, and clears the entries made
/// here for a process that has ended. Returns the uids it put back. Blocking: reads the store.
fn reconcile(state: &AppState, made: &mut Reconciled) -> Vec<String> {
    drop_ended(state, made, process_running);
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
        if !should_register(&uid, parked, controller.stable_agent_uid().as_deref(), &status) {
            continue;
        }
        let nonce = next_nonce();
        if super::jekt_start::make_addressable(state, &uid, &block_id, nonce) {
            tracing::warn!(agent_uid = %uid, block_id = %block_id, "registry: a running agent was missing from it — registered again");
            made.insert(block_id, nonce);
            registered.push(uid);
        }
    }
    registered
}

/// Runs [`reconcile`] on an interval for the life of the server.
pub(crate) fn spawn(state: AppState) {
    tokio::spawn(async move {
        let mut made = Reconciled::new();
        loop {
            tokio::time::sleep(LOOK_EVERY).await;
            let looked = state.clone();
            match tokio::task::spawn_blocking(move || {
                reconcile(&looked, &mut made);
                made
            })
            .await
            {
                Ok(kept) => made = kept,
                Err(e) => {
                    tracing::warn!("registry reconcile: {e}");
                    made = Reconciled::new();
                }
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
        assert!(reconcile(&state, &mut Reconciled::new()).is_empty());
    }

    #[tokio::test]
    async fn an_entry_made_here_goes_when_its_process_ends_but_a_respawns_stays() {
        let state = super::super::tests::test_state();
        let tag = uuid::Uuid::new_v4();
        let (gone, kept) = (format!("blk-gone-{tag}"), format!("blk-kept-{tag}"));
        let (gone_uid, kept_uid) = (format!("rc-{tag}-a"), format!("rc-{tag}-b"));
        let (gone_nonce, kept_nonce) = (next_nonce(), next_nonce());
        let h = &state.reactive_handler;
        h.register_agent_full("reconcile-a", &gone, None, gone_nonce, None, Some(&gone_uid), "t").unwrap();
        h.register_agent_full("reconcile-b", &kept, None, kept_nonce - 1, None, Some(&kept_uid), "t").unwrap(); // a respawn's own nonce
        let mut made = Reconciled::from([(gone.clone(), gone_nonce), (kept.clone(), kept_nonce)]);

        let cleared = drop_ended(&state, &mut made, |_| false);

        assert_eq!(cleared.len(), 2);
        assert!(!h.has_uid_registration(&gone_uid), "the entry made here goes with its process");
        assert!(h.has_uid_registration(&kept_uid), "an entry that changed hands is not ours to remove");
        assert!(made.is_empty());
    }
}
