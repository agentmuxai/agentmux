// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The pane card's buttons for an agent that reached its daily spend cap (`agent.spend`): raise the cap, or reset
//! today's figure (`docs/specs/SPEC_ROUTE_AGENT_BRAKES_2026_11_10.md` section 3.4). The operator's window only: an
//! agent uses `Agents update spend_cap_usd`, where a leader's change is allowed and an agent's own is lowered only. Each
//! change is audited with the cap before and after.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::backend::model_gateway::{cap, spend};
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::COMMAND_AGENT_SPEND;
use crate::server::AppState;

#[derive(Debug, Clone, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AgentSpendReq {
    /// The agent pane. Empty when `agent` names the agent instead.
    #[serde(default)]
    pub blockid: String,
    /// An agent by its id or slug, for one with no pane open (My Agents). Used when `blockid` is empty.
    #[serde(default)]
    pub agent: Option<String>,
    /// `get` the cap and today's figure, `set` the day's cap to `cap_usd` (absent: no cap), `default` to take the agent's
    /// own cap away so it has the default, or `reset` today's figure to zero.
    pub action: String,
    #[serde(default)]
    pub cap_usd: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AgentSpendResult {
    /// The agent's cap now; absent when it has none.
    pub cap_usd: Option<f64>,
    /// What it has used today.
    pub spent_usd: f64,
    /// Whether a cap applies to the agent at all: it is on a model route or an API key. An agent on a login has plan limits.
    pub applies: bool,
    /// Whether the cap is the agent's own, and not the default every agent without one has.
    pub own_cap: bool,
    /// The default cap (Settings > Advanced); absent when the default is no cap.
    pub default_cap_usd: Option<f64>,
}

fn show(c: Option<f64>) -> String {
    c.map_or("none".to_string(), |c| format!("${c:.2}"))
}

/// The agent the request is about: the one running in the pane, or the one named.
fn definition_of(state: &AppState, req: &AgentSpendReq) -> Result<crate::backend::storage::AgentDefinition, String> {
    if !req.blockid.is_empty() {
        let instance = state
            .mstore
            .instance_get_active_for_block(&req.blockid)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("no agent runs in block {}", req.blockid))?;
        return state.mstore.agent_def_get(&instance.definition_id).map_err(|e| e.to_string())?.ok_or_else(|| format!("no agent definition {}", instance.definition_id));
    }
    let named = req.agent.as_deref().filter(|a| !a.is_empty()).ok_or("name an agent pane (blockid) or an agent")?;
    let defs = state.mstore.agent_def_list().map_err(|e| e.to_string())?;
    defs.into_iter().find(|d| d.id == named || d.slug == named).ok_or_else(|| format!("no agent {named}"))
}

fn act(state: &AppState, req: &AgentSpendReq) -> Result<(AgentSpendResult, String, String), String> {
    let def = definition_of(state, req)?;
    let note = match req.action.as_str() {
        "get" => String::new(),
        "default" => {
            let change = crate::server::spend_watch::clear_cap(state, &def.id)?;
            format!("spend cap {} to {} (the default)", show(change.from), show(change.to))
        }
        "set" => {
            if let Some(c) = req.cap_usd.filter(|c| !c.is_finite() || *c < cap::MIN_CAP_USD) {
                return Err(format!("a cap is at least ${:.2}, or none (got {c})", cap::MIN_CAP_USD));
            }
            let change = crate::server::spend_watch::change_cap(state, &def.id, req.cap_usd)?;
            format!("spend cap {} to {}", show(change.from), show(change.to))
        }
        "reset" => {
            let removed = crate::server::spend_watch::reset_today(state, &def.id)?;
            format!("reset today's spend (${removed:.2} counted)")
        }
        other => return Err(format!("action is get, set, default or reset, not {other}")),
    };
    let settings = state.config_watcher.get_settings().extra;
    let result = AgentSpendResult {
        cap_usd: cap::agent_cap(&state.mstore, &settings, &def.id),
        spent_usd: spend::today(&def.slug, agentmux_common::time::now_ms()).cost_usd,
        applies: crate::server::spend_watch::spends_through_gateway(state, &def.id),
        own_cap: cap::has_own_cap(&state.mstore, &def.id),
        default_cap_usd: cap::default_agent_cap(&settings),
    };
    Ok((result, def.name, note))
}

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let s = state.clone();
    engine.register_typed(COMMAND_AGENT_SPEND, move |req: AgentSpendReq, ctx| {
        let s = s.clone();
        async move {
            ctx.operator_only("only a person changes an agent's spend cap in the window; an agent uses Agents update spend_cap_usd")?;
            tokio::task::spawn_blocking(move || {
                let outcome = act(&s, &req);
                if req.action == "get" {
                    return outcome.map(|(result, _, _)| result);
                }
                let request_id = uuid::Uuid::new_v4().to_string();
                match &outcome {
                    Ok((_, name, note)) => s.reactive_handler.log_fleet_action_audit(Some("you"), name, &req.blockid, "agent.spend", true, None, &request_id, Some(note)),
                    Err(e) => s.reactive_handler.log_fleet_action_audit(Some("you"), &req.blockid, &req.blockid, "agent.spend", false, Some(e), &request_id, Some("refused")),
                }
                outcome.map(|(result, _, _)| result)
            })
            .await
            .map_err(|e| e.to_string())?
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::model_gateway_tests::agent_on_route;
    use crate::server::tests::test_state;

    fn req(agent: &str, action: &str, cap_usd: Option<f64>) -> AgentSpendReq {
        AgentSpendReq { blockid: String::new(), agent: Some(agent.to_string()), action: action.to_string(), cap_usd }
    }

    #[tokio::test]
    async fn a_person_reads_sets_and_clears_an_agents_cap_by_naming_the_agent() {
        let state = test_state();
        let (_, def_id) = agent_on_route(&state, "spend-named", false);
        let default = Some(cap::DEFAULT_AGENT_CAP_USD);

        let (got, _, _) = act(&state, &req(&def_id, "get", None)).unwrap();
        assert_eq!((got.cap_usd, got.own_cap, got.default_cap_usd, got.applies), (default, false, default, true), "no cap of its own: the default");

        let (set, _, note) = act(&state, &req(&def_id, "set", Some(100.0))).unwrap();
        assert_eq!((set.cap_usd, set.own_cap), (Some(100.0), true));
        assert!(note.ends_with("to $100.00"), "{note}");
        assert_eq!(act(&state, &req(&def_id, "get", None)).unwrap().0.cap_usd, Some(100.0), "kept");

        let (none, _, _) = act(&state, &req(&def_id, "set", None)).unwrap();
        assert_eq!((none.cap_usd, none.own_cap), (None, true), "no cap is a cap of its own");

        let (back, _, note) = act(&state, &req(&def_id, "default", None)).unwrap();
        assert_eq!((back.cap_usd, back.own_cap), (default, false), "the default again");
        assert!(note.ends_with("(the default)"), "{note}");

        // The slug names it too, and what cannot be done is refused by name and changes nothing.
        let slug = state.mstore.agent_def_get(&def_id).unwrap().unwrap().slug;
        assert_eq!(act(&state, &req(&slug, "get", None)).unwrap().0.cap_usd, default);
        assert!(act(&state, &req("no-such-agent", "get", None)).is_err());
        assert!(act(&state, &AgentSpendReq { blockid: String::new(), agent: None, action: "get".into(), cap_usd: None }).is_err());
        assert!(act(&state, &req(&def_id, "lots", None)).is_err());
        assert!(act(&state, &req(&def_id, "set", Some(0.001))).is_err(), "a cap under a cent is not a cap anyone meant");
        assert_eq!(act(&state, &req(&def_id, "get", None)).unwrap().0.cap_usd, default, "nothing changed");
    }
}
