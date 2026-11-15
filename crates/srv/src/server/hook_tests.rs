// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! The local hook through the real router: what a script may do, and the refusals that keep a web page out
//! (docs/specs/SPEC_DRONE_REAL_WORLD_JOBS_2026_11_11.md §3.2, `webhook_local`).

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use tower::ServiceExt;

use crate::backend::cron::ActionOutcome;
use crate::backend::jobs::definition::trigger_kind;
use crate::backend::jobs::webhook_local::{hash_of, validate_filter, MAX_BODY};
use crate::backend::jobs::RunLedger;
use crate::backend::storage::cron::CronJob;
use crate::server::AppState;

const SECRET: &str = "0123456789abcdef-a-long-random-looking-secret";

/// A state with the job store and run ledger, an event job on `webhook_local` named `hook-job`, and a runner that
/// records the runs it starts.
fn state() -> (AppState, Arc<Mutex<Vec<String>>>) {
    let mut state = crate::server::tests::test_state();
    crate::server::cron::use_test_store_for_jobs(&mut state);
    let ledger = RunLedger::new(state.identity_store.clone(), Some(state.mstore.clone()), state.broker.clone());
    ledger.install_jobs(state.mstore.clone());
    state.cron_scheduler.install_runs(ledger.clone());
    let mut job = crate::backend::cron::test_job("hook-job", "", "", "", "");
    job.created_by = "operator".to_string();
    job.trigger_kind = trigger_kind::EVENT.to_string();
    job.trigger_event = json!({ "source": "webhook_local", "filter": { "secret_sha256": hash_of(SECRET) } });
    job.action_kind = "drone".to_string();
    job.action = json!({ "kind": "drone", "drone_id": "d-1" });
    state.mstore.cron_create(&job).unwrap();
    let started: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = started.clone();
    state.cron_scheduler.install_actions(Arc::new(move |job, run_id| {
        sink.lock().unwrap().push(job.id.clone());
        ledger.drone_attached(&run_id, &format!("dr-{run_id}"), &Value::Null, agentmux_common::time::now_ms());
        Box::pin(async { ActionOutcome::Drone })
    }));
    (state, started)
}

/// A script's request, which each test then spoils one way.
fn script_request(uri: &str, body: &str) -> axum::http::request::Builder {
    let _ = body;
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("Host", "127.0.0.1:5000")
        .header("Content-Type", "application/json")
        .header("X-AgentMux-Hook-Secret", SECRET)
}

async fn call(state: &AppState, req: axum::http::request::Builder, body: &str, peer: Option<&str>) -> (StatusCode, Value) {
    let mut req = req.body(Body::from(body.to_string())).unwrap();
    if let Some(p) = peer {
        req.extensions_mut().insert(ConnectInfo(p.parse::<SocketAddr>().unwrap()));
    }
    let resp = crate::server::build_router(state.clone()).oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[test]
fn a_filter_holds_the_hash_of_the_secret_and_nothing_else() {
    let ok = json!({ "secret_sha256": hash_of(SECRET) });
    assert!(validate_filter(ok.as_object().unwrap()).is_ok());
    assert_eq!(hash_of(SECRET).len(), 64);
    for bad in [json!({}), json!({ "secret_sha256": "abc" }), json!({ "secret_sha256": SECRET }), json!({ "secret_sha256": hash_of(SECRET).to_uppercase() }), json!({ "secret_sha256": hash_of(SECRET), "secret": SECRET })] {
        assert!(validate_filter(bad.as_object().unwrap()).is_err(), "{bad}");
    }
    let e = validate_filter(json!({ "secret_sha256": hash_of(SECRET), "x": 1 }).as_object().unwrap()).unwrap_err();
    assert!(e.contains("no filter key \"x\"") && e.contains("only secret_sha256"), "{e}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_script_with_the_secret_starts_one_run_that_reads_the_body_and_a_retry_with_its_id_starts_none() {
    let (state, started) = state();
    let uri = "/agentmux/hook/hook-job";
    let body = r#"{"branch":"main","kind":"push"}"#;
    let (status, v) = call(&state, script_request(uri, body).header("X-AgentMux-Hook-Id", "push-1"), body, Some("127.0.0.1:50000")).await;
    assert_eq!((status, v["ok"].clone(), v["status"].clone()), (StatusCode::ACCEPTED, json!(true), json!("started")), "{v}");
    assert_eq!(*started.lock().unwrap(), ["hook-job"]);
    let run_id = v["run_id"].as_str().unwrap().to_string();
    let ledger_input = state.cron_scheduler.runs().and_then(|l| l.event_input(&run_id)).expect("the run holds the call");
    assert_eq!(ledger_input["body"], json!({ "branch": "main", "kind": "push" }), "the body sits under its own key, so its own `kind` is no one's reserved word");
    assert!(ledger_input["received_ms"].is_number());

    let (status, again) = call(&state, script_request(uri, body).header("X-AgentMux-Hook-Id", "push-1"), body, None).await;
    assert_eq!((status, again["status"].clone()), (StatusCode::ACCEPTED, json!("not_started")), "the same call id is heard once");
    assert_eq!(started.lock().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_request_that_looks_like_a_web_pages_or_has_no_right_secret_starts_nothing() {
    let (state, started) = state();
    let uri = "/agentmux/hook/hook-job";
    let body = r#"{"a":1}"#;
    let post = |b: axum::http::request::Builder| call(&state, b, body, Some("127.0.0.1:50000"));

    // A page: an Origin, or any Sec-Fetch-* header, or someone else's Host (DNS rebinding), or a form's content type.
    for (what, req) in [
        ("an Origin", script_request(uri, body).header("Origin", "https://evil.example")),
        ("an Origin that is loopback", script_request(uri, body).header("Origin", "http://127.0.0.1:5000")),
        ("Sec-Fetch-Site", script_request(uri, body).header("Sec-Fetch-Site", "cross-site")),
        ("Sec-Fetch-Mode", script_request(uri, body).header("Sec-Fetch-Mode", "cors")),
    ] {
        assert_eq!(post(req).await.0, StatusCode::FORBIDDEN, "{what}");
    }
    let rebinding = Request::builder().method("POST").uri(uri).header("Host", "rebind.evil.example:5000").header("Content-Type", "application/json").header("X-AgentMux-Hook-Secret", SECRET);
    assert_eq!(post(rebinding).await.0, StatusCode::FORBIDDEN, "a Host that is not loopback");
    for ct in ["application/x-www-form-urlencoded", "text/plain", "multipart/form-data; boundary=x"] {
        let form = Request::builder().method("POST").uri(uri).header("Host", "localhost:5000").header("Content-Type", ct).header("X-AgentMux-Hook-Secret", SECRET);
        assert_eq!(post(form).await.0, StatusCode::UNSUPPORTED_MEDIA_TYPE, "{ct}");
    }
    let no_type = Request::builder().method("POST").uri(uri).header("Host", "[::1]:5000").header("X-AgentMux-Hook-Secret", SECRET);
    assert_eq!(post(no_type).await.0, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_eq!(call(&state, script_request(uri, body), body, Some("203.0.113.9:50000")).await.0, StatusCode::FORBIDDEN, "a peer that is not loopback");

    // No secret, a wrong one, the secret in the URL or the body instead of the header, a job that is not there: all one answer.
    let wrong = [
        Request::builder().method("POST").uri(uri).header("Host", "127.0.0.1:5000").header("Content-Type", "application/json").header("X-AgentMux-Hook-Secret", "nope"),
        Request::builder().method("POST").uri(uri).header("Host", "127.0.0.1:5000").header("Content-Type", "application/json"),
        Request::builder().method("POST").uri(format!("{uri}?secret={SECRET}")).header("Host", "127.0.0.1:5000").header("Content-Type", "application/json"),
        script_request("/agentmux/hook/no-such-job", body),
    ];
    for req in wrong {
        let (status, v) = post(req).await;
        assert_eq!((status, v["error"].clone()), (StatusCode::UNAUTHORIZED, json!("the secret is not right")));
    }
    let in_body = format!(r#"{{"secret":"{SECRET}"}}"#);
    let no_header = Request::builder().method("POST").uri(uri).header("Host", "127.0.0.1:5000").header("Content-Type", "application/json");
    assert_eq!(call(&state, no_header, &in_body, None).await.0, StatusCode::UNAUTHORIZED, "the body is not where the secret goes");
    assert!(started.lock().unwrap().is_empty(), "none of those started a run");

    // A job that is off answers like a wrong secret; a body that is not an object, or is too big, is refused after the secret.
    let mut off: CronJob = state.mstore.cron_get("hook-job").unwrap().unwrap();
    off.enabled = false;
    state.mstore.cron_set_enabled("hook-job", false).unwrap();
    assert_eq!(post(script_request(uri, body)).await.0, StatusCode::UNAUTHORIZED, "a job that is off");
    state.mstore.cron_set_enabled("hook-job", true).unwrap();
    assert_eq!(call(&state, script_request(uri, "[1]"), "[1]", None).await.0, StatusCode::BAD_REQUEST, "an array is not an object");
    assert_eq!(call(&state, script_request(uri, "x"), "not json", None).await.0, StatusCode::BAD_REQUEST);
    let big = format!(r#"{{"a":"{}"}}"#, "x".repeat(MAX_BODY));
    assert_eq!(call(&state, script_request(uri, &big), &big, None).await.0, StatusCode::PAYLOAD_TOO_LARGE, "the body is capped");
    assert!(started.lock().unwrap().is_empty());
}
