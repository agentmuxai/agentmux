// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `POST /api/v1/aws/credentials`: the agent's identity, the Audit rows and what the answers carry.

use std::sync::Arc;

use super::*;
use crate::backend::storage::AgentDefinition;
use serde_json::Value;
use crate::backend::reactive::handler::get_global_handler;
use crate::backend::storage::agents::test_agent_def;
use crate::identity::aws_broker::mint::tests::{creds_json_from, Fake};
use crate::identity::aws_broker::policy::Policy;
use crate::identity::aws_broker::sessions::Sessions;
use crate::server::tests::test_state;

const POLICY: &str = r#"{ "version": 1, "grants": {
    "read": { "tier": "read", "mint": { "kind": "command", "argv": ["/opt/read"] }, "agents": ["*"] },
    "deploy": { "tier": "write", "mint": { "kind": "command", "argv": ["/opt/escalate", "--reason", "{reason}"] }, "agents": ["*"] }
} }"#;

/// Credentials that expire `mins` minutes from the real clock, which the handler reads.
fn creds_json(mins: i64) -> String {
    creds_json_from(chrono::Utc::now(), mins)
}

fn policy() -> Result<Option<Arc<Policy>>, String> {
    Ok(Some(Arc::new(Policy::parse(POLICY).unwrap())))
}

fn agent(state: &AppState, agent_type: &str) -> AgentDefinition {
    let id = uuid::Uuid::new_v4().to_string();
    let mut def = test_agent_def(&id, &format!("Aws {}", &id[..8]), "claude", agent_type, 1, "");
    state.mstore.agent_def_insert(&mut def).unwrap();
    state.mstore.agent_def_get(&id).unwrap().unwrap()
}

fn request(profile: &str, reason: Option<&str>) -> AwsCredentialsRequest {
    serde_json::from_value(json!({ "profile": profile, "reason": reason })).unwrap()
}

async fn answer(resp: Response) -> (StatusCode, Value) {
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn call(state: &AppState, who: &AgentDefinition, fake: &Fake, sessions: &Sessions, req: AwsCredentialsRequest) -> (StatusCode, Value) {
    answer(serve(state, &who.id, who, req, policy(), sessions, fake).await).await
}

fn audited(who: &AgentDefinition) -> Vec<crate::backend::reactive::AuditLogEntry> {
    get_global_handler()
        .get_audit_log(usize::MAX)
        .into_iter()
        .filter(|e| e.source_agent.as_deref() == Some(who.slug.as_str()) && e.outcome.as_deref() != Some("system_note"))
        .collect()
}

#[tokio::test]
async fn a_container_agent_gets_credentials_the_sdk_can_read() {
    let state = test_state();
    let me = agent(&state, "container");
    let (status, body) = call(&state, &me, &Fake::ok(creds_json(60)), &Sessions::default(), request("read", None)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["Version"], 1);
    assert_eq!(body["AccessKeyId"], "AKIATESTKEY");
    assert_eq!(body["Expiration"].as_str().map(str::len), Some(20), "RFC 3339 UTC, as the SDKs read it: {body}");
}

#[tokio::test]
async fn a_fresh_grant_is_one_audit_row_and_a_cache_hit_is_none() {
    let state = test_state();
    let me = agent(&state, "container");
    let (fake, sessions) = (Fake::ok(creds_json(60)), Sessions::default());
    call(&state, &me, &fake, &sessions, request("deploy", Some("deploy stack X"))).await;
    call(&state, &me, &fake, &sessions, request("deploy", Some("deploy stack X"))).await;
    call(&state, &me, &fake, &sessions, request("deploy", Some("deploy stack X"))).await;
    assert_eq!(fake.runs(), 1);
    let rows = audited(&me);
    assert_eq!(rows.len(), 1, "one row for one session: {rows:?}");
    let row = &rows[0];
    assert!(row.success && row.error_message.is_none());
    let note = row.reason.as_deref().unwrap();
    assert!(note.contains(&me.slug) && note.contains("grant deploy") && note.contains("write tier") && note.contains("1800 s") && note.contains("minted"), "{note}");
    assert!(note.contains("reason: deploy stack X"), "the reason as the agent gave it: {note}");
}

#[tokio::test]
async fn a_refusal_is_a_failed_row_with_its_code() {
    let state = test_state();
    let me = agent(&state, "container");
    let (status, body) = call(&state, &me, &Fake::ok(creds_json(60)), &Sessions::default(), request("deploy", None)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().starts_with("REASON_REQUIRED: "), "{body}");
    let rows = audited(&me);
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].success && rows[0].error_message.as_deref() == Some("REASON_REQUIRED"), "{:?}", rows[0]);
}

#[tokio::test]
async fn a_profile_that_cannot_be_a_grant_name_is_never_recorded_or_echoed() {
    let state = test_state();
    let me = agent(&state, "container");
    let hostile = format!("x\n[admin] forged line\u{1b}[31m{}", "a".repeat(5000));
    let (status, body) = call(&state, &me, &Fake::ok(creds_json(60)), &Sessions::default(), request(&hostile, None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(!body.to_string().contains("forged"), "{body}");
    let rows = audited(&me);
    assert_eq!(rows.len(), 1);
    assert!(!format!("{:?}", rows[0]).contains("forged"), "{:?}", rows[0]);
}

#[tokio::test]
async fn a_native_agent_is_refused_and_nothing_is_minted() {
    let state = test_state();
    let me = agent(&state, "host");
    let fake = Fake::ok(creds_json(60));
    let (status, body) = call(&state, &me, &fake, &Sessions::default(), request("read", None)).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body["error"].as_str().unwrap().starts_with("NATIVE_AGENT: "), "{body}");
    assert_eq!(fake.runs(), 0);
}

#[tokio::test]
async fn no_row_answer_or_log_line_holds_a_credential() {
    let state = test_state();
    let me = agent(&state, "container");
    let (fake, sessions) = (Fake::failing("AccessDenied for AKIATESTKEY"), Sessions::default());
    let (_, refused) = call(&state, &me, &fake, &sessions, request("deploy", Some("x"))).await;
    let (_, granted_ok) = call(&state, &me, &Fake::ok(creds_json(60)), &Sessions::default(), request("read", None)).await;
    // The answer to a grant carries the credentials, by design; nothing else does.
    assert_eq!(granted_ok["SecretAccessKey"], "test-secret-value");
    let rows: Vec<String> = audited(&me).iter().map(|e| format!("{e:?}")).collect();
    for text in rows.iter().map(String::as_str).chain([refused.to_string().as_str()]) {
        assert!(!text.contains("test-secret-value") && !text.contains("test-session-token"), "{text}");
    }
}

#[tokio::test]
async fn a_request_with_no_agent_token_is_refused() {
    let state = test_state();
    let resp = handle_aws_credentials(State(state), None, Json(request("read", None))).await;
    let (status, body) = answer(resp).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    let resp = handle_aws_credentials(State(test_state()), Some(Extension(Caller::Agent { uid: "no-such-agent".into() })), Json(request("read", None))).await;
    assert_eq!(answer(resp).await.0, StatusCode::UNAUTHORIZED);
}
