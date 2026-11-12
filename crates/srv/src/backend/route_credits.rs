// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A model route whose key has no credits left (HTTP 402).
//!
//! Every agent on the route fails the same way until someone adds credits. Without this, only the agent
//! that happened to hit the 402 showed the failure; the others stayed idle and available, messages kept
//! starting turns that failed, and nobody was told.
//!
//! What follows from the first 402 of an **episode** (a route, from its first 402 to its next success):
//! - the failure (`agents::failure` classes it `UsageLimit` with `ROUTE_CREDITS_TITLE`, and
//!   `limit_reset` gives it a reset ten minutes ahead) is written to every other open pane on the route,
//!   so each shows as limited and holds its messages until the reset, when a turn probes the route;
//! - each distinct lead of those agents gets one `[AgentMux]` note naming its agents, and no note about
//!   the route after it (a worker's own failure note is left out, `worker_notes`);
//! - the first successful response on the route (`spend_watch::settled`) ends the episode and takes the
//!   failure off every pane that still shows it.
//!
//! An agent on no route (a harness on its own provider key) has an episode of its own, and is noted the
//! same way; a turn that works clears its own failure.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use crate::agents::failure::AgentFailure;
use crate::backend::agent_state::failure_of_block;
use crate::backend::eventbus::EventBus;
use crate::backend::model_gateway::{catalog, launch};
use crate::backend::mps::{Broker, MuxEvent};
use crate::backend::obj::{self, Block, Tab};
use crate::backend::storage::store::Store;

/// An episode with no success is let go after this long, so a route fixed while srv could not see it
/// (an agent that never ran again) is not silent about its next 402 forever.
const EPISODE_MAX_MS: u64 = 6 * 60 * 60 * 1000;

/// Episode key → when it began (unix ms). A route's id, or `agent:<definition id>` for an agent on no route.
static EPISODES: LazyLock<parking_lot::Mutex<HashMap<String, u64>>> = LazyLock::new(Default::default);

/// An episode that has just begun: who is on the route, for the notes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Opened {
    /// The route's label, or `None` for an agent on no route.
    pub route: Option<String>,
    /// The agents with an open pane there, as (definition id, name).
    pub agents: Vec<(String, String)>,
}

/// An open pane of an agent: (block id, definition id).
type Pane = (String, String);

/// The credits failure in a broker event, with the pane it is about.
pub(crate) fn credits_failure(event: &str, scopes: &[String], data: Option<&serde_json::Value>) -> Option<(String, AgentFailure)> {
    if event != crate::backend::mps::EVENT_AGENT_FAILURE {
        return None;
    }
    let failure: AgentFailure = serde_json::from_value(data?.clone()).ok()?;
    let block_id = scopes.iter().find_map(|s| s.strip_prefix("block:"))?.to_string();
    failure.is_route_credits().then_some((block_id, failure))
}

/// Start listening. Called once from `main`.
pub fn install(broker: Arc<Broker>, mstore: Arc<Store>, event_bus: Arc<EventBus>) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(String, AgentFailure)>();
    // Observers run inside `Broker::publish`, under its locks: parse and hand off only.
    broker.add_observer(Arc::new(move |ev: &MuxEvent| {
        if let Some(found) = credits_failure(&ev.event, &ev.scopes, ev.data.as_ref()) {
            let _ = tx.send(found);
        }
    }));
    tokio::spawn(async move {
        while let Some((block_id, failure)) = rx.recv().await {
            let (mstore, event_bus) = (mstore.clone(), event_bus.clone());
            let _ = tokio::task::spawn_blocking(move || {
                let now = agentmux_common::time::now_ms_u64();
                if let Some(opened) = on_failure(&mstore, &event_bus, &block_id, &failure, now) {
                    tell_leaders(&mstore, &opened);
                }
            })
            .await;
        }
    });
}

/// `block_id`'s agent hit the credits failure `failure`: write it to the other open panes on its route, and
/// say whether this opened the route's episode. BLOCKING (store reads and writes).
pub(crate) fn on_failure(mstore: &Arc<Store>, bus: &Arc<EventBus>, block_id: &str, failure: &AgentFailure, now: u64) -> Option<Opened> {
    let block = mstore.get::<Block>(block_id).ok()??;
    let def_id = obj::meta_get_string(&block.meta, "agentId", "");
    if def_id.is_empty() {
        return None;
    }
    let route = running_route(&block);
    let mut panes: Vec<Pane> = vec![(block_id.to_string(), def_id.clone())];
    for (other, other_def) in route.iter().flat_map(|r| panes_on(mstore, r)) {
        if other == block_id {
            continue;
        }
        if !shows_credits(mstore, &other) {
            crate::backend::blockcontroller::core::persist_last_failure(&other, Some(failure), &Some(mstore.clone()), &Some(bus.clone()));
        }
        panes.push((other, other_def));
    }
    let key = route.clone().unwrap_or_else(|| format!("agent:{def_id}"));
    if !open_episode(&key, now) {
        return None;
    }
    let mut seen = std::collections::HashSet::new();
    let agents = panes
        .into_iter()
        .filter(|(_, def)| seen.insert(def.clone()))
        .map(|(_, def)| {
            let name = mstore.agent_def_get(&def).ok().flatten().map_or_else(|| def.clone(), |d| d.name);
            (def, name)
        })
        .collect();
    let route = route.map(|r| catalog::route(&r).map_or(r, |known| known.label.to_string()));
    Some(Opened { route, agents })
}

/// A response on `route` worked: its episode ends, and the failure goes off every pane that shows it.
/// BLOCKING (store reads and writes), but only once per episode; any other call is a map lookup.
pub(crate) fn cleared(mstore: &Arc<Store>, bus: &Arc<EventBus>, route: &str) {
    if EPISODES.lock().remove(route).is_none() {
        return;
    }
    for (block_id, _) in panes_on(mstore, route) {
        if shows_credits(mstore, &block_id) {
            crate::backend::blockcontroller::core::persist_last_failure(&block_id, None, &Some(mstore.clone()), &Some(bus.clone()));
        }
    }
}

/// Whether the episode `key` is new (and now begun): none is open, or the open one is past `EPISODE_MAX_MS`.
fn open_episode(key: &str, now: u64) -> bool {
    let mut episodes = EPISODES.lock();
    match episodes.get(key) {
        Some(since) if now < since.saturating_add(EPISODE_MAX_MS) => false,
        _ => {
            episodes.insert(key.to_string(), now);
            true
        }
    }
}

/// Whether the pane's stored failure is already the credits one.
fn shows_credits(mstore: &Store, block_id: &str) -> bool {
    mstore
        .get::<Block>(block_id)
        .ok()
        .flatten()
        .and_then(|b| failure_of_block(&b))
        .is_some_and(|f| f.code == crate::agents::failure::FailureClass::UsageLimit && f.title == crate::agents::failure::ROUTE_CREDITS_TITLE)
}

/// The route the harness in `block` was launched on, as the launch recorded it on the pane. Not the agent's chosen
/// route and not its left-route mark, which are the agent's as a whole: a model switch is recorded at once, but each
/// pane is relaunched on it only when its own turn ends, so two panes of one agent can be on different routes (#47474).
fn running_route(block: &Block) -> Option<String> {
    block.meta.get(launch::BLOCK_META_KEY)?.get("route")?.as_str().map(str::to_string)
}

/// The open panes (in a tab) of agents whose harness is running on `route`.
fn panes_on(mstore: &Store, route: &str) -> Vec<Pane> {
    let Ok(tabs) = mstore.get_all::<Tab>() else { return Vec::new() };
    tabs.iter()
        .flat_map(|t| t.blockids.iter())
        .filter_map(|id| {
            let block = mstore.get::<Block>(id).ok().flatten()?;
            let def_id = obj::meta_get_string(&block.meta, "agentId", "");
            let on_it = !def_id.is_empty() && running_route(&block).is_some_and(|r| r == route);
            on_it.then(|| (id.clone(), def_id))
        })
        .collect()
}

/// What each distinct lead of the agents in `opened` is told: their own, once. A lead on another machine, or
/// an agent with none, is not told.
pub(crate) fn notes_for(mstore: &Store, opened: &Opened) -> Vec<(String, String)> {
    let mut by_lead: Vec<(String, Vec<&(String, String)>)> = Vec::new();
    for agent in &opened.agents {
        let lead = crate::backend::agent_seat::effective_seat_by_id(mstore, &agent.0)
            .and_then(|seat| seat.lead)
            .filter(|lead| !crate::backend::agent_seat::is_remote_lead(lead));
        let Some(lead) = lead else { continue };
        match by_lead.iter_mut().find(|(l, _)| *l == lead) {
            Some((_, mine)) => mine.push(agent),
            None => by_lead.push((lead, vec![agent])),
        }
    }
    by_lead
        .into_iter()
        .map(|(_, mine)| {
            let names = mine.iter().map(|(_, name)| name.as_str()).collect::<Vec<_>>().join(", ");
            let body = match &opened.route {
                Some(route) => format!(
                    "{route} is out of credits (HTTP 402), so {names} can't work ({total} on the route in all). Add credits to the route's key in the provider's account. \
                     Their messages wait, and each tries again every few minutes. This is the only note about it until it works again.",
                    total = opened.agents.len()
                ),
                None => format!(
                    "{names} is out of credits on its model provider's account (HTTP 402) and can't work. Add credits there. \
                     Its messages wait, and it tries again every few minutes. This is the only note about it until it works again."
                ),
            };
            (mine[0].0.clone(), body)
        })
        .collect()
}

/// Tell each lead (see [`notes_for`]). BLOCKING.
fn tell_leaders(mstore: &Store, opened: &Opened) {
    for (def_id, body) in notes_for(mstore, opened) {
        crate::backend::worker_notes::tell_leader(mstore, &def_id, &body);
    }
}

#[cfg(test)]
#[path = "route_credits_tests.rs"]
mod tests;
