// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The History pane's agent list changes when an agent's transcript grows, a
//! conversation is archived, or a fork takes a copy of its parent's. Those
//! writes are many (every output line) and the list wants one signal, so a
//! write only [`mark`]s, which costs one atomic store, and one task publishes
//! `history:sessions:changed` at most once per [`TICK`] while something was
//! marked. A window that shows the list reloads it on the event
//! (`frontend/app/view/history/history-agents.tsx`).
//!
//! A write by another AgentMux sharing the transcript store (another channel)
//! never reaches [`mark`]; the list's own safety poll covers it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::backend::mps::Broker;

/// The longest a change waits before the event announces it.
const TICK: Duration = Duration::from_secs(2);

/// Whether anything changed since the last announcement.
#[derive(Default)]
pub(crate) struct Changes(AtomicBool);

impl Changes {
    pub(crate) fn mark(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    /// Whether there is something to announce; clears it.
    pub(crate) fn take(&self) -> bool {
        self.0.swap(false, Ordering::Relaxed)
    }
}

static CHANGES: Changes = Changes(AtomicBool::new(false));

/// The history list may have changed. Cheap enough for the output path.
pub fn mark() {
    CHANGES.mark();
}

/// Publish the event once per [`TICK`] while something was marked.
pub fn spawn_publisher(broker: Arc<Broker>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            if CHANGES.take() {
                broker.publish_event::<crate::backend::mps_events::HistorySessionsChanged>(&(), vec![], 0);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn many_marks_are_one_announcement() {
        let changes = Changes::default();
        assert!(!changes.take(), "nothing changed: nothing to announce");
        for _ in 0..1000 {
            changes.mark();
        }
        assert!(changes.take());
        assert!(!changes.take(), "announced once, not once per mark");
        changes.mark();
        assert!(changes.take(), "a later change is announced again");
    }
}
