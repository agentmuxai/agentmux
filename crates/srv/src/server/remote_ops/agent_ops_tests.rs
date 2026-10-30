// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The agent-control operations (spec §4.3, P3): they run the local code for a named remote agent.

use std::sync::Arc;

use super::*;
use crate::backend::storage::agents::test_agent_def;
use crate::backend::storage::AgentDefinition;
use crate::registry::DefinitionStore;
use crate::server::remote_ops::{lookup, Verification};
use crate::server::tests::test_state;

fn caller() -> RemoteCaller {
    RemoteCaller {
        agent_uid: "uid-rop".into(),
        slug: "rop".into(),
        machine: "narko".into(),
        instance_id: "inst".into(),
        install_trusted: true,
        verification: Verification::Lan,
        reason: String::new(),
    }
}

/// srv with a shared definition store, where seats live.
fn seated_state() -> (AppState, tempfile::TempDir) {
    let state = test_state();
    let dir = tempfile::tempdir().unwrap();
    state.mstore.set_def_registry(Arc::new(DefinitionStore::open(dir.path().join("definitions")).unwrap()));
    (state, dir)
}

fn agent(state: &AppState, agent_type: &str) -> AgentDefinition {
    let id = uuid::Uuid::new_v4().to_string();
    let mut def = test_agent_def(&id, &format!("Ops {}", &id[..8]), "claude", agent_type, 1, "");
    state.mstore.agent_def_insert(&mut def).unwrap();
    state.mstore.agent_def_get(&id).unwrap().unwrap()
}

fn stored_env(state: &AppState, def: &AgentDefinition) -> Option<String> {
    state.mstore.agent_content_get(&def.id, "env").unwrap().map(|c| c.content)
}

#[test]
fn every_agent_control_operation_needs_a_certificate_backed_caller() {
    for op in ["agents.start", "agents.open", "agents.stop", "agents.set_seat", "agents.env_get", "agents.env_set"] {
        assert!(lookup(op).unwrap_or_else(|| panic!("{op} is registered")).changes_state, "{op}");
    }
}

#[tokio::test]
async fn an_environment_is_set_and_read_for_a_named_agent_and_a_refused_name_writes_nothing() {
    let state = test_state();
    let target = agent(&state, "host");
    let out = env_set(&state, &caller(), json!({ "agent": target.slug, "set": { "TZ": "UTC" } })).await.unwrap();
    assert_eq!(out["agent"]["id"], target.id.as_str());
    assert_eq!(stored_env(&state, &target).as_deref(), Some("TZ=UTC\n"));

    let got = env_get(&state, &caller(), json!({ "agent": target.id })).await.unwrap();
    assert!(got["lines"].as_str().unwrap().contains("TZ=UTC"), "{got}");

    // The rules a local caller meets: a name an agent may not set is refused.
    let e = env_set(&state, &caller(), json!({ "agent": target.slug, "set": { "PATH": "/x" } })).await.unwrap_err();
    assert_eq!(e.code, "invalid_arg", "{}", e.message);
    assert_eq!(stored_env(&state, &target).as_deref(), Some("TZ=UTC\n"));
}

#[tokio::test]
async fn a_call_must_name_an_agent_here() {
    let state = test_state();
    for run in [
        env_set(&state, &caller(), json!({ "set": { "TZ": "1" } })).await,
        env_get(&state, &caller(), json!({})).await,
        start(&state, &caller(), json!({})).await,
        open(&state, &caller(), json!({})).await,
        stop(&state, &caller(), json!({ "agent": "  " })).await,
    ] {
        assert_eq!(run.unwrap_err().code, "invalid_arg");
    }
    for run in [
        env_set(&state, &caller(), json!({ "agent": "nobody-here", "set": { "TZ": "1" } })).await,
        start(&state, &caller(), json!({ "agent": "nobody-here" })).await,
        open(&state, &caller(), json!({ "agent": "nobody-here" })).await,
        stop(&state, &caller(), json!({ "agent": "nobody-here" })).await,
    ] {
        assert_eq!(run.unwrap_err().code, "not_found");
    }
}

#[tokio::test]
async fn a_remote_agent_sets_a_seat_under_the_same_checks_as_a_local_one() {
    let (state, _dir) = seated_state();
    let target = agent(&state, "container");
    let out = set_seat(&state, &caller(), json!({ "seats": [{ "agent": target.slug, "seat": { "role": "worker" } }] })).await.unwrap();
    assert_eq!(out["changed"][0]["id"], target.id.as_str());
    assert_eq!(state.mstore.agent_seat_get(&target.id).map(|s| s.role).as_deref(), Some("worker"));

    // A lead that is no agent here is refused, and nothing is written.
    let e = set_seat(&state, &caller(), json!({ "seats": [{ "agent": target.slug, "seat": { "role": "worker", "lead": "ghost-lead" } }] }))
        .await
        .unwrap_err();
    assert_eq!(e.code, "not_found", "{}", e.message);
    let e = set_seat(&state, &caller(), json!({ "nope": 1 })).await.unwrap_err();
    assert_eq!(e.code, "invalid_arg");
}

#[tokio::test]
async fn a_stopped_agent_is_not_stopped_again_and_a_stop_is_never_the_remote_agents_own() {
    let state = test_state();
    let target = agent(&state, "host");
    let out = stop(&state, &caller(), json!({ "agent": target.slug })).await.unwrap();
    assert_eq!(out["stopped"], false, "{out}");
    assert_eq!(out["agent_id"], target.id.as_str());
}
