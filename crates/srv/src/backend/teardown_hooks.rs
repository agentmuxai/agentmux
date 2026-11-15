// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What the controller code asks of the agent teardown.
//!
//! The watchdog, the controller replace and the tab close below `sagas/` each end an agent's processes
//! through the one teardown path (`sagas/agent_teardown.rs`, which a structural test pins as the only place
//! that stops them). This module is the seam: `backend/` names the [`TeardownHooks`] it needs, and the sagas
//! register them once at startup ([`install`]), so `backend/` stops naming `sagas` (Area54 #1350).

use std::sync::{Arc, OnceLock};

use crate::backend::blockcontroller::Controller;

/// The teardown's entries for callers that have no `AppState`.
pub trait TeardownHooks: Send + Sync {
    /// `controller.stop()` and nothing else, as the watchdog and the pane's `AgentStop` ask. Errors with
    /// `NOT_RUNNING` when the block has no controller.
    fn stop_now(&self, block_id: &str, graceful: bool) -> Result<(), String>;

    /// Stop the old controller's own CLI process only, for a controller about to be replaced.
    fn replace_now(&self, ctrl: &dyn Controller) -> Result<(), String>;

    /// Drop a block's controller and process tracker at once, with no graceful phase.
    fn discard(&self, block_id: &str);
}

static HOOKS: OnceLock<Arc<dyn TeardownHooks>> = OnceLock::new();

/// Register the teardown's entries. Later calls change nothing.
pub fn install(hooks: Arc<dyn TeardownHooks>) {
    let _ = HOOKS.set(hooks);
}

/// The registered entries, or ones that stop and drop nothing. The sagas register them at the start of
/// startup, before any controller runs, so [`NotReady`] is only what a caller reads if that registration is
/// somehow lost. Under test, [`PlainEntries`] stand in until something registers.
pub(crate) fn hooks() -> Arc<dyn TeardownHooks> {
    if let Some(h) = HOOKS.get() {
        return h.clone();
    }
    #[cfg(any(test, feature = "test-support"))]
    {
        Arc::new(PlainEntries)
    }
    #[cfg(not(any(test, feature = "test-support")))]
    {
        Arc::new(NotReady)
    }
}

struct NotReady;

impl TeardownHooks for NotReady {
    fn stop_now(&self, block_id: &str, _: bool) -> Result<(), String> {
        Err(format!("NOT_RUNNING: AgentMux isn't ready to stop block {block_id} yet"))
    }
    fn replace_now(&self, _: &dyn Controller) -> Result<(), String> {
        Err("AgentMux isn't ready to replace a controller yet".to_string())
    }
    fn discard(&self, _: &str) {}
}

/// What a controller test gets when nothing registered: a replace stops the old controller's own CLI process
/// the way the agent teardown's replace does, and nothing else is stopped or dropped.
#[cfg(any(test, feature = "test-support"))]
struct PlainEntries;

#[cfg(any(test, feature = "test-support"))]
impl TeardownHooks for PlainEntries {
    fn stop_now(&self, block_id: &str, _: bool) -> Result<(), String> {
        Err(format!("NOT_RUNNING: no controller for block {block_id}"))
    }
    fn replace_now(&self, ctrl: &dyn Controller) -> Result<(), String> {
        ctrl.stop_for_replace(crate::backend::blockcontroller::STATUS_DONE)
    }
    fn discard(&self, _: &str) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lost_registration_stops_and_drops_nothing_and_says_so() {
        let not_ready: &dyn TeardownHooks = &NotReady;
        let err = not_ready.stop_now("blk-hook", true).unwrap_err();
        assert!(err.starts_with("NOT_RUNNING"), "{err}");
        not_ready.discard("blk-hook");
    }

    #[test]
    fn under_test_plain_entries_stand_in_until_something_registers() {
        let err = hooks().stop_now("no-such-block-hook", false).unwrap_err();
        assert!(err.starts_with("NOT_RUNNING: no controller for block no-such-block-hook"), "{err}");
        hooks().discard("no-such-block-hook");
    }
}
