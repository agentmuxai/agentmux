// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A Claude Code agent pointed at a server of its own: `ANTHROPIC_BASE_URL` in its environment names
//! something other than Anthropic (a local model, a LAN server, a proxy; set by an agent through
//! `Agents env_set` or by the person in My Agents).
//!
//! The rule, applied to every spawn after the account's credentials were put in the environment:
//! **the person's Claude sign-in goes only to Anthropic.** Whatever the account injected (its
//! config directory with the OAuth login, an OAuth token variable, an API key) is taken out, the
//! CLI is given a private config directory with no sign-in in it, and its credentials are only what
//! the agent's own environment gave it: a Vault entry the person opened to that agent, a
//! placeholder for a server that ignores the key, or nothing (then a placeholder, so the CLI does
//! not fall back to asking for a sign-in). Otherwise the sign-in would go to the other server, over
//! plain HTTP for a LAN one.
//!
//! An agent on a model route (`model_gateway`) is not touched here: the route's own variables are put
//! in after this step and name srv's gateway, which holds the key.

use crate::backend::agent_env;
use std::collections::HashMap;

/// The key the CLI is given when the agent's environment gave none: a value, so it never asks for a
/// sign-in, and one that is no credential.
pub const NO_KEY_PLACEHOLDER: &str = "agentmux-no-key";

/// The environment variables that carry a Claude login or key that the account puts in.
const ACCOUNT_VARIABLES: &[&str] = &["CLAUDE_CODE_OAUTH_TOKEN", "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_API_KEY"];

/// What the agent's own environment (`cmd:env`, before the account's variables) asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub base_url: String,
    /// `ANTHROPIC_AUTH_TOKEN` as written: a `vault:<id>` reference or a placeholder.
    pub auth_token: Option<String>,
    /// `ANTHROPIC_API_KEY` as written.
    pub api_key: Option<String>,
}

fn nonempty(env: &HashMap<String, String>, name: &str) -> Option<String> {
    env.get(name).map(|v| v.trim()).filter(|v| !v.is_empty()).map(str::to_string)
}

/// The custom endpoint `own` (the agent's own environment) asks for, or `None` where it names
/// none or names Anthropic's own API.
pub fn requested(own: &HashMap<String, String>) -> Option<Endpoint> {
    let base_url = nonempty(own, "ANTHROPIC_BASE_URL")?;
    if agent_env::is_anthropic_url(&base_url) {
        return None;
    }
    Some(Endpoint { base_url, auth_token: nonempty(own, "ANTHROPIC_AUTH_TOKEN"), api_key: nonempty(own, "ANTHROPIC_API_KEY") })
}

/// Where the CLI's private config directory is for this agent: beside the shared one, never in it,
/// so no sign-in is ever visible there.
pub fn private_config_dir(shared_auth_dir: &str, agent_slug: &str) -> String {
    let safe: String = agent_slug.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    let shared = std::path::Path::new(shared_auth_dir);
    let base = shared.parent().unwrap_or(shared);
    base.join("claude-endpoint").join(if safe.is_empty() { "agent".to_string() } else { safe }).to_string_lossy().into_owned()
}

/// Takes the account's login out of `env` and gives the CLI the endpoint and the keys the agent's
/// own environment asked for. `resolve_vault` turns an entry id into its value, or `None` when the
/// agent may not have it (the key is then left out: never a fallback to another key).
pub fn apply(env: &mut HashMap<String, String>, endpoint: &Endpoint, config_dir: &str, resolve_vault: &dyn Fn(&str) -> Option<String>) {
    for name in ACCOUNT_VARIABLES {
        env.remove(*name);
    }
    env.insert("CLAUDE_CONFIG_DIR".to_string(), config_dir.to_string());
    env.insert("ANTHROPIC_BASE_URL".to_string(), endpoint.base_url.clone());
    let value_of = |written: &Option<String>| {
        written.as_deref().and_then(|w| match agent_env::vault_ref(w) {
            Some(id) => resolve_vault(id),
            None => Some(w.to_string()),
        })
    };
    let (token, key) = (value_of(&endpoint.auth_token), value_of(&endpoint.api_key));
    match (token, &key) {
        (Some(token), _) => {
            env.insert("ANTHROPIC_AUTH_TOKEN".to_string(), token);
        }
        // A key and no token: the CLI sends the key; a token variable would take its place.
        (None, Some(_)) => {}
        (None, None) => {
            env.insert("ANTHROPIC_AUTH_TOKEN".to_string(), NO_KEY_PLACEHOLDER.to_string());
        }
    }
    // Always set, empty when the agent gave none, so no other key the account held is used.
    env.insert("ANTHROPIC_API_KEY".to_string(), key.unwrap_or_default());
}

/// The agent a spawn is for, as the endpoint step needs it.
struct Who {
    id: String,
    slug: String,
}

fn who_of(mstore: &crate::backend::storage::store::Store, block_id: &str, env: &HashMap<String, String>) -> Option<Who> {
    let instance = mstore.instance_get_active_for_block(block_id).ok().flatten()?;
    let def = mstore.agent_def_get(&instance.definition_id).ok().flatten()?;
    let slug = if def.slug.is_empty() { env.get("AGENTMUX_AGENT_SLUG").cloned().unwrap_or_default() } else { def.slug };
    Some(Who { id: def.id, slug })
}

/// The identity step of a spawn (the account's variables over the agent's own), then this step, then the
/// note of which account the run uses. `base_env_vars` is the agent's own environment.
pub(crate) async fn inject_identity(
    mstore: std::sync::Arc<crate::backend::storage::store::Store>,
    id_store: std::sync::Arc<crate::backend::storage::store::Store>,
    identity_store: std::sync::Arc<crate::backend::storage::store::Store>,
    broker: Option<std::sync::Arc<crate::backend::mps::Broker>>,
    block_meta: &crate::backend::obj::MetaMapType,
    block_id: &str,
    base_env_vars: HashMap<String, String>,
) -> Result<HashMap<String, String>, crate::identity::resolver::SpawnGateError> {
    let (own, mstore_after) = (base_env_vars.clone(), mstore.clone());
    let mut env_vars = crate::identity::token_warmup::inject_after_warmup(
        mstore,
        id_store,
        identity_store.clone(),
        broker,
        block_id.to_string(),
        base_env_vars,
    )
    .await?;
    guard(&mut env_vars, &own, block_meta, block_id, mstore_after.clone(), identity_store.clone()).await;
    super::run_account::note_run_account(&mstore_after, &identity_store, block_id);
    Ok(env_vars)
}

/// The endpoint step of a spawn (`build::build_persistent_spawn_env`): `own` is what the agent's own
/// environment asked for, `env_vars` the spawn environment after the account's variables went in. A
/// Claude Code agent that asked for a custom server has the account's login taken out and its own
/// keys put in (`apply`). Does nothing for another harness or for Anthropic's own address.
pub(crate) async fn guard(
    env_vars: &mut HashMap<String, String>,
    own: &HashMap<String, String>,
    block_meta: &crate::backend::obj::MetaMapType,
    block_id: &str,
    mstore: std::sync::Arc<crate::backend::storage::store::Store>,
    identity_store: std::sync::Arc<crate::backend::storage::store::Store>,
) {
    if crate::backend::obj::meta_get_string(block_meta, "agentProvider", "claude") != "claude" {
        return;
    }
    let Some(endpoint) = requested(own) else { return };
    let Some(provider) = crate::backend::providers::get_provider("claude") else { return };
    let (spawn_env, block) = (env_vars.clone(), block_id.to_string());
    let done = tokio::task::spawn_blocking(move || {
        let mut env = spawn_env;
        let who = who_of(&mstore, &block, &env);
        let slug = who.as_ref().map(|w| w.slug.clone()).unwrap_or_default();
        let shared = crate::backend::providers::default_auth_dir(provider).unwrap_or_default();
        let dir = private_config_dir(&shared, &slug);
        if let Err(e) = crate::backend::providers::prepare_provider_auth_dir(provider, &dir) {
            tracing::warn!(block_id = %block, dir = %dir, error = %e, "endpoint: could not prepare the agent's private config directory");
        }
        let resolve = |entry_id: &str| -> Option<String> {
            let who = who.as_ref()?;
            match crate::backend::vault::endpoint_key::for_launch(&identity_store, entry_id, &who.id, &who.slug) {
                Ok(value) => Some(value.to_string()),
                Err(why) => {
                    tracing::warn!(agent = %who.slug, error = %why, "endpoint: a Vault key named in the agent's environment was not put in");
                    None
                }
            }
        };
        apply(&mut env, &endpoint, &dir, &resolve);
        env
    })
    .await;
    match done {
        Ok(env) => *env_vars = env,
        // The step died: the agent starts with no login at all rather than with the person's.
        Err(e) => {
            tracing::error!(block_id = %block_id, error = %e, "endpoint: the step failed; the agent is started without a login");
            for name in ACCOUNT_VARIABLES.iter().copied().chain(["CLAUDE_CONFIG_DIR"]) {
                env_vars.remove(name);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn account() -> HashMap<String, String> {
        env(&[
            ("CLAUDE_CONFIG_DIR", "C:/shared/providers/claude"),
            ("CLAUDE_CODE_OAUTH_TOKEN", "sk-ant-oat01-PERSONS-LOGIN"),
            ("ANTHROPIC_API_KEY", "sk-ant-api03-PERSONS-KEY"),
            ("ANTHROPIC_AUTH_TOKEN", "persons-bearer"),
            ("TZ", "UTC"),
        ])
    }

    fn none(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn anthropic_and_no_url_are_not_an_endpoint() {
        assert_eq!(requested(&env(&[])), None);
        assert_eq!(requested(&env(&[("ANTHROPIC_BASE_URL", "")])), None);
        assert_eq!(requested(&env(&[("ANTHROPIC_BASE_URL", "https://api.anthropic.com")])), None);
        assert_eq!(requested(&env(&[("ANTHROPIC_BASE_URL", "https://api.anthropic.com/v1")])), None);
    }

    #[test]
    fn a_lan_server_a_proxy_and_a_lookalike_are_endpoints() {
        for url in ["http://192.0.2.68:11434", "https://proxy.example.com", "https://api.anthropic.com.evil.example", "http://api.anthropic.com"] {
            let e = requested(&env(&[("ANTHROPIC_BASE_URL", url), ("ANTHROPIC_AUTH_TOKEN", "ollama")])).unwrap();
            assert_eq!(e.base_url, url);
            assert_eq!(e.auth_token.as_deref(), Some("ollama"));
        }
    }

    #[test]
    fn the_persons_sign_in_is_not_sent_to_a_custom_server() {
        let endpoint = requested(&env(&[("ANTHROPIC_BASE_URL", "http://192.0.2.68:11434"), ("ANTHROPIC_AUTH_TOKEN", "ollama")])).unwrap();
        let mut spawn = account();
        apply(&mut spawn, &endpoint, "C:/shared/providers/claude-endpoint/lolly", &none);
        assert_eq!(spawn["CLAUDE_CONFIG_DIR"], "C:/shared/providers/claude-endpoint/lolly");
        assert_eq!(spawn["ANTHROPIC_BASE_URL"], "http://192.0.2.68:11434");
        assert_eq!(spawn["ANTHROPIC_AUTH_TOKEN"], "ollama");
        assert_eq!(spawn["ANTHROPIC_API_KEY"], "", "the account's key is emptied");
        assert!(!spawn.contains_key("CLAUDE_CODE_OAUTH_TOKEN"));
        assert!(!spawn.values().any(|v| v.contains("PERSONS") || v.contains("persons-bearer")), "{spawn:?}");
        assert_eq!(spawn["TZ"], "UTC", "other variables are left alone");
    }

    #[test]
    fn with_no_key_given_the_cli_gets_a_placeholder_so_it_does_not_ask_for_a_sign_in() {
        let endpoint = requested(&env(&[("ANTHROPIC_BASE_URL", "http://localhost:11434")])).unwrap();
        let mut spawn = account();
        apply(&mut spawn, &endpoint, "/d", &none);
        assert_eq!(spawn["ANTHROPIC_AUTH_TOKEN"], NO_KEY_PLACEHOLDER);
        assert_eq!(spawn["ANTHROPIC_API_KEY"], "");
    }

    #[test]
    fn an_api_key_alone_is_sent_as_the_key_with_no_token_variable() {
        let endpoint = requested(&env(&[("ANTHROPIC_BASE_URL", "http://localhost:8080"), ("ANTHROPIC_API_KEY", "local")])).unwrap();
        let mut spawn = account();
        apply(&mut spawn, &endpoint, "/d", &none);
        assert_eq!(spawn["ANTHROPIC_API_KEY"], "local");
        assert!(!spawn.contains_key("ANTHROPIC_AUTH_TOKEN"), "{spawn:?}");
    }

    #[test]
    fn a_vault_reference_is_replaced_by_the_entrys_value_and_a_refused_one_by_nothing() {
        let endpoint = requested(&env(&[("ANTHROPIC_BASE_URL", "https://llm.example.com"), ("ANTHROPIC_AUTH_TOKEN", "vault:k1")])).unwrap();
        let mut spawn = account();
        apply(&mut spawn, &endpoint, "/d", &|id| (id == "k1").then(|| "the-real-key".to_string()));
        assert_eq!(spawn["ANTHROPIC_AUTH_TOKEN"], "the-real-key");
        let mut refused = account();
        apply(&mut refused, &endpoint, "/d", &none);
        assert_eq!(refused["ANTHROPIC_AUTH_TOKEN"], NO_KEY_PLACEHOLDER, "no value, and never the account's");
        assert!(!refused.values().any(|v| v.contains("vault:") || v.contains("PERSONS")), "{refused:?}");
    }

    #[test]
    fn the_private_config_directory_is_beside_the_shared_one_per_agent() {
        // Windows joins with a backslash; the layout is what matters.
        let dir = |shared: &str, slug: &str| private_config_dir(shared, slug).replace(std::path::MAIN_SEPARATOR, "/");
        assert_eq!(dir("C:/h/shared/providers/claude", "lolly"), "C:/h/shared/providers/claude-endpoint/lolly");
        assert_eq!(dir("/h/shared/providers/claude", "../evil/x"), "/h/shared/providers/claude-endpoint/___evil_x");
        assert_eq!(dir("/h/shared/providers/claude", ""), "/h/shared/providers/claude-endpoint/agent");
        assert!(!dir("/h/shared/providers/claude", "lolly").starts_with("/h/shared/providers/claude/"));
    }
}
