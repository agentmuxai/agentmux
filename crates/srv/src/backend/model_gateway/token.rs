// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The per-agent token a harness sends to the gateway in place of a model key, minted the way a
//! container token is (`container::credential`): kept by digest, valid for one route, attributed to
//! one agent. It lives for this srv launch, as the instance key an agent is launched with does.

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// Prefix that marks a gateway token in logs and bug reports.
pub const TOKEN_PREFIX: &str = "amxg_";

/// What a token was minted for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grant {
    /// The agent's routing id (its slug), which every other record of it uses.
    pub agent: String,
    /// The agent definition's id, whose route and model the gateway reads.
    pub def_id: String,
    pub route: String,
}

#[derive(Default)]
struct Registry {
    by_digest: HashMap<[u8; 32], Grant>,
    by_agent: HashMap<(String, String), String>,
}

fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(Default::default)
}

fn digest(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

fn mint() -> String {
    format!("{TOKEN_PREFIX}{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}

/// The agent's token for `route`: minted on first use and reused after, so a relaunch of the same
/// agent keeps the token a running child still holds.
pub fn token_for(agent: &str, def_id: &str, route: &str) -> String {
    let mut reg = registry().lock().unwrap();
    let key = (def_id.to_string(), route.to_string());
    let token = reg.by_agent.get(&key).cloned().unwrap_or_else(mint);
    reg.by_agent.insert(key, token.clone());
    reg.by_digest.insert(digest(&token), Grant { agent: agent.to_string(), def_id: def_id.to_string(), route: route.to_string() });
    token
}

pub fn grant_for(token: &str) -> Option<Grant> {
    if !token.starts_with(TOKEN_PREFIX) {
        return None;
    }
    registry().lock().unwrap().by_digest.get(&digest(token)).cloned()
}

/// Ends the agent's access to `route` without touching its environment: the next request is refused.
pub fn revoke(def_id: &str, route: &str) {
    let mut reg = registry().lock().unwrap();
    if let Some(token) = reg.by_agent.remove(&(def_id.to_string(), route.to_string())) {
        reg.by_digest.remove(&digest(&token));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_is_reused_per_agent_and_route_and_can_be_revoked() {
        let a = token_for("gw-agent", "gw-def", "openrouter");
        assert!(a.starts_with(TOKEN_PREFIX));
        assert_eq!(token_for("gw-agent", "gw-def", "openrouter"), a);
        assert_ne!(token_for("gw-other", "gw-def-2", "openrouter"), a);
        let grant = grant_for(&a).unwrap();
        assert_eq!((grant.agent.as_str(), grant.route.as_str()), ("gw-agent", "openrouter"));
        revoke("gw-def", "openrouter");
        assert_eq!(grant_for(&a), None);
        assert_ne!(token_for("gw-agent", "gw-def", "openrouter"), a);
    }

    #[test]
    fn unknown_and_foreign_tokens_have_no_grant() {
        assert_eq!(grant_for("amxg_not-a-real-token"), None);
        assert_eq!(grant_for("amxc_not-a-gateway-token"), None);
        assert_eq!(grant_for(""), None);
    }
}
