// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Agents update`, `bundles`, `hide` and `unhide`, through the handlers.

use super::*;
use crate::backend::reactive::handler::get_global_handler;
use crate::backend::storage::agents::test_agent_def;
use crate::backend::storage::AgentDefinition;
use crate::server::tests::test_state;

fn agent(state: &AppState, name: &str) -> AgentDefinition {
    let id = uuid::Uuid::new_v4().to_string();
    let mut def = test_agent_def(&id, name, "claude", "host", 1, "");
    state.mstore.agent_def_insert(&mut def).unwrap();
    state.mstore.agent_def_get(&id).unwrap().unwrap()
}

fn caller_of(d: Option<&AgentDefinition>) -> Option<Extension<Caller>> {
    d.map(|d| Extension(Caller::Agent { uid: d.id.clone() }))
}

async fn body_of(resp: Response) -> (StatusCode, Value) {
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn update_call(state: &AppState, by: Option<&AgentDefinition>, req: Value) -> (StatusCode, Value) {
    body_of(handle_agent_update(State(state.clone()), caller_of(by), Json(serde_json::from_value(req).unwrap())).await).await
}

async fn bundles_call(state: &AppState, by: Option<&AgentDefinition>, req: Value) -> (StatusCode, Value) {
    body_of(handle_agent_bundles(State(state.clone()), caller_of(by), Json(serde_json::from_value(req).unwrap())).await).await
}

fn audited(by: &AgentDefinition) -> Vec<crate::backend::reactive::AuditLogEntry> {
    get_global_handler().get_audit_log(usize::MAX).into_iter().filter(|e| e.source_agent.as_deref() == Some(by.slug.as_str())).collect()
}

#[tokio::test]
async fn an_update_changes_only_the_fields_given_and_is_recorded_under_the_caller() {
    let state = test_state();
    let (me, target) = (agent(&state, "Editor"), agent(&state, "Edited"));
    let (status, body) = update_call(&state, Some(&me), json!({ "agent": target.slug, "description": "Reviews diffs", "auto_start": true, "idle_timeout_minutes": 30 })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["changed"], json!(["description", "auto_start", "idle_timeout_minutes"]));
    let after = state.mstore.agent_def_get(&target.id).unwrap().unwrap();
    assert_eq!((after.description.as_str(), after.auto_start, after.idle_timeout_minutes), ("Reviews diffs", 1, 30));
    assert_eq!((after.name.as_str(), after.slug.as_str(), after.provider.as_str()), (target.name.as_str(), target.slug.as_str(), "claude"));
    assert_eq!(after.working_directory, target.working_directory, "a field not given stays");
    let log = audited(&me);
    assert!(log.iter().any(|e| e.success && e.reason.as_deref().is_some_and(|r| r.contains("changed description, auto_start, idle_timeout_minutes"))), "{log:?}");
}

#[tokio::test]
async fn an_update_with_nothing_to_change_or_a_bad_value_is_refused_and_recorded() {
    let state = test_state();
    let (me, target) = (agent(&state, "Editor"), agent(&state, "Edited"));
    let (status, body) = update_call(&state, Some(&me), json!({ "agent": target.slug })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"].as_str().unwrap().contains("nothing to change"), "{body}");
    let (status, _) = update_call(&state, Some(&me), json!({ "agent": target.slug, "idle_timeout_minutes": -1 })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = update_call(&state, Some(&me), json!({ "agent": "no-such-agent", "icon": "x" })).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(audited(&me).iter().filter(|e| !e.success).count(), 3);
}

#[test]
fn a_field_that_decides_what_runs_or_where_traffic_goes_is_not_taken() {
    for field in ["shell", "provider_flags", "working_directory", "model_vendor_base_url"] {
        let parsed = serde_json::from_value::<UpdateRequest>(json!({ "agent": "a", field: "x" }));
        assert!(parsed.is_err(), "{field} must be refused by name");
    }
}

#[tokio::test]
async fn a_request_without_an_agent_token_is_refused() {
    let state = test_state();
    let target = agent(&state, "Edited");
    let (status, body) = update_call(&state, None, json!({ "agent": target.slug, "icon": "x" })).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert_eq!(state.mstore.agent_def_get(&target.id).unwrap().unwrap().icon, target.icon);
}

#[tokio::test]
async fn bundles_are_read_set_in_order_and_an_unknown_or_blank_one_is_refused() {
    let state = test_state();
    let (me, target) = (agent(&state, "Editor"), agent(&state, "Edited"));
    for (id, blank) in [("b1", false), ("b2", false), ("blank", true)] {
        let mut b: crate::backend::storage::bundles::Bundle =
            serde_json::from_value(json!({ "id": id, "name": id, "mcp_servers": "[]", "skills": "[]" })).unwrap();
        b.is_blank = blank;
        state.id_store.bundle_upsert(&b).unwrap();
    }
    let (status, body) = bundles_call(&state, Some(&me), json!({ "agent": target.slug, "bundle_ids": ["b2", "b1", "b2"] })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["bundle_ids"], json!(["b2", "b1"]));
    let (_, read) = bundles_call(&state, Some(&me), json!({ "agent": target.slug })).await;
    assert_eq!(read["bundle_ids"], json!(["b2", "b1"]));
    let (status, _) = bundles_call(&state, Some(&me), json!({ "agent": target.slug, "bundle_ids": ["nope"] })).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = bundles_call(&state, Some(&me), json!({ "agent": target.slug, "bundle_ids": ["blank"] })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(state.mstore.agent_bundle_ids(&target.id).unwrap(), ["b2", "b1"], "a refused list leaves the stored one");
}

#[tokio::test]
async fn only_a_seeded_template_can_be_hidden() {
    let state = test_state();
    let (me, mine) = (agent(&state, "Lead"), agent(&state, "Mine"));
    let req = json!({ "template": mine.slug });
    let (status, body) = body_of(handle_agent_hide(State(state.clone()), caller_of(Some(&me)), Json(serde_json::from_value(req).unwrap())).await).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert!(body["error"].as_str().unwrap().contains("delete"), "{body}");
    assert_eq!(state.mstore.agent_def_get(&mine.id).unwrap().unwrap().user_hidden, 0);
}

// ---- route and model on a model route ----

fn on_route(state: &AppState, who: &AgentDefinition) {
    crate::backend::model_gateway::launch::set_choice(&state.mstore, &who.id, "claude", "openrouter", "").unwrap();
}

/// The route's key, in the Vault.
fn with_route_key(state: &AppState) {
    crate::backend::vault::seal::use_test_key();
    let entry: crate::backend::vault::NewEntry = serde_json::from_value(json!({ "kind": "api-key", "label": "OpenRouter API key", "parts": { "value": "sk-or-test" } })).unwrap();
    crate::backend::vault::create(&state.identity_store, entry).unwrap();
}

#[tokio::test]
async fn the_model_changes_on_the_route_the_agent_is_on_and_the_agent_can_leave_the_route() {
    let state = test_state();
    let (me, target) = (agent(&state, "Editor"), agent(&state, "Routed"));
    on_route(&state, &target);
    with_route_key(&state);
    let token = crate::backend::model_gateway::token::token_for(&target.slug, &target.id, "openrouter");

    let (status, body) = update_call(&state, Some(&me), json!({ "agent": target.slug, "model": "deepseek/deepseek-v4-pro-0813" })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["changed"], json!(["model"]));
    let saved = crate::backend::model_gateway::launch::choice_of(&state.mstore, &target.id).unwrap();
    assert_eq!((saved.route.as_str(), saved.model.as_str()), ("openrouter", "deepseek/deepseek-v4-pro-0813"));
    let back = crate::backend::model_gateway::switch::previous_of(&state.mstore, &target.id).unwrap();
    assert_eq!((back.route.as_str(), back.model.as_str()), ("openrouter", "deepseek/deepseek-v4.1-flash-1017"), "the panel's Back row");

    let (status, body) = update_call(&state, Some(&me), json!({ "agent": target.slug, "model": "gpt-5" })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"].as_str().unwrap().contains("does not serve"), "{body}");

    let (status, body) = update_call(&state, Some(&me), json!({ "agent": target.slug, "route": "" })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["changed"], json!(["route"]));
    assert!(crate::backend::model_gateway::launch::choice_of(&state.mstore, &target.id).is_none());
    assert!(crate::backend::model_gateway::token::grant_for(&token).is_none(), "its token stops working at once");
}

#[tokio::test]
async fn an_update_does_not_put_an_agent_on_a_route_because_that_spends_the_persons_key() {
    let state = test_state();
    let (me, target) = (agent(&state, "Editor"), agent(&state, "Plain"));
    for req in [json!({ "agent": target.slug, "route": "openrouter" }), json!({ "agent": target.slug, "model": "deepseek/deepseek-v4-pro-0813" })] {
        let (status, body) = update_call(&state, Some(&me), req).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(body["error"].as_str().unwrap().contains("not on a model route"), "{body}");
    }
    assert!(crate::backend::model_gateway::launch::choice_of(&state.mstore, &target.id).is_none());
    let (status, body) = update_call(&state, Some(&me), json!({ "agent": target.slug, "route": "" })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

#[tokio::test]
async fn an_agent_changing_its_model_is_limited_to_six_an_hour() {
    let state = test_state();
    let (me, target) = (agent(&state, "Editor"), agent(&state, "Thrasher"));
    on_route(&state, &target);
    with_route_key(&state);
    let models = ["deepseek/deepseek-v4-pro-0813", "deepseek/deepseek-v4.1-flash-1017"];
    for _ in 0..8 {
        let (status, _) = update_call(&state, Some(&me), json!({ "agent": target.slug, "model": "gpt-5" })).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "a refused change is not one of the six");
    }
    for i in 0..crate::backend::model_gateway::switch::AGENT_SWITCHES_PER_HOUR {
        let (status, body) = update_call(&state, Some(&me), json!({ "agent": target.slug, "model": models[i % 2] })).await;
        assert_eq!(status, StatusCode::OK, "{i}: {body}");
    }
    let (status, body) = update_call(&state, Some(&me), json!({ "agent": target.slug, "model": models[0] })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"].as_str().unwrap().contains("times an hour"), "{body}");
}

#[tokio::test]
async fn a_leader_sets_and_resets_a_workers_spend_cap_and_a_worker_may_only_lower_its_own() {
    use crate::backend::model_gateway::cap::{self, DEFAULT_AGENT_CAP_USD};
    let state = test_state();
    let (leader, worker, stranger) = (agent(&state, "Lead"), agent(&state, "Work"), agent(&state, "Stranger"));
    state
        .mstore
        .agent_content_set(&crate::backend::storage::content::AgentContent {
            agent_id: worker.id.clone(),
            content_type: crate::backend::agent_seat::WORKER_OF_CONTENT.to_string(),
            content: json!({ "agent": leader.slug, "id": leader.id }).to_string(),
            updated_at: 1,
        })
        .unwrap();
    assert_eq!(cap::agent_cap(&state.mstore, &worker.id), Some(DEFAULT_AGENT_CAP_USD));

    let (status, body) = update_call(&state, Some(&leader), json!({ "agent": worker.slug, "spend_cap_usd": 40 })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(cap::agent_cap(&state.mstore, &worker.id), Some(40.0));
    let log = audited(&leader);
    assert!(log.iter().any(|e| e.success && e.reason.as_deref().is_some_and(|r| r.contains("spend cap $20.00 to $40.00"))), "the audit row says from and to: {log:?}");

    let (status, body) = update_call(&state, Some(&stranger), json!({ "agent": worker.slug, "spend_cap_usd": 5 })).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let (status, _) = update_call(&state, Some(&worker), json!({ "agent": worker.slug, "spend_cap_usd": 80 })).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "an agent does not raise its own cap");
    let (status, _) = update_call(&state, Some(&worker), json!({ "agent": worker.slug, "spend_cap_usd": "none" })).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "nor lift it");
    let (status, _) = update_call(&state, Some(&worker), json!({ "agent": worker.slug, "reset_spend_today": true })).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "nor reset its day");
    let (status, body) = update_call(&state, Some(&worker), json!({ "agent": worker.slug, "spend_cap_usd": 10 })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(cap::agent_cap(&state.mstore, &worker.id), Some(10.0));

    let (status, body) = update_call(&state, Some(&leader), json!({ "agent": worker.slug, "reset_spend_today": true })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, _) = update_call(&state, Some(&leader), json!({ "agent": worker.slug, "spend_cap_usd": "lots" })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_refused_spend_change_leaves_the_rest_of_the_request_unapplied() {
    let state = test_state();
    let (stranger, target) = (agent(&state, "Stranger"), agent(&state, "Target"));
    let (status, _) = update_call(&state, Some(&stranger), json!({ "agent": target.slug, "description": "changed", "spend_cap_usd": 5 })).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let after = state.mstore.agent_def_get(&target.id).unwrap().unwrap();
    assert_eq!(after.description, target.description, "the description was not applied");
    let (status, _) = update_call(&state, Some(&stranger), json!({ "agent": stranger.slug, "spend_cap_usd": 0.004 })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "a cap under a cent is refused, not read back as none");
}
