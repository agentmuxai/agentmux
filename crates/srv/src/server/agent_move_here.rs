// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Move an agent here from another pane of this AgentMux.
//!
//! A second pane of an agent that is live in another pane is refused (`open_in_other_pane`). This
//! is the recovery the refused pane offers beside "Go there": stop the pane that holds the agent
//! and let the refused pane have it. The holder pane stays open, showing the same notice with the
//! roles swapped, so the move can be undone from there.
//! Spec: docs/specs/SPEC_AGENT_SAME_PROCESS_DUPLICATE_PANE_RECOVERY_2026_10_01.md §4.3.

use std::time::{Duration, Instant};

use crate::backend::blockcontroller::{self, persistent};
use crate::backend::obj::{self, Block};
use crate::backend::rpc_types::AgentMoveHereResult;

use super::AppState;

/// How long the holder's turn in flight may finish before its CLI process is killed.
const MOVE_GRACE: Duration = Duration::from_secs(10);
/// How long the claimant waits for the holder's lease to drop after the process is told to stop.
const MOVE_WAIT: Duration = Duration::from_secs(25);

/// Stop the pane that holds `block_id`'s agent, wait for the agent to be free, and tell the holder
/// pane where it went. `Ok` with no `from_block` when no other pane held it.
pub(crate) async fn move_here(state: &AppState, block_id: &str) -> Result<AgentMoveHereResult, String> {
    let mstore = state.mstore.clone();
    let id = block_id.to_string();
    let block = tokio::task::spawn_blocking(move || mstore.get::<Block>(&id))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "block not found".to_string())?;
    let uid = obj::meta_get_string(&block.meta, "agentId", "");
    if uid.is_empty() {
        return Err("this pane has no agent".to_string());
    }
    let Some(holder) = persistent::lifecycle::local_holder_of(&uid, block_id) else {
        return Ok(AgentMoveHereResult { from_block: None });
    };
    let stopping = blockcontroller::get_controller(&holder)
        .as_ref()
        .and_then(|c| c.as_any().downcast_ref::<persistent::PersistentSubprocessController>())
        .is_some_and(|p| p.release_to_other_pane(Instant::now() + MOVE_GRACE));
    // A holder with no process to signal may already be exiting: the wait below decides.
    tracing::info!(block_id, holder = %holder, stopping, "agent_admission.move_here: asked the holder pane to let go");

    let deadline = Instant::now() + MOVE_WAIT;
    while persistent::lifecycle::local_holder_of(&uid, block_id).is_some() {
        if Instant::now() >= deadline {
            return Err("The other pane did not let go in time. Try again in a few seconds.".to_string());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    // The pane that lost the agent says so and offers the way back.
    super::agent_takeover::surface_failure(state, &holder, &persistent::held_elsewhere_error(block_id, false));
    Ok(AgentMoveHereResult { from_block: Some(holder) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::tests::test_state;

    #[tokio::test]
    async fn a_pane_with_no_agent_is_refused() {
        let state = test_state();
        let e = move_here(&state, "mh-no-such-block").await.unwrap_err();
        assert_eq!(e, "block not found");
    }
}
