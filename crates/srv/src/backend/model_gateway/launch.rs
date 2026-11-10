// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! An agent's route and model, and the environment that points its harness at the gateway.
//!
//! The choice is kept on the agent's definition as a content blob, so it follows the agent's id and
//! needs no column. The environment is built at every spawn and never persisted: the token in it lives
//! for this srv launch.

use super::catalog::{self, Model, Route};
use super::{token, PATH_PREFIX};
use crate::backend::storage::content::AgentContent;
use crate::backend::storage::store::Store;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The definition content type that holds the [`Choice`].
pub const CONTENT_TYPE: &str = "agent:model_route";
/// The definition content type that marks a route the agent has left but whose harness may still be running a turn on it:
/// the gateway keeps answering that route's token until the harness is relaunched ([`super::switch::finish_leaving`]).
pub const LEAVING_TYPE: &str = "agent:model_route_leaving";
/// A model with a context of this many tokens or more gets Claude Code's `[1m]` mark, so it does not compact early.
const LONG_CONTEXT: u32 = 1_000_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Choice {
    pub route: String,
    pub model: String,
}

/// The route and model of the agent definition `def_id`, if it has chosen one.
pub fn choice_of(mstore: &Store, def_id: &str) -> Option<Choice> {
    let content = mstore.agent_content_get(def_id, CONTENT_TYPE).ok()??;
    serde_json::from_str(&content.content).ok()
}

/// The route and model the agent has left but may still be running a turn on, if any.
pub fn leaving(mstore: &Store, def_id: &str) -> Option<Choice> {
    serde_json::from_str(&mstore.agent_content_get(def_id, LEAVING_TYPE).ok()??.content).ok()
}

/// The model the gateway sends the agent's requests for `route` to: its chosen one, or, on a route it has just left, the
/// one it was on, so a turn in flight finishes (see [`super::switch::finish_leaving`]). None: the agent is not set to it.
pub fn model_on_route(mstore: &Store, def_id: &str, route: &'static Route) -> Option<&'static Model> {
    let chosen = choice_of(mstore, def_id).filter(|c| c.route == route.id).and_then(|c| route.model(&c.model));
    // The model it was running, so what the rest of the turn costs is counted at that model's prices.
    chosen.or_else(|| leaving(mstore, def_id).filter(|c| c.route == route.id).and_then(|c| route.model(&c.model)))
}

/// Saves the agent's route and model after the shared validation, with the model made explicit.
pub fn set_choice(mstore: &Store, def_id: &str, harness: &str, route: &str, model: &str) -> Result<Choice, String> {
    let (route, model) = catalog::validate(harness, route, model)?;
    let choice = Choice { route: route.id.to_string(), model: model.id.to_string() };
    let content = serde_json::to_string(&choice).map_err(|e| e.to_string())?;
    mstore
        .agent_content_set(&AgentContent { agent_id: def_id.to_string(), content_type: CONTENT_TYPE.to_string(), content, updated_at: agentmux_common::time::now_ms() })
        .map_err(|e| e.to_string())?;
    Ok(choice)
}

/// The variables that point Claude Code at the gateway on `route`. The key is not among them: the
/// token is the harness's whole credential, and `ANTHROPIC_API_KEY` is emptied so no other key is used.
pub fn env_for(route: &Route, model: &Model, token: &str, local_url: &str) -> Vec<(String, String)> {
    let marked = if model.context >= LONG_CONTEXT { format!("{}[1m]", model.id) } else { model.id.to_string() };
    let var = |k: &str, v: String| (k.to_string(), v);
    vec![
        var("ANTHROPIC_BASE_URL", format!("{}{PATH_PREFIX}{}", local_url.trim_end_matches('/'), route.id)),
        var("ANTHROPIC_AUTH_TOKEN", token.to_string()),
        var("ANTHROPIC_API_KEY", String::new()),
        var("ANTHROPIC_DEFAULT_OPUS_MODEL", marked.clone()),
        var("ANTHROPIC_DEFAULT_SONNET_MODEL", marked.clone()),
        var("ANTHROPIC_DEFAULT_HAIKU_MODEL", marked),
        var("CLAUDE_CODE_SUBAGENT_MODEL", model.id.to_string()),
    ]
}

/// The block meta key the pane, its footer and its header read for the model the agent really runs: the route's
/// and the model's names, or absent on the harness's own sign-in. Without it they show the harness's nominal model
/// (the alias the pane was configured with), which on a route is not what answers.
pub const BLOCK_META_KEY: &str = "agent:route_model";

/// What the pane shows for a route and model.
pub fn pane_meta(route: &Route, model: &Model) -> serde_json::Value {
    serde_json::json!({ "route": route.id, "route_label": route.label, "model": model.id, "label": model.label })
}

/// Keeps the pane's `agent:route_model` as the launch has it. The next block update the start sends carries it.
fn publish_to_block(mstore: &Store, block_id: &str, shown: Option<(&Route, &Model)>) {
    let mut update = crate::backend::obj::MetaMapType::new();
    update.insert(BLOCK_META_KEY.to_string(), shown.map_or(serde_json::Value::Null, |(r, m)| pane_meta(r, m)));
    if let Err(e) = crate::backend::wcore::object_meta::update_object_meta(mstore, &format!("block:{block_id}"), &update) {
        tracing::warn!(block_id, error = %e, "model route: the pane's model name was not stored");
    }
}

/// Adds the gateway environment to the spawn of the agent on `block_id` when its definition has a route, and
/// records the real model on the pane. Over whatever a persisted `cmd:env` carried: these are reserved names, set by
/// srv at every spawn.
pub fn carry_for_block(mstore: &Store, block_id: &str, env_vars: &mut HashMap<String, String>) {
    let Ok(Some(instance)) = mstore.instance_get_active_for_block(block_id) else { return };
    let Ok(Some(def)) = mstore.agent_def_get(&instance.definition_id) else { return };
    // A relaunch is the end of a turn on a route the agent has left: its token stops working now.
    super::switch::finish_leaving_after_relaunch(mstore, &def.id, block_id, |b| super::switch::pane_in_a_turn(mstore, b));
    let choice = choice_of(mstore, &def.id);
    let validated = choice.as_ref().and_then(|c| catalog::validate(&def.provider, &c.route, &c.model).ok());
    publish_to_block(mstore, block_id, validated);
    // The harness starts on the recorded place, a route or its own sign-in: its first turn decides a change waiting on it.
    super::trial::arm(mstore, &def.id, block_id);
    let Some((route, model)) = validated else {
        if let Some(c) = choice {
            tracing::warn!(agent = %def.slug, route = %c.route, "model route: the agent's saved route no longer validates; launching without it");
        }
        return;
    };
    let Ok(local_url) = std::env::var("AGENTMUX_LOCAL_URL") else {
        tracing::warn!(agent = %def.slug, "model route: srv's own address is not known; launching without the route");
        return;
    };
    let token = token::token_for(&def.slug, &def.id, route.id);
    env_vars.extend(env_for(route, model, &token, &local_url));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_harness_is_given_the_gateway_and_never_a_key() {
        let route = catalog::route("openrouter").unwrap();
        let model = route.model("deepseek/deepseek-v4.1-flash-1017").unwrap();
        let env: HashMap<_, _> = env_for(route, model, "amxg_tok", "http://127.0.0.1:4000/").into_iter().collect();
        assert_eq!(env["ANTHROPIC_BASE_URL"], "http://127.0.0.1:4000/api/v1/model/openrouter");
        assert_eq!(env["ANTHROPIC_AUTH_TOKEN"], "amxg_tok");
        assert_eq!(env["ANTHROPIC_API_KEY"], "");
        assert_eq!(env["ANTHROPIC_DEFAULT_SONNET_MODEL"], "deepseek/deepseek-v4.1-flash-1017[1m]");
        assert_eq!(env["CLAUDE_CODE_SUBAGENT_MODEL"], "deepseek/deepseek-v4.1-flash-1017");
        assert!(!env.values().any(|v| v.contains("openrouter.ai")), "the upstream address stays in srv");
    }

    #[test]
    fn a_route_just_left_keeps_answering_until_the_harness_is_relaunched() {
        let store = Store::open_in_memory().unwrap();
        let mut def = crate::backend::storage::agents::test_agent_def("def-leave", "Leaver", "claude", "host", 1, "");
        store.agent_def_insert(&mut def).unwrap();
        let route = catalog::route("openrouter").unwrap();
        assert!(model_on_route(&store, "def-leave", route).is_none(), "never on the route");
        set_choice(&store, "def-leave", "claude", "openrouter", "deepseek/deepseek-v4-pro-0813").unwrap();
        assert_eq!(model_on_route(&store, "def-leave", route).unwrap().id, "deepseek/deepseek-v4-pro-0813");
        super::super::switch::move_to(&store, "def-leave", "claude", super::super::switch::Target::OwnLogin, |_| true).unwrap();
        assert_eq!(model_on_route(&store, "def-leave", route).unwrap().id, "deepseek/deepseek-v4-pro-0813", "a turn in flight finishes, and is counted at the model it runs");
        super::super::switch::finish_leaving(&store, "def-leave");
        assert!(model_on_route(&store, "def-leave", route).is_none(), "after the relaunch it is refused");
    }

    #[test]
    fn the_pane_is_told_the_model_that_really_answers_and_forgets_it_off_the_route() {
        use crate::backend::obj::{Block, MetaMapType};
        const BLOCK: &str = "5b1a3c52-7d0e-4d43-9a43-2f6a0f4f3c11";
        let store = Store::open_in_memory().unwrap();
        let mut def = crate::backend::storage::agents::test_agent_def("def-pane", "Paned", "claude", "host", 1, "");
        store.agent_def_insert(&mut def).unwrap();
        let mut meta = MetaMapType::new();
        meta.insert("view".to_string(), serde_json::json!("agent"));
        let mut block = Block { oid: BLOCK.to_string(), parentoref: String::new(), version: 0, runtimeopts: None, stickers: None, meta, subblockids: None };
        store.insert(&mut block).unwrap();
        let instance = crate::backend::storage::agents::AgentInstance {
            id: "inst-pane".into(), definition_id: "def-pane".into(), parent_instance_id: String::new(), block_id: BLOCK.into(), session_id: String::new(),
            status: crate::backend::storage::store::InstanceStatus::Running.as_str().to_string(), github_context: String::new(), started_at: 0, ended_at: 0, created_at: 0,
            identity_id: String::new(), memory_id: String::new(), instance_name: String::new(), working_directory: String::new(), display_hidden: false,
        };
        store.instance_create(&instance).unwrap();
        let shown = |store: &Store| store.get::<Block>(BLOCK).unwrap().unwrap().meta.get(BLOCK_META_KEY).cloned().unwrap_or(serde_json::Value::Null);

        set_choice(&store, "def-pane", "claude", "openrouter", "deepseek/deepseek-v4-pro-0813").unwrap();
        carry_for_block(&store, BLOCK, &mut HashMap::new());
        assert_eq!(shown(&store), serde_json::json!({ "route": "openrouter", "route_label": "OpenRouter", "model": "deepseek/deepseek-v4-pro-0813", "label": "DeepSeek V4 Pro" }));

        store.agent_content_delete("def-pane", CONTENT_TYPE).unwrap();
        carry_for_block(&store, BLOCK, &mut HashMap::new());
        assert_eq!(shown(&store), serde_json::Value::Null, "back on the harness's own sign-in the pane shows its own model again");
    }
}
