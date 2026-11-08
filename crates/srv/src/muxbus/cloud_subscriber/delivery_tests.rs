// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::sync::{Arc, Mutex};

use super::{claim_and_deliver, wan_request, AgentSyncOutcome, PendingInj, PendingResp};
use crate::backend::reactive::handler::ReactiveHandler;
use crate::backend::reactive::types::JektTier;
use crate::backend::storage::store::Store;

fn pulled(extra: serde_json::Value) -> PendingInj {
    let mut row = serde_json::json!({
        "id": "inj-1",
        "source_agent": "camper",
        "message": "hello",
        "priority": "normal",
    });
    row.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    serde_json::from_value(row).unwrap()
}

#[test]
fn a_declared_sensitive_tier_travels_with_a_wan_jekt() {
    for (raw, want) in [("sensitive", Some(JektTier::Sensitive)), ("info", None), ("coord", None)] {
        let req = wan_request(&pulled(serde_json::json!({ "jekt_tier": raw })), "agent2");
        assert_eq!(req.jekt_tier, want, "{raw}: only sensitive is taken from the relay");
        assert_eq!(req.delivery_tier.as_deref(), Some("wan"));
        assert_eq!(req.target_agent, "agent2");
        assert_eq!(req.request_id.as_deref(), Some("inj-1"));
    }
}

#[test]
fn an_unknown_tier_is_read_as_none_declared() {
    for raw in [
        serde_json::json!("SENSITIVE"),
        serde_json::json!("urgent"),
        serde_json::json!(""),
        serde_json::json!(null),
    ] {
        let req = wan_request(&pulled(serde_json::json!({ "jekt_tier": raw })), "agent2");
        assert_eq!(req.jekt_tier, None, "{raw}");
    }
}

#[test]
fn a_row_without_a_tier_declares_none() {
    let req = wan_request(&pulled(serde_json::json!({})), "agent2");
    assert_eq!(req.jekt_tier, None);
}

/// Issue #3894: the sender's deadline the relay carries rides into the
/// delivery request, where the hold and the deferred queue then honour it.
/// A row without one, or with an unreadable one, behaves exactly as today.
#[test]
fn the_senders_deadline_from_the_relay_rides_into_the_request() {
    let req = wan_request(&pulled(serde_json::json!({ "expires_at": "2026-10-08T14:30:00Z" })), "agent2");
    assert_eq!(req.expires_at_ms, Some(1_791_469_800_000));
    for raw in [serde_json::json!(null), serde_json::json!("not a time"), serde_json::json!("")] {
        let req = wan_request(&pulled(serde_json::json!({ "expires_at": raw })), "agent2");
        assert_eq!(req.expires_at_ms, None, "{raw}");
    }
}

#[test]
fn a_pull_with_fields_this_build_does_not_know_still_parses() {
    let body: PendingResp = serde_json::from_value(serde_json::json!({
        "injections": [{ "id": "inj-1", "message": "hello", "jekt_tier": "sensitive", "some_later_field": 1 }],
        "count": 1,
    }))
    .unwrap();
    assert_eq!(body.injections.len(), 1);
}

/// A cloud job's run event arrives as its own frame, with the signed body untouched.
#[test]
fn a_job_event_frame_carries_the_signed_body_and_nothing_else() {
    use super::super::ServerMsg;
    let frame = serde_json::json!({ "type": "job_event", "body": "{\"a\": 1}", "public_key": "k", "signature": "s" });
    match serde_json::from_value::<ServerMsg>(frame).unwrap() {
        ServerMsg::JobEvent { body, public_key, signature } => assert_eq!((body.as_str(), public_key.as_str(), signature.as_str()), ("{\"a\": 1}", "k", "s")),
        _ => panic!("not a job event"),
    }
    // A frame without the body or the signature is not one, so it can never reach the ledger half-formed.
    assert!(matches!(serde_json::from_value::<ServerMsg>(serde_json::json!({ "type": "job_event", "body": "x" })), Err(_)));
}

// ── claim_and_deliver, against a fake relay and a handler that records what it was given ──

const STAMP: &str = "2026-11-06T22:00:00Z";

/// One scripted answer of the fake relay: `"METHOD /path"`, status, JSON body.
type Route = (String, u16, serde_json::Value);

/// What the relay was asked: `"METHOD /path"`, the JSON body, the bearer token.
type Asked = Arc<Mutex<Vec<(String, serde_json::Value, String)>>>;

/// A fake relay. A route is answered in the order its entries are listed, the last one for as
/// long as the requests keep coming; a route not listed is a 404.
async fn relay(script: Vec<Route>) -> (String, Asked) {
    let asked: Asked = Arc::default();
    let left = Arc::new(Mutex::new(script));
    let app = axum::Router::new().fallback({
        let asked = Arc::clone(&asked);
        move |req: axum::extract::Request| {
            let (asked, left) = (Arc::clone(&asked), Arc::clone(&left));
            async move {
                let line = format!("{} {}", req.method(), req.uri().path());
                let auth = req.headers().get("authorization").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
                let bytes = axum::body::to_bytes(req.into_body(), 1 << 20).await.unwrap_or_default();
                let body = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
                asked.lock().unwrap().push((line.clone(), body, auth));
                let mut left = left.lock().unwrap();
                let matching: Vec<usize> = (0..left.len()).filter(|i| left[*i].0 == line).collect();
                let answer = match matching.as_slice() {
                    [] => (404, serde_json::Value::Null),
                    [only] => (left[*only].1, left[*only].2.clone()),
                    [first, ..] => {
                        let entry = left.remove(*first);
                        (entry.1, entry.2)
                    }
                };
                (axum::http::StatusCode::from_u16(answer.0).unwrap(), axum::Json(answer.1))
            }
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), asked)
}

fn won(ids: &[&str]) -> Route {
    ("POST /reactive/ack".into(), 200, serde_json::json!({ "acknowledged": ids, "delivered_at": STAMP }))
}

fn released() -> Route {
    ("POST /reactive/release".into(), 200, serde_json::json!({ "ok": true }))
}

/// A running agent pane: its handler (leaked, as the process-global one is) and what it was typed.
fn live_pane(agent: &str) -> (&'static ReactiveHandler, Arc<Mutex<Vec<String>>>) {
    let typed: Arc<Mutex<Vec<String>>> = Arc::default();
    let handler: &'static ReactiveHandler = Box::leak(Box::new(ReactiveHandler::new()));
    let sink = Arc::clone(&typed);
    handler.set_input_sender(Arc::new(move |_block: &str, data: &[u8]| {
        sink.lock().unwrap().push(String::from_utf8_lossy(data).into_owned());
        Ok(())
    }));
    handler.register_agent(agent, &format!("blk-{agent}"), None).unwrap();
    (handler, typed)
}

fn agent_name() -> String {
    format!("agent-{}", uuid::Uuid::new_v4())
}

fn rows(rows: serde_json::Value) -> PendingResp {
    serde_json::from_value(serde_json::json!({ "injections": rows })).unwrap()
}

/// One pull of `body` for `agent`, claimed with the agent's own token (`using_per_agent`) or the shared one.
async fn claim(
    base: &str,
    agent: &str,
    handler: &'static ReactiveHandler,
    body: PendingResp,
    using_per_agent: bool,
) -> AgentSyncOutcome {
    let mstore = Arc::new(Store::open_in_memory().unwrap());
    claim_and_deliver(base, agent, "shared-token", &reqwest::Client::new(), &mstore, handler, body, "agent-token".into(), using_per_agent)
        .await
}

fn lines(asked: &Asked) -> Vec<String> {
    asked.lock().unwrap().iter().map(|(line, _, _)| line.clone()).collect()
}

fn body_of(asked: &Asked, line: &str) -> Vec<serde_json::Value> {
    asked.lock().unwrap().iter().filter(|(l, _, _)| l == line).map(|(_, body, _)| body.clone()).collect()
}

#[tokio::test]
async fn a_pull_with_nothing_in_it_claims_nothing() {
    let agent = agent_name();
    let (handler, typed) = live_pane(&agent);
    let (base, asked) = relay(vec![]).await;

    assert!(matches!(claim(&base, &agent, handler, rows(serde_json::json!([])), true).await, AgentSyncOutcome::Ok));

    assert!(lines(&asked).is_empty(), "no request to the relay");
    assert!(typed.lock().unwrap().is_empty());
}

#[tokio::test]
async fn every_pulled_id_is_claimed_once_before_any_is_delivered_and_each_won_one_is_delivered_once() {
    let agent = agent_name();
    let (handler, typed) = live_pane(&agent);
    let (base, asked) = relay(vec![won(&["inj-1", "inj-2"])]).await;

    let pulled = rows(serde_json::json!([
        { "id": "inj-1", "source_agent": "camper", "message": "first words" },
        { "id": "inj-2", "source_agent": "camper", "message": "second words" },
    ]));
    assert!(matches!(claim(&base, &agent, handler, pulled, true).await, AgentSyncOutcome::Ok));

    assert_eq!(lines(&asked), ["POST /reactive/ack"], "one claim for the whole pull, nothing released");
    assert_eq!(body_of(&asked, "POST /reactive/ack"), [serde_json::json!({ "injection_ids": ["inj-1", "inj-2"] })]);
    let typed = typed.lock().unwrap().join("\n");
    let (first, second) = (typed.find("first words"), typed.find("second words"));
    assert!(first.is_some() && second.is_some_and(|s| s > first.unwrap()), "delivered once each, in pull order: {typed}");
    assert_eq!(typed.matches("first words").count(), 1, "{typed}");
}

#[tokio::test]
async fn the_claim_carries_the_token_the_pull_used_and_the_agent_it_is_for() {
    let agent = agent_name();
    let (handler, _typed) = live_pane(&agent);
    let (base, asked) = relay(vec![won(&[])]).await;

    claim(&base, &agent, handler, rows(serde_json::json!([{ "id": "inj-1", "message": "hello" }])), true).await;

    assert_eq!(asked.lock().unwrap()[0].2, "Bearer agent-token");
}

#[tokio::test]
async fn an_id_another_seat_won_is_not_delivered_here_and_not_released() {
    let agent = agent_name();
    let (handler, typed) = live_pane(&agent);
    let (base, asked) = relay(vec![won(&["inj-2"])]).await;

    let pulled = rows(serde_json::json!([
        { "id": "inj-1", "source_agent": "camper", "message": "taken by the other seat" },
        { "id": "inj-2", "source_agent": "camper", "message": "ours" },
    ]));
    claim(&base, &agent, handler, pulled, true).await;

    let typed = typed.lock().unwrap().join("\n");
    assert!(typed.contains("ours") && !typed.contains("taken by the other seat"), "{typed}");
    assert_eq!(lines(&asked), ["POST /reactive/ack"], "an id we never won is not ours to give back");
}

#[tokio::test]
async fn a_pull_whose_ids_were_all_won_elsewhere_delivers_nothing() {
    let agent = agent_name();
    let (handler, typed) = live_pane(&agent);
    let (base, asked) = relay(vec![won(&[])]).await;

    claim(&base, &agent, handler, rows(serde_json::json!([{ "id": "inj-1", "message": "redelivered" }])), true).await;

    assert!(typed.lock().unwrap().is_empty());
    assert_eq!(lines(&asked), ["POST /reactive/ack"]);
}

#[tokio::test]
async fn a_delivery_that_fails_gives_its_claim_back_with_the_stamp_the_claim_returned() {
    let agent = agent_name();
    // No pane holds the name: the handler refuses.
    let (handler, _) = live_pane(&agent_name());
    let (base, asked) = relay(vec![won(&["inj-1"]), released()]).await;

    claim(&base, &agent, handler, rows(serde_json::json!([{ "id": "inj-1", "message": "hello" }])), true).await;

    assert_eq!(lines(&asked), ["POST /reactive/ack", "POST /reactive/release"]);
    assert_eq!(body_of(&asked, "POST /reactive/release"), [serde_json::json!({ "injection_id": "inj-1", "delivered_at": STAMP })]);
    assert_eq!(asked.lock().unwrap()[1].2, "Bearer agent-token");
}

#[tokio::test]
async fn a_delivery_that_lands_is_not_released() {
    let agent = agent_name();
    let (handler, typed) = live_pane(&agent);
    let (base, asked) = relay(vec![won(&["inj-1"]), released()]).await;

    claim(&base, &agent, handler, rows(serde_json::json!([{ "id": "inj-1", "message": "hello" }])), true).await;

    assert_eq!(typed.lock().unwrap().join("").matches("hello").count(), 1);
    assert!(!lines(&asked).contains(&"POST /reactive/release".to_string()));
}

#[tokio::test]
async fn a_rejected_agent_credential_is_retried_once_with_the_shared_token() {
    let agent = agent_name();
    let (handler, typed) = live_pane(&agent);
    let (base, asked) = relay(vec![
        ("POST /reactive/ack".into(), 401, serde_json::json!({})),
        won(&["inj-1"]),
    ])
    .await;

    claim(&base, &agent, handler, rows(serde_json::json!([{ "id": "inj-1", "message": "hello" }])), true).await;

    let auths: Vec<String> = asked.lock().unwrap().iter().map(|(_, _, auth)| auth.clone()).collect();
    assert_eq!(auths, ["Bearer agent-token", "Bearer shared-token"]);
    assert_eq!(typed.lock().unwrap().join("").matches("hello").count(), 1, "delivered after the retry");
}

#[tokio::test]
async fn a_rejected_shared_token_ends_the_call_by_what_the_status_says() {
    let agent = agent_name();
    let (handler, typed) = live_pane(&agent);
    let one = || rows(serde_json::json!([{ "id": "inj-1", "message": "hello" }]));

    let (base, asked) = relay(vec![("POST /reactive/ack".into(), 401, serde_json::json!({}))]).await;
    let expired = claim(&base, &agent, handler, one(), false).await;
    assert!(matches!(expired, AgentSyncOutcome::ReconnectSharedTokenExpired), "401: reconnect for a fresh token");
    assert_eq!(lines(&asked), ["POST /reactive/ack"], "no retry");

    let (base, asked) = relay(vec![("POST /reactive/ack".into(), 403, serde_json::json!({}))]).await;
    assert!(matches!(claim(&base, &agent, handler, one(), false).await, AgentSyncOutcome::Ok), "403: skip this agent");
    assert_eq!(lines(&asked), ["POST /reactive/ack"]);
    assert!(typed.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_claim_that_did_not_succeed_delivers_and_releases_nothing() {
    let agent = agent_name();
    let (handler, typed) = live_pane(&agent);
    let one = || rows(serde_json::json!([{ "id": "inj-1", "message": "hello" }]));
    let not_json = ("POST /reactive/ack".to_string(), 200, serde_json::json!("not an ack"));
    for answer in [
        ("POST /reactive/ack".to_string(), 500, serde_json::json!({})),
        ("POST /reactive/ack".to_string(), 409, serde_json::json!({ "error": "not_holder" })),
        not_json,
    ] {
        let status = answer.1;
        let (base, asked) = relay(vec![answer, released()]).await;
        assert!(matches!(claim(&base, &agent, handler, one(), true).await, AgentSyncOutcome::Ok), "{status}");
        assert_eq!(lines(&asked), ["POST /reactive/ack"], "{status}: nothing to give back, nothing claimed");
    }
    assert!(typed.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_relay_that_cannot_be_reached_for_the_claim_delivers_nothing() {
    let agent = agent_name();
    let (handler, typed) = live_pane(&agent);

    let outcome = claim("http://127.0.0.1:1", &agent, handler, rows(serde_json::json!([{ "id": "inj-1", "message": "hello" }])), true).await;

    assert!(matches!(outcome, AgentSyncOutcome::Ok));
    assert!(typed.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_release_the_relay_refuses_does_not_stop_the_next_one() {
    // The pane holds another name: every delivery here is refused.
    let nobody = agent_name();
    let (handler, _) = live_pane(&agent_name());
    let (base, asked) = relay(vec![
        won(&["inj-1", "inj-2"]),
        ("POST /reactive/release".into(), 500, serde_json::json!({})),
    ])
    .await;

    let pulled = rows(serde_json::json!([{ "id": "inj-1", "message": "a" }, { "id": "inj-2", "message": "b" }]));
    assert!(matches!(claim(&base, &nobody, handler, pulled, true).await, AgentSyncOutcome::Ok));

    assert_eq!(body_of(&asked, "POST /reactive/release").len(), 2);
}

#[tokio::test]
async fn a_manifest_the_cloud_limits_refuse_is_delivered_with_a_note_and_the_claim_kept() {
    let agent = agent_name();
    let (handler, typed) = live_pane(&agent);
    let (base, asked) = relay(vec![won(&["inj-1"]), released()]).await;

    let pulled = rows(serde_json::json!([{
        "id": "inj-1",
        "source_agent": "camper",
        "message": "with a file",
        "files": [{ "sha256": "not a hash", "name": "a.txt", "size": 3, "key": "AAAA", "blob_id": "blob-1" }],
    }]));
    claim(&base, &agent, handler, pulled, true).await;

    let typed = typed.lock().unwrap().join("");
    assert!(typed.contains("with a file") && typed.contains("malformed or over the cloud's limits"), "{typed}");
    assert_eq!(lines(&asked), ["POST /reactive/ack"], "nothing to retry: no file was fetched");
}

#[tokio::test]
async fn what_the_sender_declared_and_signed_reaches_the_marker_unchanged() {
    let agent = agent_name();
    let (handler, typed) = live_pane(&agent);
    let (base, _asked) = relay(vec![won(&["inj-1"])]).await;

    let pulled = rows(serde_json::json!([{
        "id": "inj-1",
        "source_agent": "camper",
        "message": "do the thing",
        "priority": "high",
        "jekt_tier": "sensitive",
    }]));
    claim(&base, &agent, handler, pulled, true).await;

    let typed = typed.lock().unwrap().join("\n");
    for label in ["FROM=camper", "TIER=sensitive", "DELIVERY=wan", "TRUST=network-claimed", "PRIORITY=high", "MSGID=inj-1"] {
        assert!(typed.contains(label), "{label} missing: {typed}");
    }
    assert!(!typed.contains("SIG="), "unsigned: no SIG label: {typed}");
    assert!(typed.contains("do the thing"));
}

#[tokio::test]
async fn an_unsigned_message_with_no_tier_is_a_plain_network_claimed_coord_one() {
    let agent = agent_name();
    let (handler, typed) = live_pane(&agent);
    let (base, _asked) = relay(vec![won(&["inj-1"])]).await;

    claim(&base, &agent, handler, rows(serde_json::json!([{ "id": "inj-1", "source_agent": "camper", "message": "hi" }])), true).await;

    let typed = typed.lock().unwrap().join("\n");
    for label in ["TIER=coord", "DELIVERY=wan", "TRUST=network-claimed", "PRIORITY=normal"] {
        assert!(typed.contains(label), "{label} missing: {typed}");
    }
    assert!(!typed.contains("SIG="), "{typed}");
}

#[tokio::test]
async fn a_reagent_signature_that_does_not_check_is_marked_invalid_and_forced_sensitive() {
    let agent = agent_name();
    let (handler, typed) = live_pane(&agent);
    let (base, _asked) = relay(vec![won(&["inj-1"])]).await;
    let now = agentmux_common::time::now_secs();

    let pulled = rows(serde_json::json!([{
        "id": "inj-1",
        "source_agent": "reagent",
        "message": "review ready",
        "reagent_sig": "AAAA",
        "reagent_key_id": "not-a-key",
        "reagent_msg_id": "m-1",
        "reagent_ts_secs": now,
    }]));
    claim(&base, &agent, handler, pulled, true).await;

    let typed = typed.lock().unwrap().join("\n");
    assert!(typed.contains("SIG=invalid") && typed.contains("TIER=sensitive"), "{typed}");
}

#[tokio::test]
async fn a_partial_reagent_signature_is_read_as_unsigned_not_as_broken() {
    let agent = agent_name();
    let (handler, typed) = live_pane(&agent);
    let (base, _asked) = relay(vec![won(&["inj-1"])]).await;

    let pulled = rows(serde_json::json!([{
        "id": "inj-1",
        "source_agent": "reagent",
        "message": "review ready",
        "reagent_sig": "AAAA",
    }]));
    claim(&base, &agent, handler, pulled, true).await;

    let typed = typed.lock().unwrap().join("\n");
    assert!(!typed.contains("SIG="), "{typed}");
    assert!(typed.contains("TIER=coord"), "{typed}");
}

#[tokio::test]
async fn a_message_the_relay_held_for_minutes_says_when_it_was_sent() {
    let agent = agent_name();
    let (handler, typed) = live_pane(&agent);
    let (base, _asked) = relay(vec![won(&["inj-1", "inj-2"])]).await;
    let old = chrono::Utc::now() - chrono::Duration::minutes(30);
    let fresh = chrono::Utc::now();

    let pulled = rows(serde_json::json!([
        { "id": "inj-1", "source_agent": "camper", "message": "old news", "created_at": old.to_rfc3339() },
        { "id": "inj-2", "source_agent": "camper", "message": "fresh news", "created_at": fresh.to_rfc3339() },
    ]));
    claim(&base, &agent, handler, pulled, true).await;

    let typed = typed.lock().unwrap().join("\n");
    assert!(typed.contains("(sent ") && typed.contains(") old news"), "{typed}");
    assert!(!typed.contains(") fresh news"), "{typed}");
}
