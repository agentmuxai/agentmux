// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A container turn on the model gateway: a model route, or an API-key account (`model_gateway::catalog::KEY_ROUTE_ID`).
//! The container is given only the gateway's address and a token for that one agent (and account); a key stays in the
//! keychain. At the start of each turn this module makes that token current (a changed address or account, or a container
//! made again, gets a new one and the earlier ends; a turn that overlaps another on the same grant shares its token) and ties
//! it to the container's own address on the agents network, so the gateway accepts it from that container and no other:
//!
//! - **Linux:** srv listens on the network's gateway address and sees each container's own
//!   ([`listener::check_gateway`](super::listener::check_gateway)).
//! - **Docker Desktop (macOS, Windows):** every container reaches srv from one address, so the container is pointed at a
//!   small forwarder on the agents network ([`gateway_fwd`]) which sees the container's own address and says it to srv in a
//!   stamp ([`agentmux_common::gateway_stamp`]). With no forwarder there is no way in, and the turn is refused.

use std::net::IpAddr;

use super::{gateway_fwd, listener, ContainerError, ContainerManager};
use crate::backend::model_gateway::{catalog::KEY_ROUTE_ID, token, PATH_PREFIX};

/// Linux, and srv is not serving the agents network (a rootless daemon keeps its networks out of reach, or the bind failed).
const NO_PATH: &str = "This agent's model access can't be used in a container on this machine: AgentMux could not open its gateway on the agents network (rootless Docker does this). Run the agent on this machine instead of in a container, or move it to a plan account.";
/// The container has no address on the agents network yet.
const NO_ADDRESS: &str = "This agent's container is not on the agents network yet, so its model access can't be tied to it. Send the message again; if it keeps failing, restart the agent.";
/// The agent's token is gone (it was moved off the key, or the route was left, while the turn was starting).
const GONE: &str = "This agent's model access for this turn has ended. Send the message again.";

/// Ends every gateway token of the agent whose container this is (named from its id, `container_name_for_slug`).
/// Called when the container is stopped, recreated (which stops and removes it) or removed, whatever the daemon answers.
pub(super) fn revoke_container_tokens(container_name: &str) {
    if let Some(agent) = container_name.strip_prefix("agentmux-") {
        token::revoke_agent_everywhere(agent);
    }
}

/// The route a base URL names on the gateway, if it names one.
pub(super) fn route_named(base_url: &str) -> Option<&str> {
    let tail = &base_url[base_url.find(PATH_PREFIX)? + PATH_PREFIX.len()..];
    Some(tail.split(['/', '?']).next().unwrap_or("")).filter(|r| !r.is_empty())
}

/// Whether a base URL names the gateway's key route.
pub(super) fn names_key_route(base_url: &str) -> bool {
    route_named(base_url) == Some(KEY_ROUTE_ID)
}

/// The route the turn's environment points the agent at, if it is the gateway's.
fn gateway_route(env: &[(String, String)]) -> Option<&str> {
    env.iter().find(|(k, _)| k == "ANTHROPIC_BASE_URL").and_then(|(_, v)| route_named(v))
}

/// The address the turn's token is tied to, or why it can't be. On Linux that needs the listener on the agents network.
fn address_plan(linux: bool, listener_up: bool, address: Option<&str>) -> Result<IpAddr, &'static str> {
    if linux && !listener_up {
        return Err(NO_PATH);
    }
    address.and_then(|a| a.trim().parse().ok()).ok_or(NO_ADDRESS)
}

/// Whose gateway token a turn is for, taken before the container is made ready: readying it can recreate the container,
/// which ends the token the spawn made, and the turn's own token is made after.
pub(crate) struct Claim {
    agent: String,
    def_id: String,
    route: String,
    account: String,
}

/// The agent, route and account of a turn on the gateway, or `None` for any other turn. A turn whose token is already gone
/// is refused.
pub(crate) fn claim(env: &[(String, String)]) -> Result<Option<Claim>, ContainerError> {
    let Some(route) = gateway_route(env) else {
        return Ok(None);
    };
    // A message queued during the agent's first turn was set up with the token that turn then replaced: it names the agent's
    // current one while the agent still holds one.
    let grant = env
        .iter()
        .find(|(k, _)| k == "ANTHROPIC_AUTH_TOKEN")
        .and_then(|(_, t)| token::grant_for(t).or_else(|| token::current_grant_of_replaced(t)))
        .filter(|g| g.route == route);
    match grant {
        Some(g) => Ok(Some(Claim { agent: g.agent, def_id: g.def_id, route: g.route, account: g.account })),
        None => Err(ContainerError::KeyRoute(GONE.to_string())),
    }
}

/// Makes the turn's gateway token current and tied to the container, in place in `env`; where there is no listener on the
/// agents network, also points the container at the forwarder. Called once the container is up (the listener is bound by
/// then) and before the exec starts.
pub(crate) async fn bind(cm: &ContainerManager, container_name: &str, env: &mut [(String, String)], claim: &Claim) -> Result<(), ContainerError> {
    let linux = cfg!(target_os = "linux");
    // A fresh look at the container at every turn start, never one kept from when it was made.
    let address = cm.agent_network_ip(container_name).await;
    let forwarder = if linux { Ok(None) } else { gateway_fwd::ensure(cm, container_name, env).await.map(Some) };
    tie(env, claim, linux, listener::url().is_some(), address.as_deref(), forwarder)
}

/// The part of [`bind`] that follows from what it found. `forwarder` is the forwarder's base URL where one is used.
fn tie(env: &mut [(String, String)], claim: &Claim, linux: bool, listener_up: bool, address: Option<&str>, forwarder: Result<Option<String>, String>) -> Result<(), ContainerError> {
    let refuse = |why: String| {
        // The turn does not run, so nothing is to hold the token the spawn made for it.
        token::revoke(&claim.def_id, &claim.route);
        Err(ContainerError::KeyRoute(why))
    };
    let peer = match address_plan(linux, listener_up, address) {
        Ok(peer) => peer,
        Err(why) => return refuse(why.to_string()),
    };
    let forwarder = match forwarder {
        Ok(url) => url,
        Err(why) => return refuse(why),
    };
    if let Some(base) = forwarder {
        point_at_forwarder(env, &base);
    }
    let fresh = token::renew_for_account(&claim.agent, &claim.def_id, &claim.route, &claim.account, Some(peer));
    if let Some(slot) = env.iter_mut().find(|(k, _)| k == "ANTHROPIC_AUTH_TOKEN") {
        slot.1 = fresh;
    }
    Ok(())
}

/// The gateway's base URL, on the forwarder's address instead of srv's.
fn point_at_forwarder(env: &mut [(String, String)], forwarder: &str) {
    if let Some((_, url)) = env.iter_mut().find(|(k, v)| k == "ANTHROPIC_BASE_URL" && v.contains(PATH_PREFIX)) {
        if let Some(at) = url.find(PATH_PREFIX) {
            *url = format!("{}{}", forwarder.trim_end_matches('/'), &url[at..]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_for(route: &str) -> Vec<(String, String)> {
        vec![
            ("ANTHROPIC_BASE_URL".to_string(), format!("http://host.docker.internal:1{PATH_PREFIX}{route}")),
            ("ANTHROPIC_AUTH_TOKEN".to_string(), "x".to_string()),
        ]
    }

    fn at() -> String {
        // Written with `_` so no dotted private address sits in the source.
        "172_30_0_9".replace('_', ".")
    }

    #[test]
    fn only_a_turn_pointed_at_the_gateway_is_one_and_it_names_its_route() {
        assert_eq!(gateway_route(&env_for(KEY_ROUTE_ID)), Some(KEY_ROUTE_ID));
        assert_eq!(gateway_route(&env_for("openrouter")), Some("openrouter"), "a model route's turn is tied to its container too");
        assert_eq!(gateway_route(&[("ANTHROPIC_BASE_URL".to_string(), "https://api.anthropic.com".to_string())]), None);
        assert_eq!(gateway_route(&[("ANTHROPIC_BASE_URL".to_string(), format!("http://h:1{PATH_PREFIX}")) ]), None, "no route named");
        assert_eq!(gateway_route(&[]), None);
        assert_eq!(route_named(&format!("http://h:1{PATH_PREFIX}anthropic/v1?x=1")), Some("anthropic"));
    }

    #[test]
    fn the_token_is_tied_to_the_containers_address_and_on_linux_without_a_way_in_it_is_refused() {
        let at = at();
        let here: IpAddr = at.parse().unwrap();
        assert_eq!(address_plan(true, true, Some(&at)), Ok(here));
        assert_eq!(address_plan(true, true, Some(&format!(" {at}\n"))), Ok(here));
        assert_eq!(address_plan(true, false, Some(&at)), Err(NO_PATH), "no listener on the agents network: no path, and nothing falls back");
        assert_eq!(address_plan(true, true, None), Err(NO_ADDRESS));
        assert_eq!(address_plan(true, true, Some("")), Err(NO_ADDRESS));
        assert_eq!(address_plan(true, true, Some("not an address")), Err(NO_ADDRESS));
    }

    #[test]
    fn elsewhere_the_token_is_tied_to_the_address_too_and_the_listener_is_not_needed() {
        let at = at();
        assert_eq!(address_plan(false, false, Some(&at)), Ok(at.parse().unwrap()));
        assert_eq!(address_plan(false, false, None), Err(NO_ADDRESS), "an address is what the forwarder vouches for: without it, no token");
        assert_eq!(address_plan(false, true, Some("not an address")), Err(NO_ADDRESS));
    }

    #[test]
    fn the_refusals_say_what_to_do_and_name_no_key_or_account() {
        for why in [NO_PATH, NO_ADDRESS, GONE] {
            assert!(why.contains("agent"), "{why}");
            assert!(!why.contains("amxg_") && !why.contains("sk-ant"), "{why}");
        }
        assert!(NO_PATH.contains("Run the agent on this machine") && NO_PATH.contains("plan account"));
    }

    #[test]
    fn a_turn_not_on_the_gateway_has_no_claim_and_one_with_no_live_token_is_refused_with_a_plain_reason() {
        assert!(claim(&[("ANTHROPIC_BASE_URL".to_string(), "https://api.anthropic.com".to_string())]).unwrap().is_none());
        assert!(claim(&[]).unwrap().is_none());
        let err = claim(&env_for(KEY_ROUTE_ID)).err().expect("no live token");
        assert!(matches!(&err, ContainerError::KeyRoute(why) if why == GONE), "{err}");
        let err = claim(&env_for("openrouter")).err().expect("no live token on a model route either");
        assert!(matches!(&err, ContainerError::KeyRoute(why) if why == GONE), "{err}");
    }

    #[test]
    fn a_message_queued_behind_the_first_turn_is_still_that_agents_when_the_turn_replaced_its_token() {
        let here: IpAddr = at().parse().unwrap();
        let queued_with = token::renew_for_account("kq-slug", "kq-def", KEY_ROUTE_ID, "kq-acct", None);
        let mut env = env_for(KEY_ROUTE_ID);
        env[1].1 = queued_with.clone();
        // The running turn binds the token to the container's address, which replaces the one the message holds.
        token::renew_for_account("kq-slug", "kq-def", KEY_ROUTE_ID, "kq-acct", Some(here));
        assert_eq!(token::grant_for(&queued_with), None);
        let taken = claim(&env).unwrap().expect("still a turn of that agent");
        assert_eq!((taken.agent.as_str(), taken.def_id.as_str(), taken.account.as_str()), ("kq-slug", "kq-def", "kq-acct"));
        // Once the agent's token ends (the container was stopped, or the agent left the key) the message is refused.
        revoke_container_tokens("agentmux-kq-def");
        assert!(matches!(claim(&env), Err(ContainerError::KeyRoute(why)) if why == GONE));
    }

    #[test]
    fn a_stopped_container_ends_every_route_of_its_agent_not_only_the_key() {
        let key = token::token_for_account("ev-slug", "ev-def", KEY_ROUTE_ID, "acct");
        let model = token::token_for("ev-slug", "ev-def", "openrouter");
        let other = token::token_for("ev-other", "ev-def-2", "openrouter");
        revoke_container_tokens("agentmux-ev-def");
        assert_eq!((token::grant_for(&key), token::grant_for(&model)), (None, None));
        assert!(token::grant_for(&other).is_some(), "another agent's token stays");
        token::revoke("ev-def-2", "openrouter");
    }

    /// What `tie` does with each way a turn can reach the gateway: the claim, taken before the container was readied, holds when
    /// readying it ended the spawn's token.
    #[test]
    fn a_turn_gets_a_fresh_token_tied_to_its_container_or_the_way_out() {
        let here: IpAddr = at().parse().unwrap();
        for route in [KEY_ROUTE_ID, "openrouter"] {
            let first = token::renew_for_account("kr-slug", "kr-def", route, "kr-acct", None);
            let mut env = env_for(route);
            env[1].1 = first.clone();
            let taken = claim(&env).unwrap().expect("a gateway turn");
            // The container is recreated while it is readied: that ends the token the spawn made.
            revoke_container_tokens("agentmux-kr-def");
            assert_eq!(token::grant_for(&first), None);
            // Linux: the listener sees the address; the base URL stays.
            tie(&mut env, &taken, true, true, Some(&at()), Ok(None)).unwrap();
            let grant = token::grant_for(&env[1].1).unwrap();
            assert_eq!((grant.agent.as_str(), grant.def_id.as_str(), grant.route.as_str(), grant.account.as_str(), grant.peer), ("kr-slug", "kr-def", route, "kr-acct", Some(here)));
            assert!(env[0].1.starts_with("http://host.docker.internal:1"), "{}", env[0].1);
            assert_ne!(env[1].1, first, "each turn has a token of its own");
            token::revoke("kr-def", route);
        }
    }

    #[test]
    fn on_docker_desktop_the_turn_is_pointed_at_the_forwarder_and_with_none_it_is_refused_with_no_new_token() {
        let here: IpAddr = at().parse().unwrap();
        let first = token::renew_for_account("kf-slug", "kf-def", KEY_ROUTE_ID, "kf-acct", None);
        let mut env = env_for(KEY_ROUTE_ID);
        env[1].1 = first.clone();
        let taken = claim(&env).unwrap().unwrap();
        tie(&mut env, &taken, false, false, Some(&at()), Ok(Some("http://agentmux-gateway-fwd:4799".to_string()))).unwrap();
        assert_eq!(env[0].1, format!("http://agentmux-gateway-fwd:4799{PATH_PREFIX}{KEY_ROUTE_ID}"));
        assert_eq!(token::grant_for(&env[1].1).unwrap().peer, Some(here), "the token holds for one container's address");
        // No forwarder: the turn is refused with the plain reason, the token ends and nothing falls back to srv's own address.
        let mut down = env_for(KEY_ROUTE_ID);
        down[1].1 = env[1].1.clone();
        let current = env[1].1.clone();
        let refused = tie(&mut down, &taken, false, false, Some(&at()), Err("the gateway forwarder is not running".to_string()));
        assert!(matches!(&refused, Err(ContainerError::KeyRoute(why)) if why == "the gateway forwarder is not running"), "{refused:?}");
        assert_eq!(down[0].1, env_for(KEY_ROUTE_ID)[0].1, "not pointed anywhere else");
        assert_eq!(token::grant_for(&current), None, "the refused turn leaves no live token");
        // No address either: refused.
        let again = token::renew_for_account("kf-slug", "kf-def", KEY_ROUTE_ID, "kf-acct", None);
        down[1].1 = again.clone();
        assert!(matches!(tie(&mut down, &taken, false, false, None, Ok(Some("http://f:1".to_string()))), Err(ContainerError::KeyRoute(why)) if why == NO_ADDRESS));
        assert_eq!(token::grant_for(&again), None);
    }
}
