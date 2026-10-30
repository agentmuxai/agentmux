// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use serde_json::json;

use super::super::{lookup, Verification};
use super::*;
use crate::backend::storage::identities::{IdentityAccount, SecretRef};
use crate::server::tests::test_state;

fn caller() -> RemoteCaller {
    RemoteCaller {
        agent_uid: "uid-claimed-by-the-requester".into(),
        slug: "agenty".into(),
        machine: "narko".into(),
        instance_id: "inst-1".into(),
        install_trusted: true,
        verification: Verification::Lan,
        reason: "the account is out".into(),
    }
}

fn account(id: &str, email: &str) -> IdentityAccount {
    IdentityAccount {
        id: id.to_string(),
        name: format!("{id}-name"),
        provider: "claude".to_string(),
        kind: "oauth".to_string(),
        display_name: String::new(),
        secret_ref: SecretRef::OAuthConfigDir { dir: format!("/tmp/{id}") },
        context: json!({ "email": email }),
        status: "valid".to_string(),
        created_at: 0,
        updated_at: 0,
    }
}

/// Two accounts, and an agent `ag` linked to `a`.
fn setup() -> AppState {
    let state = test_state();
    for a in [account("a", "a@x.com"), account("b", "b@x.com")] {
        state.id_store.identity_upsert(&a).unwrap();
    }
    let mut def = crate::backend::storage::agents::test_agent_def("ag", "Ag", "claude", "host", 1, "");
    state.mstore.agent_def_insert(&mut def).unwrap();
    state.identity_store.agent_identity_link("ag", "a", "claude").unwrap();
    state
}

#[test]
fn listing_is_a_read_and_switching_a_write() {
    assert!(!lookup("identity.accounts").unwrap().changes_state);
    assert!(lookup("identity.switch").unwrap().changes_state, "moving an agent needs a certificate-backed caller");
}

#[tokio::test]
async fn the_listing_has_stable_id_quota_and_no_secret() {
    let state = setup();
    let answer = accounts(&state, &caller(), json!({})).await.unwrap();
    let list = answer["accounts"].as_array().unwrap();
    assert_eq!(list.len(), 2);
    let a = list.iter().find(|a| a["account_id"] == "a").unwrap();
    assert!(a["stable_id"].as_str().is_some_and(|s| s.starts_with("claude:")), "{a}");
    assert_eq!((a["quota"]["state"].as_str(), a["email"].as_str()), (Some("ok"), Some("a@x.com")));
    let text = answer.to_string();
    assert!(!text.contains("/tmp/") && !text.contains("secret"), "no config dir or secret reference: {text}");
}

#[tokio::test]
async fn a_switch_moves_the_agent_and_is_recorded_as_the_remote_agent() {
    let state = setup();
    let b_stable = accounts(&state, &caller(), json!({})).await.unwrap()["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["account_id"] == "b")
        .unwrap()["stable_id"]
        .as_str()
        .unwrap()
        .to_string();

    let answer = switch(&state, &caller(), json!({ "agent_id": "ag", "account": b_stable })).await.unwrap();
    assert_eq!((answer["status"].as_str(), answer["account_id"].as_str()), (Some("switched"), Some("b")));
    let links: Vec<String> = state.identity_store.agent_identity_list_for_agent("ag").unwrap().into_iter().map(|l| l.account_id).collect();
    assert_eq!(links, ["b"]);
}

#[tokio::test]
async fn any_with_quota_and_all_limited_work_remotely_and_a_refusal_lists_the_accounts() {
    let state = setup();
    let answer = switch(&state, &caller(), json!({ "agent": "ag", "account": "any with quota" })).await.unwrap();
    assert_eq!(answer["account_id"], "b");

    let none = switch(&state, &caller(), json!({ "agent_id": "all limited", "account": "any with quota" })).await.unwrap();
    assert_eq!(none["agents"], json!([]), "nobody is out of quota");

    let refused = switch(&state, &caller(), json!({ "agent_id": "ag", "account": "nobody@x.com" })).await.unwrap_err();
    assert!(refused.message.starts_with("agent.account.switch") || refused.message.contains("UNKNOWN_ACCOUNT"), "{refused:?}");
    assert!(refused.message.contains("a@x.com"), "{refused:?}");
}

#[tokio::test]
async fn a_switch_has_to_name_the_agent_and_the_account() {
    let state = setup();
    for params in [json!({ "account": "any with quota" }), json!({ "agent_id": " ", "account": "b" }), json!({ "agent_id": "ag", "account": "" })] {
        let refused = switch(&state, &caller(), params.clone()).await.unwrap_err();
        assert_eq!(refused.code, "bad_params", "{params}: {refused:?}");
    }
}
