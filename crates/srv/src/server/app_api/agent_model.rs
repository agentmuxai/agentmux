// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The composer's model panel, srv's half (`docs/specs/SPEC_AGENT_MODEL_ROUTES_ANY_MODEL_SPAWN_AND_SWITCH_2026_11_06.md`
//! section 7a). `agent.model.panel` says what the panel may offer for a pane's agent and what cannot be picked, with
//! the reason; `agent.model.switch` is a person's pick: it moves the agent to a model on a route, or back to the
//! harness's own sign-in, and restarts its panes so the next turn runs there. Both are the operator's window only: the
//! click is the approval of the spend, which an agent cannot give for itself.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::backend::model_gateway::catalog::{self, Model, Route};
use crate::backend::model_gateway::launch::{self, Choice};
use crate::backend::model_gateway::switch::{self, Target};
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{COMMAND_AGENT_MODEL_PANEL, COMMAND_AGENT_MODEL_SWITCH, COMMAND_AGENT_PEER_LABELS};
use crate::backend::storage::agents::AgentDefinition;
use crate::backend::vault::route_key;
use crate::server::AppState;

/// A conversation over this many tokens makes a switch worth a warning: the new model reads all of it again at full
/// input price, because a prompt cache belongs to one model.
const REREAD_WARNING_TOKENS: u32 = 50_000;

/// What a conversation is taken to need after `/compact`: the summary, the system prompt and the tool definitions. A
/// model whose context is smaller than the conversation can still take it compacted when its context is above this.
const COMPACTED_TOKENS: u32 = 40_000;

#[derive(Debug, Clone, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AgentModelPanelReq {
    /// The agent pane.
    pub blockid: String,
    /// How much the conversation holds now, from the harness's last usage report; absent when not known.
    #[serde(default)]
    pub conversation_tokens: Option<u32>,
    /// How many images the conversation holds; absent when not known.
    #[serde(default)]
    pub images: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AgentModelPanelModel {
    pub id: String,
    pub label: String,
    pub context: u32,
    /// US dollars per million tokens.
    pub price_in: f64,
    pub price_out: f64,
    pub tools: bool,
    pub thinking: bool,
    pub images: bool,
    /// The route's default model.
    pub default: bool,
    /// The agent runs this now.
    pub current: bool,
    /// Why it cannot be picked, or absent.
    pub unavailable: Option<String>,
    /// Why the conversation has to be compacted before this model can take it (it holds more than the model's context,
    /// and a compacted one fits), or absent. The row is offered as "Compact, then switch".
    pub compact_first: Option<String>,
    /// About what the switch costs in re-reading the conversation at this model's input price, when it is large.
    pub reread_cost_usd: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AgentModelPanelRoute {
    pub id: String,
    pub label: String,
    pub key_present: bool,
    /// The Vault label of the key, for the "add it in the Vault" row.
    pub key_label: String,
    pub models: Vec<AgentModelPanelModel>,
}

/// A route and model the agent is on or was on. An empty `route` is the harness's own sign-in.
#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AgentModelPlace {
    pub route: String,
    pub model: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AgentModelPanel {
    pub harness: String,
    /// The route the agent is on, or absent on the harness's own sign-in.
    pub current: Option<AgentModelPlace>,
    /// Where it was before its last change.
    pub previous: Option<AgentModelPlace>,
    pub routes: Vec<AgentModelPanelRoute>,
}

#[derive(Debug, Clone, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AgentModelSwitchReq {
    pub blockid: String,
    /// The route, or empty for the harness's own sign-in.
    pub route: String,
    /// The model on the route; empty is the route's default.
    #[serde(default)]
    pub model: String,
    /// Restart the agent's panes (at the end of a turn in progress). The caller that goes on to change the
    /// harness's own model itself leaves it out, so there is one restart and not two.
    #[serde(default = "yes")]
    pub restart: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AgentModelSwitchResult {
    /// False when the agent was already there.
    pub changed: bool,
    pub restarted: u32,
    pub failed: Vec<String>,
}

/// What `agent.peer-labels` answers.
#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AgentPeerLabels {
    /// By agent definition id: the model the agent really runs and the route that serves it, such as
    /// "DeepSeek V4.1 Flash via OpenRouter". An agent on its harness's own sign-in is not listed.
    pub labels: std::collections::HashMap<String, String>,
}

/// The model and route an agent with `choice` runs on `harness`, or None when it has no choice or the saved one no longer
/// validates (the launch falls back to the harness's own sign-in then, so the label must too).
fn peer_label(harness: &str, choice: Option<&Choice>) -> Option<String> {
    let choice = choice?;
    let (route, model) = catalog::validate(harness, &choice.route, &choice.model).ok()?;
    Some(format!("{} via {}", model.label, route.label))
}

fn peer_labels(state: &AppState) -> Result<AgentPeerLabels, String> {
    let defs = state.mstore.agent_def_list().map_err(|e| e.to_string())?;
    let labels = defs
        .iter()
        .filter_map(|def| {
            let harness = state.id_store.resolve_effective_provider_id(def);
            peer_label(&harness, launch::choice_of(&state.mstore, &def.id).as_ref()).map(|l| (def.id.clone(), l))
        })
        .collect();
    Ok(AgentPeerLabels { labels })
}

fn agent_of_block(state: &AppState, block_id: &str) -> Result<AgentDefinition, String> {
    let instance = state
        .mstore
        .instance_get_active_for_block(block_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no agent runs in block {block_id}"))?;
    state.mstore.agent_def_get(&instance.definition_id).map_err(|e| e.to_string())?.ok_or_else(|| format!("no agent definition {}", instance.definition_id))
}

fn place(route: &str, model: &str) -> AgentModelPlace {
    let label = catalog::route(route).and_then(|r| r.model(model)).map_or_else(|| model.to_string(), |m| m.label.to_string());
    AgentModelPlace { route: route.to_string(), model: model.to_string(), label }
}

/// Why the model cannot be picked now, if it cannot.
fn unavailable(route: &Route, model: &Model, key_present: bool, images: u32) -> Option<String> {
    if !key_present {
        return Some(format!("key missing: add \"{}\" in the Vault", route.key_label));
    }
    if images > 0 && !model.images {
        return Some(format!("can't read images ({images} in this conversation)"));
    }
    None
}

/// Whether a conversation of `tokens` fits the model's context: as it is, only once compacted (`Some` with the sizes, to
/// say so), or not even then.
#[derive(Debug, PartialEq, Eq)]
enum Fit {
    Yes,
    AfterCompact(String),
    No(String),
}

fn kilo(tokens: u32) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}M", f64::from(tokens) / 1_000_000.0).replace(".0M", "M")
    } else {
        format!("{}K", tokens.div_ceil(1000))
    }
}

fn fit(context: u32, tokens: u32) -> Fit {
    if tokens <= context {
        return Fit::Yes;
    }
    let sizes = format!("the conversation ({}) is larger than this model's context ({})", kilo(tokens), kilo(context));
    if context > COMPACTED_TOKENS {
        Fit::AfterCompact(sizes)
    } else {
        Fit::No(format!("{sizes}, even compacted"))
    }
}

fn panel_model(route: &Route, model: &Model, current: Option<&Choice>, key_present: bool, req: &AgentModelPanelReq) -> AgentModelPanelModel {
    let tokens = req.conversation_tokens.unwrap_or(0);
    let is_current = current.is_some_and(|c| c.route == route.id && c.model == model.id);
    let mut unavailable = unavailable(route, model, key_present, req.images.unwrap_or(0));
    // The model the agent runs now is never made unpickable by its own size: it is where the conversation already is.
    let mut compact_first = None;
    if unavailable.is_none() && !is_current {
        match fit(model.context, tokens) {
            Fit::Yes => {}
            Fit::AfterCompact(why) => compact_first = Some(why),
            Fit::No(why) => unavailable = Some(why),
        }
    }
    AgentModelPanelModel {
        id: model.id.to_string(),
        label: model.label.to_string(),
        context: model.context,
        price_in: model.price_in,
        price_out: model.price_out,
        tools: model.tools,
        thinking: model.thinking,
        images: model.images,
        default: model.id == route.default_model,
        current: is_current,
        unavailable,
        compact_first,
        reread_cost_usd: (tokens >= REREAD_WARNING_TOKENS).then(|| f64::from(tokens) / 1_000_000.0 * model.price_in),
    }
}

/// The images in the agent's conversation, as its transcript lists them: the pane's own count only sees the feed it has
/// loaded, and a text-only model chosen for a conversation that still holds an image fails on it. 0 when it is not known.
fn images_in_conversation(def: &AgentDefinition) -> u32 {
    let Some(gfs) = crate::backend::agent_session::global_transcript_store() else { return 0 };
    let zone = crate::backend::agent_session::agent_current_zone(&def.id);
    crate::backend::transcript::images::count_in_zone(gfs, &zone).unwrap_or_else(|e| {
        tracing::warn!(agent = %def.slug, error = %e, "model panel: the images in the conversation were not counted");
        0
    })
}

fn panel(state: &AppState, req: &AgentModelPanelReq) -> Result<AgentModelPanel, String> {
    let def = agent_of_block(state, &req.blockid)?;
    let harness = state.id_store.resolve_effective_provider_id(&def);
    // The pane's count is a hint: the larger of the two is the one to go by.
    let req = &AgentModelPanelReq { images: Some(req.images.unwrap_or(0).max(images_in_conversation(&def))), ..req.clone() };
    let current = launch::choice_of(&state.mstore, &def.id);
    let routes = catalog::ROUTES
        .iter()
        .filter(|r| r.harness == harness)
        .map(|route| {
            let key_present = route_key::present(&state.identity_store, route.key_label);
            AgentModelPanelRoute {
                id: route.id.to_string(),
                label: route.label.to_string(),
                key_present,
                key_label: route.key_label.to_string(),
                models: route.models.iter().map(|m| panel_model(route, m, current.as_ref(), key_present, req)).collect(),
            }
        })
        .collect();
    Ok(AgentModelPanel {
        harness,
        current: current.map(|c| place(&c.route, &c.model)),
        previous: switch::previous_of(&state.mstore, &def.id).map(|p| place(&p.route, &p.model)),
        routes,
    })
}

/// A model that cannot read the images in the conversation is refused, with the reason the panel gives for it. A route or
/// model that does not validate is left to the switch, which says so.
fn refuse_unreadable(harness: &str, route: &str, model: &str, images: u32) -> Result<(), String> {
    match catalog::validate(harness, route, model) {
        Ok((r, m)) => unavailable(r, m, true, images).map_or(Ok(()), Err),
        Err(_) => Ok(()),
    }
}

fn switch_model(state: &AppState, req: &AgentModelSwitchReq) -> Result<AgentModelSwitchResult, String> {
    let def = agent_of_block(state, &req.blockid)?;
    let harness = state.id_store.resolve_effective_provider_id(&def);
    let route = req.route.trim();
    let target = if route.is_empty() { Target::OwnLogin } else { Target::Route { route, model: req.model.trim() } };
    if let Target::Route { route, model } = target {
        refuse_unreadable(&harness, route, model, images_in_conversation(&def))?;
    }
    let moved = switch::move_to(&state.mstore, &def.id, &harness, target, |r| route_key::present(&state.identity_store, r.key_label))?;
    if !moved.changed() || !req.restart {
        return Ok(AgentModelSwitchResult { changed: moved.changed(), restarted: 0, failed: Vec::new() });
    }
    let (restarted, failed) = restart_panes(state, &def.id);
    Ok(AgentModelSwitchResult { changed: true, restarted, failed })
}

/// Restarts the agent's panes on its new place, and ends the access of a route it left when no relaunch is coming to do it:
/// the pane restarts at the end of its turn, and its relaunch is what takes the old route's token away.
pub(crate) fn restart_panes(state: &AppState, def_id: &str) -> (u32, Vec<String>) {
    let (restarted, next_turn, failed) = crate::server::account_episodes::restart_panes_of(state, def_id);
    // Nothing is coming to relaunch it, or a pane could not be: end it now, unless a pane is still in a turn on the old route.
    if (restarted == 0 && next_turn == 0) || !failed.is_empty() {
        switch::finish_leaving_after_relaunch(&state.mstore, def_id, "", |b| switch::pane_in_a_turn(&state.mstore, b));
    }
    (restarted, failed)
}

/// A request slower than this is worth a line in the log: the panel is waiting for it with its harness models showing.
const SLOW_REQUEST_MS: u128 = 2_000;

/// What became of a request from the window. Nothing else records it: a refused or failed one reaches only the pane, and
/// the pane had nothing to say about a panel it could not read.
fn logged<T>(command: &str, block: &str, started: std::time::Instant, result: Result<T, String>) -> Result<T, String> {
    let ms = started.elapsed().as_millis();
    match &result {
        Err(error) => tracing::warn!(command, block, ms = ms as u64, %error, "model panel: the request failed"),
        Ok(_) if ms >= SLOW_REQUEST_MS => tracing::warn!(command, block, ms = ms as u64, "model panel: the request was slow"),
        Ok(_) => {}
    }
    result
}

/// Both commands are the window's: an agent may change its own model through `Agents update`, which stays on its
/// route and is counted against its hourly limit.
pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let s = state.clone();
    engine.register_typed(COMMAND_AGENT_MODEL_PANEL, move |req: AgentModelPanelReq, ctx| {
        let s = s.clone();
        async move {
            let (block, started) = (req.blockid.clone(), std::time::Instant::now());
            let result = match ctx.operator_only("only the AgentMux window reads the model panel") {
                Ok(()) => tokio::task::spawn_blocking(move || panel(&s, &req)).await.map_err(|e| e.to_string())?,
                Err(refused) => Err(refused),
            };
            logged(COMMAND_AGENT_MODEL_PANEL, &block, started, result)
        }
    });
    let s = state.clone();
    // No argument: `Value` takes whatever the caller sends. Any caller may read it: it names no key and no price.
    engine.register_typed(COMMAND_AGENT_PEER_LABELS, move |_data: serde_json::Value, _ctx| {
        let s = s.clone();
        async move { tokio::task::spawn_blocking(move || peer_labels(&s)).await.map_err(|e| e.to_string())? }
    });
    let s = state.clone();
    engine.register_typed(COMMAND_AGENT_MODEL_SWITCH, move |req: AgentModelSwitchReq, ctx| {
        let s = s.clone();
        async move {
            let (block, started) = (req.blockid.clone(), std::time::Instant::now());
            let result = match ctx.operator_only("only a person moves an agent to a model route, because it spends their key") {
                Ok(()) => tokio::task::spawn_blocking(move || switch_model(&s, &req)).await.map_err(|e| e.to_string())?,
                Err(refused) => Err(refused),
            };
            logged(COMMAND_AGENT_MODEL_SWITCH, &block, started, result)
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(tokens: Option<u32>, images: Option<u32>) -> AgentModelPanelReq {
        AgentModelPanelReq { blockid: "b".into(), conversation_tokens: tokens, images }
    }

    #[test]
    fn a_logged_request_answers_as_it_did() {
        let now = std::time::Instant::now();
        assert_eq!(logged("c", "b", now, Ok::<_, String>(3)), Ok(3));
        assert_eq!(logged::<u8>("c", "b", now, Err("no agent runs in block b".into())), Err("no agent runs in block b".to_string()));
    }

    #[test]
    fn a_route_agent_is_labelled_with_its_model_and_route() {
        let on_route = Choice { route: "openrouter".into(), model: "deepseek/deepseek-v4.1-flash-1017".into() };
        assert_eq!(peer_label("claude", Some(&on_route)).as_deref(), Some("DeepSeek V4.1 Flash via OpenRouter"));
        // No choice, or a saved one that no longer validates (another harness, an unknown model): the harness's own sign-in.
        assert_eq!(peer_label("claude", None), None);
        assert_eq!(peer_label("codex", Some(&on_route)), None);
        let gone = Choice { route: "openrouter".into(), model: "nobody/retired".into() };
        assert_eq!(peer_label("claude", Some(&gone)), None);
    }

    #[test]
    fn a_model_that_cannot_be_picked_says_why() {
        let route = catalog::route("openrouter").unwrap();
        let pro = route.model("deepseek/deepseek-v4-pro-0813").unwrap();
        let no_key = unavailable(route, pro, false, 0).unwrap();
        assert!(no_key.starts_with("key missing") && no_key.contains("OpenRouter API key"), "{no_key}");
        assert!(unavailable(route, pro, true, 0).is_none());
        assert_eq!(unavailable(route, pro, true, 3).unwrap(), "can't read images (3 in this conversation)");
        let flash = route.model("deepseek/deepseek-v4.1-flash-1017").unwrap();
        assert!(unavailable(route, flash, true, 3).is_none(), "this one reads images");
    }

    #[test]
    fn a_switch_to_a_model_that_cannot_read_the_conversations_images_is_refused_with_the_panels_reason() {
        let pro = "deepseek/deepseek-v4-pro-0813";
        assert_eq!(refuse_unreadable("claude", "openrouter", pro, 2).unwrap_err(), "can't read images (2 in this conversation)");
        assert!(refuse_unreadable("claude", "openrouter", pro, 0).is_ok(), "no image, no refusal");
        assert!(refuse_unreadable("claude", "openrouter", "deepseek/deepseek-v4.1-flash-1017", 2).is_ok(), "this one reads images");
        assert!(refuse_unreadable("claude", "openrouter", "nobody/retired", 2).is_ok(), "the switch says what is wrong with it");
    }

    #[test]
    fn a_big_conversation_shows_what_re_reading_it_costs() {
        let route = catalog::route("openrouter").unwrap();
        let flash = route.model("deepseek/deepseek-v4.1-flash-1017").unwrap();
        let big = panel_model(route, flash, None, true, &req(Some(200_000), None));
        assert!((big.reread_cost_usd.unwrap() - 0.03).abs() < 1e-9, "{:?}", big.reread_cost_usd);
        assert!(panel_model(route, flash, None, true, &req(Some(10_000), None)).reread_cost_usd.is_none());
        assert!(panel_model(route, flash, None, true, &req(None, None)).reread_cost_usd.is_none());
    }

    #[test]
    fn the_model_the_agent_runs_is_marked_current() {
        let route = catalog::route("openrouter").unwrap();
        let flash = route.model("deepseek/deepseek-v4.1-flash-1017").unwrap();
        let on = Choice { route: "openrouter".into(), model: flash.id.into() };
        assert!(panel_model(route, flash, Some(&on), true, &req(None, None)).current);
        assert!(!panel_model(route, flash, None, true, &req(None, None)).current);
    }

    #[test]
    fn sizes_read_in_thousands_and_in_millions_to_one_decimal() {
        assert_eq!(kilo(40_000), "40K");
        assert_eq!(kilo(1_000_000), "1M");
        assert_eq!(kilo(1_048_576), "1M");
        assert_eq!(kilo(1_500_000), "1.5M");
    }

    #[test]
    fn a_conversation_larger_than_a_models_context_is_offered_compacted_when_that_would_fit() {
        assert_eq!(fit(1_000_000, 200_000), Fit::Yes);
        assert_eq!(fit(128_000, 128_000), Fit::Yes, "exactly the size");
        assert_eq!(fit(128_000, 200_000), Fit::AfterCompact("the conversation (200K) is larger than this model's context (128K)".into()));
        assert_eq!(fit(32_000, 100_000), Fit::No("the conversation (100K) is larger than this model's context (32K), even compacted".into()));
        assert_eq!(fit(40_000, 100_000), Fit::No("the conversation (100K) is larger than this model's context (40K), even compacted".into()), "no room left over for the work");
    }

    #[test]
    fn a_model_too_small_for_the_conversation_is_offered_compacted_and_the_current_one_is_left_alone() {
        let route = catalog::route("openrouter").unwrap();
        let flash = route.model("deepseek/deepseek-v4.1-flash-1017").unwrap();
        let over = req(Some(flash.context + 1), None);
        let offered = panel_model(route, flash, None, true, &over);
        assert!(offered.unavailable.is_none() && offered.compact_first.is_some(), "{offered:?}");
        let on = Choice { route: "openrouter".into(), model: flash.id.into() };
        let running = panel_model(route, flash, Some(&on), true, &over);
        assert!(running.current && running.unavailable.is_none() && running.compact_first.is_none(), "{running:?}");
        assert!(panel_model(route, flash, None, true, &req(Some(10_000), None)).compact_first.is_none());
    }
}
