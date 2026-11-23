// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What the service route's `QuitAgent` and `ShutdownAgent` may ask of the person: the same who-may-stop-whom as
//! `Stop agents` (an agent's pane, never the caller's own, asked by an agent the token names), one countdown per
//! agent, and a limit on the countdowns one caller opens.

use std::collections::{HashMap, VecDeque};
use std::sync::{LazyLock, Mutex};

use crate::backend::obj::Block;
use crate::backend::service::WebReturnType;
use crate::server::AppState;
use agentmux_common::time::now_ms;

/// Who asked, for the person's countdown, when the request carries no agent credential.
const UNVERIFIED_CALLER: &str = "an unverified caller";

/// The countdowns a caller may open in `WINDOW_MS`: an agent, and a request with no agent credential (all of them
/// count together, as nothing tells them apart).
const WINDOW_MS: i64 = 60_000;
const AGENT_OPENS: usize = 10;
const UNVERIFIED_OPENS: usize = 3;

/// When each caller opened its countdowns inside the window, oldest first.
#[derive(Default)]
struct Limiter(Mutex<HashMap<String, VecDeque<i64>>>);

impl Limiter {
    /// Counts one more countdown for `key`, or says how many ms until its oldest one leaves the window.
    fn take(&self, key: &str, limit: usize, now: i64) -> Result<(), i64> {
        let mut map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        map.retain(|_, opened| {
            while opened.front().is_some_and(|t| now - t >= WINDOW_MS) {
                opened.pop_front();
            }
            !opened.is_empty()
        });
        let opened = map.entry(key.to_string()).or_default();
        if opened.len() >= limit {
            return Err(WINDOW_MS - (now - opened[0]));
        }
        opened.push_back(now);
        Ok(())
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

/// Opens the person's countdown for the agent in `block_id`, in the name of the agent the request's token names (or as
/// an unverified caller), or says why not.
pub(super) fn ask_the_person_to_stop(state: &AppState, block_id: &str, caller_uid: Option<&str>, method: &str) -> Result<WebReturnType, WebReturnType> {
    if crate::backend::blockcontroller::get_controller(block_id).is_none() {
        return Err(refused(format!("{method}: no running agent in block {block_id}")));
    }
    if !crate::sagas::orphan_reaper::is_agent_block(state, block_id) {
        return Err(refused(format!("{method}: not an agent pane: close it with Stop with action \"pane\"")));
    }
    let named = match caller_uid {
        Some(uid) => match state.mstore.agent_names_by_id(uid).ok().flatten().filter(|row| !row.slug.is_empty()) {
            Some(row) => Some((uid, row.slug)),
            None => return Err(refused(format!("{method}: FORBIDDEN: the token's agent is unknown"))),
        },
        None => None,
    };
    if let (Some((uid, slug)), Some(target)) = (&named, agent_of(state, block_id)) {
        if target == *uid || target == *slug {
            return Err(refused(format!("{method}: your own block: to end your own session use Stop with action \"self\"")));
        }
    }
    let by = named.as_ref().map(|(_, slug)| slug.clone()).unwrap_or_else(|| UNVERIFIED_CALLER.to_string());
    // A request for an agent that already has a countdown joins it and opens nothing, so it is not counted.
    if crate::sagas::pending_shutdown::active_for(block_id).is_none() {
        let (key, limit) = match &named {
            Some((uid, _)) => (*uid, AGENT_OPENS),
            None => ("", UNVERIFIED_OPENS),
        };
        if let Err(wait_ms) = LIMITER.take(key, limit, now_ms()) {
            return Err(refused(format!(
                "{method}: RATE_LIMITED: {by} has opened {limit} stop countdowns in the last minute; wait {} s, or leave it to the person",
                wait_ms / 1000 + 1
            )));
        }
    }
    let action = crate::sagas::pending_shutdown::Action::CloseTab { detail: format!("asked by {by} ({method}, on the service route)") };
    let pending = crate::sagas::pending_shutdown::request(state, block_id, &by, "Service", "", action);
    Ok(WebReturnType::success(serde_json::json!({ "pending": pending })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_caller_may_open_its_limit_in_a_minute_and_a_slot_frees_as_the_oldest_leaves() {
        let limiter = Limiter::default();
        for t in 0..3 {
            assert!(limiter.take("a", 3, t * 1_000).is_ok());
        }
        assert_eq!(limiter.take("a", 3, 10_000), Err(WINDOW_MS - 10_000), "the fourth waits for the first to leave the minute");
        assert!(limiter.take("b", 3, 10_000).is_ok(), "another caller has its own count");
        assert!(limiter.take("a", 3, WINDOW_MS).is_ok(), "the first has left the window");
        assert!(limiter.take("a", 3, WINDOW_MS + 500).is_err(), "and the one that took its place counts");
    }

    #[test]
    fn a_request_with_no_agent_credential_gets_fewer_than_an_agent() {
        assert!(UNVERIFIED_OPENS < AGENT_OPENS);
    }
}
