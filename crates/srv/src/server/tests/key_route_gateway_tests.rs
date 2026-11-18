// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The gateway's key route end to end: an agent's own Anthropic API key, an account of Connectors > Accounts
//! (docs/specs/SPEC_AGENT_PANE_API_KEY_ACCOUNT_SWITCH_2026_11_16.md). A real request through the real router to a
//! stand-in upstream on loopback; nothing reaches a network outside the machine.

use axum::http::{header, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use serde_json::json;
use std::sync::{Arc, Mutex};

use super::model_gateway_tests::{agent_on_route, call, upstream, Seen, SSE, UPSTREAM_IN_USE};
use crate::backend::model_gateway::{catalog, spend, token};
use crate::backend::storage::agents::test_agent_def;
use crate::backend::storage::identities::{IdentityAccount, SecretRef};
use crate::server::tests::test_state;
use crate::server::AppState;

/// A made-up key of the shape the redaction code knows, built from pieces so no key-shaped literal sits in the file.
fn made_up_key() -> String {
    ["sk-ant", "-api03-", "MADEUPMADEUPMADEUPMADEUPMADEUP0123456789"].concat()
}

/// A Claude agent linked to the API-key account `acct`, whose key is `key` (none: the key is missing).
/// Returns its key-route token and its definition id.
fn agent_on_key(state: &AppState, name: &str, acct: &str, key: Option<&str>) -> (String, String) {
    let uid = format!("uid-{name}");
    let mut def = test_agent_def(&uid, name, "claude", "agent", 1, "");
    def.slug = name.to_string();
    state.mstore.agent_def_insert(&mut def).unwrap();
    let env_var = format!("AMX_TEST_GW_KEY_{}", name.replace('-', "_"));
    if let Some(key) = key {
        std::env::set_var(&env_var, key);
    }
    let account = IdentityAccount {
        id: acct.to_string(),
        name: format!("{acct}-name"),
        provider: "anthropic".to_string(),
        kind: "api_key".to_string(),
        display_name: "Work key".to_string(),
        secret_ref: SecretRef::Env { env_var },
        context: json!({}),
        status: "valid".to_string(),
        created_at: 0,
        updated_at: 0,
    };
    state.id_store.identity_upsert(&account).unwrap();
    state.identity_store.agent_identity_link(&uid, acct, "anthropic").unwrap();
    (token::token_for_account(name, &uid, catalog::KEY_ROUTE_ID, acct), uid)
}

const URI: &str = "/api/v1/model/anthropic/v1/messages";

#[tokio::test]
async fn a_request_reaches_anthropic_with_the_key_and_the_body_unchanged_and_is_counted() {
    let _upstream = UPSTREAM_IN_USE.lock().await;
    let state = test_state();
    let seen = Arc::new(Mutex::new(Seen::default()));
    *catalog::TEST_BASE_URL.lock().unwrap() = Some(upstream(seen.clone()).await);
    let key = made_up_key();
    let (token, _) = agent_on_key(&state, "gw-key-e2e", "acct-gw-1", Some(&key));

    let body = json!({
        "model": "claude-opus-5-5", "max_tokens": 16, "stream": true,
        "system": [{ "type": "text", "text": "stable prefix", "cache_control": { "type": "ephemeral" } }],
        "messages": [{ "role": "user", "content": "hi" }]
    });
    let (status, answer) = call(&state, Some(&token), URI, body.clone()).await;

    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer, SSE, "the stream passes through unchanged");
    assert!(!answer.contains(&key) && !answer.contains(&token));
    let seen = seen.lock().unwrap();
    assert_eq!(seen.x_api_key.as_deref(), Some(key.as_str()), "the upstream gets the account's key, the way Anthropic takes it");
    assert_eq!(seen.authorization, None, "and not the agent's token");
    assert_eq!(seen.body, body, "the body goes through as it is: the model, and the cache_control that keeps the prompt cached");

    let used = spend::today("gw-key-e2e", agentmux_common::time::now_ms());
    assert_eq!((used.requests, used.tokens.input, used.tokens.output), (1, 1000, 2000));
    assert!((used.cost_usd - spend::cost(catalog::key_route_price("claude-opus-5-5"), &used.tokens)).abs() < 1e-12, "counted at the model's price tier");
    assert!(used.cost_usd > 0.05, "{}", used.cost_usd);
}

#[tokio::test]
async fn a_rejected_key_comes_back_naming_only_the_account_and_never_the_key() {
    let _upstream = UPSTREAM_IN_USE.lock().await;
    async fn rejected() -> impl IntoResponse {
        (StatusCode::UNAUTHORIZED, [(header::CONTENT_TYPE, "application/json")], r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#)
    }
    let app = axum::Router::new().route("/v1/messages", post(rejected));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    *catalog::TEST_BASE_URL.lock().unwrap() = Some(format!("http://{}", listener.local_addr().unwrap()));
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let state = test_state();
    let key = made_up_key();
    let (token, _) = agent_on_key(&state, "gw-key-bad", "acct-gw-bad", Some(&key));
    let (status, answer) = call(&state, Some(&token), URI, json!({ "model": "claude-opus-5-5" })).await;

    assert_eq!(status, StatusCode::UNAUTHORIZED, "{answer}");
    assert!(answer.contains("invalid x-api-key") && answer.contains("Work key"), "{answer}");
    assert!(!answer.contains(&key) && !answer.contains("MADEUP"), "{answer}");
    assert!(answer.contains("authentication_error"), "the harness still reads it as the error it is: {answer}");
}

#[tokio::test]
async fn an_agent_moved_off_the_key_is_cut_off_at_once_even_with_its_token() {
    let state = test_state();
    let (token, uid) = agent_on_key(&state, "gw-key-moved", "acct-gw-2", Some(&made_up_key()));
    state.identity_store.agent_identity_unlink(&uid, "anthropic").unwrap();
    let (status, why) = call(&state, Some(&token), URI, json!({ "model": "claude-opus-5-5" })).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{why}");
    assert!(why.contains("no longer on that API key account"), "{why}");

    // Linked to a different key: the token's account is not the agent's now.
    let (token, uid) = agent_on_key(&state, "gw-key-swapped", "acct-gw-3", Some(&made_up_key()));
    let (_, _) = agent_on_key(&state, "gw-key-other", "acct-gw-other", None);
    state.identity_store.agent_identity_link(&uid, "acct-gw-other", "anthropic").unwrap();
    let (status, _) = call(&state, Some(&token), URI, json!({ "model": "claude-opus-5-5" })).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    token::revoke(&uid, catalog::KEY_ROUTE_ID);
    let (status, _) = call(&state, Some(&token), URI, json!({ "model": "claude-opus-5-5" })).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "a revoked token stops at once");
}

#[tokio::test]
async fn a_token_is_for_its_own_route_and_an_account_without_a_key_says_so_naming_only_the_account() {
    let state = test_state();
    let (key_token, _) = agent_on_key(&state, "gw-key-route", "acct-gw-4", Some(&made_up_key()));
    let (status, _) = call(&state, Some(&key_token), "/api/v1/model/openrouter/v1/messages", json!({ "model": "x" })).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "the key route's token is not a model route's");
    let (route_token, _) = agent_on_route(&state, "gw-route-only", false);
    let (status, _) = call(&state, Some(&route_token), URI, json!({ "model": "x" })).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "and a model route's token is not the key route's");

    let (empty, _) = agent_on_key(&state, "gw-key-empty", "acct-gw-5", None);
    let (status, why) = call(&state, Some(&empty), URI, json!({ "model": "x" })).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(why.contains("Work key") && why.contains("Connectors"), "{why}");
}

#[tokio::test]
async fn the_key_route_stops_at_the_agents_daily_cap_with_a_billing_message() {
    let state = test_state();
    let (token, _) = agent_on_key(&state, "gw-key-cap", "acct-gw-6", Some(&made_up_key()));
    let used = crate::agents::types::TokenCounts { input: 10, ..Default::default() };
    spend::record("gw-key-cap", catalog::KEY_ROUTE_ID, catalog::key_route_price("opus"), &used, Some(25.0), agentmux_common::time::now_ms());
    let (status, answer) = call(&state, Some(&token), URI, json!({ "model": "claude-opus-5-5" })).await;
    assert_eq!(status, StatusCode::PAYMENT_REQUIRED, "{answer}");
    assert!(answer.contains("billing_error") && answer.contains("spend cap"), "{answer}");
}

#[test]
fn a_model_is_counted_at_its_familys_price_and_an_unknown_one_at_the_dearest() {
    for (name, tier) in [("claude-opus-5-5", "opus"), ("claude-sonnet-5-5", "sonnet"), ("claude-haiku-4-5-20251001", "haiku"), ("claude-fable-5-2", "fable")] {
        assert_eq!(catalog::key_route_price(name).id, tier, "{name}");
    }
    assert_eq!(catalog::key_route_price("something-new").id, "fable");
    assert_eq!(catalog::key_route_price("").id, "fable");
    assert!(catalog::route(catalog::KEY_ROUTE_ID).is_none(), "an agent is never set to the key route as a model route");
    assert!(catalog::gateway_route(catalog::KEY_ROUTE_ID).is_some());
}
