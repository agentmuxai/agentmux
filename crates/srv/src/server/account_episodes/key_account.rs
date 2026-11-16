// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! An Anthropic API key as an account a Claude agent can be moved onto
//! (`docs/specs/SPEC_AGENT_PANE_API_KEY_ACCOUNT_SWITCH_2026_11_16.md`).
//!
//! A key account is stored under provider `anthropic`, not `claude`, and its link stays under `anthropic`: the spawn
//! then carries it through the gateway's key route (`identity::resolver::inject_steps::inject_key_route`), so the agent
//! never holds the key. An agent has exactly one payer: moving it onto a key removes its plan link, and moving it onto a
//! plan removes the key link. Nothing here is reached by the automatic moves: a failover picks only accounts of the
//! agent's own provider, and `machine::scan` leaves a pane on a key out of the episodes.

use crate::backend::model_gateway::{catalog::KEY_ROUTE_ID, launch, token};
use crate::backend::obj;
use crate::backend::storage::identities::IdentityAccount;
use crate::server::AppState;

use super::switch::open_panes;
use super::canonical;

/// An Anthropic API key: provider `anthropic`, kind `api_key`.
pub fn is_key(a: &IdentityAccount) -> bool {
    a.kind == "api_key" && a.provider.eq_ignore_ascii_case(KEY_ROUTE_ID)
}

/// Whether `a` can be the account of an agent whose (canonical) provider is `provider`: an account of that provider, or,
/// for a Claude agent, an Anthropic API key.
pub(super) fn serves(provider: &str, a: &IdentityAccount) -> bool {
    canonical(&a.provider) == provider || (provider == "claude" && is_key(a))
}

/// Whether a link stored under `link_provider` is one of the account links of an agent whose provider is `provider`.
pub(super) fn is_link_of(provider: &str, link_provider: &str) -> bool {
    canonical(link_provider) == provider || (provider == "claude" && link_provider.eq_ignore_ascii_case(KEY_ROUTE_ID))
}

/// The provider a link to `to` is stored under for an agent of `provider`: a key's own, else the agent's.
pub(super) fn link_provider<'a>(to: &IdentityAccount, provider: &'a str) -> &'a str {
    if is_key(to) {
        KEY_ROUTE_ID
    } else {
        provider
    }
}

/// Why `def_id` cannot be moved onto a key account now, or `Ok`. A container agent is never given a key account yet
/// (it would need the gateway's address from inside the container); an agent on a model route is billed to the route.
pub(super) fn can_take_a_key(state: &AppState, def_id: &str) -> Result<(), String> {
    if open_panes(state, def_id).iter().any(|b| obj::meta_get_string(&b.meta, "agentMode", "host") == "container") {
        return Err("ACCOUNT_CANT_RUN: API keys aren't offered to container agents yet".to_string());
    }
    if launch::choice_of(&state.mstore, def_id).is_some() {
        return Err("ACCOUNT_CANT_RUN: this agent runs on a model route, which is billed to the route's key; move it back to its own sign-in first".to_string());
    }
    Ok(())
}

/// The agent just left its API key: its gateway token stops working now, whatever a running process still holds.
pub(super) fn left_the_key(def_id: &str) {
    token::revoke(def_id, KEY_ROUTE_ID);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(provider: &str, kind: &str) -> IdentityAccount {
        IdentityAccount {
            id: "a".into(),
            name: "n".into(),
            provider: provider.into(),
            kind: kind.into(),
            display_name: String::new(),
            secret_ref: crate::backend::storage::identities::SecretRef::Env { env_var: "X".into() },
            context: serde_json::json!({}),
            status: "valid".into(),
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn only_an_anthropic_api_key_is_a_key_account() {
        assert!(is_key(&account("anthropic", "api_key")));
        assert!(!is_key(&account("anthropic", "oauth")));
        assert!(!is_key(&account("openai", "api_key")));
        assert!(!is_key(&account("claude", "oauth")));
    }

    #[test]
    fn a_claude_agent_is_served_by_plans_and_keys_and_no_other_agent_by_a_key() {
        assert!(serves("claude", &account("claude", "oauth")));
        assert!(serves("claude", &account("claude-code", "oauth")));
        assert!(serves("claude", &account("anthropic", "api_key")));
        assert!(!serves("codex", &account("anthropic", "api_key")), "another harness is not offered an Anthropic key");
        assert!(!serves("claude", &account("openai", "api_key")));
        assert!(is_link_of("claude", "anthropic") && is_link_of("claude", "claude-code") && !is_link_of("codex", "anthropic"));
    }

    #[test]
    fn a_key_is_linked_under_its_own_provider_and_a_plan_under_the_agents() {
        assert_eq!(link_provider(&account("anthropic", "api_key"), "claude"), "anthropic");
        assert_eq!(link_provider(&account("claude", "oauth"), "claude"), "claude");
    }
}
