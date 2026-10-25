// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! Who may raise a job's caps, and the operator's confirmation of a job on a
//! provider that reports no cost, through the real router
//! (SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §5.3, §7, §8).

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use serde_json::{json, Value};
use tower::ServiceExt;

use crate::backend::jobs::RunLedger;
use crate::backend::storage::agents::test_agent_def;
use crate::server::AppState;

const HOST_TOKEN: &str = "host-ipc-token";

/// A state with a job store and ledger, two agents on claude (one owns the
/// jobs) and one on codex, which reports no cost.
fn state() -> (AppState, String) {
    let mut state = crate::server::tests::test_state();
    crate::server::cron::use_test_store_for_jobs(&mut state);
    for (uid, slug, provider) in [("uid-caps-owner", "caps-owner", "claude"), ("uid-caps-claude", "caps-claude", "claude"), ("uid-caps-codex", "caps-codex", "codex")] {
        let mut def = test_agent_def(uid, slug, provider, "agent", 1, "");
        def.slug = slug.to_string();
        state.mstore.agent_def_insert(&mut def).unwrap();
    }
    state.mstore.attach_token_index().unwrap();
    let owner = state.mstore.agent_token_ensure("uid-caps-owner").unwrap();
    let ledger = RunLedger::new(state.identity_store.clone(), Some(state.mstore.clone()), state.broker.clone());
    ledger.install_jobs(state.mstore.clone());
    state.cron_scheduler.install_runs(ledger);
    (state, owner)
}

async fn call(state: &AppState, method: Method, uri: &str, headers: &[(&str, &str)], body: Option<Value>) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri).header("X-AuthKey", "test-secret-key");
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let body = match body {
        Some(b) => {
            req = req.header("Content-Type", "application/json");
            Body::from(b.to_string())
        }
        None => Body::empty(),
    };
    let resp = crate::server::build_router(state.clone()).oneshot(req.body(body).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

/// An ordinary route: with an agent's token, or without one (the app's window
/// looks the same as an agent that left its token out).
async fn send(state: &AppState, token: Option<&str>, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    match token {
        Some(t) => call(state, method, uri, &[("X-Agent-Token", t)], body).await,
        None => call(state, method, uri, &[], body).await,
    }
}

/// The host route, with the host's token.
async fn host(state: &AppState, body: Value) -> (StatusCode, Value) {
    call(state, Method::POST, "/api/v1/host/jobs", &[("X-Host-Token", HOST_TOKEN)], Some(body)).await
}

fn definition(target: &str, caps: Value) -> Value {
    json!({
        "jobVersion": 1, "name": "caps-job",
        "trigger": { "kind": "cron", "cron": "0 6 * * *" },
        "action": { "kind": "message", "to": target, "brief": "check the build" },
        "policy": { "caps": caps },
    })
}

fn caps_of(state: &AppState, id: &str) -> Value {
    let job = state.mstore.cron_get(id).unwrap().unwrap();
    serde_json::to_value(&job.policy.caps).unwrap()
}

fn created_id(v: &Value) -> String {
    v["job"]["id"].as_str().unwrap().to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agents_higher_caps_are_clamped_to_the_defaults_and_the_reply_says_so() {
    let (state, owner) = state();
    let body = definition("uid-caps-claude", json!({ "run_usd": 50, "day_usd": 400, "messages_per_day": 1000 }));
    let (status, v) = send(&state, Some(&owner), Method::POST, "/agentmux/jobs", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    let id = created_id(&v);
    state.cron_scheduler.cancel_job(&id);
    assert_eq!(caps_of(&state, &id), json!({ "run_usd": 5.0, "day_usd": 20.0, "messages_per_day": 24 }));
    let notes = v["notes"].as_array().unwrap();
    assert_eq!(notes.len(), 3, "{v}");
    assert!(notes[0].as_str().unwrap().contains("only the operator raises a cap"), "{v}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agents_lower_caps_are_kept() {
    let (state, owner) = state();
    let body = definition("uid-caps-claude", json!({ "run_usd": 1, "day_usd": 3 }));
    let (status, v) = send(&state, Some(&owner), Method::POST, "/agentmux/jobs", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    let id = created_id(&v);
    state.cron_scheduler.cancel_job(&id);
    assert_eq!(caps_of(&state, &id), json!({ "run_usd": 1.0, "day_usd": 3.0 }));
    assert!(v["notes"].as_array().unwrap().is_empty(), "{v}");
}

/// The ordinary routes serve agents, never the app's window, so a request on
/// them (or one without the hosts's token) raises no cap.
#[tokio::test(flavor = "multi_thread")]
async fn a_request_without_the_hosts_token_cannot_raise_a_cap() {
    let (state, owner) = state();
    *state.host_ipc.lock().await = Some(crate::server::state::HostIpc { port: 1, token: HOST_TOKEN.into() });
    let body = definition("uid-caps-claude", json!({ "run_usd": 50 }));
    let (status, _) = send(&state, None, Method::POST, "/agentmux/jobs", Some(body.clone())).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "a request with no token is refused");
    let (status, v) = send(&state, Some(&owner), Method::POST, "/agentmux/jobs", Some(body.clone())).await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    let id = created_id(&v);
    state.cron_scheduler.cancel_job(&id);
    assert_eq!(caps_of(&state, &id)["run_usd"], 5.0, "a create is held to the defaults");

    let (status, v) = send(&state, Some(&owner), Method::PATCH, &format!("/agentmux/jobs/{id}"), Some(json!({ "policy": { "caps": { "run_usd": 80 } } }))).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(caps_of(&state, &id)["run_usd"], 5.0);

    // A guessed token raises nothing either.
    let create = json!({ "action": "create", "definition": body });
    let (status, _) = call(&state, Method::POST, "/api/v1/host/jobs", &[("X-Host-Token", "guess")], Some(create.clone())).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call(&state, Method::POST, "/api/v1/host/jobs", &[], Some(create)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let update = json!({ "action": "update", "id": id, "fields": { "policy": { "caps": { "run_usd": 80 } } } });
    let (status, _) = call(&state, Method::POST, "/api/v1/host/jobs", &[], Some(update)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(caps_of(&state, &id)["run_usd"], 5.0);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_operator_raises_a_cap_through_the_host_and_an_agents_edits_keep_it() {
    let (state, owner) = state();
    *state.host_ipc.lock().await = Some(crate::server::state::HostIpc { port: 1, token: HOST_TOKEN.into() });
    let create = json!({ "action": "create", "definition": definition("uid-caps-claude", json!({ "run_usd": 50, "day_usd": 400 })) });
    let (status, v) = host(&state, create).await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    let id = created_id(&v);
    state.cron_scheduler.cancel_job(&id);
    assert_eq!(caps_of(&state, &id), json!({ "run_usd": 50.0, "day_usd": 400.0 }), "the operator's figures stand");
    assert!(v["notes"].as_array().unwrap().is_empty());

    // An agent that owns the job edits its brief: the raised caps are untouched.
    let mut job = state.mstore.cron_get(&id).unwrap().unwrap();
    (job.created_by_uid, job.owner) = ("uid-caps-owner".into(), "uid-caps-owner".into());
    state.mstore.cron_update_definition(&job).unwrap();
    let edit = json!({ "action": { "kind": "message", "to": "uid-caps-claude", "brief": "check the build twice" } });
    let (status, v) = send(&state, Some(&owner), Method::PATCH, &format!("/agentmux/jobs/{id}"), Some(edit)).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(caps_of(&state, &id), json!({ "run_usd": 50.0, "day_usd": 400.0 }));

    // The agent asking for more than it had is held to the default.
    let more = json!({ "policy": { "caps": { "run_usd": 99, "day_usd": 400 } } });
    let (status, v) = send(&state, Some(&owner), Method::PATCH, &format!("/agentmux/jobs/{id}"), Some(more)).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(caps_of(&state, &id), json!({ "run_usd": 5.0, "day_usd": 400.0 }));
    assert_eq!(v["notes"].as_array().unwrap().len(), 1);

    // The operator raises it again, through the host.
    let update = json!({ "action": "update", "id": id, "fields": { "policy": { "caps": { "run_usd": 80, "day_usd": 400 } } } });
    let (status, v) = host(&state, update).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(caps_of(&state, &id), json!({ "run_usd": 80.0, "day_usd": 400.0 }));
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_move_the_defaults_an_agent_is_held_to() {
    let (state, owner) = state();
    let mut settings = state.config_watcher.get_settings();
    settings.jobs_cap_run_usd = Some(2.0);
    settings.jobs_cap_messages_day = Some(3);
    state.config_watcher.update_settings(settings);
    let body = definition("uid-caps-claude", json!({ "run_usd": 3, "messages_per_day": 10 }));
    let (status, v) = send(&state, Some(&owner), Method::POST, "/agentmux/jobs", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    let id = created_id(&v);
    state.cron_scheduler.cancel_job(&id);
    assert_eq!(caps_of(&state, &id), json!({ "run_usd": 2.0, "messages_per_day": 3 }));
}

// ── a provider that reports no cost ─────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn a_job_on_a_provider_with_no_cost_needs_the_operators_confirmation() {
    let (state, owner) = state();
    *state.host_ipc.lock().await = Some(crate::server::state::HostIpc { port: 1, token: HOST_TOKEN.into() });
    let body = definition("uid-caps-codex", json!({}));

    // An agent can't confirm it.
    let (status, v) = send(&state, Some(&owner), Method::POST, "/agentmux/jobs", Some(body.clone())).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{v}");
    let why = v["error"].as_str().unwrap();
    assert!(why.contains("codex") && why.contains("doesn't report what a run costs") && why.contains("operator"), "{why}");
    let (status, _) = send(&state, None, Method::POST, "/agentmux/jobs", Some(body.clone())).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "nor a request without the host's token");
    assert!(state.mstore.cron_list().unwrap().is_empty());

    // The host, without the confirmation, is told what to confirm.
    let create = json!({ "action": "create", "definition": body });
    let (status, v) = host(&state, create.clone()).await;
    assert_eq!(status, StatusCode::CONFLICT, "{v}");
    assert_eq!((v["needs_confirmation"].as_str(), v["provider"].as_str()), (Some("no_cost"), Some("codex")));
    assert!(v["error"].as_str().unwrap().contains("confirm_no_cost"));
    assert!(state.mstore.cron_list().unwrap().is_empty());

    // Confirmed: created, with a timeout, since it is all that limits the job.
    let mut confirmed = create;
    confirmed["confirm_no_cost"] = json!(true);
    let (status, v) = host(&state, confirmed).await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    let id = created_id(&v);
    state.cron_scheduler.cancel_job(&id);
    assert_eq!(state.mstore.cron_get(&id).unwrap().unwrap().policy.timeout_minutes, Some(60));
    assert!(v["notes"][0].as_str().unwrap().contains("timeout_minutes"), "{v}");

    // The job is marked for the tab.
    let job = state.mstore.cron_get(&id).unwrap().unwrap();
    assert_eq!(super::no_cost_provider(&state, &job).as_deref(), Some("codex"));

    // Its owner may still change the brief (the same agent), not point it at another no-cost agent.
    let mut owned = job.clone();
    (owned.created_by_uid, owned.owner) = ("uid-caps-owner".into(), "uid-caps-owner".into());
    state.mstore.cron_update_definition(&owned).unwrap();
    let edit = json!({ "action": { "kind": "message", "to": "uid-caps-codex", "brief": "check it again" } });
    let (status, v) = send(&state, Some(&owner), Method::PATCH, &format!("/agentmux/jobs/{id}"), Some(edit)).await;
    assert_eq!(status, StatusCode::OK, "{v}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_job_on_a_provider_that_reports_cost_needs_no_confirmation_and_no_timeout() {
    let (state, owner) = state();
    let (status, v) = send(&state, Some(&owner), Method::POST, "/agentmux/jobs", Some(definition("uid-caps-claude", json!({})))).await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    let id = created_id(&v);
    state.cron_scheduler.cancel_job(&id);
    let job = state.mstore.cron_get(&id).unwrap().unwrap();
    assert_eq!(job.policy.timeout_minutes, None);
    assert_eq!(super::no_cost_provider(&state, &job), None);
}
