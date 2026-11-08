// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Moving an agent to another route or model in place (spec section 7a): what is recorded, and the guard on how
//! often an agent may do it. The harness never changes. Restarting the panes at a turn boundary is the caller's:
//! the choice is read at every spawn, so recording it is what makes the next launch use it.

use super::catalog::{self, Route};
use super::launch::{self, Choice};
use super::token;
use crate::backend::storage::content::AgentContent;
use crate::backend::storage::store::Store;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// The definition content type that holds the [`Previous`].
pub const PREVIOUS_TYPE: &str = "agent:model_route_previous";
/// How many times an agent may change its own model in an hour: each change costs the prompt cache (decision D12).
pub const AGENT_SWITCHES_PER_HOUR: usize = 6;
const HOUR: Duration = Duration::from_secs(3600);

/// What the agent ran on before its last change, for the panel's "Back" row. An empty `route` is the harness's own
/// sign-in, whose model is the agent's runtime setting and not kept here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Previous {
    pub route: String,
    pub model: String,
}

/// The previous route and model of the agent definition `def_id`.
pub fn previous_of(mstore: &Store, def_id: &str) -> Option<Previous> {
    let content = mstore.agent_content_get(def_id, PREVIOUS_TYPE).ok()??;
    serde_json::from_str(&content.content).ok()
}

/// Where a change takes an agent.
#[derive(Debug, Clone, Copy)]
pub enum Target<'a> {
    /// A model on a route (an empty model is the route's default).
    Route { route: &'a str, model: &'a str },
    /// The harness's own sign-in: no route.
    OwnLogin,
}

/// What a change did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Moved {
    pub from: Option<Choice>,
    pub to: Option<Choice>,
}

impl Moved {
    pub fn changed(&self) -> bool {
        self.from != self.to
    }
}

/// Records the agent's move to `target`, after the shared validation and with its old place kept as `previous`.
/// `key_present` says whether a route's key is in the Vault: a route without one is not moved to. Leaving a route
/// takes its gateway token away at once, so the running harness stops reaching the key.
pub fn move_to(mstore: &Store, def_id: &str, harness: &str, target: Target<'_>, key_present: impl Fn(&Route) -> bool) -> Result<Moved, String> {
    let from = launch::choice_of(mstore, def_id);
    let to = match target {
        Target::OwnLogin => None,
        Target::Route { route, model } => {
            let (route, model) = catalog::validate(harness, route, model)?;
            if !key_present(route) {
                return Err(format!("KEY_MISSING: route {} needs a Vault api-key entry labelled \"{}\", and the Vault has none", route.id, route.key_label));
            }
            Some(Choice { route: route.id.to_string(), model: model.id.to_string() })
        }
    };
    let moved = Moved { from, to };
    if !moved.changed() {
        return Ok(moved);
    }
    remember_previous(mstore, def_id, moved.from.as_ref())?;
    match &moved.to {
        Some(c) => {
            launch::set_choice(mstore, def_id, harness, &c.route, &c.model)?;
        }
        None => {
            mstore.agent_content_delete(def_id, launch::CONTENT_TYPE).map_err(|e| e.to_string())?;
        }
    }
    if let Some(left) = moved.from.as_ref().filter(|f| moved.to.as_ref().is_none_or(|t| t.route != f.route)) {
        token::revoke(def_id, &left.route);
    }
    Ok(moved)
}

fn remember_previous(mstore: &Store, def_id: &str, from: Option<&Choice>) -> Result<(), String> {
    let previous = from.map_or(Previous { route: String::new(), model: String::new() }, |c| Previous { route: c.route.clone(), model: c.model.clone() });
    let content = serde_json::to_string(&previous).map_err(|e| e.to_string())?;
    mstore
        .agent_content_set(&AgentContent { agent_id: def_id.to_string(), content_type: PREVIOUS_TYPE.to_string(), content, updated_at: agentmux_common::time::now_ms() })
        .map_err(|e| e.to_string())
}

fn switches() -> &'static Mutex<HashMap<String, VecDeque<Instant>>> {
    static SWITCHES: OnceLock<Mutex<HashMap<String, VecDeque<Instant>>>> = OnceLock::new();
    SWITCHES.get_or_init(Default::default)
}

/// Whether an agent may change its own model now: it has made fewer than [`AGENT_SWITCHES_PER_HOUR`] changes in the last
/// hour. Counts nothing; [`count_agent_switch`] is called once the change has been made, so a refused one costs none.
/// A person's change in the panel is not counted at all.
pub fn agent_switch_allowed(agent: &str) -> Result<(), String> {
    check_at(&mut switches().lock().unwrap(), agent, Instant::now())
}

/// Counts one change the agent made to its own model.
pub fn count_agent_switch(agent: &str) {
    switches().lock().unwrap().entry(agent.to_string()).or_default().push_back(Instant::now());
}

fn check_at(all: &mut HashMap<String, VecDeque<Instant>>, agent: &str, now: Instant) -> Result<(), String> {
    let recent = all.entry(agent.to_string()).or_default();
    while recent.front().is_some_and(|t| now.duration_since(*t) >= HOUR) {
        recent.pop_front();
    }
    if recent.len() >= AGENT_SWITCHES_PER_HOUR {
        return Err(format!(
            "an agent may change its model {AGENT_SWITCHES_PER_HOUR} times an hour, because each change makes the new model read the whole conversation again at full price; try again later"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FLASH: &str = "deepseek/deepseek-v4.1-flash-1017";
    const PRO: &str = "deepseek/deepseek-v4-pro-0813";

    /// A store holding agents "a1" and "a2": their content is kept on the definition row.
    fn store() -> Store {
        let store = Store::open_in_memory().unwrap();
        for id in ["a1", "a2"] {
            let mut def = crate::backend::storage::agents::test_agent_def(id, id, "claude", "host", 1, "");
            store.agent_def_insert(&mut def).unwrap();
        }
        store
    }

    #[test]
    fn a_move_keeps_where_the_agent_was_and_a_move_back_returns_to_it() {
        let s = store();
        let on = |model| Target::Route { route: "openrouter", model };
        let first = move_to(&s, "a1", "claude", on(FLASH), |_| true).unwrap();
        assert_eq!((first.from, first.to.unwrap().model), (None, FLASH.to_string()));
        assert_eq!(previous_of(&s, "a1"), Some(Previous { route: String::new(), model: String::new() }));
        let second = move_to(&s, "a1", "claude", on(PRO), |_| true).unwrap();
        assert!(second.changed());
        assert_eq!(previous_of(&s, "a1"), Some(Previous { route: "openrouter".into(), model: FLASH.into() }));
        assert_eq!(launch::choice_of(&s, "a1").unwrap().model, PRO);
        let left = move_to(&s, "a1", "claude", Target::OwnLogin, |_| true).unwrap();
        assert_eq!(left.to, None);
        assert!(launch::choice_of(&s, "a1").is_none());
        assert_eq!(previous_of(&s, "a1").unwrap().model, PRO);
    }

    #[test]
    fn staying_where_it_is_changes_nothing_and_forgets_nothing() {
        let s = store();
        move_to(&s, "a1", "claude", Target::Route { route: "openrouter", model: FLASH }, |_| true).unwrap();
        move_to(&s, "a1", "claude", Target::Route { route: "openrouter", model: PRO }, |_| true).unwrap();
        let again = move_to(&s, "a1", "claude", Target::Route { route: "openrouter", model: PRO }, |_| true).unwrap();
        assert!(!again.changed());
        assert_eq!(previous_of(&s, "a1").unwrap().model, FLASH, "a no-op is not a change of place");
        assert!(!move_to(&s, "a2", "claude", Target::OwnLogin, |_| true).unwrap().changed());
        assert!(previous_of(&s, "a2").is_none());
    }

    #[test]
    fn a_route_without_its_key_or_the_wrong_harness_is_not_moved_to() {
        let s = store();
        let e = move_to(&s, "a1", "claude", Target::Route { route: "openrouter", model: "" }, |_| false).unwrap_err();
        assert!(e.starts_with("KEY_MISSING") && e.contains("OpenRouter API key"), "{e}");
        assert!(move_to(&s, "a1", "codex", Target::Route { route: "openrouter", model: "" }, |_| true).is_err());
        assert!(launch::choice_of(&s, "a1").is_none() && previous_of(&s, "a1").is_none(), "a refused move leaves no trace");
    }

    #[test]
    fn an_agent_may_switch_six_times_an_hour_and_then_waits_and_a_refused_change_costs_nothing() {
        let mut all = HashMap::new();
        let start = Instant::now();
        for i in 0..AGENT_SWITCHES_PER_HOUR {
            let at = start + Duration::from_secs(i as u64);
            check_at(&mut all, "x", at).unwrap();
            all.get_mut("x").unwrap().push_back(at);
        }
        let refused = check_at(&mut all, "x", start + Duration::from_secs(60)).unwrap_err();
        assert!(refused.contains("6 times an hour"), "{refused}");
        assert_eq!(all["x"].len(), AGENT_SWITCHES_PER_HOUR, "checking counts nothing");
        check_at(&mut all, "y", start + Duration::from_secs(60)).unwrap();
        check_at(&mut all, "x", start + HOUR + Duration::from_secs(1)).unwrap();
    }
}
