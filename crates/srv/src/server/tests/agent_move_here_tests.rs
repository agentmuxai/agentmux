// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Move here (`server::agent_move_here`) against real persistent panes: the holder is told to stop
//! and left with a notice, the claimant waits for the lease. They need `AppState` and the move, which
//! belong to the layer above `backend::blockcontroller`, so they live here and not with the
//! controller's single-live-instance tests (SPEC_AGENT_SINGLE_LIVE_INSTANCE_2026_09_24.md Phase 1).

use std::sync::Arc;

use crate::backend::blockcontroller::persistent::PersistentSubprocessController;
use crate::backend::blockcontroller::Controller;

/// An agent's lease store in a temp registry, and panes that take its lease.
struct Fixture {
    _tmp: tempfile::TempDir,
    registry: Arc<crate::registry::Registry>,
    uid: String,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let registry = Arc::new(crate::registry::Registry::open(tmp.path().to_path_buf()).unwrap());
        Self { _tmp: tmp, registry, uid: format!("uid-{}", uuid::Uuid::new_v4()) }
    }

    fn store(&self) -> Arc<crate::registry::LeaseStore> {
        crate::backend::agent_admission::lease_store_for(Some(Arc::clone(&self.registry))).unwrap()
    }

    fn controller(&self, boot: &str) -> Arc<PersistentSubprocessController> {
        self.controller_in(boot, format!("block-{}", uuid::Uuid::new_v4()))
    }

    /// A controller for a block id the store can address (`ORef` wants a bare UUID).
    fn controller_in(&self, boot: &str, block_id: String) -> Arc<PersistentSubprocessController> {
        Arc::new(
            PersistentSubprocessController::new("tab".to_string(), block_id, None, None, None, None)
                .with_agent_lease_store(Some(Arc::clone(&self.registry)), Arc::from(boot)),
        )
    }
}

/// Move here: the holder pane is told to stop (gracefully), the claimant waits until the agent is
/// free, and the holder pane is left with the same notice, pointing back at the claimant.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn moving_the_agent_here_stops_the_holder_pane_and_waits_for_its_lease() {
    let f = Fixture::new();
    let state = crate::server::tests::test_state();
    let holder = f.controller_in("this-srv", uuid::Uuid::new_v4().to_string());
    let claimant = f.controller_in("this-srv", uuid::Uuid::new_v4().to_string());
    let held = crate::backend::agent_admission::acquire(
        &f.store(), &f.uid, "Agent3", &Arc::from("this-srv"), holder.block_id(), None, |_| {},
    )
    .unwrap();
    let stop = holder.hold_lease_for_test(held);
    crate::backend::blockcontroller::register_controller(holder.block_id(), holder.clone());
    for (block_id, uid) in [(holder.block_id(), &f.uid), (claimant.block_id(), &f.uid)] {
        let mut pane = crate::backend::obj::Block { oid: block_id.to_string(), ..Default::default() };
        pane.meta.insert("agentId".to_string(), serde_json::json!(uid));
        state.mstore.insert(&mut pane).unwrap();
    }
    // The holder's process exits when told to stop; its lease drops with it.
    let exiting = Arc::clone(&holder);
    let stopped = tokio::spawn(async move {
        let graceful = stop.was_graceful().await;
        exiting.drop_lease_for_test();
        graceful
    });

    let moved = crate::server::agent_move_here::move_here(&state, claimant.block_id()).await;
    let graceful = stopped.await.unwrap();
    let notice = state.mstore.get::<crate::backend::obj::Block>(holder.block_id()).unwrap().unwrap();
    crate::backend::blockcontroller::delete_controller(holder.block_id());

    assert_eq!(moved.unwrap().from_block.as_deref(), Some(holder.block_id()));
    assert!(graceful, "a graceful stop, not a kill");
    let failure = notice.meta.get("agent:last_failure").map(|v| v.to_string()).unwrap_or_else(|| format!("no failure in {:?}", notice.meta));
    assert!(failure.contains("open_in_other_pane") && failure.contains(claimant.block_id()), "got: {failure}");
}

/// Nobody else holds the agent: nothing to move, and the caller just retries.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn moving_an_agent_nobody_else_holds_is_a_no_op() {
    let f = Fixture::new();
    let state = crate::server::tests::test_state();
    let claimant = f.controller("this-srv");
    let mut pane = crate::backend::obj::Block { oid: claimant.block_id().to_string(), ..Default::default() };
    pane.meta.insert("agentId".to_string(), serde_json::json!(f.uid));
    state.mstore.insert(&mut pane).unwrap();
    let moved = crate::server::agent_move_here::move_here(&state, claimant.block_id()).await.unwrap();
    assert_eq!(moved.from_block, None);
}
