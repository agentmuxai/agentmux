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
    /// The agent pane.
    pub blockid: String,
    /// `set` the day's cap to `cap_usd` (absent: no cap), or `reset` today's figure to zero.
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
}

fn show(c: Option<f64>) -> String {
    c.map_or("none".to_string(), |c| format!("${c:.2}"))
}

fn act(state: &AppState, req: &AgentSpendReq) -> Result<(AgentSpendResult, String, String), String> {
    let instance = state
        .mstore
        .instance_get_active_for_block(&req.blockid)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no agent runs in block {}", req.blockid))?;
    let def = state.mstore.agent_def_get(&instance.definition_id).map_err(|e| e.to_string())?.ok_or_else(|| format!("no agent definition {}", instance.definition_id))?;
    let note = match req.action.as_str() {
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
        other => return Err(format!("action is set or reset, not {other}")),
    };
    let result = AgentSpendResult { cap_usd: cap::agent_cap(&state.mstore, &def.id), spent_usd: spend::today(&def.slug, agentmux_common::time::now_ms()).cost_usd };
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
