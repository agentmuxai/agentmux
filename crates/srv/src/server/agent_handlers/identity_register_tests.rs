// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What the identity and named-agent RPCs answer, through the engine that routes them.

use std::sync::Arc;

use serde_json::json;

use super::identity::register;
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::*;
use crate::backend::storage::store::AgentInstance;
use crate::server::tests::test_state;
use crate::server::AppState;

struct Harness {
    state: AppState,
    engine: Arc<WshRpcEngine>,
    rx: tokio::sync::mpsc::UnboundedReceiver<RpcMessage>,
}

fn harness() -> Harness {
    let state = test_state();
    let (engine, rx) = WshRpcEngine::new();
    register(&engine, &state);
    Harness { state, engine, rx }
}

impl Harness {
    async fn call(&mut self, command: &str, data: serde_json::Value) -> Result<serde_json::Value, String> {
        self.engine.handle_message(RpcMessage {
            command: command.to_string(),
            reqid: "req-1".to_string(),
            data: Some(data),
            ..Default::default()
        });
        let resp = tokio::time::timeout(std::time::Duration::from_secs(10), self.rx.recv()).await.unwrap().unwrap();
        if resp.error.is_empty() {
            Ok(resp.data.unwrap_or_default())
        } else {
            Err(resp.error)
        }
    }

    fn agent(&self, id: &str, name: &str) {
        let mut def = crate::backend::storage::agents::test_agent_def(id, name, "claude", "host", 1, "");
        self.state.mstore.agent_def_insert(&mut def).unwrap();
    }

    fn named_instance(&self, id: &str, definition_id: &str, name: &str, started_at: i64) {
        let inst = AgentInstance {
            id: id.to_string(),
            definition_id: definition_id.to_string(),
            parent_instance_id: String::new(),
            block_id: format!("block-{id}"),
            session_id: String::new(),
            status: "running".to_string(),
            github_context: String::new(),
            started_at,
            ended_at: 0,
            created_at: started_at,
            identity_id: String::new(),
            memory_id: String::new(),
            instance_name: name.to_string(),
            working_directory: format!("/work/{name}"),
            display_hidden: false,
        };
        self.state.mstore.instance_create(&inst).unwrap();
    }
}

fn key_account(name: &str, provider: &str) -> serde_json::Value {
    json!({
        "id": "",
        "name": name,
        "provider": provider,
        "kind": "api_key",
        "secret_ref": { "backend": "env", "env_var": "SOME_KEY" },
    })
}

#[tokio::test]
async fn an_upserted_account_gets_an_id_and_times_and_is_listed_and_fetched() {
    let mut h = harness();
    let before = agentmux_common::time::now_ms();
    let saved = h.call(COMMAND_UPSERT_IDENTITY_ACCOUNT, key_account("work", "anthropic")).await.unwrap();
    let id = saved["id"].as_str().unwrap().to_string();
    assert!(!id.is_empty(), "an empty id is minted");
    assert!(saved["created_at"].as_i64().unwrap() >= before);
    assert_eq!(saved["created_at"], saved["updated_at"]);

    let got = h.call(COMMAND_GET_IDENTITY_ACCOUNT, json!({ "id": id })).await.unwrap();
    assert_eq!(got["name"], "work");

    h.call(COMMAND_UPSERT_IDENTITY_ACCOUNT, key_account("other", "openai")).await.unwrap();
    let all = h.call(COMMAND_LIST_IDENTITY_ACCOUNTS, json!({})).await.unwrap();
    assert_eq!(all.as_array().unwrap().len(), 2);
    let only = h.call(COMMAND_LIST_IDENTITY_ACCOUNTS, json!({ "provider": "openai" })).await.unwrap();
    let only = only.as_array().unwrap();
    assert_eq!(only.len(), 1);
    assert_eq!(only[0]["name"], "other");
}

#[tokio::test]
async fn an_account_that_does_not_exist_is_not_found() {
    let mut h = harness();
    let err = h.call(COMMAND_GET_IDENTITY_ACCOUNT, json!({ "id": "ghost" })).await.unwrap_err();
    assert!(err.contains("getidentityaccount: not found id=ghost"), "{err}");
}

#[tokio::test]
async fn an_upsert_that_is_not_an_account_is_refused() {
    let mut h = harness();
    let err = h.call(COMMAND_UPSERT_IDENTITY_ACCOUNT, json!({ "name": 3 })).await.unwrap_err();
    assert!(err.starts_with("upsertidentityaccount:"), "{err}");
}

#[tokio::test]
async fn an_agent_is_linked_to_an_account_listed_and_unlinked() {
    let mut h = harness();
    let account = h.call(COMMAND_UPSERT_IDENTITY_ACCOUNT, key_account("work", "anthropic")).await.unwrap();
    let account_id = account["id"].as_str().unwrap().to_string();
    h.agent("a-1", "Loap");
    h.call(COMMAND_LINK_AGENT_IDENTITY, json!({ "agent_id": "a-1", "account_id": account_id, "provider": "anthropic" }))
        .await
        .unwrap();

    let mine = h.call(COMMAND_LIST_AGENT_IDENTITIES, json!({ "agent_id": "a-1" })).await.unwrap();
    assert_eq!(mine.as_array().unwrap().len(), 1);
    assert_eq!(mine[0]["account_id"], json!(account_id));
    let theirs = h.call(COMMAND_LIST_AGENT_IDENTITIES, json!({ "agent_id": "a-2" })).await.unwrap();
    assert!(theirs.as_array().unwrap().is_empty());
    let all = h.call(COMMAND_LIST_ALL_AGENT_IDENTITIES, json!({})).await.unwrap();
    assert_eq!(all.as_array().unwrap().len(), 1);

    let gone = h.call(COMMAND_UNLINK_AGENT_IDENTITY, json!({ "agent_id": "a-1", "provider": "anthropic" })).await.unwrap();
    assert_eq!(gone["unlinked"], true);
    let again = h.call(COMMAND_UNLINK_AGENT_IDENTITY, json!({ "agent_id": "a-1", "provider": "anthropic", "silent": true })).await.unwrap();
    assert_eq!(again["unlinked"], false, "nothing left to unlink");
    let all = h.call(COMMAND_LIST_ALL_AGENT_IDENTITIES, json!({})).await.unwrap();
    assert!(all.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn named_agents_list_newest_first_with_their_definition_and_can_be_filtered_and_hidden() {
    let mut h = harness();
    h.agent("def-1", "Loap");
    h.agent("def-2", "Korp");
    h.named_instance("inst-old", "def-1", "old one", 10);
    h.named_instance("inst-new", "def-2", "new one", 20);

    let rows = h.call(COMMAND_LIST_NAMED_AGENTS, json!({})).await.unwrap();
    let rows = rows.as_array().unwrap();
    let ids: Vec<&str> = rows.iter().map(|r| r["instance_id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["def-2", "def-1"], "a row is keyed by its agent");
    assert_eq!(rows[0]["definition_name"], "new one", "the instance row is the agent's own definition row");
    assert_eq!(rows[0]["provider"], "claude");
    assert_eq!(rows[0]["memory_name"], "(vanilla CLI)");
    assert_eq!(rows[0]["status"], "running");
    assert_eq!(rows[0]["definition_id"], "def-2");
    assert_eq!(rows[0]["working_directory"], "/work/new one");

    let one = h.call(COMMAND_LIST_NAMED_AGENTS, json!({ "limit": 1 })).await.unwrap();
    assert_eq!(one.as_array().unwrap().len(), 1);
    let of_def = h.call(COMMAND_LIST_NAMED_AGENTS, json!({ "definition_id": "def-1" })).await.unwrap();
    assert_eq!(of_def[0]["instance_id"], "def-1");
    assert_eq!(of_def.as_array().unwrap().len(), 1);
    let garbled = h.call(COMMAND_LIST_NAMED_AGENTS, json!("not an object")).await.unwrap();
    assert_eq!(garbled.as_array().unwrap().len(), 2, "a malformed request means the defaults");

    let hidden = h.call(COMMAND_HIDE_NAMED_AGENT, json!({ "id": "def-2" })).await.unwrap();
    assert_eq!(hidden["hidden"], true);
    let rows = h.call(COMMAND_LIST_NAMED_AGENTS, json!({})).await.unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 1);
    let missing = h.call(COMMAND_HIDE_NAMED_AGENT, json!({ "id": "inst-ghost" })).await.unwrap();
    assert_eq!(missing["hidden"], false);
}

#[tokio::test]
async fn a_named_agents_identity_name_is_its_linked_account() {
    let mut h = harness();
    h.agent("def-1", "Loap");
    h.named_instance("inst-1", "def-1", "one", 10);
    let account = h.call(COMMAND_UPSERT_IDENTITY_ACCOUNT, key_account("work-account", "claude")).await.unwrap();
    let account_id = account["id"].as_str().unwrap().to_string();
    let before = h.call(COMMAND_LIST_NAMED_AGENTS, json!({})).await.unwrap();
    assert!(!before[0]["identity_name"].as_str().unwrap().contains("work-account"), "{before}");
    h.call(COMMAND_LINK_AGENT_IDENTITY, json!({ "agent_id": "def-1", "account_id": account_id, "provider": "claude" })).await.unwrap();
    let after = h.call(COMMAND_LIST_NAMED_AGENTS, json!({})).await.unwrap();
    assert!(after[0]["identity_name"].as_str().unwrap().contains("work-account"), "{after}");
}

#[tokio::test]
async fn an_oauth_session_that_does_not_exist_cannot_be_polled_and_cancels_to_false() {
    let mut h = harness();
    let err = h.call(COMMAND_ACCOUNT_OAUTH_POLL, json!({ "sessionId": "nope" })).await.unwrap_err();
    assert!(err.contains("account.oauth.poll: unknown session nope"), "{err}");
    let cancelled = h.call(COMMAND_ACCOUNT_OAUTH_CANCEL, json!({ "sessionId": "nope" })).await.unwrap();
    assert_eq!(cancelled["cancelled"], false);
}

#[tokio::test]
async fn an_oauth_client_of_an_unknown_provider_is_refused() {
    let mut h = harness();
    let err = h.call(COMMAND_ACCOUNT_OAUTH_CLIENT_CLEAR, json!({ "provider": "nowhere" })).await.unwrap_err();
    assert!(err.contains("account.oauth.client.clear: unknown OAuth provider: nowhere"), "{err}");
    assert!(h.call(COMMAND_ACCOUNT_OAUTH_CLIENT_GET, json!({ "provider": "nowhere" })).await.is_err());
}

#[tokio::test]
async fn an_oauth_start_that_cannot_begin_answers_with_an_error_field_not_a_failure() {
    let mut h = harness();
    let got = h.call(COMMAND_ACCOUNT_OAUTH_START, json!({ "provider": "nowhere" })).await.unwrap();
    assert_eq!(got["error"], "unknown OAuth provider: nowhere");
    assert!(got["session_id"].is_null());
    let err = h.call(COMMAND_ACCOUNT_OAUTH_START, json!(7)).await.unwrap_err();
    assert!(err.starts_with("account.oauth.start:"), "{err}");
}

#[tokio::test]
async fn a_key_that_is_not_a_request_is_refused_before_anything_is_stored() {
    let mut h = harness();
    let err = h.call(COMMAND_ACCOUNT_KEY_VERIFY, json!(7)).await.unwrap_err();
    assert!(err.starts_with("account.key.verify:"), "{err}");
    let all = h.call(COMMAND_LIST_IDENTITY_ACCOUNTS, json!({})).await.unwrap();
    assert!(all.as_array().unwrap().is_empty());
}
