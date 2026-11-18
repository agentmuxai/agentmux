// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A gateway token made for a container agent's turn is bound to that container's address, and srv takes it only from that
//! container: from the container listener (which checked the peer) or with a forwarder's stamp (Docker Desktop). A token
//! with no address, a host agent's, is not asked for one. The real router, no upstream: a request that passes the check
//! reaches the key lookup, which here has no key to give.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use std::net::IpAddr;
use tower::ServiceExt;

use crate::backend::container::gateway_fwd;
use crate::backend::container::listener::AgentsNetworkPeer;
use crate::backend::model_gateway::{catalog, token};
use crate::backend::storage::agents::test_agent_def;
use crate::backend::storage::identities::{IdentityAccount, SecretRef};
use crate::server::routes::build_router;
use crate::server::tests::test_state;
use crate::server::AppState;

const URI: &str = "/api/v1/model/anthropic/v1/messages";
const WRONG: &str = "this token is not for this container";

fn ip(last: u8) -> IpAddr {
    IpAddr::from([172, 30, 0, last])
}

/// A Claude agent on an API-key account with no key behind it, and its key-route token bound to `peer` (none: a host agent's).
fn bound_agent(state: &AppState, name: &str, peer: Option<IpAddr>) -> String {
    let uid = format!("uid-{name}");
    let mut def = test_agent_def(&uid, name, "claude", "agent", 1, "");
    def.slug = name.to_string();
    state.mstore.agent_def_insert(&mut def).unwrap();
    let acct = format!("acct-{name}");
    let account = IdentityAccount {
        id: acct.clone(),
        name: format!("{acct}-name"),
        provider: "anthropic".to_string(),
        kind: "api_key".to_string(),
        display_name: "Work key".to_string(),
        // No such variable: the key lookup, which a request that passes the peer check reaches, has nothing to give.
        secret_ref: SecretRef::Env { env_var: format!("AMX_TEST_GWB_NO_KEY_{}", name.replace('-', "_")) },
        context: serde_json::json!({}),
        status: "valid".to_string(),
        created_at: 0,
        updated_at: 0,
    };
    state.id_store.identity_upsert(&account).unwrap();
    state.identity_store.agent_identity_link(&uid, &acct, "anthropic").unwrap();
    match peer {
        Some(peer) => token::renew_for_account(name, &uid, catalog::KEY_ROUTE_ID, &acct, Some(peer)),
        None => token::token_for_account(name, &uid, catalog::KEY_ROUTE_ID, &acct),
    }
}

/// One request through the router with the extras given: a stamp's headers, or the peer the container listener checked.
async fn send(state: &AppState, token: &str, uri: &str, stamp: Option<(IpAddr, &str, &str)>, checked: Option<IpAddr>) -> (StatusCode, String) {
    let mut req = Request::builder().method(Method::POST).uri(uri).header("authorization", format!("Bearer {token}"));
    if let Some((peer, method, target)) = stamp {
        for (name, value) in gateway_fwd::stamp_headers(peer, method, target) {
            req = req.header(name, value);
        }
    }
    let mut req = req.body(Body::from("{}")).unwrap();
    if let Some(peer) = checked {
        req.extensions_mut().insert(AgentsNetworkPeer(peer));
    }
    let resp = build_router(state.clone()).oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8_lossy(&bytes).to_string())
}

fn passed_the_check(status: StatusCode, answer: &str) -> bool {
    !answer.contains(WRONG) && status != StatusCode::UNAUTHORIZED
}

#[tokio::test]
async fn a_bound_token_straight_to_srv_with_no_proof_is_refused_and_ends() {
    let state = test_state();
    let token = bound_agent(&state, "gwb-direct", Some(ip(9)));
    let (status, answer) = send(&state, &token, URI, None, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{answer}");
    assert!(answer.contains(WRONG), "{answer}");
    assert_eq!(token::grant_for(&token), None, "shown to someone it is not for: the token ends");
    let (status, _) = send(&state, &token, URI, Some((ip(9), "POST", URI)), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "and even the right container cannot use it again");
}

#[tokio::test]
async fn a_stamp_for_the_tokens_own_container_is_accepted_and_one_for_another_is_not() {
    let state = test_state();
    let token = bound_agent(&state, "gwb-stamped", Some(ip(9)));
    let (status, answer) = send(&state, &token, URI, Some((ip(9), "POST", URI)), None).await;
    assert!(passed_the_check(status, &answer), "{status} {answer}");
    assert!(token::grant_for(&token).is_some(), "an accepted request leaves the token as it was");

    let (status, answer) = send(&state, &token, URI, Some((ip(10), "POST", URI)), None).await;
    assert_eq!((status, answer.contains(WRONG)), (StatusCode::FORBIDDEN, true), "another container's address: {answer}");
    assert_eq!(token::grant_for(&token), None, "and the token ends");
}

#[tokio::test]
async fn a_stamp_cannot_be_moved_to_another_request() {
    let state = test_state();
    let token = bound_agent(&state, "gwb-moved", Some(ip(9)));
    // A stamp made for another path, and one made for another method, sent with this request.
    let other_path = "/api/v1/model/anthropic/v1/complete";
    let (status, answer) = send(&state, &token, URI, Some((ip(9), "POST", other_path)), None).await;
    assert_eq!((status, answer.contains(WRONG)), (StatusCode::FORBIDDEN, true), "{answer}");
    let token = bound_agent(&state, "gwb-moved-2", Some(ip(9)));
    let (status, answer) = send(&state, &token, URI, Some((ip(9), "GET", URI)), None).await;
    assert_eq!((status, answer.contains(WRONG)), (StatusCode::FORBIDDEN, true), "{answer}");
}

#[tokio::test]
async fn a_stamp_on_a_path_outside_the_gateway_is_no_proof() {
    let state = test_state();
    let token = bound_agent(&state, "gwb-outside", Some(ip(9)));
    let (status, answer) = send(&state, &token, "/api/v1/model/anthropic/%2e%2e/v1/messages", Some((ip(9), "POST", "/api/v1/model/anthropic/%2e%2e/v1/messages")), None).await;
    assert_eq!((status, answer.contains(WRONG)), (StatusCode::FORBIDDEN, true), "{answer}");
}

#[tokio::test]
async fn the_peer_the_container_listener_checked_is_proof_and_another_is_not() {
    let state = test_state();
    let token = bound_agent(&state, "gwb-listener", Some(ip(9)));
    let (status, answer) = send(&state, &token, URI, None, Some(ip(9))).await;
    assert!(passed_the_check(status, &answer), "{status} {answer}");
    let (status, answer) = send(&state, &token, URI, None, Some(ip(11))).await;
    assert_eq!((status, answer.contains(WRONG)), (StatusCode::FORBIDDEN, true), "{answer}");
    assert_eq!(token::grant_for(&token), None);
}

#[tokio::test]
async fn a_host_agents_token_has_no_address_and_needs_no_proof() {
    let state = test_state();
    let token = bound_agent(&state, "gwb-host", None);
    let (status, answer) = send(&state, &token, URI, None, None).await;
    assert!(passed_the_check(status, &answer), "{status} {answer}");
    assert!(token::grant_for(&token).is_some());
    token::revoke("uid-gwb-host", catalog::KEY_ROUTE_ID);
}
