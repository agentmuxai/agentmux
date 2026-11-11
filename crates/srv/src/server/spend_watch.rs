// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What srv does as an agent's spend on a model route reaches its caps (`docs/specs/SPEC_ROUTE_AGENT_BRAKES_2026_11_10.md`
//! section 3.3). The gateway counts each response and refuses a request once a cap is used
//! (`model_gateway::cap`); it tells this watcher of both, and the watcher, off the stream's task:
//!
//! - **50%** of the agent's cap: one non-urgent note to its leader.
//! - **80%:** the agent's pane gets a warning line (block meta `agent:spend_warning`).
//! - **100%** (or the route's cap): the pane carries `agent:spend_stopped`, its turn in flight is interrupted, and the
//!   leader is told. The gateway has refused (or will refuse) the agent's next request.
//!
//! Each level is acted on once per agent per UTC day; the level already sent is kept with the day's spend, so a restart
//! does not send it again. A raised or reset cap, or the first response of a new day, takes the stop off.

use std::sync::OnceLock;

use serde_json::{json, Value};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};

use crate::backend::blockcontroller::{self, BlockInputUnion};
use crate::backend::model_gateway::cap::{self, Hit, Scope};
use crate::backend::model_gateway::token::Grant;
use crate::backend::model_gateway::{catalog, launch, spend};
use crate::backend::{obj, worker_notes};
use crate::server::AppState;

/// Block meta the pane's stop card reads; absent when the agent is not stopped.
pub const STOPPED_META_KEY: &str = "agent:spend_stopped";
/// Block meta the pane's warning line reads, from 80% of the cap.
pub const WARNING_META_KEY: &str = "agent:spend_warning";
/// Block meta the chips read: what the agent has used today and its cap (`day` is days since the epoch, UTC).
pub const TODAY_META_KEY: &str = "agent:spend_today";
/// The least time between two writes of [`TODAY_META_KEY`] for an agent, unless its level or cap changed.
const TODAY_EVERY_MS: i64 = 15_000;
/// How often every active agent's figure is written again, so a skipped one and a new pane catch up.
const REFRESH_EVERY: std::time::Duration = std::time::Duration::from_secs(60);

enum Event {
    Settled(Grant),
    Refused(Grant, Hit),
}

fn inbox() -> &'static OnceLock<UnboundedSender<Event>> {
    static INBOX: OnceLock<UnboundedSender<Event>> = OnceLock::new();
    &INBOX
}

/// A response of `grant`'s agent has been counted.
pub(crate) fn settled(grant: &Grant) {
    if let Some(tx) = inbox().get() {
        let _ = tx.send(Event::Settled(grant.clone()));
    }
}

/// The gateway refused `grant`'s agent a request because `hit` is used.
pub(crate) fn refused(grant: &Grant, hit: Hit) {
    if let Some(tx) = inbox().get() {
        let _ = tx.send(Event::Refused(grant.clone(), hit));
    }
}

/// Starts the watcher.
pub(crate) fn start(state: AppState) {
    let (tx, mut rx) = unbounded_channel();
    if inbox().set(tx).is_err() {
        return;
    }
    tokio::spawn(async move {
        let mut refresh = tokio::time::interval(REFRESH_EVERY);
        refresh.tick().await;
        loop {
            let state = state.clone();
            tokio::select! {
                event = rx.recv() => {
                    let Some(event) = event else { break };
                    // Store reads, block-meta writes and a note to the leader: not on the async worker.
                    let _ = tokio::task::spawn_blocking(move || handle(&state, event)).await;
                }
                // The chips catch up: a figure skipped inside the throttle window, and a pane opened since the last write.
                _ = refresh.tick() => {
                    let _ = tokio::task::spawn_blocking(move || refresh_chips(&state)).await;
                }
            }
        }
    });
}

/// Writes today's figure to the open panes of every agent that has spent anything today.
fn refresh_chips(state: &AppState) {
    let now = agentmux_common::time::now_ms();
    let Ok(defs) = state.mstore.agent_def_list() else { return };
    for agent in spend::agents_today(now) {
        let Some(def) = defs.iter().find(|d| d.slug == agent) else { continue };
        let spent = spend::today(&agent, now).cost_usd;
        publish_today(state, &def.id, &agent, cap::agent_cap(&state.mstore, &def.id), spent, now, true);
    }
}

fn handle(state: &AppState, event: Event) {
    let now = agentmux_common::time::now_ms();
    match event {
        Event::Settled(grant) => on_settled(state, &grant, now),
        Event::Refused(grant, hit) => {
            if spend::alerted(&grant.agent, now) < 100 {
                spend::set_alerted(&grant.agent, 100, now);
                stop(state, &grant, &hit, now);
            }
        }
    }
}

/// Writes the figure the chips show to the agent's open panes: at most every [`TODAY_EVERY_MS`] unless `force`.
fn publish_today(state: &AppState, def_id: &str, agent: &str, cap: Option<f64>, spent: f64, now: i64, force: bool) {
    static LAST: OnceLock<std::sync::Mutex<std::collections::HashMap<String, i64>>> = OnceLock::new();
    {
        let mut last = LAST.get_or_init(Default::default).lock().unwrap();
        if !force && last.get(agent).is_some_and(|at| now - at < TODAY_EVERY_MS) {
            return;
        }
        last.insert(agent.to_string(), now);
    }
    let value = json!({ "day": now.div_euclid(86_400_000), "spent": spent, "cap": cap });
    for block_id in worker_notes::live_panes_of(&state.mstore, def_id) {
        let same = state.mstore.get::<obj::Block>(&block_id).ok().flatten().is_some_and(|b| b.meta.get(TODAY_META_KEY) == Some(&value));
        if !same {
            set_meta(state, &block_id, TODAY_META_KEY, value.clone());
        }
    }
}

fn on_settled(state: &AppState, grant: &Grant, now: i64) {
    let (level, cap, spent) = cap::agent_level(&state.mstore, grant, now);
    let sent = spend::alerted(&grant.agent, now);
    publish_today(state, &grant.def_id, &grant.agent, cap, spent, now, level > sent);
    let settings = state.config_watcher.get_settings().extra;
    // The first response of a day, or one after a cap was lifted in Settings: the stop is over.
    if spend::today(&grant.agent, now).requests == 1 || (sent >= 100 && level < 100 && cap::reached(&state.mstore, &settings, grant, now).is_none()) {
        clear(state, &grant.def_id, STOPPED_META_KEY);
        if sent >= 100 {
            spend::set_alerted(&grant.agent, level, now);
        }
    }
    let (Some(cap), true) = (cap, level > sent) else { return };
    spend::set_alerted(&grant.agent, level, now);
    let hit = Hit { scope: Scope::Agent, cap, spent };
    match level {
        100 => stop(state, grant, &hit, now),
        80 => warn(state, grant, &hit, now),
        _ => {
            let note = format!("{} has used ${spent:.2} of its ${cap:.2} for today on {}.", name_of(state, &grant.def_id), place_of(state, grant));
            // Nothing for the leader to act on yet: it reads this with its next turn, and is not woken for it.
            worker_notes::tell_leader_held(&state.mstore, &grant.def_id, &note, &format!("spend-{}", grant.agent));
        }
    }
}

/// "OpenRouter (DeepSeek V4.1 Flash)": where the agent's spend goes.
fn place_of(state: &AppState, grant: &Grant) -> String {
    let Some(route) = catalog::route(&grant.route) else { return grant.route.clone() };
    match launch::model_on_route(&state.mstore, &grant.def_id, route) {
        Some(model) => format!("{} ({})", route.label, model.label),
        None => route.label.to_string(),
    }
}

fn name_of(state: &AppState, def_id: &str) -> String {
    state.mstore.agent_def_get(def_id).ok().flatten().map_or_else(|| def_id.to_string(), |d| d.name)
}

fn card(state: &AppState, grant: &Grant, hit: &Hit, now: i64) -> Value {
    let scope = if hit.scope == Scope::Agent { "agent" } else { "route" };
    json!({ "at": now, "scope": scope, "cap": hit.cap, "spent": hit.spent, "route": grant.route, "place": place_of(state, grant) })
}

fn set_meta(state: &AppState, block_id: &str, key: &str, value: Value) {
    let mut update = obj::MetaMapType::new();
    update.insert(key.to_string(), value);
    match crate::server::service::update_object_meta(&state.mstore, &format!("block:{block_id}"), &update) {
        Ok(()) => blockcontroller::core::broadcast_block_update(&state.mstore, &state.event_bus, block_id),
        Err(e) => tracing::warn!(block_id, key, error = %e, "spend cap: the pane was not told"),
    }
}

/// Takes `key` off every open pane of the agent that carries it.
pub(crate) fn clear(state: &AppState, def_id: &str, key: &str) {
    for block_id in worker_notes::live_panes_of(&state.mstore, def_id) {
        let carries = state.mstore.get::<obj::Block>(&block_id).ok().flatten().is_some_and(|b| b.meta.get(key).is_some_and(|v| !v.is_null()));
        if carries {
            set_meta(state, &block_id, key, Value::Null);
        }
    }
}

fn warn(state: &AppState, grant: &Grant, hit: &Hit, now: i64) {
    let card = card(state, grant, hit, now);
    for block_id in worker_notes::live_panes_of(&state.mstore, &grant.def_id) {
        set_meta(state, &block_id, WARNING_META_KEY, card.clone());
    }
}

fn stop(state: &AppState, grant: &Grant, hit: &Hit, now: i64) {
    let card = card(state, grant, hit, now);
    for block_id in worker_notes::live_panes_of(&state.mstore, &grant.def_id) {
        set_meta(state, &block_id, STOPPED_META_KEY, card.clone());
        set_meta(state, &block_id, WARNING_META_KEY, Value::Null);
        // The turn in flight ends now rather than retrying against the refusal.
        if let Some(ctrl) = blockcontroller::get_controller(&block_id) {
            if let Err(e) = ctrl.send_input(BlockInputUnion::signal("SIGINT"), None) {
                tracing::warn!(block_id, error = %e, "spend cap: the turn was not interrupted");
            }
        }
    }
    let name = name_of(state, &grant.def_id);
    let note = match hit.scope {
        Scope::Agent => format!(
            "{name} is stopped: it has used ${:.2} of its ${:.2} for today on {}. The gateway refuses its requests until its cap is raised (Agents update spend_cap_usd) or the UTC day ends.",
            hit.spent, hit.cap, place_of(state, grant)
        ),
        Scope::Route => format!(
            "{name} is stopped: the cap of ${:.2} for today on its model route is used (${:.2}). It can go on when the cap is raised in Settings or the UTC day ends.",
            hit.cap, hit.spent
        ),
    };
    tracing::warn!(agent = %grant.agent, route = %grant.route, spent = hit.spent, cap = hit.cap, "spend cap: the agent is stopped");
    worker_notes::tell_leader(&state.mstore, &grant.def_id, &note);
}

/// What `Agents list` shows for an agent on a model route: what it has used today and its cap. `None` for an agent that is
/// not on one.
pub(crate) fn view(mstore: &crate::backend::storage::store::Store, defs: &[crate::backend::storage::AgentDefinition], def_id: &str) -> Option<Value> {
    launch::choice_of(mstore, def_id)?;
    let slug = &defs.iter().find(|d| d.id == def_id)?.slug;
    let spent = spend::today(slug, agentmux_common::time::now_ms()).cost_usd;
    let cap = cap::agent_cap(mstore, def_id);
    Some(json!({ "spent_usd": spent, "cap_usd": cap, "stopped": cap.is_some_and(|c| spent >= c) }))
}

/// The cap before and after a change.
pub(crate) struct CapChange {
    pub from: Option<f64>,
    pub to: Option<f64>,
}

/// Sets the agent's daily cap (`None`: no cap). The alert level goes back to where the day now stands, and a stop the
/// new cap no longer reaches is taken off the agent's panes. The caller has checked who may do this and records it.
pub(crate) fn change_cap(state: &AppState, def_id: &str, to: Option<f64>) -> Result<CapChange, String> {
    let from = cap::agent_cap(&state.mstore, def_id);
    cap::set_agent_cap(&state.mstore, def_id, to)?;
    let Some(def) = state.mstore.agent_def_get(def_id).map_err(|e| e.to_string())?.filter(|d| !d.slug.is_empty()) else {
        return Ok(CapChange { from, to });
    };
    let now = agentmux_common::time::now_ms();
    let spent = spend::today(&def.slug, now).cost_usd;
    let level = to.map_or(0, |c| cap::level(spent, c));
    // A raised cap takes the level down. A lowered one leaves it where it was: the next response, or the next request the
    // gateway refuses, finds the level above it and sends the warning or the stop, which setting it here would skip.
    if level < spend::alerted(&def.slug, now) {
        spend::set_alerted(&def.slug, level, now);
    }
    publish_today(state, def_id, &def.slug, to, spent, now, true);
    if level < 100 {
        clear(state, def_id, STOPPED_META_KEY);
    }
    if level < 80 {
        clear(state, def_id, WARNING_META_KEY);
    }
    Ok(CapChange { from, to })
}

/// Zeroes the agent's spend for today and takes its stop and warning off. Returns the dollars that were counted.
pub(crate) fn reset_today(state: &AppState, def_id: &str) -> Result<f64, String> {
    let def = state.mstore.agent_def_get(def_id).map_err(|e| e.to_string())?.ok_or_else(|| format!("no agent {def_id}"))?;
    let now = agentmux_common::time::now_ms();
    let removed = spend::reset_today(&def.slug, now);
    publish_today(state, def_id, &def.slug, cap::agent_cap(&state.mstore, def_id), 0.0, now, true);
    clear(state, def_id, STOPPED_META_KEY);
    clear(state, def_id, WARNING_META_KEY);
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::types::TokenCounts;
    use crate::backend::storage::store::Store;

    #[test]
    fn agents_list_shows_spend_for_an_agent_on_a_route_and_nothing_for_any_other() {
        let store = Store::open_in_memory().unwrap();
        for (id, slug) in [("view-def", "view-agent"), ("plain-def", "plain-agent")] {
            let mut def = crate::backend::storage::agents::test_agent_def(id, slug, "claude", "host", 1, "");
            def.slug = slug.to_string();
            store.agent_def_insert(&mut def).unwrap();
        }
        launch::set_choice(&store, "view-def", "claude", "openrouter", "").unwrap();
        let model = catalog::route("openrouter").unwrap().model("deepseek/deepseek-v4.1-flash-1017").unwrap();
        let used = TokenCounts { input: 1, ..Default::default() };
        let now = agentmux_common::time::now_ms();
        spend::record("view-agent", "openrouter", model, &used, Some(4.5), now);
        let defs = store.agent_def_list().unwrap();
        let shown = view(&store, &defs, "view-def").expect("an agent on a route");
        assert_eq!((shown["spent_usd"].as_f64(), shown["cap_usd"].as_f64(), shown["stopped"].as_bool()), (Some(4.5), Some(20.0), Some(false)));
        assert!(view(&store, &defs, "plain-def").is_none(), "a login agent has no spend line");
        spend::record("view-agent", "openrouter", model, &used, Some(15.5), now);
        assert_eq!(view(&store, &defs, "view-def").unwrap()["stopped"], json!(true));
    }
}
