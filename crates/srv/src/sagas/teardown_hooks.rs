// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The sagas' side of `backend::teardown_hooks`: the watchdog, the controller replace and the tab close call
//! these through the hook, so `backend/` does not name the agent teardown.

use std::sync::Arc;

use crate::backend::blockcontroller::Controller;
use crate::backend::teardown_hooks::TeardownHooks;

struct Hooks;

/// The teardown's entries, for `backend::teardown_hooks::install`.
pub(crate) fn hooks() -> Arc<dyn TeardownHooks> {
    Arc::new(Hooks)
}

/// Register them. Called once at startup, before any controller runs.
pub(crate) fn install() {
    crate::backend::teardown_hooks::install(hooks());
}

impl TeardownHooks for Hooks {
    fn stop_now(&self, block_id: &str, graceful: bool) -> Result<(), String> {
        super::agent_teardown::stop_now(block_id, graceful)
    }

    fn replace_now(&self, ctrl: &dyn Controller) -> Result<(), String> {
        super::agent_teardown::replace_now(ctrl)
    }

    fn discard(&self, block_id: &str) {
        super::agent_teardown::discard(block_id)
    }
}
