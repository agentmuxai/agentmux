// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use serde_json::json;

use super::super::{lookup, Verification};
use super::*;
use crate::backend::google::Service;
use crate::backend::storage::google_grants::GrantLevel;
use crate::server::tests::test_state;

fn caller() -> RemoteCaller {
    RemoteCaller {
        agent_uid: "uid-claimed-by-the-requester".into(),
        slug: "naki-2".into(),
        machine: "narko".into(),
        instance_id: "inst-1".into(),
        install_trusted: true,
        verification: Verification::Lan,
        reason: "Gmail for the agent here".into(),
    }
}

/// One Google account, connected with Calendar writes and Gmail reads, and an agent `Peer` on this machine.
fn setup() -> AppState {
    let state = test_state();
    let mut peer = crate::backend::storage::agents::test_agent_def("def-2", "Peer", "claude", "host", 1, "");
    state.mstore.agent_def_insert(&mut peer).unwrap();
    let mut scopes = json!(Service::Calendar.scopes(GrantLevel::Write)).as_array().unwrap().clone();
    scopes.extend(json!(Service::Gmail.scopes(GrantLevel::Read)).as_array().unwrap().clone());
    let mut acct = crate::identity::oauth_tokens::tests::oauth_account("acc-1", "google", "cid");
    acct.context["email"] = json!("me@example.com");
    acct.context["granted_scopes"] = json!(scopes);
    state.identity_store.identity_upsert(&acct).unwrap();
    state
}

#[test]
fn the_listing_and_the_grant_both_need_an_approved_install() {
    let (list, grant) = (lookup("google.status").unwrap(), lookup("google.grant").unwrap());
    assert!(!list.changes_state && list.needs_approved_install, "account emails are personal data: a first-seen pin is not enough");
    assert!(grant.changes_state && grant.needs_approved_install);
}

#[tokio::test]
async fn the_status_lists_accounts_and_holders_and_no_level_of_the_caller() {
    let state = setup();
    state.identity_store.google_grant_set("def-2", "acc-1", "calendar", Some(GrantLevel::Read), 1).unwrap();
    let answer = status(&state, &caller(), json!({})).await.unwrap();
    assert!(answer.get("you").is_none(), "{answer}");
    let account = &answer["accounts"][0];
    assert_eq!(account["account"], "me@example.com");
    let calendar = &account["services"]["calendar"];
    assert_eq!((calendar["connected_for"].as_str(), calendar["holders"][0]["agent"].as_str()), (Some("write"), Some("Peer")));
    assert!(calendar.get("yours").is_none(), "a caller from another machine has no agent here: {calendar}");
    assert!(!answer.to_string().contains("token"), "no secret: {answer}");
}

#[tokio::test]
async fn a_grant_goes_to_the_named_agent_and_is_recorded_as_the_remote_agent() {
    let state = setup();
    let set = |level: &str| json!({ "account": "ME@example.com", "agent": "peer", "service": "gmail", "level": level });
    let done = grant(&state, &caller(), set("read")).await.unwrap();
    assert_eq!(done, json!({ "account": "me@example.com", "agent": "Peer", "service": "gmail", "level": "read" }));
    assert_eq!(state.identity_store.google_grant_get("def-2", "acc-1", "gmail").unwrap(), Some(GrantLevel::Read));
    let calls = state.identity_store.google_calls("acc-1", 10).unwrap();
    let line = calls.iter().find(|c| c.action == "grant").expect("the grant is logged");
    assert_eq!((line.agent_name.as_str(), line.ids.as_str()), ("naki-2@narko", "Peer:read"), "never the signed agent_uid, never the operator");
    // Off takes it away again.
    grant(&state, &caller(), set("off")).await.unwrap();
    assert_eq!(state.identity_store.google_grant_get("def-2", "acc-1", "gmail").unwrap(), None);
}

#[tokio::test]
async fn a_grant_with_no_agent_is_refused_because_there_is_no_yourself_across_machines() {
    let state = setup();
    let refusal = grant(&state, &caller(), json!({ "account": "me@example.com", "service": "gmail", "level": "read" })).await.unwrap_err();
    assert_eq!(refusal.code, "bad_params");
    assert!(refusal.message.contains("yourself"), "{}", refusal.message);
    let blank = grant(&state, &caller(), json!({ "account": "me@example.com", "agent": "  ", "service": "gmail", "level": "read" })).await.unwrap_err();
    assert_eq!(blank.code, "bad_params");
    assert!(state.identity_store.google_grants_for_agent("def-2").unwrap().is_empty());
}

#[tokio::test]
async fn a_grant_the_consent_cannot_back_fails_with_what_to_do() {
    let state = setup();
    // Gmail was connected for reads only.
    let refusal = grant(&state, &caller(), json!({ "account": "me@example.com", "agent": "Peer", "service": "gmail", "level": "write" })).await.unwrap_err();
    assert_eq!(refusal.code, "failed");
    assert!(refusal.message.contains("reconnects it"), "{}", refusal.message);
    let unknown = grant(&state, &caller(), json!({ "account": "me@example.com", "agent": "ghost", "service": "gmail", "level": "read" })).await.unwrap_err();
    assert!(unknown.message.contains("no agent"), "{}", unknown.message);
    let malformed = grant(&state, &caller(), json!({ "agent": "Peer", "service": "gmail" })).await.unwrap_err();
    assert_eq!(malformed.code, "bad_params");
    assert!(state.identity_store.google_grants_for_agent("def-2").unwrap().is_empty());
}
