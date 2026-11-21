// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The per-agent token a harness sends to the gateway in place of a model key, minted the way a
//! container token is (`container::credential`): kept by digest, valid for one route, attributed to
//! one agent. It lives for this srv launch, as the instance key an agent is launched with does.

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

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
    /// The one address the token may be used from: the container's own on the agents network, set at the start of each
    /// container turn, on every platform (on Docker Desktop srv learns it from the gateway forwarder's stamp). `None` only
    /// for a host agent's token.
    pub peer: Option<IpAddr>,
}

#[derive(Default)]
struct Registry {
    by_digest: HashMap<[u8; 32], Grant>,
    by_agent: HashMap<(String, String), String>,
    /// The key-route token that [`renew_for_account`] last replaced for an agent and route (one per agent and route), and whose
    /// it was: only so a turn that was set up with it can still say whose turn it is ([`current_grant_of_replaced`]). It
    /// authorizes nothing.
    replaced: HashMap<[u8; 32], (String, String)>,
    /// When each live token was made, so an event about a container that ended can leave alone a token made after it.
    minted: HashMap<[u8; 32], Instant>,
}

fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(Default::default)
}

fn digest(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

fn mint() -> String {
    let token = format!("{TOKEN_PREFIX}{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
    // A token an agent's process holds is never written into a transcript.
    crate::backend::transcript::withhold::register(&token);
    token
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
    reg.minted.entry(digest(&token)).or_insert_with(Instant::now);
    reg.by_agent.insert(key, token.clone());
    // A token a container turn is running on keeps its address when the next message's environment asks for it again.
    let peer = reg.by_digest.get(&digest(&token)).and_then(|g| g.peer);
    reg.by_digest.insert(digest(&token), Grant { agent: agent.to_string(), def_id: def_id.to_string(), route: route.to_string(), account: account.to_string(), peer });
    token
}

/// A gateway route's token (the key route or a model route) for a container turn, accepted only from `peer` when there is one. The agent's earlier token stops
/// working at once unless it already holds exactly this grant (the same agent, account and address): two panes of one
/// container agent run turns that overlap, and the second must not end the token the first is running on. A container's
/// token ends with the container ([`revoke_agent`]), so a recreated container, a new address or another account is always
/// given a new one.
pub fn renew_for_account(agent: &str, def_id: &str, route: &str, account: &str, peer: Option<IpAddr>) -> String {
    let mut reg = registry().lock().unwrap();
    let key = (def_id.to_string(), route.to_string());
    let grant = Grant { agent: agent.to_string(), def_id: def_id.to_string(), route: route.to_string(), account: account.to_string(), peer };
    if let Some(old) = reg.by_agent.get(&key).cloned() {
        if reg.by_digest.get(&digest(&old)) == Some(&grant) {
            return old;
        }
        reg.by_agent.remove(&key);
        reg.by_digest.remove(&digest(&old));
        reg.minted.remove(&digest(&old));
        // Only the latest replaced token is kept for this agent and route: the ones before it are not asked for again.
        reg.replaced.retain(|_, k| *k != key);
        reg.replaced.insert(digest(&old), key.clone());
    }
    let token = mint();
    reg.by_agent.insert(key, token.clone());
    reg.by_digest.insert(digest(&token), grant);
    reg.minted.insert(digest(&token), Instant::now());
    token
}

/// The agent's current grant for a token that [`renew_for_account`] replaced, while the agent still holds one for that route:
/// a message queued during a container agent's first turn was set up with the token that turn then replaced. The replaced
/// token itself is never accepted ([`grant_for`] says none); this only names whose turn it is. Gone with the agent's token.
pub fn current_grant_of_replaced(token: &str) -> Option<Grant> {
    let reg = registry().lock().unwrap();
    let key = reg.replaced.get(&digest(token))?;
    reg.by_digest.get(&digest(reg.by_agent.get(key)?)).cloned()
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
    let key = (def_id.to_string(), route.to_string());
    end_token(&mut reg, &key);
}

/// Ends every token for `route` that was made for the agent `name` names: its definition id or its routing id (slug). A
/// container is named from the agent's id, so this is what its stop, recreation and removal can say.
pub fn revoke_agent(name: &str, route: &str) {
    revoke_agent_older_than(name, route, Duration::ZERO);
}

/// [`revoke_agent`] for the tokens made at least `min_age` ago. A container that ended outside AgentMux is only heard of
/// from Docker's event stream, which can run behind: a token made since (for the container that took its place) is left alone.
pub fn revoke_agent_older_than(name: &str, route: &str, min_age: Duration) {
    end_agent_tokens(name, Some(route), min_age);
}

/// [`revoke_agent`] for every route: a container's stop, recreation or removal ends all of its agent's gateway tokens.
pub fn revoke_agent_everywhere(name: &str) {
    end_agent_tokens(name, None, Duration::ZERO);
}

/// [`revoke_agent_older_than`] for every route: a container's stop ends all of its agent's gateway tokens, model routes
/// and the key route alike.
pub fn revoke_agent_everywhere_older_than(name: &str, min_age: Duration) {
    end_agent_tokens(name, None, min_age);
}

fn end_agent_tokens(name: &str, route: Option<&str>, min_age: Duration) {
    let mut reg = registry().lock().unwrap();
    let ended: Vec<(String, String)> = reg
        .by_agent
        .iter()
        .filter(|((_, r), _)| route.map_or(true, |route| r == route))
        .filter(|(_, token)| reg.by_digest.get(&digest(token)).is_some_and(|g| g.def_id == name || g.agent == name))
        .filter(|(_, token)| reg.minted.get(&digest(token)).map_or(true, |at| at.elapsed() >= min_age))
        .map(|(key, _)| key.clone())
        .collect();
    for key in ended {
        end_token(&mut reg, &key);
    }
}

/// Drops the token for `key` and every record of it.
fn end_token(reg: &mut Registry, key: &(String, String)) {
    if let Some(token) = reg.by_agent.remove(key) {
        reg.by_digest.remove(&digest(&token));
        reg.minted.remove(&digest(&token));
    }
    reg.replaced.retain(|_, k| k != key);
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
    fn a_turn_that_overlaps_another_on_the_same_grant_keeps_its_token_and_any_change_ends_it() {
        let ip: IpAddr = "172_30_0_9".replace('_', ".").parse().unwrap();
        let other: IpAddr = "172_30_0_10".replace('_', ".").parse().unwrap();
        let first = renew_for_account("ov-agent", "ov-def", "anthropic", "acct", Some(ip));
        // A second pane's turn on the same container: the first turn's token keeps working.
        assert_eq!(renew_for_account("ov-agent", "ov-def", "anthropic", "acct", Some(ip)), first);
        assert!(grant_for(&first).is_some());
        // A new address (the container was made again) or another account is given a new token and the old one ends.
        let moved = renew_for_account("ov-agent", "ov-def", "anthropic", "acct", Some(other));
        assert_ne!(moved, first);
        assert_eq!(grant_for(&first), None);
        let switched = renew_for_account("ov-agent", "ov-def", "anthropic", "acct-2", Some(other));
        assert_ne!(switched, moved);
        assert_eq!(grant_for(&moved), None);
        revoke("ov-def", "anthropic");
    }

    #[test]
    fn a_replaced_token_names_the_agents_current_grant_only_while_the_agent_holds_one() {
        let ip: IpAddr = "172_30_0_9".replace('_', ".").parse().unwrap();
        let spawned = token_for_account("rp-agent", "rp-def", "anthropic", "acct");
        let bound = renew_for_account("rp-agent", "rp-def", "anthropic", "acct", Some(ip));
        assert_eq!(grant_for(&spawned), None, "the replaced token is never accepted");
        assert_eq!(current_grant_of_replaced(&spawned), grant_for(&bound));
        assert_eq!(current_grant_of_replaced(&bound), None, "a live token is not a replaced one");
        assert_eq!(current_grant_of_replaced("amxg_never-made"), None);
        revoke_agent("rp-def", "anthropic");
        assert_eq!(current_grant_of_replaced(&spawned), None, "it ends with the agent's token");
        // The same after a revoke by id.
        let spawned = token_for_account("rp-agent", "rp-def", "anthropic", "acct");
        renew_for_account("rp-agent", "rp-def", "anthropic", "acct", Some(ip));
        revoke("rp-def", "anthropic");
        assert_eq!(current_grant_of_replaced(&spawned), None);
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

    #[test]
    fn only_the_latest_replaced_token_is_kept_for_an_agent_and_route() {
        let ip: IpAddr = "172_30_0_9".replace('_', ".").parse().unwrap();
        let other: IpAddr = "172_30_0_10".replace('_', ".").parse().unwrap();
        let first = token_for_account("lr-agent", "lr-def", "anthropic", "acct");
        let second = renew_for_account("lr-agent", "lr-def", "anthropic", "acct", Some(ip));
        let third = renew_for_account("lr-agent", "lr-def", "anthropic", "acct", Some(other));
        assert_ne!(second, third);
        let reg = registry().lock().unwrap();
        let kept = reg.replaced.values().filter(|k| **k == ("lr-def".to_string(), "anthropic".to_string())).count();
        assert_eq!(kept, 1, "one entry per agent and route, not one per renewal");
        drop(reg);
        assert_eq!(current_grant_of_replaced(&second), grant_for(&third), "the latest replaced token still names the agent's turn");
        assert_eq!(current_grant_of_replaced(&first), None);
        assert_eq!(grant_for(&first), None);
        revoke("lr-def", "anthropic");
    }

    #[test]
    fn an_older_token_ends_with_a_container_that_stopped_and_a_newer_one_stays() {
        let old = token_for_account("ag-agent", "ag-def", "anthropic", "acct");
        // Just made: an event about the container that ended before it was made must not end it.
        revoke_agent_older_than("ag-agent", "anthropic", Duration::from_secs(3600));
        assert!(grant_for(&old).is_some(), "a token made within the age is left alone");
        // Made longer ago than the age: it ends.
        revoke_agent_older_than("ag-agent", "anthropic", Duration::ZERO);
        assert_eq!(grant_for(&old), None);
        // A new token is made for the next turn, never the ended one.
        let next = token_for_account("ag-agent", "ag-def", "anthropic", "acct");
        assert_ne!(next, old);
        revoke("ag-def", "anthropic");
    }
}
