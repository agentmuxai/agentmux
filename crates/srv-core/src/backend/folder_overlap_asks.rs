// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! An agent's ask for the person's go-ahead with a host agent that shares a folder with a container agent
//! (`backend::folder_overlap`), waiting for the person.
//!
//! The agent names the pair and srv keeps the ask here; the window shows it as a card in the asking agent's pane. The
//! person's answer reaches srv only through the window (`folderoverlapaccept`, `folderoverlapdecline`), which ends
//! the ask. An agent can ask and look again; it can never answer. A declined pair is not asked about again for a
//! while, and the pair stays unchosen: every other refusal for it stands.
//!
//! An ask with no asking agent (`agent` empty) is one AgentMux made itself: restart resume left the host agent stopped
//! for want of the person's go-ahead. It waits in the host agent's own block until the person answers, and neither
//! ends with the hour nor takes the room of the agents' asks.

use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

/// A host agent and a container agent, and the agent that asked the person to go ahead with them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct FolderAsk {
    pub host_id: String,
    pub host_name: String,
    pub container_id: String,
    pub container_name: String,
    /// The agent that asked, by its name; empty for the ask restart resume makes.
    pub agent: String,
    /// The asking agent's pane, where the card shows.
    pub block_id: String,
    #[ts(type = "number")]
    pub requested_ms: u64,
}

/// What an ask comes to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Asked {
    /// The person is asked: the card is shown. `new` when this ask made it, not one already waiting.
    Waiting { ask: FolderAsk, new: bool },
    /// The person said no to this pair a short while ago; it is not asked again until then.
    Declined { retry_after_ms: u64 },
}

/// At most this many asks wait at once; the oldest goes first.
const MAX_OPEN: usize = 16;
/// An ask nobody answers ends after this long.
const OPEN_MS: u64 = 60 * 60 * 1000;
/// After the person says no, the pair is not asked about again for this long.
pub const QUIET_MS: u64 = 10 * 60 * 1000;

#[derive(Default)]
struct State {
    open: Vec<FolderAsk>,
    /// The pairs the person said no to, and when.
    declined: Vec<(String, String, u64)>,
}

#[derive(Default)]
pub struct Asks {
    state: Mutex<State>,
}

pub fn asks() -> &'static Asks {
    static ASKS: OnceLock<Asks> = OnceLock::new();
    ASKS.get_or_init(Asks::default)
}

fn same_pair(ask: &FolderAsk, host_id: &str, container_id: &str) -> bool {
    ask.host_id == host_id && ask.container_id == container_id
}

impl Asks {
    fn lock(&self, now_ms: u64) -> std::sync::MutexGuard<'_, State> {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.open.retain(|ask| ask.agent.is_empty() || now_ms.saturating_sub(ask.requested_ms) < OPEN_MS);
        state.declined.retain(|(_, _, at)| now_ms.saturating_sub(*at) < QUIET_MS);
        state
    }

    /// Asks the person. A pair that is already waiting is asked once: the newest asker's pane shows the card. A pair
    /// the person said no to a short while ago is not asked again.
    pub fn ask(&self, ask: FolderAsk, now_ms: u64) -> Asked {
        let mut state = self.lock(now_ms);
        if let Some((_, _, at)) = state.declined.iter().find(|(h, c, _)| *h == ask.host_id && *c == ask.container_id) {
            return Asked::Declined { retry_after_ms: QUIET_MS.saturating_sub(now_ms.saturating_sub(*at)) };
        }
        if let Some(kept) = state.open.iter_mut().find(|k| same_pair(k, &ask.host_id, &ask.container_id)) {
            // An ask restart resume made stays where it is, in the stopped agent's block, until the person answers; the
            // asks of agents move to the newest asker's pane.
            if !kept.agent.is_empty() && !ask.agent.is_empty() {
                kept.agent = ask.agent;
                kept.block_id = ask.block_id;
            }
            return Asked::Waiting { ask: kept.clone(), new: false };
        }
        if state.open.iter().filter(|kept| !kept.agent.is_empty()).count() >= MAX_OPEN {
            if let Some(oldest) = state.open.iter().position(|kept| !kept.agent.is_empty()) {
                state.open.remove(oldest);
            }
        }
        state.open.push(ask.clone());
        Asked::Waiting { ask, new: true }
    }

    /// The asks waiting for the person.
    pub fn list(&self, now_ms: u64) -> Vec<FolderAsk> {
        self.lock(now_ms).open.clone()
    }

    /// The person's answer for the pair: ends its ask, and a no is remembered for a while. Returns whether an ask
    /// waited.
    pub fn answer(&self, host_id: &str, container_id: &str, go_ahead: bool, now_ms: u64) -> bool {
        let mut state = self.lock(now_ms);
        let before = state.open.len();
        state.open.retain(|ask| !same_pair(ask, host_id, container_id));
        state.declined.retain(|(h, c, _)| h != host_id || c != container_id);
        if !go_ahead {
            state.declined.push((host_id.to_string(), container_id.to_string(), now_ms));
        }
        state.open.len() != before
    }

    /// Ends every ask and every no that names agent `id`: the agent is deleted.
    pub fn forget_agent(&self, id: &str) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.open.retain(|ask| ask.host_id != id && ask.container_id != id);
        state.declined.retain(|(h, c, _)| h != id && c != id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(host: &str, container: &str, agent: &str, block: &str, at: u64) -> FolderAsk {
        FolderAsk {
            host_id: host.into(),
            host_name: format!("{host}-name"),
            container_id: container.into(),
            container_name: format!("{container}-name"),
            agent: agent.into(),
            block_id: block.into(),
            requested_ms: at,
        }
    }

    #[test]
    fn a_pair_is_asked_once_and_the_newest_asker_shows_the_card() {
        let asks = Asks::default();
        assert!(matches!(asks.ask(ask("h", "c", "Lead", "b1", 10), 10), Asked::Waiting { new: true, .. }));
        let Asked::Waiting { ask: kept, new } = asks.ask(ask("h", "c", "Other", "b2", 20), 20) else { panic!("asked") };
        assert!(!new);
        assert_eq!((kept.agent.as_str(), kept.block_id.as_str(), kept.requested_ms), ("Other", "b2", 10));
        assert_eq!(asks.list(20).len(), 1);
    }

    #[test]
    fn an_answer_ends_the_ask_and_a_no_is_remembered_for_a_while() {
        let asks = Asks::default();
        asks.ask(ask("h", "c", "Lead", "b1", 0), 0);
        assert!(asks.answer("h", "c", false, 100));
        assert!(asks.list(100).is_empty());
        assert_eq!(asks.ask(ask("h", "c", "Lead", "b1", 200), 200), Asked::Declined { retry_after_ms: QUIET_MS - 100 });
        assert!(matches!(asks.ask(ask("h", "c", "Lead", "b1", 100 + QUIET_MS), 100 + QUIET_MS), Asked::Waiting { .. }));
    }

    #[test]
    fn an_agents_ask_for_a_pair_restart_resume_is_waiting_on_leaves_that_ask_where_it_is() {
        let asks = Asks::default();
        asks.ask(ask("h", "c", "", "parked-block", 0), 0);
        let Asked::Waiting { ask: kept, new } = asks.ask(ask("h", "c", "Lead", "lead-pane", 10), 10) else { panic!("asked") };
        assert!(!new);
        assert_eq!((kept.agent.as_str(), kept.block_id.as_str(), kept.requested_ms), ("", "parked-block", 0));
        assert_eq!(asks.list(OPEN_MS + 1_000).len(), 1, "and it still does not end with the hour");
    }

    #[test]
    fn an_ask_restart_resume_makes_waits_for_the_person_and_takes_no_room_from_the_agents_asks() {
        let asks = Asks::default();
        asks.ask(ask("resumed", "c", "", "parked-block", 0), 0);
        for i in 0..MAX_OPEN {
            asks.ask(ask(&format!("h{i}"), "c", "Lead", "b", i as u64), i as u64);
        }
        asks.ask(ask("h-extra", "c", "Lead", "b", 99), 99);
        let soon = asks.list(100);
        assert_eq!(soon.len(), MAX_OPEN + 1, "the cap is for the agents' asks: the oldest of them went, the restart's stayed");
        assert!(soon.iter().any(|a| a.host_id == "resumed") && !soon.iter().any(|a| a.host_id == "h0"));
        let later = OPEN_MS + 1_000;
        let left = asks.list(later);
        assert_eq!(left.len(), 1, "the agents' asks ended with the hour and the oldest went first");
        assert_eq!((left[0].host_id.as_str(), left[0].agent.as_str(), left[0].block_id.as_str()), ("resumed", "", "parked-block"));
        assert!(asks.answer("resumed", "c", true, later));
        assert!(asks.list(later).is_empty(), "the person's answer ends it");
    }

    #[test]
    fn a_go_ahead_ends_the_ask_and_clears_an_old_no() {
        let asks = Asks::default();
        asks.answer("h", "c", false, 0);
        assert!(!asks.answer("h", "c", true, 10), "nothing waited");
        assert!(matches!(asks.ask(ask("h", "c", "Lead", "b1", 20), 20), Asked::Waiting { .. }));
    }

    #[test]
    fn an_ask_nobody_answers_ends_and_the_oldest_goes_when_too_many_wait() {
        let asks = Asks::default();
        asks.ask(ask("h", "c", "Lead", "b1", 0), 0);
        assert!(asks.list(OPEN_MS).is_empty());
        for n in 0..=MAX_OPEN {
            asks.ask(ask(&format!("h{n}"), "c", "Lead", "b1", 1), 1);
        }
        let waiting = asks.list(1);
        assert_eq!(waiting.len(), MAX_OPEN);
        assert!(waiting.iter().all(|a| a.host_id != "h0"));
    }

    #[test]
    fn deleting_an_agent_ends_its_asks_and_its_nos() {
        let asks = Asks::default();
        asks.ask(ask("h", "c", "Lead", "b1", 0), 0);
        asks.answer("h2", "c", false, 0);
        asks.forget_agent("c");
        assert!(asks.list(1).is_empty());
        assert!(matches!(asks.ask(ask("h2", "c", "Lead", "b1", 2), 2), Asked::Waiting { .. }));
    }
}
