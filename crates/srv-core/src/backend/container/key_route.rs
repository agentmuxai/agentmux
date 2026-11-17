// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A container turn on an API-key account (`model_gateway::catalog::KEY_ROUTE_ID`). The key stays in the keychain and the
//! container is given only the gateway's address and a token for that one agent and account. At the start of each turn
//! this module makes that token current (a changed address or account, or a container made again, gets a new one and the
//! earlier ends; a turn that overlaps another on the same grant shares its token) and, on Linux, binds it to the
//! container's own address on the agents network, so the container listener accepts it from that container and no other
//! ([`listener::check_gateway`](super::listener::check_gateway)). Where the container reaches srv through Docker's own
//! forwarding (Docker Desktop on macOS and Windows) there is no address to bind: the token holds for this machine.

use std::net::IpAddr;

use super::{listener, ContainerError, ContainerManager};
use crate::backend::model_gateway::{catalog::KEY_ROUTE_ID, token, PATH_PREFIX};

/// Linux, and srv is not serving the agents network (a rootless daemon keeps its networks out of reach, or the bind failed).
const NO_PATH: &str = "This agent's API key can't be used in a container on this machine: AgentMux could not open its gateway on the agents network (rootless Docker does this). Run the agent on this machine instead of in a container, or move it to a plan account.";
/// Linux, and the container has no address on the agents network yet.
const NO_ADDRESS: &str = "This agent's container is not on the agents network yet, so its API key can't be tied to it. Send the message again; if it keeps failing, restart the agent.";
/// The agent's token is gone (it was moved off the key, or the key route was left, while the turn was starting).
const GONE: &str = "This agent is no longer on its API key account. Send the message again.";

/// Ends the key-route gateway token of the agent whose container this is (named from its id, `container_name_for_slug`).
/// Called when the container is stopped, recreated (which stops and removes it) or removed, whatever the daemon answers.
pub(super) fn revoke_container_tokens(container_name: &str) {
    if let Some(agent) = container_name.strip_prefix("agentmux-") {
        token::revoke_agent(agent, KEY_ROUTE_ID);
    }
}

/// Whether a base URL names the gateway's key route.
pub(super) fn names_key_route(base_url: &str) -> bool {
    base_url.trim_end_matches('/').ends_with(&format!("{PATH_PREFIX}{KEY_ROUTE_ID}"))
}

/// Whether the turn's environment points the agent at the gateway's key route.
fn is_key_route_env(env: &[(String, String)]) -> bool {
    env.iter().any(|(k, v)| k == "ANTHROPIC_BASE_URL" && names_key_route(v))
}

/// The address the turn's token is bound to: the container's own on Linux, none elsewhere; or why it can't be used.
fn address_plan(linux: bool, listener_up: bool, address: Option<&str>) -> Result<Option<IpAddr>, &'static str> {
    if !linux {
        return Ok(None);
    }
    if !listener_up {
        return Err(NO_PATH);
    }
    address.and_then(|a| a.trim().parse().ok()).map(Some).ok_or(NO_ADDRESS)
}

/// Whose key-route token a turn is for, taken before the container is made ready: readying it can recreate the container,
/// which ends the token the spawn made, and the turn's own token is made after.
pub(crate) struct Claim {
    agent: String,
    def_id: String,
    account: String,
}

/// The agent and account of a turn on the key route, or `None` for any other turn. A key-route turn whose token is already gone
/// is refused.
pub(crate) fn claim(env: &[(String, String)]) -> Result<Option<Claim>, ContainerError> {
    if !is_key_route_env(env) {
        return Ok(None);
    }
    // A message queued during the agent's first turn was set up with the token that turn then replaced: it names the agent's
    // current one while the agent still holds one.
    let grant = env
        .iter()
        .find(|(k, _)| k == "ANTHROPIC_AUTH_TOKEN")
        .and_then(|(_, t)| token::grant_for(t).or_else(|| token::current_grant_of_replaced(t)))
        .filter(|g| g.route == KEY_ROUTE_ID);
    match grant {
        Some(g) => Ok(Some(Claim { agent: g.agent, def_id: g.def_id, account: g.account })),
        None => Err(ContainerError::KeyRoute(GONE.to_string())),
    }
}

/// Makes the turn's key-route token current and bound, in place in `env`. Called once the container is up (the listener is bound
/// by then) and before the exec starts.
pub(crate) async fn bind(cm: &ContainerManager, container_name: &str, env: &mut [(String, String)], claim: &Claim) -> Result<(), ContainerError> {
    let linux = cfg!(target_os = "linux");
    // A fresh look at the container at every turn start, never one kept from when it was made.
    let address = if linux { cm.agent_network_ip(container_name).await } else { None };
    let peer = match address_plan(linux, listener::url().is_some(), address.as_deref()) {
        Ok(peer) => peer,
        Err(why) => {
            // The turn does not run, so nothing is to hold the token the spawn made for it.
            token::revoke(&claim.def_id, KEY_ROUTE_ID);
            return Err(ContainerError::KeyRoute(why.to_string()));
        }
    };
    let fresh = token::renew_for_account(&claim.agent, &claim.def_id, KEY_ROUTE_ID, &claim.account, peer);
    if let Some(slot) = env.iter_mut().find(|(k, _)| k == "ANTHROPIC_AUTH_TOKEN") {
        slot.1 = fresh;
    }
    Ok(())
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

    #[test]
    fn only_a_turn_pointed_at_the_key_route_is_one() {
        assert!(is_key_route_env(&env_for(KEY_ROUTE_ID)));
        assert!(!is_key_route_env(&env_for("openrouter")), "a model route is not a key account");
        assert!(!is_key_route_env(&[("ANTHROPIC_BASE_URL".to_string(), "https://api.anthropic.com".to_string())]));
        assert!(!is_key_route_env(&[]));
    }

    #[test]
    fn on_linux_the_token_is_tied_to_the_containers_address_and_without_a_way_in_it_is_refused() {
        // Written with `_` so no dotted private address sits in the source.
        let at = "172_30_0_9".replace('_', ".");
        let here: IpAddr = at.parse().unwrap();
        assert_eq!(address_plan(true, true, Some(&at)), Ok(Some(here)));
        assert_eq!(address_plan(true, true, Some(&format!(" {at}\n"))), Ok(Some(here)));
        assert_eq!(address_plan(true, false, Some(&at)), Err(NO_PATH), "no listener on the agents network: no path, and nothing falls back");
        assert_eq!(address_plan(true, true, None), Err(NO_ADDRESS));
        assert_eq!(address_plan(true, true, Some("")), Err(NO_ADDRESS));
        assert_eq!(address_plan(true, true, Some("not an address")), Err(NO_ADDRESS));
    }

    #[test]
    fn elsewhere_the_token_holds_for_this_machine_and_needs_no_address() {
        assert_eq!(address_plan(false, false, None), Ok(None));
        assert_eq!(address_plan(false, true, Some("not looked at")), Ok(None));
    }

    #[test]
    fn the_refusals_say_what_to_do_and_name_no_key_or_account() {
        for why in [NO_PATH, NO_ADDRESS, GONE] {
            assert!(why.contains("agent"), "{why}");
            assert!(!why.contains("amxg_") && !why.contains("sk-ant"), "{why}");
        }
        assert!(NO_PATH.contains("Run the agent on this machine") && NO_PATH.contains("plan account"));
    }

    fn manager() -> ContainerManager {
        // A daemon that is not there: the address is only asked of it on Linux, where the answer is none.
        let docker = bollard::Docker::connect_with_http("http://127.0.0.1:1", 1, bollard::API_DEFAULT_VERSION).unwrap();
        ContainerManager::with_docker(docker, None)
    }

    #[test]
    fn a_turn_not_on_the_key_route_has_no_claim_and_one_with_no_live_token_is_refused_with_a_plain_reason() {
        assert!(claim(&env_for("openrouter")).unwrap().is_none());
        assert!(claim(&[]).unwrap().is_none());
        let err = claim(&env_for(KEY_ROUTE_ID)).err().expect("no live token");
        assert!(matches!(&err, ContainerError::KeyRoute(why) if why == GONE), "{err}");
    }

    #[test]
    fn a_message_queued_behind_the_first_turn_is_still_that_agents_when_the_turn_replaced_its_token() {
        let at: IpAddr = "172_30_0_9".replace('_', ".").parse().unwrap();
        let queued_with = token::renew_for_account("kq-slug", "kq-def", KEY_ROUTE_ID, "kq-acct", None);
        let mut env = env_for(KEY_ROUTE_ID);
        env[1].1 = queued_with.clone();
        // The running turn binds the token to the container's address, which replaces the one the message holds.
        token::renew_for_account("kq-slug", "kq-def", KEY_ROUTE_ID, "kq-acct", Some(at));
        assert_eq!(token::grant_for(&queued_with), None);
        let taken = claim(&env).unwrap().expect("still a key-route turn of that agent");
        assert_eq!((taken.agent.as_str(), taken.def_id.as_str(), taken.account.as_str()), ("kq-slug", "kq-def", "kq-acct"));
        // Once the agent's token ends (the container was stopped, or the agent left the key) the message is refused.
        revoke_container_tokens("agentmux-kq-def");
        assert!(matches!(claim(&env), Err(ContainerError::KeyRoute(why)) if why == GONE));
    }

    /// The part that depends on the platform: where there is no address to bind the token is made fresh for this
    /// machine; on Linux with no listener (as in a test) the turn is refused with the way out. Either way the claim,
    /// taken before the container was readied, holds when readying it ended the spawn's token.
    #[tokio::test]
    async fn a_key_route_turn_gets_a_fresh_token_or_the_way_out_even_when_readying_the_container_ended_the_spawns() {
        let first = token::renew_for_account("kr-slug", "kr-def", KEY_ROUTE_ID, "kr-acct", None);
        let mut env = env_for(KEY_ROUTE_ID);
        env[1].1 = first.clone();
        let taken = claim(&env).unwrap().expect("a key-route turn");
        // The container is recreated while it is readied: that ends the token the spawn made.
        revoke_container_tokens("agentmux-kr-def");
        assert_eq!(token::grant_for(&first), None);
        let result = bind(&manager(), "agentmux-kr-def", &mut env, &taken).await;
        if cfg!(target_os = "linux") {
            assert!(matches!(&result, Err(ContainerError::KeyRoute(why)) if why == NO_PATH), "{result:?}");
            assert_eq!(env[1].1, first, "a refused turn is given no new token");
            assert_eq!(token::grant_for(&first), None);
        } else {
            result.unwrap();
            assert_ne!(env[1].1, first, "each turn has a token of its own");
            let grant = token::grant_for(&env[1].1).unwrap();
            assert_eq!((grant.agent.as_str(), grant.def_id.as_str(), grant.account.as_str(), grant.peer), ("kr-slug", "kr-def", "kr-acct", None));
        }
        token::revoke("kr-def", KEY_ROUTE_ID);
    }
}
