// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! How a Drone run is stopped. It lives in the core so the run ledger
//! (`backend/jobs`: a job's timeout, `cancel_other`, a stop from the Jobs tab) can
//! stop the run without reaching up into `drone/`; the executor re-exports it.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use tokio_util::sync::CancellationToken;

/// How a run is stopped: a flag the scheduler looks at before each block and
/// that ends the block in flight, with the reason it was set for.
#[derive(Clone, Default)]
pub struct Cancel {
    token: CancellationToken,
    reason: Arc<Mutex<Option<String>>>,
}

impl Cancel {
    /// Stop the run, for `reason`. The first reason stays. `false` when the run
    /// was stopped already.
    pub fn cancel(&self, reason: &str) -> bool {
        let mut slot = self.reason.lock().unwrap_or_else(|e| e.into_inner());
        if self.token.is_cancelled() {
            return false;
        }
        *slot = Some(reason.to_string());
        self.token.cancel();
        true
    }

    pub fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }

    pub async fn cancelled(&self) {
        self.token.cancelled().await;
    }

    pub fn reason(&self) -> String {
        self.reason.lock().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_else(|| "stopped".to_string())
    }
}

/// The runs going now, by id.
static LIVE: LazyLock<Mutex<HashMap<String, Cancel>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// A run has started: `cancel_run` can stop it until `forget_run`.
pub fn register_run(run_id: &str, cancel: &Cancel) {
    LIVE.lock().unwrap_or_else(|e| e.into_inner()).insert(run_id.to_string(), cancel.clone());
}

/// A run has ended.
pub fn forget_run(run_id: &str) {
    LIVE.lock().unwrap_or_else(|e| e.into_inner()).remove(run_id);
}

/// Stop the run `run_id` for `reason` (a Drone run, not a ledger row). The block
/// in flight ends, an Agent block's process with it, and the run ends
/// `cancelled`. `false` when no such run is going.
pub fn cancel_run(run_id: &str, reason: &str) -> bool {
    let cancel = LIVE.lock().unwrap_or_else(|e| e.into_inner()).get(run_id).cloned();
    cancel.is_some_and(|c| c.cancel(reason))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_is_stopped_once_and_the_first_reason_stays() {
        let cancel = Cancel::default();
        assert!(!cancel.is_cancelled());
        assert!(cancel.cancel("a person"));
        assert!(!cancel.cancel("the timeout"));
        assert!(cancel.is_cancelled());
        assert_eq!(cancel.reason(), "a person");
    }

    #[test]
    fn only_a_registered_run_can_be_cancelled_by_id() {
        let cancel = Cancel::default();
        assert!(!cancel_run("core-test-run", "x"), "not registered yet");
        register_run("core-test-run", &cancel);
        assert!(cancel_run("core-test-run", "x"));
        assert!(cancel.is_cancelled());
        forget_run("core-test-run");
        assert!(!cancel_run("core-test-run", "again"), "a forgotten run is not kept");
    }
}
