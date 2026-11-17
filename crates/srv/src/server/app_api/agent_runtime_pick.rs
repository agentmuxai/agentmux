// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `model` and `effort` of `Agents update` for an agent that is not on a model route: the same runtime choice the pane's
//! picker makes (the Runtime menu, `/model`, `/effort`; `frontend/app/view/agent/runtime-apply.ts`), by an agent, on
//! this machine or on another (`host`).
//!
//! It writes what the picker writes: `agent:runtime` on each of the agent's panes, `cmd:args` without its own `--model` and
//! `--effort` (the next launch builds them from the runtime: `agent_runtime::with_runtime_flags`), and the agent's
//! remembered runtime, so a closed agent opens on it. Then the agent's persistent panes restart at the end of their turn,
//! with their conversation; a one-shot pane (a container agent's) takes it at its next turn.
//!
//! What it does not do: change a model the definition's own provider flags fix (the operator's, in My Agents), or an agent
//! pointed at a model server of its own (its model is `ANTHROPIC_MODEL` in its environment, `Agents env_set`).

use axum::http::StatusCode;
use serde_json::{json, Value};

use super::agent_lifecycle::{refuse, Refusal};
use crate::backend::agent_runtime;
use crate::backend::obj::MetaMapType;
use crate::backend::storage::AgentDefinition;
use crate::server::state::AppState;

fn bad(message: impl Into<String>) -> Refusal {
    refuse(StatusCode::BAD_REQUEST, format!("INVALID_ARG: {}", message.into()))
}

/// What a pick did, for the answer.
pub(super) struct Picked {
    pub(super) model: Option<String>,
    pub(super) effort: Option<String>,
    pub(super) panes: usize,
    pub(super) restarted: u32,
    pub(super) next_turn: u32,
    pub(super) failed: Vec<String>,
}

impl Picked {
    /// The answer's `applied` object.
    pub(super) fn to_json(&self) -> Value {
        let when = if self.panes == 0 {
            "the agent has no pane open: it opens on this choice"
        } else if self.restarted > 0 {
            "its pane restarts at the end of the turn it is in, with its conversation; no fresh session is needed"
        } else {
            "it applies at the agent's next turn; no restart is needed"
        };
        json!({
            "model": self.model,
            "effort": self.effort,
            "panes": self.panes,
            "restarting": self.restarted,
            "applies_at_next_turn": self.next_turn,
            "not_restarted": self.failed,
            "when": when,
        })
    }
}

/// The harness the pick is for, or why there is none.
fn harness_of(state: &AppState, agent: &AgentDefinition, model: Option<&str>, effort: Option<&str>) -> Result<&'static str, Refusal> {
    let harness = state.id_store.resolve_effective_provider_id(agent);
    match crate::backend::providers::resolve_provider_alias(&harness) {
        "claude" => Ok("claude"),
        "codex" if effort.is_some() => Err(bad("a Codex agent's effort depends on the model it runs; its pane's picker sets it (Agents update takes only its model)")),
        "codex" if model.is_some() => Ok("codex"),
        other => Err(bad(format!(
            "Agents update model and effort are for Claude and Codex agents, and this agent runs {}: its model is chosen in its own pane's picker",
            if other.is_empty() { harness.as_str() } else { other }
        ))),
    }
}

/// Whether `model` can be given to this agent's harness, by name.
fn check_model(harness: &str, model: &str) -> Result<(), Refusal> {
    match harness {
        "claude" if agent_runtime::is_claude_model_name(model) => Ok(()),
        "claude" => Err(bad(format!("{model} is not a Claude model: use sonnet, opus, haiku or a model id such as claude-fable-5-2 (a model on a model route needs the agent on that route, which the person approves)"))),
        _ => {
            let listed = crate::backend::codex_models::installed_common_model_ids();
            if listed.iter().any(|l| l == model) {
                Ok(())
            } else {
                Err(bad(format!("{model} is not a model this agent's account lists: {}", listed.join(", "))))
            }
        }
    }
}

/// Why this pane cannot take the pick, or none.
fn pane_refusal(meta: &MetaMapType, model: Option<&str>, effort: Option<&str>) -> Option<String> {
    let (pins_model, pins_effort) = agent_runtime::provider_flags_pin(meta);
    if model.is_some() && pins_model {
        return Some("the agent's definition carries its own --model in its provider flags: the operator changes it in My Agents".to_string());
    }
    if effort.is_some() && pins_effort {
        return Some("the agent's definition carries its own --effort in its provider flags: the operator changes it in My Agents".to_string());
    }
    if model.is_some() && agent_runtime::endpoint_model(meta).is_some() {
        return Some("the agent is pointed at a model server of its own, which it asks for the model ANTHROPIC_MODEL names: change that with Agents env_set".to_string());
    }
    None
}

/// The agent's remembered runtime with `model` and `effort` laid over it.
fn remembered_with(state: &AppState, agent: &AgentDefinition, model: Option<&str>, effort: Option<&str>) -> Result<String, Refusal> {
    let internal = |e: String| refuse(StatusCode::INTERNAL_SERVER_ERROR, format!("Agents update: {e}"));
    let current = state.mstore.agent_last_runtime_get(&agent.id).map_err(|e| internal(e.to_string()))?;
    let mut remembered = serde_json::from_str::<Value>(&current).ok().and_then(|v| v.as_object().cloned()).unwrap_or_default();
    for (key, value) in [("model", model), ("effort", effort)] {
        if let Some(value) = value {
            remembered.insert(key.to_string(), json!(value));
        }
    }
    crate::backend::storage::agents::normalize_last_runtime(&Value::Object(remembered).to_string()).map_err(internal)
}

/// Make the pick. Everything is checked before anything is written.
pub(super) fn pick(state: &AppState, agent: &AgentDefinition, model: Option<&str>, effort: Option<&str>) -> Result<Picked, Refusal> {
    let harness = harness_of(state, agent, model, effort)?;
    if let Some(model) = model {
        check_model(harness, model)?;
    }
    if let Some(effort) = effort.filter(|e| !agent_runtime::is_claude_effort(e)) {
        return Err(bad(format!("{effort} is not an effort: low, medium, high, xhigh or max")));
    }
    let panes = crate::server::account_episodes::panes_of(state, &agent.id);
    if let Some(why) = panes.iter().find_map(|b| pane_refusal(&b.meta, model, effort)) {
        return Err(bad(why));
    }
    let remembered = remembered_with(state, agent, model, effort)?;
    for pane in &panes {
        let (_, pins_effort) = agent_runtime::provider_flags_pin(&pane.meta);
        let choice = agent_runtime::runtime_choice(&pane.meta, model, effort, pins_effort);
        let mut updates = vec![("agent:runtime", choice.runtime)];
        if let Some(args) = choice.cli_args {
            updates.push(("cmd:args", json!(args)));
        }
        crate::server::prep::set_block_meta(state, &[pane.oid.as_str()], &updates);
    }
    state
        .mstore
        .agent_last_runtime_set(&agent.id, &remembered)
        .map_err(|e| refuse(StatusCode::INTERNAL_SERVER_ERROR, format!("Agents update: {e}")))?;
    let (restarted, next_turn, failed) = crate::server::account_episodes::restart_panes_of(state, &agent.id);
    for why in &failed {
        tracing::warn!(agent = %agent.slug, why, "Agents update: a pane was not restarted on the new runtime");
    }
    Ok(Picked { model: model.map(str::to_string), effort: effort.map(str::to_string), panes: panes.len(), restarted, next_turn, failed })
}
