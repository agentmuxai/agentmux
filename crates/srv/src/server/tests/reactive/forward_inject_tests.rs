// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// `forward_inject_tests`, moved out of server/reactive.rs unchanged
// (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4).

use super::*;
use crate::server::tests::test_state;

/// A peer srv that answers `/agentmux/reactive/inject` with a fixed status
/// and body. The returned guard shuts it down when dropped.
async fn stub_peer(
    status: StatusCode,
    body: &'static str,
) -> (String, tokio_util::sync::DropGuard) {
    let app = axum::Router::new().route(
        "/agentmux/reactive/inject",
        axum::routing::post(move || async move {
            (
                status,
                [(axum::http::header::CONTENT_TYPE, "application/json")],
                body,
            )
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let token = tokio_util::sync::CancellationToken::new();
    let child = token.clone();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async move { child.cancelled().await })
            .await;
    });
    (format!("http://{addr}"), token.drop_guard())
}

fn req() -> InjectionRequest {
    InjectionRequest {
        target_agent: "agenty".to_string(),
        message: "hello".to_string(),
        source_agent: None,
        ..Default::default()
    }
}

fn peer(url: &str) -> ForwardPeer<'_> {
    ForwardPeer {
        url,
        auth_key: "",
        kind: "test",
        channel: None,
        trust: EchoTrust {
            delivery_tier: "lan",
            sig_verified: None,
            reagent_verified: None,
            lan_verified: None,
            channel_verified: None,
        },
    }
}

async fn outcome(status: StatusCode, body: &'static str) -> ForwardOutcome {
    let state = test_state();
    let (url, _guard) = stub_peer(status, body).await;
    let r = req();
    forward_inject_to_peer(&state, &r, &r, peer(&url)).await
}

// SPEC_JEKT_CROSS_CHANNEL_TRUST_2026_09_02.md Phase B, codex P2 on
// #3064: the receiver's channel verdict rides back on the response and
// wins over the forwarder's (always-None) view for the echoed marker.
#[test]
fn echo_trust_takes_the_receivers_channel_verdict_from_the_body() {
    let base = EchoTrust {
        delivery_tier: "channel",
        sig_verified: Some(true),
        reagent_verified: None,
        lan_verified: None,
        channel_verified: None,
    };
    let body = serde_json::json!({ "success": true, "channel_verified": true });
    let t = echo_trust_from_peer_body(&body, base);
    assert_eq!(t.channel_verified, Some(true));
    assert_eq!(t.sig_verified, Some(true), "every other field is untouched");
    assert_eq!(t.delivery_tier, "channel");

    let body = serde_json::json!({ "success": true, "channel_verified": false });
    assert_eq!(echo_trust_from_peer_body(&body, base).channel_verified, Some(false));
}

#[test]
fn echo_trust_keeps_the_callers_value_when_an_older_peer_omits_the_field() {
    let base = EchoTrust {
        delivery_tier: "host",
        sig_verified: None,
        reagent_verified: None,
        lan_verified: None,
        channel_verified: Some(true),
    };
    let body = serde_json::json!({ "success": true });
    assert_eq!(echo_trust_from_peer_body(&body, base), base);
}

#[tokio::test]
async fn success_true_is_delivered() {
    let out = outcome(StatusCode::OK, r#"{"success":true,"request_id":"m1"}"#).await;
    match out {
        ForwardOutcome::Delivered(body) => {
            assert_eq!(body.get("request_id").and_then(|v| v.as_str()), Some("m1"));
        }
        _ => panic!("expected Delivered"),
    }
}

#[tokio::test]
async fn success_false_is_stale() {
    let out = outcome(StatusCode::OK, r#"{"success":false,"error":"agent not found"}"#).await;
    assert!(matches!(out, ForwardOutcome::Stale), "expected Stale");
}

/// **The one behaviour this refactor unified rather than preserved.**
///
/// The LAN tier used to deliver on "anything but `success: false`", so a
/// 200 whose body omits `success` counted as a successful delivery — and
/// echoed a delivery confirmation to the sender for a message that may
/// never have arrived. Both host tiers already required `success: true`;
/// all three now do.
#[tokio::test]
async fn a_body_without_success_is_not_delivered() {
    let out = outcome(StatusCode::OK, r#"{"request_id":"m1"}"#).await;
    assert!(
        matches!(out, ForwardOutcome::Stale),
        "a peer that never said it delivered must not be treated as having delivered"
    );
}

/// A non-2xx says the peer is up but unhappy — that is not evidence the
/// *candidate entry* is stale, so it must not drive an eviction.
#[tokio::test]
async fn non_2xx_is_inconclusive() {
    let out = outcome(StatusCode::INTERNAL_SERVER_ERROR, r#"{"success":true}"#).await;
    assert!(
        matches!(out, ForwardOutcome::Inconclusive),
        "expected Inconclusive"
    );
}

#[tokio::test]
async fn an_unparseable_body_is_inconclusive() {
    let out = outcome(StatusCode::OK, "not json at all").await;
    assert!(
        matches!(out, ForwardOutcome::Inconclusive),
        "expected Inconclusive"
    );
}

/// Transport failure is consistent with a registry/cache entry pointing at
/// something that is gone, so it reports `Stale` and lets each tier apply
/// its own eviction policy.
#[tokio::test]
async fn an_unreachable_peer_is_stale() {
    let state = test_state();
    // Bind then immediately release, so the port is almost certainly free
    // and nothing is listening on it.
    let url = {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        format!("http://{}", l.local_addr().unwrap())
    };
    let r = req();
    let out = forward_inject_to_peer(&state, &r, &r, peer(&url)).await;
    assert!(matches!(out, ForwardOutcome::Stale), "expected Stale");
}
