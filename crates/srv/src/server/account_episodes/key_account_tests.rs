// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! An Anthropic API key as an account a Claude agent can be moved onto
//! (`docs/specs/SPEC_AGENT_PANE_API_KEY_ACCOUNT_SWITCH_2026_11_16.md`).

use super::*;
use switch::AccountSwitchWhen::AfterTurn;

fn key(id: &str, status: &str) -> IdentityAccount {
    IdentityAccount {
        id: id.to_string(),
        name: format!("{id}-name"),
        provider: "anthropic".to_string(),
        kind: "api_key".to_string(),
        display_name: "Work key".to_string(),
        secret_ref: SecretRef::Env { env_var: format!("AMX_TEST_KEY_{id}") },
        context: json!({}),
        status: status.to_string(),
        created_at: 0,
        updated_at: 0,
    }
}

/// An agent `ag` on the plan `old`, with `key1` (unchecked) and `plan2` in the store, and one idle pane.
async fn setup(state: &AppState) -> String {
    state.id_store.identity_upsert(&account("old", "claude", "valid")).unwrap();
    state.id_store.identity_upsert(&account("plan2", "claude", "valid")).unwrap();
    state.id_store.identity_upsert(&key("key1", "unknown")).unwrap();
    agent_def(state, "ag");
    state.identity_store.agent_identity_link("ag", "old", "claude").unwrap();
    let tab = new_tab(state).await;
    pane(state, &tab, "ag", "Ag", "old", None, false).await
}

fn links(state: &AppState) -> Vec<(String, String)> {
    state.identity_store.agent_identity_list_for_agent("ag").unwrap().into_iter().map(|l| (l.provider, l.account_id)).collect()
}

#[tokio::test]
async fn a_claude_agent_moves_onto_a_key_and_back_with_exactly_one_payer_each_time() {
    let state = test_state();
    let block = setup(&state).await;

    let r = switch::switch(&state, "", &switch_req("ag", "key1", AfterTurn)).unwrap();
    assert_eq!((r.status.clone(), r.restarted), (switch::AccountSwitchStatus::Switched, 1), "an unchecked key is offered and taken");
    assert_eq!(r.account_label, "Work key");
    assert_eq!(links(&state), [("anthropic".to_string(), "key1".to_string())], "the plan link went with the move: one payer");
    assert!(crate::server::spend_watch::spends_through_gateway(&state, "ag"), "an agent on a key is counted and capped at the gateway, so its spend card is kept");
    let defs = state.mstore.agent_def_list().unwrap();
    let shown = crate::server::spend_watch::view(&state.mstore, &state.identity_store, &defs, "ag").expect("an agent on a key shows its spend and cap");
    assert_eq!(shown["stopped"], serde_json::json!(false));
    assert!(crate::server::spend_watch::on_a_route(&state, "ag").is_ok(), "its cap can be read, raised and reset");
    // One payer the other way too: the model panel does not put an agent on a key onto a model route.
    let onto_route = crate::server::app_api::agent_model::AgentModelSwitchReq { blockid: block.clone(), route: "openrouter".into(), model: String::new(), restart: false };
    let refused = crate::server::app_api::agent_model::switch_model(&state, &onto_route).unwrap_err();
    assert!(refused.contains("API key"), "{refused}");
    assert_eq!(links(&state), [("anthropic".to_string(), "key1".to_string())], "nothing changed");
    assert!(crate::backend::model_gateway::launch::choice_of(&state.mstore, "ag").is_none());

    let again = switch::switch(&state, "", &switch_req("ag", "key1", AfterTurn)).unwrap();
    assert_eq!(again.status, switch::AccountSwitchStatus::Unchanged, "the key it is on changes nothing");

    let r = switch::switch(&state, "", &switch_req("ag", "plan2", AfterTurn)).unwrap();
    assert_eq!(r.status, switch::AccountSwitchStatus::Switched);
    assert_eq!(links(&state), [("claude".to_string(), "plan2".to_string())], "the key link went with the move back");
    assert!(!crate::server::spend_watch::spends_through_gateway(&state, "ag"));
    // An agent of another provider that holds an Anthropic link sends its own requests: nothing is capped for it.
    let mut other = crate::backend::storage::agents::test_agent_def("muxc", "Muxc", "muxcode", "host", 1, "");
    state.mstore.agent_def_insert(&mut other).unwrap();
    state.identity_store.agent_identity_link("muxc", "key1", "anthropic").unwrap();
    assert!(!crate::server::spend_watch::spends_through_gateway(&state, "muxc"), "only a Claude agent's key goes through the gateway");
    assert!(crate::server::spend_watch::on_a_route(&state, "ag").is_err(), "a plan has no spend cap");
    cleanup(&[&block]);
}

#[tokio::test]
async fn the_gateway_token_of_a_key_stops_working_when_the_agent_moves_to_a_plan() {
    use crate::backend::model_gateway::{catalog::KEY_ROUTE_ID, token};
    let state = test_state();
    let block = setup(&state).await;
    switch::switch(&state, "", &switch_req("ag", "key1", AfterTurn)).unwrap();
    let t = token::token_for_account("ag", "ag", KEY_ROUTE_ID, "key1");
    assert!(token::grant_for(&t).is_some());
    switch::switch(&state, "", &switch_req("ag", "old", AfterTurn)).unwrap();
    assert!(token::grant_for(&t).is_none(), "revoked, whatever a running process still holds");
    cleanup(&[&block]);
}

#[tokio::test]
async fn a_key_known_to_be_rejected_is_refused_saying_so() {
    let state = test_state();
    let block = setup(&state).await;
    state.id_store.identity_upsert(&key("bad", "invalid")).unwrap();
    let err = switch::switch(&state, "", &switch_req("ag", "bad", AfterTurn)).unwrap_err();
    assert!(err.contains("ACCOUNT_CANT_RUN") && err.contains("Work key") && err.contains("rejected"), "{err}");
    assert_eq!(links(&state), [("claude".to_string(), "old".to_string())]);
    cleanup(&[&block]);
}

#[tokio::test]
async fn a_container_agent_and_an_agent_on_a_model_route_are_not_moved_onto_a_key() {
    let state = test_state();
    let _plain = setup(&state).await;
    // The agent's pane runs in a container.
    let tab = new_tab(&state).await;
    let meta = json!({ "view": "agent", "agentId": "box", "agentName": "Box", "agentProvider": "claude", "agentMode": "container" });
    let boxed = dispatch_apply(&state, Command::CreateBlock { tab_id: tab, meta })
        .await
        .iter()
        .find_map(|e| match e {
            Event::BlockCreated { block_id, .. } => Some(block_id.clone()),
            _ => None,
        })
        .unwrap();
    agent_def(&state, "box");
    state.identity_store.agent_identity_link("box", "old", "claude").unwrap();
    let err = switch::switch(&state, "", &switch_req("box", "key1", AfterTurn)).unwrap_err();
    assert!(err.contains("container agents"), "{err}");
    assert_eq!(linked_account(&state, "box"), ["old"], "nothing changed");
    // …but a plan is still a move.
    assert_eq!(switch::switch(&state, "", &switch_req("box", "plan2", AfterTurn)).unwrap().status, switch::AccountSwitchStatus::Switched);

    assert!(switch::receiving(&state, "ag", "key1").is_some(), "before it is on a route, a key can be received");
    crate::backend::model_gateway::launch::set_choice(&state.mstore, "ag", "claude", "openrouter", "").unwrap();
    assert!(switch::receiving(&state, "ag", "key1").is_none(), "a switch that waited for a turn is rechecked when it is applied");
    let err = switch::switch(&state, "", &switch_req("ag", "key1", AfterTurn)).unwrap_err();
    assert!(err.contains("model route"), "{err}");
    cleanup(&[&boxed]);
}

#[tokio::test]
async fn any_with_quota_never_picks_a_key_and_a_key_is_never_a_move_between_agents_of_another_provider() {
    let state = test_state();
    let block = setup(&state).await;
    state.id_store.identity_upsert(&account("plan2", "claude", "expired")).unwrap();
    let err = switch::switch(&state, "", &switch_req("ag", "any with quota", AfterTurn)).unwrap_err();
    assert!(err.contains("NO_ACCOUNT_WITH_QUOTA"), "a valid key is there, and it is not what `any` stands for: {err}");
    assert_eq!(links(&state), [("claude".to_string(), "old".to_string())]);

    // An agent of another harness is never offered the Anthropic key.
    let mut other = crate::backend::storage::agents::test_agent_def("cx", "Agent cx", "codex", "host", 1, "");
    state.mstore.agent_def_insert(&mut other).unwrap();
    let err = switch::switch(&state, "", &switch_req("cx", "key1", AfterTurn)).unwrap_err();
    assert!(err.contains("UNKNOWN_ACCOUNT"), "{err}");
    cleanup(&[&block]);
}

#[tokio::test]
async fn a_failover_never_moves_an_agent_onto_a_key_nor_off_one() {
    let state = test_state();
    state.id_store.identity_upsert(&mail("a", "valid", "a@x.com")).unwrap();
    state.id_store.identity_upsert(&key("key1", "valid")).unwrap();
    state.id_store.identity_upsert(&key("key2", "valid")).unwrap();
    agent_def(&state, "ag");
    state.identity_store.agent_identity_link("ag", "a", "claude").unwrap();
    let tab = new_tab(&state).await;
    let limited = || Some(failure("usage_limit", Some(in_an_hour()), Some("seven_day")));

    // On a plan out of quota, with only keys to go to: it stays.
    let on_plan = pane(&state, &tab, "ag", "Ag", "a", limited(), false).await;
    failover::run(&state, &scan(&state, agentmux_common::time::now_ms_u64()));
    assert_eq!(links(&state), [("claude".to_string(), "a".to_string())], "a plan's limit is not a reason to start paying per token");

    // On a key that hit its limit (or its credit): no episode, no move to the other key.
    state.identity_store.agent_identity_link("ag", "key1", "anthropic").unwrap();
    state.identity_store.agent_identity_unlink("ag", "claude").unwrap();
    let on_key = pane(&state, &tab, "ag", "Ag", "key1", limited(), false).await;
    let panes = scan(&state, agentmux_common::time::now_ms_u64());
    assert!(panes.iter().all(|p| p.block_id != on_key), "a pane on a key is in no episode: {panes:?}");
    failover::run(&state, &panes);
    assert_eq!(links(&state), [("anthropic".to_string(), "key1".to_string())], "its failure stands on the pane; nothing else pays in its place");
    assert!(stored_failure(&state, &on_key).is_some());
    cleanup(&[&on_plan, &on_key]);
}
