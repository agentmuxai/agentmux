// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What the service route's `QuitAgent` and `ShutdownAgent` may ask of the person: the same who-may-stop-whom as
//! `Stop agents` (asked by an agent the token names, of an agent's pane that is not the caller's own), one countdown
//! per agent, and a limit on the countdowns one caller opens.

use std::collections::{HashMap, VecDeque};
use std::sync::{LazyLock, Mutex};

use crate::backend::obj::Block;
use crate::backend::service::WebReturnType;
use crate::server::AppState;
use agentmux_common::time::now_ms;

/// The countdowns an agent may open in `WINDOW_MS`.
const WINDOW_MS: i64 = 60_000;
const AGENT_OPENS: usize = 10;

/// When each caller opened its countdowns inside the window, oldest first.
#[derive(Default)]
struct Limiter(Mutex<HashMap<String, VecDeque<i64>>>);

impl Limiter {
    /// Whether `key` may open another countdown, or how many ms until its oldest one leaves the window. Counts nothing.
    fn room(&self, key: &str, limit: usize, now: i64) -> Result<(), i64> {
        let mut map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        Self::forget_old(&mut map, now);
        match map.get(key) {
            Some(opened) if opened.len() >= limit => Err(WINDOW_MS - (now - opened[0])),
            _ => Ok(()),
        }
    }

    /// Counts a countdown `key` opened.
    fn charge(&self, key: &str, now: i64) {
        let mut map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        Self::forget_old(&mut map, now);
        map.entry(key.to_string()).or_default().push_back(now);
    }

    fn forget_old(map: &mut HashMap<String, VecDeque<i64>>, now: i64) {
        map.retain(|_, opened| {
            while opened.front().is_some_and(|t| now - t >= WINDOW_MS) {
                opened.pop_front();
            }
            !opened.is_empty()
        });
    }
}

static LIMITER: LazyLock<Limiter> = LazyLock::new(Default::default);

fn refused(text: String) -> WebReturnType {
    WebReturnType::error(text)
}

/// The agent a pane belongs to.
fn agent_of(state: &AppState, block_id: &str) -> Option<String> {
    let from_meta = || {
        let block = state.mstore.get::<Block>(block_id).ok().flatten()?;
        Some(crate::backend::obj::meta_get_string(&block.meta, "agentId", "")).filter(|id| !id.is_empty())
    };
    state.reactive_handler.get_agent_by_block(block_id).map(|a| a.agent_id).or_else(from_meta)
}

/// Opens the person's countdown for the agent in `block_id`, in the name of the agent the request's token names, or
/// says why not.
pub(super) fn ask_the_person_to_stop(state: &AppState, block_id: &str, caller_uid: Option<&str>, method: &str) -> Result<WebReturnType, WebReturnType> {
    let Some(uid) = caller_uid else {
        return Err(refused(format!(
            "{method}: FORBIDDEN: the request must carry an agent's token (AGENTMUX_AGENT_TOKEN), as Stop agents does; or the person stops the agent from the window"
        )));
    };
    if crate::backend::blockcontroller::get_controller(block_id).is_none() {
        return Err(refused(format!("{method}: no running agent in block {block_id}")));
    }
    if !crate::sagas::orphan_reaper::is_agent_block(state, block_id) {
        return Err(refused(format!("{method}: not an agent pane: close it with Stop with action \"pane\"")));
    }
    let Some(by) = state.mstore.agent_names_by_id(uid).ok().flatten().map(|row| row.slug).filter(|slug| !slug.is_empty()) else {
        return Err(refused(format!("{method}: FORBIDDEN: the token's agent is unknown")));
    };
    if agent_of(state, block_id).is_some_and(|target| target == uid || target == by) {
        return Err(refused(format!("{method}: your own block: to end your own session use Stop with action \"self\"")));
    }
    // A request for an agent that already has a countdown joins it and opens nothing, so it needs no room; only a
    // countdown that is opened is counted.
    if crate::sagas::pending_shutdown::active_for(block_id).is_none() {
        if let Err(wait_ms) = LIMITER.room(uid, AGENT_OPENS, now_ms()) {
            return Err(refused(format!(
                "{method}: RATE_LIMITED: {by} has opened {AGENT_OPENS} stop countdowns in the last minute; wait {} s, or leave it to the person",
                wait_ms / 1000 + 1
            )));
        }
    }
    let action = crate::sagas::pending_shutdown::Action::CloseTab { detail: format!("asked by {by} ({method}, on the service route)") };
    let pending = crate::sagas::pending_shutdown::request(state, block_id, &by, "Service", "", action);
    if !pending.joined {
        LIMITER.charge(uid, now_ms());
    }
    Ok(WebReturnType::success(serde_json::json!({ "pending": pending })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_caller_may_open_its_limit_in_a_minute_and_a_slot_frees_as_the_oldest_leaves() {
        let limiter = Limiter::default();
        for t in 0..3 {
            assert!(limiter.room("a", 3, t * 1_000).is_ok());
            limiter.charge("a", t * 1_000);
        }
        assert_eq!(limiter.room("a", 3, 10_000), Err(WINDOW_MS - 10_000), "the fourth waits for the first to leave the minute");
        assert!(limiter.room("b", 3, 10_000).is_ok(), "another caller has its own count");
        assert!(limiter.room("a", 3, WINDOW_MS).is_ok(), "the first has left the window");
        limiter.charge("a", WINDOW_MS);
        assert!(limiter.room("a", 3, WINDOW_MS + 500).is_err(), "and the one that took its place counts");
    }

    #[test]
    fn asking_for_room_counts_nothing() {
        let limiter = Limiter::default();
        for _ in 0..10 {
            assert!(limiter.room("a", 1, 0).is_ok());
        }
    }
}
