// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! An agent whose pane is open but whose controller is not there yet.
//!
//! A pane registers its agent as it mounts; its controller is made by the
//! resync that follows. When AgentMux starts with panes to resume, that gap
//! is seconds long, for every agent at once. A message for such an agent
//! can't be delivered (the sender answers [`NO_CONTROLLER`]), and that is not
//! a refusal: the agent is starting. So what is held for it is not tried
//! ([`awaits_controller`]), a failure that slips through is not counted
//! against the message ([`is_no_controller`]), and the controller registering
//! wakes whatever waits ([`controller_registered`]).

use std::sync::{Arc, OnceLock};

/// How the delivery path says a block has no controller (`bootstrap::delivery`).
pub const NO_CONTROLLER: &str = "no controller for block";

type HasController = Arc<dyn Fn(&str) -> bool + Send + Sync>;

static HAS_CONTROLLER: OnceLock<HasController> = OnceLock::new();

/// Say how to ask whether a block has a controller. Set once, when srv
/// starts; until then every block counts as having one.
pub fn set_has_controller(probe: HasController) {
    let _ = HAS_CONTROLLER.set(probe);
}

/// Whether `block_id`, where an agent is registered, still waits for its
/// controller. A lookup: no rate-limit token, no audit entry.
pub fn awaits_controller(block_id: &str) -> bool {
    HAS_CONTROLLER.get().is_some_and(|has| !has(block_id))
}

/// Whether a delivery error is the delivery path finding no controller on
/// the block.
pub fn is_no_controller(error: &str) -> bool {
    error.starts_with(NO_CONTROLLER)
}

/// A controller registered: wake the replay of held messages, held input and
/// pending cron fires, which left its agent alone until now. No I/O.
pub fn controller_registered() {
    super::held_jekt_wake().notify_one();
    super::held_input_wake().notify_one();
    crate::backend::jobs::pending_wake().notify_one();
}

/// For tests: blocks a test says have no controller yet. The probe this
/// installs answers "has one" for every other block, so tests running beside
/// it see no difference.
#[cfg(any(test, feature = "test-support"))]
pub mod testing {
    use std::collections::HashSet;
    use std::sync::{Arc, LazyLock, Mutex};

    static WAITING: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(Default::default);

    fn waiting() -> std::sync::MutexGuard<'static, HashSet<String>> {
        WAITING.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `block_id` has no controller from now on.
    pub fn without_controller(block_id: &str) {
        super::set_has_controller(Arc::new(|block_id: &str| !waiting().contains(block_id)));
        waiting().insert(block_id.to_string());
    }

    /// `block_id`'s controller registered.
    pub fn controller_arrived(block_id: &str) {
        waiting().remove(block_id);
        super::controller_registered();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_block_waits_only_while_the_probe_says_it_has_no_controller() {
        let block = format!("cw-{}", uuid::Uuid::new_v4());
        testing::without_controller(&block);
        assert!(awaits_controller(&block));
        assert!(!awaits_controller("some-other-block"), "every other block is untouched");
        testing::controller_arrived(&block);
        assert!(!awaits_controller(&block));
    }

    #[test]
    fn the_no_controller_error_is_recognised_and_waits_like_a_starting_agent() {
        let error = format!("{NO_CONTROLLER} blk-1");
        assert!(is_no_controller(&error));
        assert!(crate::backend::reactive::is_transient_delivery_error(&error), "retried, never counted as a refusal");
        for other in ["agent not found: x", "persistent process not running", "NOT_RUNNING: no controller for block x", ""] {
            assert!(!is_no_controller(other), "{other:?}");
        }
    }
}
