// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The per-agent token a harness sends to the gateway in place of a model key, minted the way a
//! container token is (`container::credential`): kept by digest, valid for one route, attributed to
//! one agent. It lives for this srv launch, as the instance key an agent is launched with does.

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::net::IpAddr;
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
    /// The API-key account the token spends, for the key route (`catalog::KEY_ROUTE_ID`); empty on a model route.
    pub account: String,
    /// The one address the token may be used from: the container's own on the agents network (Linux), set at the start of
    /// each container turn. `None` for a host agent's token, and for a container's on a platform where the container reaches
    /// srv through Docker's own forwarding (Docker Desktop), where the token holds for this machine.
    pub peer: Option<IpAddr>,
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
    token_for_account(agent, def_id, route, "")
}

/// [`token_for`] for the key route: the token also names the account it spends, which the gateway checks against the
/// agent's link at every request. Asked again with another account, the same token now spends that one.
pub fn token_for_account(agent: &str, def_id: &str, route: &str, account: &str) -> String {
    let mut reg = registry().lock().unwrap();
    let key = (def_id.to_string(), route.to_string());
    let token = reg.by_agent.get(&key).cloned().unwrap_or_else(mint);
    reg.by_agent.insert(key, token.clone());
    // A token a container turn is running on keeps its address when the next message's environment asks for it again.
    let peer = reg.by_digest.get(&digest(&token)).and_then(|g| g.peer);
    reg.by_digest.insert(digest(&token), Grant { agent: agent.to_string(), def_id: def_id.to_string(), route: route.to_string(), account: account.to_string(), peer });
    token
}

/// A new token for the key route, for a container turn: the agent's earlier one stops working at once, and the new one is
/// accepted only from `peer` when there is one. A container's token is made at the start of each turn and ends with the
/// container ([`revoke_agent`]).
pub fn renew_for_account(agent: &str, def_id: &str, route: &str, account: &str, peer: Option<IpAddr>) -> String {
    let mut reg = registry().lock().unwrap();
    let key = (def_id.to_string(), route.to_string());
    if let Some(old) = reg.by_agent.remove(&key) {
        reg.by_digest.remove(&digest(&old));
    }
    let token = mint();
    reg.by_agent.insert(key, token.clone());
    reg.by_digest.insert(digest(&token), Grant { agent: agent.to_string(), def_id: def_id.to_string(), route: route.to_string(), account: account.to_string(), peer });
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

/// Ends every token for `route` that was made for the agent `name` names: its definition id or its routing id (slug). A
/// container is named from the agent's id, so this is what its stop, recreation and removal can say.
pub fn revoke_agent(name: &str, route: &str) {
    let mut reg = registry().lock().unwrap();
    let ended: Vec<(String, String)> = reg
        .by_agent
        .keys()
        .filter(|(def_id, r)| r == route && reg.by_agent.get(&(def_id.clone(), r.clone())).and_then(|t| reg.by_digest.get(&digest(t))).is_some_and(|g| g.def_id == name || g.agent == name))
        .cloned()
        .collect();
    for key in ended {
        if let Some(token) = reg.by_agent.remove(&key) {
            reg.by_digest.remove(&digest(&token));
        }
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
    fn a_renewed_token_replaces_the_old_one_and_holds_its_address() {
        let ip: IpAddr = "172_30_0_9".replace('_', ".").parse().unwrap();
        let first = token_for_account("tk-agent", "tk-def", "anthropic", "acct");
        let second = renew_for_account("tk-agent", "tk-def", "anthropic", "acct", Some(ip));
        assert_ne!(first, second);
        assert_eq!(grant_for(&first), None, "the earlier token ends when the next turn's begins");
        assert_eq!(grant_for(&second).unwrap().peer, Some(ip));
        // The next message's environment asks for the agent's token again: the running turn's token keeps its address.
        assert_eq!(token_for_account("tk-agent", "tk-def", "anthropic", "acct"), second);
        assert_eq!(grant_for(&second).unwrap().peer, Some(ip));
        revoke("tk-def", "anthropic");
    }

    #[test]
    fn revoking_an_agent_ends_its_token_for_that_route_only() {
        let key = token_for_account("rv-agent", "rv-def", "anthropic", "acct");
        let model = token_for("rv-agent", "rv-def", "openrouter");
        let other = token_for_account("rv-other", "rv-def-2", "anthropic", "acct");
        revoke_agent("rv-def", "anthropic");
        assert_eq!(grant_for(&key), None);
        assert!(grant_for(&model).is_some(), "another route's token stays");
        assert!(grant_for(&other).is_some(), "another agent's token stays");
        revoke("rv-def", "openrouter");
        revoke("rv-def-2", "anthropic");
    }

    #[test]
    fn unknown_and_foreign_tokens_have_no_grant() {
        assert_eq!(grant_for("amxg_not-a-real-token"), None);
        assert_eq!(grant_for("amxc_not-a-gateway-token"), None);
        assert_eq!(grant_for(""), None);
    }
}
