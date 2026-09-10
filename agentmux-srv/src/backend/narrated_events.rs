// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Remembers which events have already been narrated, so one background launch
//! produces one ambient narration.
//!
//! WHY THIS IS PROCESS-WIDE AND NOT A LOCAL IN `register_handlers`.
//! That function runs once per WebSocket CONNECTION, so a local set is reset by
//! every reconnect — and a reconnect is exactly when duplicates arrive. An
//! honored truncate clears the frontend's own `nodeIdSet` and re-parses the
//! transcript, re-emitting every node as new, which re-invokes the narrate RPC
//! for ids already narrated before the drop. Nothing else backstops it: the
//! `AmbientGateway` singleton only guards calls that are concurrently in
//! flight for a key, and releases its guard as soon as the first completes.
//! Living on `AppState` alongside `dock_snapshots` and `pending_background_pids`
//! is what makes the "narrate once" claim actually true. (reagent P1 on #3169.)
//!
//! Bounded, unlike a bare `HashSet`: this accumulates one entry per narrated
//! event for the lifetime of the process, which for a long-lived instance
//! running many background tasks is unbounded growth for a guard that only
//! needs recent history. Oldest entries are evicted first — a reconnect replays
//! recent nodes, not ancient ones, so forgetting the distant past costs nothing.

use std::collections::{HashSet, VecDeque};

use parking_lot::Mutex;

/// How many narrated event ids to remember.
///
/// Generous relative to what the guard actually needs (a reconnect replays the
/// current transcript, not the whole session) and still trivially small in
/// memory — a few hundred short strings.
const MAX_REMEMBERED: usize = 512;

#[derive(Default)]
struct Inner {
    seen: HashSet<String>,
    order: VecDeque<String>,
}

#[derive(Default)]
pub struct NarratedEvents {
    inner: Mutex<Inner>,
}

impl NarratedEvents {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record `event_id` and report whether it is NEW.
    ///
    /// `true` means "not seen before — go ahead and narrate". `false` means it
    /// has already been narrated and this call should do nothing.
    pub fn mark_new(&self, event_id: &str) -> bool {
        let mut g = self.inner.lock();
        if !g.seen.insert(event_id.to_string()) {
            return false;
        }
        g.order.push_back(event_id.to_string());
        while g.order.len() > MAX_REMEMBERED {
            if let Some(evicted) = g.order.pop_front() {
                g.seen.remove(&evicted);
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_sighting_is_new_and_the_second_is_not() {
        let n = NarratedEvents::new();
        assert!(n.mark_new("toolu_1"));
        assert!(!n.mark_new("toolu_1"));
    }

    #[test]
    fn distinct_events_are_each_new() {
        let n = NarratedEvents::new();
        assert!(n.mark_new("toolu_1"));
        assert!(n.mark_new("toolu_2"));
    }

    #[test]
    fn it_stays_bounded_and_evicts_oldest_first() {
        let n = NarratedEvents::new();
        for i in 0..(MAX_REMEMBERED + 10) {
            assert!(n.mark_new(&format!("id{i}")));
        }
        // The oldest have been forgotten — re-narrating one is the accepted
        // cost of not growing without bound.
        assert!(n.mark_new("id0"));
        // Recent ones are still remembered, which is what the guard is for.
        let newest = format!("id{}", MAX_REMEMBERED + 9);
        assert!(!n.mark_new(&newest));
    }
}
