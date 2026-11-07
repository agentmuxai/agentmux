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

/// Adds the gateway environment to the spawn of the agent on `block_id` when its definition has a route.
/// Over whatever a persisted `cmd:env` carried: these are reserved names, set by srv at every spawn.
pub fn carry_for_block(mstore: &Store, block_id: &str, env_vars: &mut HashMap<String, String>) {
    let Ok(Some(instance)) = mstore.instance_get_active_for_block(block_id) else { return };
    let Ok(Some(def)) = mstore.agent_def_get(&instance.definition_id) else { return };
    let Some(choice) = choice_of(mstore, &def.id) else { return };
    let Ok((route, model)) = catalog::validate(&def.provider, &choice.route, &choice.model) else {
        tracing::warn!(agent = %def.slug, route = %choice.route, "model route: the agent's saved route no longer validates; launching without it");
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
}
