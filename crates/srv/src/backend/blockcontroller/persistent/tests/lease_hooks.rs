// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What the tests of the layer above (`server::agent_move_here`) need from a persistent pane: to
//! make it hold an agent's lease and to see how it is asked to stop. A child of the controller's
//! test module so it reaches the controller's private state without widening it.

use super::super::*;

/// The stop request a pane given a lease by `hold_lease_for_test` receives.
pub(crate) struct StopRequest(tokio::sync::oneshot::Receiver<KillRequest>);

impl StopRequest {
    /// Waits for the request; whether it was a graceful stop (stdin closed, then a deadline) and not a kill.
    pub(crate) async fn was_graceful(self) -> bool {
        matches!(self.0.await, Ok(KillRequest::Graceful(_)))
    }
}

impl PersistentSubprocessController {
    /// For the tests of the layer above (`server::agent_move_here`): make this pane hold `held` as if
    /// its process were running, and hand back what its stop request will say.
    pub(crate) fn hold_lease_for_test(&self, held: crate::backend::agent_admission::HeldAgentLease) -> StopRequest {
        let (kill_tx, kill_rx) = tokio::sync::oneshot::channel::<KillRequest>();
        let mut g = self.inner.lock().unwrap();
        g.agent_lease = Some(Arc::new(held));
        g.kill_tx = Some(kill_tx);
        StopRequest(kill_rx)
    }

    /// The process this pane stood for has exited: its lease is dropped.
    pub(crate) fn drop_lease_for_test(&self) {
        self.inner.lock().unwrap().agent_lease = None;
    }
}
