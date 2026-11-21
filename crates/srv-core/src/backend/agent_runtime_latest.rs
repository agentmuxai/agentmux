// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The latest `agentruntime` event of each pane, kept in memory so a reader that did not subscribe (the
//! live facts `Agents list` and `Agents status` read) can ask what a pane runs with now: what it was
//! asked to run (`model`, `effort`, `permission_mode`) and what its CLI reported using
//! (`effective_model`, `effective_effort`). Written where the event is published, so it is as current as
//! the event; lost with the process, like the event's own replay.

use std::collections::HashMap;
use std::sync::LazyLock;

use crate::backend::agent_runtime::AgentRuntimeEvent;

static LATEST: LazyLock<parking_lot::Mutex<HashMap<String, AgentRuntimeEvent>>> = LazyLock::new(Default::default);

/// Remember `event` as its pane's latest.
pub fn record(event: &AgentRuntimeEvent) {
    LATEST.lock().insert(event.blockid.clone(), event.clone());
}

/// `block_id`'s latest event, if one was published since srv started.
pub fn get(block_id: &str) -> Option<AgentRuntimeEvent> {
    LATEST.lock().get(block_id).cloned()
}

/// Forget `block_id` (its pane closed).
pub fn forget(block_id: &str) {
    LATEST.lock().remove(block_id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::agent_runtime::agent_runtime_event;

    #[test]
    fn the_latest_event_of_a_pane_replaces_the_one_before_and_forget_drops_it() {
        let id = format!("latest-{}", uuid::Uuid::new_v4());
        assert!(get(&id).is_none());
        record(&agent_runtime_event(&id, None, false, None));
        assert!(!get(&id).unwrap().running);
        let mut later = agent_runtime_event(&id, None, true, None);
        later.model = Some("claude-sonnet-5-5".to_string());
        record(&later);
        assert_eq!(get(&id).unwrap().model.as_deref(), Some("claude-sonnet-5-5"));
        assert!(get(&id).unwrap().restart_pending);
        forget(&id);
        assert!(get(&id).is_none());
    }
}
