// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The quiet recheck of a held message's spawn gate makes the same decision as the spawn: an unlinked Claude
//! agent that was refused (no account) and then got a model endpoint of its own launches, so what was held
//! for it is released; without the endpoint it stays held.

use super::*;
use crate::backend::storage::agents::{test_agent_def, AgentInstance};
use crate::server::tests::test_state;

/// A state with an unlinked Claude agent on a pane whose `cmd:env` is `env`, refused by the gate some time ago.
fn refused_agent(env: serde_json::Value) -> (AppState, String) {
    let mut state = test_state();
    state.reactive_handler = Box::leak(Box::new(crate::backend::reactive::handler::ReactiveHandler::new()));
    let slug = format!("gate-{}", &uuid::Uuid::new_v4().to_string()[..8]);
    let uid = format!("uid-{slug}");
    let mut def = test_agent_def(&uid, "Gate Y", "claude", "agent", 1, "");
    def.slug = slug.clone();
    state.mstore.agent_def_insert(&mut def).unwrap();
    let mut block = Block { oid: uuid::Uuid::new_v4().to_string(), ..Default::default() };
    block.meta.insert("view".to_string(), serde_json::json!("agent"));
    block.meta.insert("agentId".to_string(), serde_json::json!(uid));
    block.meta.insert("cmd:env".to_string(), env);
    state.mstore.insert(&mut block).unwrap();
    state
        .mstore
        .instance_create(&AgentInstance {
            id: format!("inst-{slug}"),
            definition_id: uid.clone(),
            parent_instance_id: String::new(),
            block_id: block.oid.clone(),
            session_id: String::new(),
            status: "running".to_string(),
            github_context: String::new(),
            started_at: 1000,
            ended_at: 0,
            created_at: 1000,
            identity_id: String::new(),
            memory_id: String::new(),
            instance_name: String::new(),
            working_directory: String::new(),
            display_hidden: false,
        })
        .unwrap();
    state.reactive_handler.register_agent_full(&slug, &block.oid, None, 0, None, Some(&uid), "test").unwrap();
    // Refused, and the last check is older than the recheck interval, so the next replay runs the gate.
    let long_ago = std::time::Instant::now() - GATE_RECHECK_EVERY - Duration::from_secs(1);
    needs_login().lock().unwrap().insert(uid.clone(), long_ago);
    (state, uid)
}

#[tokio::test]
async fn an_unlinked_claude_agent_with_its_own_endpoint_is_released() {
    let (state, uid) = refused_agent(serde_json::json!({ "ANTHROPIC_BASE_URL": "http://127.0.0.1:9/v1", "ANTHROPIC_AUTH_TOKEN": "none" }));
    assert!(!still_needs_login(&state, &uid).await, "the spawn would pass, so the held message goes");
    assert!(!needs_login().lock().unwrap().contains_key(&uid), "and the target is no longer marked");
}

#[tokio::test]
async fn an_unlinked_claude_agent_without_an_endpoint_stays_held() {
    let (state, uid) = refused_agent(serde_json::json!({}));
    assert!(still_needs_login(&state, &uid).await, "no account and no endpoint: still refused");
    assert!(needs_login().lock().unwrap().contains_key(&uid), "and marked again for the next minute");
}

#[tokio::test]
async fn anthropics_own_address_is_not_an_endpoint_of_its_own() {
    let (state, uid) = refused_agent(serde_json::json!({ "ANTHROPIC_BASE_URL": "https://api.anthropic.com" }));
    assert!(still_needs_login(&state, &uid).await, "the person's account is still needed on Anthropic's address");
}
