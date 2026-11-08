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
use crate::backend::rpc_types::{COMMAND_AGENT_MODEL_PANEL, COMMAND_AGENT_MODEL_SWITCH};
use crate::backend::storage::agents::AgentDefinition;
use crate::backend::vault::route_key;
use crate::server::AppState;

/// A conversation over this many tokens makes a switch worth a warning: the new model reads all of it again at full
/// input price, because a prompt cache belongs to one model.
const REREAD_WARNING_TOKENS: u32 = 50_000;

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

fn panel_model(route: &Route, model: &Model, current: Option<&Choice>, key_present: bool, req: &AgentModelPanelReq) -> AgentModelPanelModel {
    let tokens = req.conversation_tokens.unwrap_or(0);
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
        current: current.is_some_and(|c| c.route == route.id && c.model == model.id),
        unavailable: unavailable(route, model, key_present, req.images.unwrap_or(0)),
        reread_cost_usd: (tokens >= REREAD_WARNING_TOKENS).then(|| f64::from(tokens) / 1_000_000.0 * model.price_in),
    }
}

fn panel(state: &AppState, req: &AgentModelPanelReq) -> Result<AgentModelPanel, String> {
    let def = agent_of_block(state, &req.blockid)?;
    let harness = state.id_store.resolve_effective_provider_id(&def);
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

fn switch_model(state: &AppState, req: &AgentModelSwitchReq) -> Result<AgentModelSwitchResult, String> {
    let def = agent_of_block(state, &req.blockid)?;
    let harness = state.id_store.resolve_effective_provider_id(&def);
    let route = req.route.trim();
    let target = if route.is_empty() { Target::OwnLogin } else { Target::Route { route, model: req.model.trim() } };
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
    let (restarted, failed) = crate::server::account_episodes::restart_panes_of(state, def_id);
    if restarted == 0 || !failed.is_empty() {
        switch::finish_leaving(&state.mstore, def_id);
    }
    (restarted, failed)
}

/// Both commands are the window's: an agent may change its own model through `Agents update`, which stays on its
/// route and is counted against its hourly limit.
pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let s = state.clone();
    engine.register_typed(COMMAND_AGENT_MODEL_PANEL, move |req: AgentModelPanelReq, ctx| {
        let s = s.clone();
        async move {
            ctx.operator_only("only the AgentMux window reads the model panel")?;
            tokio::task::spawn_blocking(move || panel(&s, &req)).await.map_err(|e| e.to_string())?
        }
    });
    let s = state.clone();
    engine.register_typed(COMMAND_AGENT_MODEL_SWITCH, move |req: AgentModelSwitchReq, ctx| {
        let s = s.clone();
        async move {
            ctx.operator_only("only a person moves an agent to a model route, because it spends their key")?;
            tokio::task::spawn_blocking(move || switch_model(&s, &req)).await.map_err(|e| e.to_string())?
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
}
