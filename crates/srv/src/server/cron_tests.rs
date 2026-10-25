// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! The Schedule run routes through the real router: history, Run now and
//! report, and who may call them (SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §3.3,
//! §7).

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use serde_json::{json, Value};
use tower::ServiceExt;

use crate::backend::jobs::RunLedger;
use crate::backend::storage::agents::test_agent_def;
use crate::server::AppState;

/// A state whose cron store and run ledger are its (identity-schema) test
/// store, with two agents and their tokens: the job's creator and another.
fn state() -> (AppState, String, String) {
    state_delivering_to("b-target")
}

/// [`state`], with fires delivered to the pane `block`.
fn state_delivering_to(block: &'static str) -> (AppState, String, String) {
    let mut state = crate::server::tests::test_state();
    crate::server::cron::use_test_store_for_jobs(&mut state);
    for (uid, slug) in [("uid-jobs-owner", "jobs-owner"), ("uid-jobs-other", "jobs-other")] {
        let mut def = test_agent_def(uid, slug, "claude", "agent", 1, "");
        def.slug = slug.to_string();
        state.mstore.agent_def_insert(&mut def).unwrap();
    }
    state.mstore.attach_token_index().unwrap();
    let owner = state.mstore.agent_token_ensure("uid-jobs-owner").unwrap();
    let other = state.mstore.agent_token_ensure("uid-jobs-other").unwrap();
    let ledger = RunLedger::new(state.identity_store.clone(), Some(state.mstore.clone()), state.broker.clone());
    ledger.install_jobs(state.mstore.clone());
    state.cron_scheduler.install_runs(ledger);
    let sched = state.cron_scheduler.clone();
    sched.install_delivery(Arc::new(move |_req, _uid| Box::pin(async move { json!({"success": true, "block_id": block}) })));
    (state, owner, other)
}

async fn send(state: &AppState, method: Method, token: Option<&str>, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri).header("X-AuthKey", "test-secret-key");
    if let Some(t) = token {
        req = req.header("X-Agent-Token", t);
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

/// The owner creates a job; returns its id.
async fn create(state: &AppState, owner: &str, extra: Value) -> String {
    let mut body = json!({
        "name": "jobs-routes", "expression": "0 0 1 1 *", "prompt": "hi",
        "target": "jobs-target", "target_uid": "uid-jobs-target", "created_by": "jobs-owner",
    });
    body.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    let (status, v) = send(state, Method::POST, Some(owner), "/agentmux/cron", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    let id = v["job"]["id"].as_str().unwrap().to_string();
    state.cron_scheduler.cancel_job(&id);
    id
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_job_defaults_to_catch_up_skip_and_refuses_another_value() {
    let (state, owner, _) = state();
    let id = create(&state, &owner, json!({})).await;
    assert_eq!(state.mstore.cron_get(&id).unwrap().unwrap().catch_up, "skip");
    let id = create(&state, &owner, json!({"catch_up": "latest"})).await;
    assert_eq!(state.mstore.cron_get(&id).unwrap().unwrap().catch_up, "latest");
    let body = json!({"name": "n", "expression": "0 0 1 1 *", "prompt": "p", "target": "t", "catch_up": "all"});
    let (status, v) = send(&state, Method::POST, Some(&owner), "/agentmux/cron", Some(body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
}

/// Run now: the owner and the operator may; another agent may not. The run
/// shows in the history and in list's last run.
#[tokio::test(flavor = "multi_thread")]
async fn run_now_is_for_the_owner_or_the_operator_and_shows_in_history() {
    let (state, owner, other) = state();
    let id = create(&state, &owner, json!({})).await;
    let uri = format!("/agentmux/cron/{id}/run");

    let (status, v) = send(&state, Method::POST, Some(&other), &uri, Some(json!({}))).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{v}");
    assert!(state.identity_store.job_runs_for_job(&id, 10, None).unwrap().is_empty(), "nothing ran");

    let (status, v) = send(&state, Method::POST, Some(&owner), &uri, Some(json!({}))).await;
    assert_eq!((status, v["outcome"].as_str()), (StatusCode::OK, Some("delivered")), "{v}");
    // While the first runs, the default overlap policy (skip) refuses another.
    let (_, busy) = send(&state, Method::POST, None, &uri, Some(json!({}))).await;
    assert_eq!(busy["outcome"], "failed", "{busy}");
    let first = v["run_id"].as_str().unwrap().to_string();
    state.cron_scheduler.runs().unwrap().turn_finished(&first, crate::backend::storage::job_runs::infra::OK, None, "");
    let (status, v) = send(&state, Method::POST, None, &uri, Some(json!({}))).await;
    assert_eq!((status, v["outcome"].as_str()), (StatusCode::OK, Some("delivered")), "the operator: {v}");

    let (status, v) = send(&state, Method::GET, None, &format!("/agentmux/cron/{id}/runs?limit=5"), None).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let runs = v["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0]["triggered_by"], "operator");
    assert_eq!(runs[1]["triggered_by"], "uid-jobs-owner");
    assert_eq!(runs[0]["trigger"], "manual");

    let (_, v) = send(&state, Method::GET, None, "/agentmux/cron", None).await;
    let job = v["jobs"].as_array().unwrap().iter().find(|j| j["id"] == id.as_str()).unwrap().clone();
    assert_eq!(job["last_run"]["id"], runs[0]["id"]);
    assert_eq!(job["catch_up"], "skip");
    assert!(job.get("next_fire").is_some());
}

/// The agent that ran a run reports its outcome; another agent can't; a
/// bad outcome is refused.
#[tokio::test(flavor = "multi_thread")]
async fn a_run_is_reported_by_the_agent_that_ran_it() {
    let (state, owner, other) = state();
    let id = create(&state, &owner, json!({})).await;
    let (_, v) = send(&state, Method::POST, None, &format!("/agentmux/cron/{id}/run"), Some(json!({}))).await;
    let run_id = v["run_id"].as_str().unwrap().to_string();
    let uri = format!("/agentmux/cron/runs/{run_id}/report");

    let (status, _) = send(&state, Method::POST, Some(&other), &uri, Some(json!({"outcome": "ok"}))).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "neither the runner nor the owner");
    let (status, _) = send(&state, Method::POST, None, &uri, Some(json!({"outcome": "unknown"}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // The target agent itself (here: the owner may too).
    state.mstore.agent_def_insert(&mut {
        let mut def = test_agent_def("uid-jobs-target", "jobs-target", "claude", "agent", 1, "");
        def.slug = "jobs-target".into();
        def
    })
    .unwrap();
    let target = state.mstore.agent_token_ensure("uid-jobs-target").unwrap();
    let body = json!({"outcome": "findings", "summary": "2 stale branches", "links": ["https://example.com/r"]});
    let (status, v) = send(&state, Method::POST, Some(&target), &uri, Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let run = state.identity_store.job_run_get(&run_id).unwrap().unwrap();
    assert_eq!((run.outcome.as_str(), run.summary.as_str()), ("findings", "2 stale branches"));
    assert_eq!(run.links["reported"][0], "https://example.com/r");

    let (status, _) = send(&state, Method::POST, None, "/agentmux/cron/runs/no-such-run/report", Some(json!({"outcome": "ok"}))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// A job created by name records no target UID. The run's receiver is
/// resolved when it is delivered: from the pane's agent, or the agent
/// registered in that pane. That agent can report; another can't.
#[tokio::test(flavor = "multi_thread")]
async fn the_target_of_a_name_only_job_reports_its_run() {
    for via_meta in [true, false] {
        let block = if via_meta { "b-jobs-name-only-meta" } else { "b-jobs-name-only-reg" };
        let (state, owner, other) = state_delivering_to(block);
        let uid = format!("uid-{block}");
        let mut def = test_agent_def(&uid, block, "claude", "agent", 1, "");
        def.slug = block.to_string();
        state.mstore.agent_def_insert(&mut def).unwrap();
        let target = state.mstore.agent_token_ensure(&uid).unwrap();
        if via_meta {
            let mut pane = crate::backend::obj::Block { oid: block.to_string(), ..Default::default() };
            pane.meta.insert("agentId".to_string(), json!(uid));
            state.mstore.insert(&mut pane).unwrap();
        } else {
            state.reactive_handler.register_agent_full(block, block, None, 0, None, Some(&uid), "test").unwrap();
        }
        let body = json!({"name": "by-name", "expression": "0 0 1 1 *", "prompt": "hi", "target": block, "created_by": "jobs-owner"});
        let (_, v) = send(&state, Method::POST, Some(&owner), "/agentmux/cron", Some(body)).await;
        let id = v["job"]["id"].as_str().unwrap().to_string();
        state.cron_scheduler.cancel_job(&id);
        assert_eq!(state.mstore.cron_get(&id).unwrap().unwrap().target_uid, "", "a job by name");

        let (_, v) = send(&state, Method::POST, None, &format!("/agentmux/cron/{id}/run"), Some(json!({}))).await;
        let run_id = v["run_id"].as_str().unwrap().to_string();
        let run = state.identity_store.job_run_get(&run_id).unwrap().unwrap();
        assert_eq!((run.target_agent_uid.as_str(), run.block_id.as_str()), (uid.as_str(), block), "via_meta={via_meta}");

        let uri = format!("/agentmux/cron/runs/{run_id}/report");
        let (status, _) = send(&state, Method::POST, Some(&other), &uri, Some(json!({"outcome": "ok"}))).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "via_meta={via_meta}");
        let (status, v) = send(&state, Method::POST, Some(&target), &uri, Some(json!({"outcome": "ok"}))).await;
        assert_eq!(status, StatusCode::OK, "via_meta={via_meta}: {v}");
        if !via_meta {
            let _ = state.reactive_handler.unregister_block(block);
        }
    }
}

/// Standard cron weekdays: `0` (Sunday) is accepted, the expression is
/// stored as written, and its next fire is on a Sunday (`backend::cron::expr`).
#[tokio::test(flavor = "multi_thread")]
async fn a_standard_weekday_expression_is_kept_as_written_and_fires_on_its_day() {
    use chrono::Datelike;
    let (state, owner, _) = state();
    let (status, v) = send(
        &state,
        Method::POST,
        Some(&owner),
        "/agentmux/cron",
        Some(json!({"name": "sundays", "expression": "0 6 * * 0", "prompt": "hi", "target": "jobs-target", "created_by": "jobs-owner"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    let id = v["job"]["id"].as_str().unwrap().to_string();
    state.cron_scheduler.cancel_job(&id);
    assert_eq!(state.mstore.cron_get(&id).unwrap().unwrap().expression, "0 6 * * 0", "stored as written");
    let next = chrono::DateTime::parse_from_rfc3339(v["job"]["next_fire"].as_str().unwrap()).unwrap();
    assert_eq!(next.weekday(), chrono::Weekday::Sun);
}

/// A job definition the operator sends to `/agentmux/jobs`.
fn definition(extra: Value) -> Value {
    let mut body = json!({
        "jobVersion": 1, "name": "docs drift",
        "trigger": { "kind": "cron", "cron": "0 6 * * 1", "tz": "America/New_York" },
        "action": { "kind": "message", "to": "uid-jobs-owner", "brief": "check the docs" },
    });
    body.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    body
}

/// A definition is created on `/agentmux/jobs`, kept with its zone, listed
/// on both routes, and its next fire is 06:00 in New York.
#[tokio::test(flavor = "multi_thread")]
async fn a_definition_is_created_with_its_time_zone_and_listed_on_both_routes() {
    use chrono::Timelike;
    let (state, _, _) = state();
    let (status, v) = send(&state, Method::POST, None, "/agentmux/jobs", Some(definition(json!({})))).await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    let id = v["job"]["id"].as_str().unwrap().to_string();
    state.cron_scheduler.cancel_job(&id);
    assert_eq!((v["job"]["tz"].as_str(), v["job"]["trigger_kind"].as_str()), (Some("America/New_York"), Some("cron")));
    assert_eq!(v["job"]["definition"]["action"]["to"], "uid-jobs-owner");
    let job = state.mstore.cron_get(&id).unwrap().unwrap();
    assert_eq!((job.target.as_str(), job.target_uid.as_str()), ("jobs-owner", "uid-jobs-owner"), "a UID names its agent");
    assert_eq!(job.created_by, "operator");
    let next = chrono::DateTime::parse_from_rfc3339(v["job"]["next_fire"].as_str().unwrap()).unwrap();
    let local = next.with_timezone(&chrono_tz::America::New_York);
    assert_eq!((local.hour(), local.minute()), (6, 0));

    for route in ["/agentmux/jobs", "/agentmux/cron"] {
        let (_, v) = send(&state, Method::GET, None, route, None).await;
        assert!(v["jobs"].as_array().unwrap().iter().any(|j| j["id"] == id.as_str()), "{route}");
    }
}

/// What the definition refuses, the route refuses, with the reason.
#[tokio::test(flavor = "multi_thread")]
async fn refused_definitions_say_why() {
    let (state, owner, _) = state();
    let cases = [
        (definition(json!({ "trigger": { "kind": "event", "source": "github" } })), "trigger kind event is available in a later version"),
        (definition(json!({ "action": { "kind": "drone", "drone_id": "d" } })), "action kind drone is available in a later version"),
        (definition(json!({ "policy": { "approval": "operator" } })), "policy.approval"),
        (definition(json!({ "trigger": { "kind": "cron", "cron": "*/5 * * * *" } })), "at most every 15 minutes"),
        (definition(json!({ "template": { "id": "agentmux.not-installed", "version": "1.0.0" } })), "template"),
        (definition(json!({ "policy": { "max_turns": 50 } })), "policy.max_turns is available in a later version"),
        (definition(json!({ "policy": { "retries": 4 } })), "policy.retries: at most 3"),
    ];
    for (body, why) in cases {
        let (status, v) = send(&state, Method::POST, None, "/agentmux/jobs", Some(body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{why}: {v}");
        assert!(v["error"].as_str().unwrap().contains(why), "{why}: {v}");
    }
    // The cron shape the Schedule tool sends: the same rules.
    let body = json!({ "name": "n", "expression": "*/5 * * * *", "prompt": "p", "target": "jobs-target" });
    let (status, v) = send(&state, Method::POST, Some(&owner), "/agentmux/cron", Some(body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    let body = json!({ "name": "n", "expression": "0 9 * * *", "prompt": "p", "target": "t", "template": { "id": "a.b", "version": "1" } });
    let (status, v) = send(&state, Method::POST, Some(&owner), "/agentmux/cron", Some(body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"].as_str().unwrap().contains("template"), "a template that isn't installed and approved makes no job: {v}");
}

/// The Schedule tool's shape takes a zone and a policy too.
#[tokio::test(flavor = "multi_thread")]
async fn the_cron_shape_takes_a_time_zone_and_a_policy() {
    let (state, owner, _) = state();
    let id = create(&state, &owner, json!({ "tz": "Europe/Berlin", "policy": { "catch_up": "latest" } })).await;
    let job = state.mstore.cron_get(&id).unwrap().unwrap();
    assert_eq!((job.tz.as_str(), job.catch_up.as_str(), job.owner_uid()), ("Europe/Berlin", "latest", "uid-jobs-owner"));
    let body = json!({ "name": "n", "expression": "0 9 * * *", "prompt": "p", "target": "t", "tz": "Nowhere/Land" });
    let (status, v) = send(&state, Method::POST, Some(&owner), "/agentmux/cron", Some(body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(v["error"].as_str().unwrap().contains("unknown time zone"), "{v}");
}

/// Section 7: an agent's job is its own; a spawn job needs the operator; a
/// container agent reads only.
#[tokio::test(flavor = "multi_thread")]
async fn agents_create_their_own_message_jobs_and_containers_only_read() {
    let (state, owner, _) = state();
    let (status, v) = send(&state, Method::POST, Some(&owner), "/agentmux/jobs", Some(definition(json!({ "owner": "uid-jobs-other" })))).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{v}");
    let spawn = json!({ "action": { "kind": "spawn", "leader": "uid-jobs-other", "brief": "go" } });
    let (status, v) = send(&state, Method::POST, Some(&owner), "/agentmux/jobs", Some(definition(spawn))).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{v}");
    assert!(v["error"].as_str().unwrap().contains("itself as the leader"), "{v}");
    let spawn = json!({ "action": { "kind": "spawn", "leader": "uid-jobs-owner", "brief": "go" } });
    let (status, v) = send(&state, Method::POST, Some(&owner), "/agentmux/jobs", Some(definition(spawn))).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{v}");
    assert!(v["error"].as_str().unwrap().contains("approval"), "{v}");

    let (status, v) = send(&state, Method::POST, Some(&owner), "/agentmux/jobs", Some(definition(json!({})))).await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    let id = v["job"]["id"].as_str().unwrap().to_string();
    state.cron_scheduler.cancel_job(&id);
    let job = state.mstore.cron_get(&id).unwrap().unwrap();
    assert_eq!((job.owner_uid(), job.created_by.as_str()), ("uid-jobs-owner", "jobs-owner"));

    let mut boxed = test_agent_def("uid-jobs-boxed", "jobs-boxed", "claude", "container", 1, "");
    boxed.slug = "jobs-boxed".into();
    state.mstore.agent_def_insert(&mut boxed).unwrap();
    let boxed = state.mstore.agent_token_ensure("uid-jobs-boxed").unwrap();
    let (status, _) = send(&state, Method::POST, Some(&boxed), "/agentmux/jobs", Some(definition(json!({})))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    for (method, uri, body) in [
        (Method::PATCH, format!("/agentmux/jobs/{id}"), Some(json!({ "action": "pause" }))),
        (Method::POST, format!("/agentmux/jobs/{id}/run"), Some(json!({}))),
        (Method::DELETE, format!("/agentmux/jobs/{id}"), None),
    ] {
        let (status, v) = send(&state, method.clone(), Some(&boxed), &uri, body).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}: {v}");
    }
    let (status, _) = send(&state, Method::GET, Some(&boxed), &format!("/agentmux/jobs/{id}/runs"), None).await;
    assert_eq!(status, StatusCode::OK, "history is readable");
}

/// An edit lays fields over the definition. The 15-minute minimum applies
/// when the edit sets the trigger, not to a job kept as it was; the agent a
/// job messages may pause it but not rewrite it.
#[tokio::test(flavor = "multi_thread")]
async fn an_edit_changes_the_definition_and_keeps_old_jobs_editable() {
    let (state, owner, other) = state();
    let mut old = crate::backend::cron::test_job("jobs-minutely", "tick", "jobs-other", "uid-jobs-other", "uid-jobs-owner");
    old.created_at = agentmux_common::time::now_ms() / 1000;
    state.mstore.cron_create(&old).unwrap();
    let uri = "/agentmux/jobs/jobs-minutely";

    let (status, v) = send(&state, Method::PATCH, Some(&owner), uri, Some(json!({ "name": "renamed" }))).await;
    assert_eq!(status, StatusCode::OK, "a one-minute job from before can still be renamed: {v}");
    state.cron_scheduler.cancel_job("jobs-minutely");
    let job = state.mstore.cron_get("jobs-minutely").unwrap().unwrap();
    assert_eq!((job.name.as_str(), job.expression.as_str(), job.target_uid.as_str()), ("renamed", "* * * * *", "uid-jobs-other"));

    let (status, v) = send(&state, Method::PATCH, Some(&owner), uri, Some(json!({ "trigger": { "kind": "cron", "cron": "*/5 * * * *" } }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    let edit = json!({ "trigger": { "kind": "cron", "cron": "30 2 * * *", "tz": "America/New_York" } });
    let (status, v) = send(&state, Method::PATCH, Some(&owner), uri, Some(edit)).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    state.cron_scheduler.cancel_job("jobs-minutely");
    let job = state.mstore.cron_get("jobs-minutely").unwrap().unwrap();
    assert_eq!((job.expression.as_str(), job.tz.as_str()), ("30 2 * * *", "America/New_York"));
    assert_eq!(job.fire_count, 0, "fires and misses are not part of an edit");

    let (status, _) = send(&state, Method::PATCH, Some(&other), uri, Some(json!({ "name": "mine now" }))).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "the target may not rewrite it");
    let (status, _) = send(&state, Method::PATCH, Some(&other), uri, Some(json!({ "action": "pause" }))).await;
    assert_eq!(status, StatusCode::OK, "the target may pause it");
    let (status, v) = send(&state, Method::PATCH, Some(&owner), uri, Some(json!({ "id": "other-id" }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
}

/// A job with no recorded creator is the operator's (the UI is unattributed,
/// and so are jobs from before creators were recorded): any caller may pause
/// it, but only the operator rewrites what it sends.
#[tokio::test(flavor = "multi_thread")]
async fn a_job_without_a_creator_is_rewritten_only_by_the_operator() {
    let (state, owner, _other) = state();
    let mut job = crate::backend::cron::test_job("jobs-nobody", "tick", "jobs-other", "uid-jobs-other", "");
    job.created_at = agentmux_common::time::now_ms() / 1000;
    state.mstore.cron_create(&job).unwrap();
    let uri = "/agentmux/jobs/jobs-nobody";
    let (status, _) = send(&state, Method::PATCH, Some(&owner), uri, Some(json!({ "name": "mine now" }))).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "an agent may not rewrite an operator's job");
    let (status, _) = send(&state, Method::PATCH, Some(&owner), uri, Some(json!({ "action": "pause" }))).await;
    assert_eq!(status, StatusCode::OK, "any caller may pause it");
    let (status, v) = send(&state, Method::PATCH, None, uri, Some(json!({ "name": "renamed" }))).await;
    assert_eq!(status, StatusCode::OK, "the operator may rewrite it: {v}");
    state.cron_scheduler.cancel_job("jobs-nobody");
}

// ── task and spawn jobs, and the other trigger kinds ────────────────────

fn spawn_definition(leader: &str) -> Value {
    definition(json!({ "action": { "kind": "spawn", "leader": leader, "brief": "work on the backlog" } }))
}

/// A request to the host route, with or without the host's token.
async fn host(state: &AppState, token: Option<&str>, body: Value) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method(Method::POST)
        .uri("/api/v1/host/jobs")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json");
    if let Some(t) = token {
        req = req.header("X-Host-Token", t);
    }
    let resp = crate::server::build_router(state.clone()).oneshot(req.body(Body::from(body.to_string())).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

/// A spawn job starts workers that spend until stopped, so no connection
/// of agent's or the app's ordinary one creates it, edits it into being, or
/// runs it: only the host, with its own token.
#[tokio::test(flavor = "multi_thread")]
async fn a_spawn_job_is_created_and_run_only_through_the_host() {
    let (state, owner, _) = state();
    *state.host_ipc.lock().await = Some(crate::server::state::HostIpc { port: 1, token: "host-ipc-token".into() });

    let (status, v) = send(&state, Method::POST, None, "/agentmux/jobs", Some(spawn_definition("uid-jobs-owner"))).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "even the instance key without a token: {v}");
    assert!(v["error"].as_str().unwrap().contains("operator's own host"), "{v}");
    let (status, _) = send(&state, Method::POST, Some(&owner), "/agentmux/jobs", Some(spawn_definition("uid-jobs-owner"))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(state.mstore.cron_list().unwrap().is_empty());

    let create_body = json!({ "action": "create", "definition": spawn_definition("uid-jobs-owner") });
    assert_eq!(host(&state, None, create_body.clone()).await.0, StatusCode::FORBIDDEN);
    assert_eq!(host(&state, Some("guess"), create_body.clone()).await.0, StatusCode::FORBIDDEN);
    assert!(state.mstore.cron_list().unwrap().is_empty());
    let (status, v) = host(&state, Some("host-ipc-token"), create_body).await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    let id = v["job"]["id"].as_str().unwrap().to_string();
    state.cron_scheduler.cancel_job(&id);
    let job = state.mstore.cron_get(&id).unwrap().unwrap();
    assert_eq!((job.action_kind.as_str(), job.created_by.as_str(), job.created_by_uid.as_str()), ("spawn", "operator", ""));
    assert!(crate::server::job_actions::operator_made(&job));

    // It can't be run, rewritten or turned into a different job from the ordinary routes.
    let (status, _) = send(&state, Method::POST, None, &format!("/agentmux/jobs/{id}/run"), Some(json!({}))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = send(&state, Method::PATCH, None, &format!("/agentmux/jobs/{id}"), Some(json!({ "name": "x" }))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = send(&state, Method::PATCH, None, &format!("/agentmux/jobs/{id}"), Some(json!({ "action": "pause" }))).await;
    assert_eq!(status, StatusCode::OK, "pausing and deleting stay open");
    let (status, _) = send(&state, Method::DELETE, None, &format!("/agentmux/jobs/{id}"), None).await;
    assert_eq!(status, StatusCode::OK);

    // Another job can't be edited into a spawn job.
    let plain = create(&state, &owner, json!({})).await;
    let edit = json!({ "action": { "kind": "spawn", "leader": "uid-jobs-owner", "brief": "go" } });
    let (status, _) = send(&state, Method::PATCH, None, &format!("/agentmux/jobs/{plain}"), Some(edit)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(state.mstore.cron_get(&plain).unwrap().unwrap().action_kind, "message");

    // The host runs one now, subject to the same policy.
    let (status, v) = host(&state, Some("host-ipc-token"), json!({ "action": "create", "definition": spawn_definition("uid-jobs-owner") })).await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    let id = v["job"]["id"].as_str().unwrap().to_string();
    state.cron_scheduler.cancel_job(&id);
    let (status, v) = host(&state, Some("host-ipc-token"), json!({ "action": "run_now", "id": id })).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["outcome"], "failed", "no action runner in this state: nothing is spent");
    assert_eq!(host(&state, None, json!({ "action": "run_now", "id": id })).await.0, StatusCode::FORBIDDEN);
}

/// A task job is an ordinary job: an agent may create its own.
#[tokio::test(flavor = "multi_thread")]
async fn an_agent_creates_its_own_task_job_and_the_task_goes_on_the_board() {
    let (state, owner, _) = state();
    let task = definition(json!({
        "trigger": { "kind": "cron", "cron": "0 6 * * 1" },
        "action": { "kind": "task", "title": "Review the docs", "payload": "Look for drift.", "target": "uid-jobs-other" },
    }));
    let (status, v) = send(&state, Method::POST, Some(&owner), "/agentmux/jobs", Some(task)).await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    let id = v["job"]["id"].as_str().unwrap().to_string();
    state.cron_scheduler.cancel_job(&id);
    assert_eq!(v["job"]["action_kind"], "task");

    crate::server::job_actions::install(&state);
    let (status, v) = send(&state, Method::POST, Some(&owner), &format!("/agentmux/jobs/{id}/run"), Some(json!({}))).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["outcome"], "delivered", "{v}");
    let run = state.identity_store.job_run_get(v["run_id"].as_str().unwrap()).unwrap().unwrap();
    assert_eq!((run.state.as_str(), run.outcome.as_str()), ("finished", "ok"));
    let num = run.links["kanban_task"].as_i64().unwrap();
    let task = state.identity_store.work_board_get(&num.to_string()).unwrap().expect("the task is on the board");
    assert_eq!((task.item.title.as_str(), task.item.target_agent_uid.as_str(), task.item.target_agent.as_str()), ("Review the docs", "uid-jobs-other", "jobs-other"));
    assert_eq!((task.card.review_required, task.card.filed_by.as_str()), (false, "jobs"));
    let history = state.identity_store.work_events(&task.item.id, 10).unwrap();
    assert!(history[0].body.contains(&run.id) && history[0].body.contains(&id), "{}", history[0].body);
}

/// A spawn run never starts for a job an agent made, whatever put it in the
/// store, and a leader that isn't running opens nothing.
#[tokio::test(flavor = "multi_thread")]
async fn a_spawn_action_runs_only_for_the_operators_job_with_a_running_leader() {
    use crate::backend::cron::ActionOutcome;
    let (state, _, _) = state();
    let mut job = crate::backend::cron::test_job("j-sp", "p", "", "", "uid-jobs-owner");
    job.action_kind = "spawn".into();
    job.action = json!({ "kind": "spawn", "leader": "uid-jobs-owner", "brief": "go" });
    let ActionOutcome::Failed(why) = crate::server::job_actions::run_action(&state, job.clone(), "r1").await else { panic!() };
    assert!(why.contains("only when the operator created it"), "{why}");

    (job.created_by, job.created_by_uid) = ("operator".into(), String::new());
    let ActionOutcome::Failed(why) = crate::server::job_actions::run_action(&state, job, "r1").await else { panic!() };
    assert!(why.contains("leader isn't running"), "{why}");
}

/// Manual, one-time and interval triggers.
#[tokio::test(flavor = "multi_thread")]
async fn manual_once_and_every_triggers() {
    let (state, owner, _) = state();
    let now = agentmux_common::time::now_ms();
    let make = |trigger: Value| definition(json!({ "trigger": trigger }));

    let (status, v) = send(&state, Method::POST, Some(&owner), "/agentmux/jobs", Some(make(json!({ "kind": "manual" })))).await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    let manual = v["job"]["id"].as_str().unwrap().to_string();
    assert!(!state.cron_scheduler.is_scheduled(&manual), "a manual job has no timer");
    assert!(v["job"]["next_fire"].is_null());
    let (status, v) = send(&state, Method::POST, Some(&owner), &format!("/agentmux/jobs/{manual}/run"), Some(json!({}))).await;
    assert_eq!((status, v["outcome"].as_str()), (StatusCode::OK, Some("delivered")), "Run now is how it runs: {v}");

    let at = now + 3_600_000;
    let (status, v) = send(&state, Method::POST, Some(&owner), "/agentmux/jobs", Some(make(json!({ "kind": "once", "at": at })))).await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    let once = v["job"]["id"].as_str().unwrap().to_string();
    state.cron_scheduler.cancel_job(&once);
    let next = chrono::DateTime::parse_from_rfc3339(v["job"]["next_fire"].as_str().unwrap()).unwrap();
    assert_eq!(next.timestamp_millis(), at);
    assert_eq!(state.mstore.cron_get(&once).unwrap().unwrap().once_at_ms, Some(at));
    let (status, v) = send(&state, Method::POST, Some(&owner), "/agentmux/jobs", Some(make(json!({ "kind": "once", "at": now - 1000 })))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "a time that has passed: {v}");

    let (status, v) = send(&state, Method::POST, Some(&owner), "/agentmux/jobs", Some(make(json!({ "kind": "every", "seconds": 3600 })))).await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    let every = v["job"]["id"].as_str().unwrap().to_string();
    state.cron_scheduler.cancel_job(&every);
    let next = chrono::DateTime::parse_from_rfc3339(v["job"]["next_fire"].as_str().unwrap()).unwrap().timestamp_millis();
    assert!((now + 3_500_000..=now + 3_700_000).contains(&next), "one period after creation: {}", next - now);
    let (status, v) = send(&state, Method::POST, Some(&owner), "/agentmux/jobs", Some(make(json!({ "kind": "every", "seconds": 600 })))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "10 minutes is under the agent minimum: {v}");
    // The operator's own is the same rule: an agent runs unattended either way.
    let (status, _) = send(&state, Method::POST, None, "/agentmux/jobs", Some(make(json!({ "kind": "every", "seconds": 600 })))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
