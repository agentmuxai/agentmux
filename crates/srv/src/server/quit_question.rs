// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What the launcher asks before the tray's Quit: which agents a quit stops, and whether to ask.
//!
//! Quitting stops every agent. A window's close warns first when it stops agents in the middle of
//! a turn (`frontend/app/window/window-close.ts`). The tray's Quit never passes a page, and it is
//! most often used with no window open, when the agents running are the ones nobody is looking at.
//! So the launcher asks here, and puts the question in a dialog of its own
//! (`crates/launcher/src/quit_ask.rs`).

use axum::extract::State;
use axum::response::Json;
use serde_json::{json, Value};

use super::AppState;

/// `GET` this.
pub(crate) const PATH: &str = "/agentmux/quit-question";

/// The close warning's setting. Its "Don't ask again" turns this question off too.
const CONFIRM_SETTING: &str = "window:confirmcloseworking";

/// The answer, from each running agent's `(name, mid-turn)`: `ask` (an agent is mid-turn and the
/// warning is on), `working` (their names, sorted) and `agents` (how many agents the quit stops).
pub(crate) fn question(agents: &[(String, bool)], confirm: bool) -> Value {
    let mut working: Vec<&str> = agents.iter().filter(|(_, working)| *working).map(|(name, _)| name.as_str()).collect();
    working.sort_unstable();
    json!({ "ask": confirm && !working.is_empty(), "working": working, "agents": agents.len() })
}

pub(super) async fn handle_quit_question(State(state): State<AppState>) -> Json<Value> {
    let agents: Vec<(String, bool)> = crate::backend::blockcontroller::get_all_controllers()
        .keys()
        .filter_map(|block_id| super::service::session_closed::agent_of(&state.mstore, block_id))
        .collect();
    let confirm = state.config_watcher.get_full_config().settings.extra.get(CONFIRM_SETTING) != Some(&Value::Bool(false));
    Json(question(&agents, confirm))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agents(list: &[(&str, bool)]) -> Vec<(String, bool)> {
        list.iter().map(|(name, working)| (name.to_string(), *working)).collect()
    }

    #[test]
    fn it_asks_only_when_an_agent_is_mid_turn_and_names_those() {
        let q = question(&agents(&[("Loap", true), ("Korp", true), ("Camper", false)]), true);
        assert_eq!(q, json!({ "ask": true, "working": ["Korp", "Loap"], "agents": 3 }));
        let idle = question(&agents(&[("Camper", false)]), true);
        assert_eq!(idle, json!({ "ask": false, "working": [], "agents": 1 }));
        assert_eq!(question(&[], true), json!({ "ask": false, "working": [], "agents": 0 }));
    }

    #[test]
    fn dont_ask_again_turns_the_question_off_and_still_says_who_is_working() {
        let q = question(&agents(&[("Korp", true)]), false);
        assert_eq!(q, json!({ "ask": false, "working": ["Korp"], "agents": 1 }));
    }
}
