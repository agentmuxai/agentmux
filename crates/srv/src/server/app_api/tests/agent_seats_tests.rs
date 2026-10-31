// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Agents set_seat` (`POST /api/v1/agent/seats`), through the handler.

use super::*;
use crate::backend::reactive::handler::get_global_handler;
use crate::backend::storage::agents::test_agent_def;
use crate::backend::storage::AgentDefinition;
use crate::registry::DefinitionStore;
use crate::server::tests::test_state;
use std::sync::Arc;

/// srv with a shared definition store, where seats live.
fn seated_state() -> (AppState, tempfile::TempDir) {
    let state = test_state();
    let dir = tempfile::tempdir().unwrap();
    state.mstore.set_def_registry(Arc::new(DefinitionStore::open(dir.path().join("definitions")).unwrap()));
    (state, dir)
}

fn agent(state: &AppState, agent_type: &str, seeded: bool) -> AgentDefinition {
    let id = uuid::Uuid::new_v4().to_string();
    let mut def = test_agent_def(&id, &format!("Seat {}", &id[..8]), "claude", agent_type, 1, "");
    def.is_seeded = i64::from(seeded);
    state.mstore.agent_def_insert(&mut def).unwrap();
    state.mstore.agent_def_get(&id).unwrap().unwrap()
}

async fn call(state: &AppState, caller: Option<&AgentDefinition>, body: Value) -> (StatusCode, Value) {
    let caller = caller.map(|d| Extension(Caller::Agent { uid: d.id.clone() }));
    let req: SetSeatsRequest = serde_json::from_value(body).unwrap();
    let resp = handle_agent_seats(State(state.clone()), caller, Json(req)).await;
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn seat_of(state: &AppState, def: &AgentDefinition) -> Option<crate::registry::AgentSeat> {
    state.mstore.agent_seat_get(&def.id)
}

fn audited(by: &AgentDefinition) -> Vec<crate::backend::reactive::AuditLogEntry> {
    get_global_handler()
        .get_audit_log(usize::MAX)
        .into_iter()
        .filter(|e| e.source_agent.as_deref() == Some(by.slug.as_str()))
        .collect()
}

#[tokio::test]
async fn an_agent_sets_a_whole_roster_in_one_call_with_no_words_from_the_operator() {
    let (state, _dir) = seated_state();
    let (me, lead, worker) = (agent(&state, "host", false), agent(&state, "host", false), agent(&state, "host", false));
    let (status, body) = call(
        &state,
        Some(&me),
        json!({ "seats": [
            { "agent": lead.slug, "seat": { "role": "leader", "qualifier": "platform", "area": "relay, CI" } },
            // The agent by id, its lead by slug, a lead set in this same call.
            { "agent": worker.id, "seat": { "role": "worker", "lead": lead.slug, "area": "docs" } },
        ] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let l = seat_of(&state, &lead).expect("the lead's seat");
    assert_eq!((l.role.as_str(), l.qualifier.as_deref(), l.area.as_deref(), l.lead.clone()), ("leader", Some("platform"), Some("relay, CI"), None));
    let w = seat_of(&state, &worker).expect("the worker's seat");
    assert_eq!((w.role.as_str(), w.lead.as_deref()), ("worker", Some(lead.id.as_str())), "a lead named by slug is stored by uid");
    assert_eq!(body["changed"].as_array().unwrap().len(), 2, "{body}");
    assert_eq!(body["changed"][1]["seat"]["role"], "worker");
    // It shows in the directory My Agents, the pane header and the Swarm read.
    let listed = state.mstore.agent_def_list().unwrap();
    assert_eq!(listed.iter().find(|d| d.id == lead.id).unwrap().seat, Some(l));
    assert!(seat_of(&state, &me).is_none(), "the caller's own seat is not touched");
}

#[tokio::test]
async fn an_agent_clears_a_seat_with_null() {
    let (state, _dir) = seated_state();
    let (me, other) = (agent(&state, "host", false), agent(&state, "host", false));
    call(&state, Some(&me), json!({ "seats": [{ "agent": other.slug, "seat": { "role": "specialist" } }] })).await;
    assert!(seat_of(&state, &other).is_some());
    let (status, body) = call(&state, Some(&me), json!({ "seats": [{ "agent": other.slug, "seat": null }] })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(seat_of(&state, &other).is_none());
    assert!(body["changed"][0]["seat"].is_null(), "{body}");
}

/// The change is the calling agent's, never the operator's: the audit log
/// names the caller and each agent whose seat changed.
#[tokio::test]
async fn the_audit_log_names_the_calling_agent_and_never_the_operator() {
    let (state, _dir) = seated_state();
    let (me, a, b) = (agent(&state, "host", false), agent(&state, "host", false), agent(&state, "host", false));
    let (status, body) = call(
        &state,
        Some(&me),
        json!({ "seats": [{ "agent": a.slug, "seat": { "role": "leader" } }, { "agent": b.slug, "seat": { "role": "worker", "lead": a.slug } }] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let entries = audited(&me);
    assert_eq!(entries.len(), 2, "one entry per agent changed: {entries:?}");
    // The log's order is not part of the contract: match each entry to its agent.
    for target in [&a, &b] {
        let entry = entries.iter().find(|e| e.target_agent == target.slug).expect("an entry per agent changed");
        assert!(entry.success);
        let reason = entry.reason.as_deref().unwrap();
        assert!(reason.contains(&me.id) && reason.contains(&target.id), "names the caller and the agent: {reason}");
        assert!(!reason.to_lowercase().contains("operator") || reason.contains("reports to the operator"), "{reason}");
    }
    let ids: Vec<&str> = body["audit_ids"].as_array().unwrap().iter().filter_map(|v| v.as_str()).collect();
    assert!(entries.iter().all(|e| ids.contains(&e.request_id.as_str())), "{ids:?}");

    // A refused attempt is recorded as one, under the same name.
    let (status, _) = call(&state, Some(&me), json!({ "seats": [{ "agent": "no-such-agent", "seat": { "role": "worker" } }] })).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(audited(&me).iter().any(|e| !e.success && e.reason.as_deref().is_some_and(|r| r.contains("refused"))));
}

#[tokio::test]
async fn a_request_without_an_agent_token_is_refused_and_never_the_operator() {
    let (state, _dir) = seated_state();
    let target = agent(&state, "host", false);
    let (status, body) = call(&state, None, json!({ "seats": [{ "agent": target.slug, "seat": { "role": "leader" } }] })).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert!(seat_of(&state, &target).is_none());
}

#[tokio::test]
async fn a_container_agent_sets_seats_as_a_host_agent_does() {
    let (state, _dir) = seated_state();
    let (me, target) = (agent(&state, "container", false), agent(&state, "container", false));
    let (status, body) = call(&state, Some(&me), json!({ "seats": [{ "agent": target.slug, "seat": { "role": "worker" } }] })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(seat_of(&state, &target).map(|s| s.role.clone()).as_deref(), Some("worker"));
    // The same rules: a lead that is not an agent here is refused, and nothing is written.
    let (status, _) = call(&state, Some(&me), json!({ "seats": [{ "agent": target.slug, "seat": { "role": "worker", "lead": "ghost-lead" } }] })).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn an_unknown_agent_or_lead_or_a_template_is_refused_and_nothing_is_written() {
    let (state, _dir) = seated_state();
    let (me, a) = (agent(&state, "host", false), agent(&state, "host", false));
    let template = agent(&state, "host", true);

    // The valid first entry is not written when a later one is refused.
    let (status, body) = call(
        &state,
        Some(&me),
        json!({ "seats": [{ "agent": a.slug, "seat": { "role": "leader" } }, { "agent": "nobody-here", "seat": { "role": "worker" } }] }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    let message = body["error"].as_str().unwrap();
    assert!(message.contains("nobody-here") && message.contains("on this machine"), "a clear error: {message}");
    assert!(seat_of(&state, &a).is_none(), "all or none");

    let (status, body) = call(&state, Some(&me), json!({ "seats": [{ "agent": a.slug, "seat": { "role": "worker", "lead": "ghost-lead" } }] })).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert!(body["error"].as_str().unwrap().contains("ghost-lead"), "{body}");

    let (status, body) = call(&state, Some(&me), json!({ "seats": [{ "agent": template.slug, "seat": { "role": "leader" } }] })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"].as_str().unwrap().contains("template"), "{body}");
    assert!(seat_of(&state, &template).is_none() && seat_of(&state, &a).is_none());
}

/// A seat's lead may be an agent on another machine on the LAN: named `Name@host` (any case), kept as the
/// LAN lists it and shown that way; one the LAN does not list is refused, with what is known.
#[tokio::test]
async fn a_lead_on_another_machine_is_accepted_when_the_lan_lists_it_and_shown_as_name_at_host() {
    let (state, _dir) = seated_state();
    let (me, a) = (agent(&state, "host", false), agent(&state, "host", false));
    let known = vec!["AgentY@narko".to_string(), "AgentX@narko".to_string()];
    let body = json!({ "seats": [{ "agent": a.slug, "seat": { "role": "worker", "lead": "agenty@NARKO" } }] });
    let (changes, remote) = changes_of_with(&state, serde_json::from_value(body).unwrap(), &known).unwrap();
    assert_eq!(remote, vec!["AgentY@narko".to_string()], "kept as the LAN lists it");
    let actor = Actor { uid: me.id.clone(), slug: me.slug.clone(), name: me.name.clone() };
    apply(&state, &actor, changes, &remote).unwrap();
    assert_eq!(seat_of(&state, &a).unwrap().lead.as_deref(), Some("AgentY@narko"));
    let shown = crate::backend::agent_seat::summary_by_id(&state.mstore, &a.id).unwrap().lead.unwrap();
    assert_eq!((shown.id.as_str(), shown.name.as_deref()), ("AgentY@narko", Some("AgentY@narko")));

    let ghost = json!({ "seats": [{ "agent": a.slug, "seat": { "role": "worker", "lead": "ghost@nowhere" } }] });
    let refused = changes_of_with(&state, serde_json::from_value(ghost).unwrap(), &known).err().expect("refused");
    assert_eq!(refused.status, StatusCode::NOT_FOUND);
    assert!(refused.message.contains("ghost@nowhere") && refused.message.contains("AgentY@narko, AgentX@narko"), "{}", refused.message);
    // Not listed on the LAN, through the handler: never accepted on a guess.
    let (status, body) = call(&state, Some(&me), json!({ "seats": [{ "agent": a.slug, "seat": { "role": "worker", "lead": "AgentY@narko" } }] })).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert!(body["error"].as_str().unwrap().contains("Known now: none right now"), "{body}");
}

/// A sibling channel on this machine is on the LAN list too, but its agents are this machine's own: they
/// are named by their local id, not offered as `Name@host`.
#[test]
fn an_agent_on_this_host_is_not_a_remote_lead() {
    let peers = vec![
        ("narko".to_string(), vec!["AgentY".to_string(), "AgentX".to_string()]),
        ("Area54".to_string(), vec!["Manoz".to_string()]),
        ("starpower".to_string(), vec!["Manoz".to_string()]),
    ];
    assert_eq!(remote_agents_on(peers, "area54"), vec!["AgentY@narko", "AgentX@narko", "Manoz@starpower"]);
}

/// The answer for a batch doesn't depend on the order of its changes: it is
/// judged against the leads as they will stand once every change is written.
#[tokio::test]
async fn a_batch_that_reverses_a_reporting_line_is_judged_as_a_whole_in_any_order() {
    let (state, _dir) = seated_state();
    let (me, a, b) = (agent(&state, "host", false), agent(&state, "host", false), agent(&state, "host", false));
    // b reports to a.
    let (status, body) = call(&state, Some(&me), json!({ "seats": [{ "agent": b.slug, "seat": { "role": "worker", "lead": a.slug } }] })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // a now reports to b, and b's seat is cleared: acyclic whichever change comes first.
    let a_to_b = json!({ "agent": a.slug, "seat": { "role": "worker", "lead": b.slug } });
    let clear_b = json!({ "agent": b.slug, "seat": null });
    let (status, body) = call(&state, Some(&me), json!({ "seats": [a_to_b.clone(), clear_b.clone()] })).await;
    assert_eq!(status, StatusCode::OK, "a then b: {body}");
    assert!(seat_of(&state, &b).is_none());
    assert_eq!(seat_of(&state, &a).unwrap().lead.as_deref(), Some(b.id.as_str()));
    // Back to where it was, then the other order.
    call(&state, Some(&me), json!({ "seats": [{ "agent": a.slug, "seat": null }, { "agent": b.slug, "seat": { "role": "worker", "lead": a.slug } }] })).await;
    let (status, body) = call(&state, Some(&me), json!({ "seats": [clear_b, a_to_b] })).await;
    assert_eq!(status, StatusCode::OK, "b then a: {body}");
}

#[tokio::test]
async fn a_cycle_of_leads_is_refused_including_one_made_by_two_changes_in_a_call() {
    let (state, _dir) = seated_state();
    let (me, a, b, c) = (agent(&state, "host", false), agent(&state, "host", false), agent(&state, "host", false), agent(&state, "host", false));
    // Within one call: a reports to b, b reports to a.
    let (status, body) = call(
        &state,
        Some(&me),
        json!({ "seats": [{ "agent": a.slug, "seat": { "role": "worker", "lead": b.slug } }, { "agent": b.slug, "seat": { "role": "worker", "lead": a.slug } }] }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"].as_str().unwrap().contains("cycle"), "{body}");
    assert!(seat_of(&state, &a).is_none() && seat_of(&state, &b).is_none(), "nothing written");

    // Against a seat already stored: c reports to b reports to a; a under c would close the loop.
    let (status, _) = call(
        &state,
        Some(&me),
        json!({ "seats": [{ "agent": b.slug, "seat": { "role": "worker", "lead": a.slug } }, { "agent": c.slug, "seat": { "role": "worker", "lead": b.slug } }] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = call(&state, Some(&me), json!({ "seats": [{ "agent": a.slug, "seat": { "role": "worker", "lead": c.slug } }] })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"].as_str().unwrap().contains("cycle"), "{body}");
    assert!(seat_of(&state, &a).is_none());

    // Its own lead.
    let (status, body) = call(&state, Some(&me), json!({ "seats": [{ "agent": a.slug, "seat": { "role": "worker", "lead": a.slug } }] })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

#[tokio::test]
async fn a_bad_seat_the_same_size_and_shape_limits_as_the_windows_are_refused() {
    let (state, _dir) = seated_state();
    let (me, a) = (agent(&state, "host", false), agent(&state, "host", false));
    for (seats, why) in [
        (json!([]), "empty"),
        (json!([{ "agent": a.slug, "seat": { "role": "boss" } }]), "unknown role"),
        (json!([{ "agent": a.slug, "seat": { "role": "worker", "area": "x".repeat(121) } }]), "area"),
        (json!([{ "agent": a.slug, "seat": { "role": "worker" } }, { "agent": a.id, "seat": null }]), "twice"),
    ] {
        let (status, body) = call(&state, Some(&me), json!({ "seats": seats })).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{why}: {body}");
        assert!(body["error"].as_str().unwrap().contains(why), "{why}: {body}");
    }
    let many: Vec<Value> = (0..=MAX_SEATS_PER_CALL).map(|_| json!({ "agent": a.slug, "seat": null })).collect();
    let (status, body) = call(&state, Some(&me), json!({ "seats": many })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(seat_of(&state, &a).is_none());
}
