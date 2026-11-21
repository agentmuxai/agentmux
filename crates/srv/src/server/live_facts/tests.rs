// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;

use serde_json::{json, Value};

use super::query::{apply, status, Params};
use super::*;
use crate::backend::agent_runtime::agent_runtime_event;
use crate::server::tests::test_state;

#[test]
fn a_list_asks_for_groups_by_name_by_detail_or_for_all() {
    assert!(!Select::parse(None, None).unwrap().any(), "a default call asks for no group");
    let one = Select::parse(Some("model, context"), None).unwrap();
    assert!(one.model && one.context && !one.turn && !one.host);
    let facts = Select::parse(None, Some("facts")).unwrap();
    assert!(facts.turn && facts.usage && !facts.host, "`facts` is every group but the host's");
    assert!(Select::parse(None, Some("full")).unwrap().host);
    assert!(Select::parse(Some("all"), None).unwrap().host);
    let e = Select::parse(Some("model,nope"), None).unwrap_err();
    assert!(e.contains("nope") && e.contains("session") && e.contains("or all"), "{e}");
}

/// A discovery row, as `handle_discovery` builds it, for an agent whose pane is `block`.
fn row(uid: &str, name: &str, block: &str, state: &str, runtime: &str) -> Value {
    json!({
        "name": name, "id": uid, "seat": null,
        "runtime": { "agent_id": uid, "state": state, "block_id": block, "name": name, "provider": "claude", "runtime": runtime },
    })
}

/// Two panes on two models (what the CLI reported), as a fake runtime.
fn two_models() -> (Vec<Value>, Vec<String>) {
    let tag = uuid::Uuid::new_v4().to_string();
    let (b1, b2, b3) = (format!("lf-{tag}-1"), format!("lf-{tag}-2"), format!("lf-{tag}-3"));
    for (block, model) in [(&b1, "claude-sonnet-5-5"), (&b2, "claude-opus-5-5")] {
        let mut e = agent_runtime_event(block, None, false, None);
        e.running = true;
        e.effective_model = Some(model.to_string());
        e.effective_effort = Some("high".to_string());
        crate::backend::agent_runtime_latest::record(&e);
    }
    let rows = vec![
        row(&format!("u1-{tag}"), "OnSonnet", &b1, "idle", "host"),
        row(&format!("u2-{tag}"), "OnOpus", &b2, "working", "host"),
        row(&format!("u3-{tag}"), "InContainer", &b3, "idle", "container"),
    ];
    (rows, vec![b1, b2, b3])
}

fn params(pairs: &[(&str, &str)]) -> Params {
    let map: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    Params::from_map(&map, "this-box").unwrap()
}

fn names(rows: &[Value]) -> Vec<String> {
    rows.iter().map(|r| r["name"].as_str().unwrap().to_string()).collect()
}

#[tokio::test]
async fn a_default_row_keeps_what_it_had_and_gains_only_the_short_model() {
    let state = test_state();
    let (rows, blocks) = two_models();
    let before = rows[0].clone();
    let (out, page) = apply(&state, rows, &params(&[]), 1_000);
    assert_eq!(out.len(), 3, "a default call keeps every agent");
    let first = out[0].as_object().unwrap();
    for (key, value) in before.as_object().unwrap() {
        assert_eq!(first.get(key), Some(value), "{key} is unchanged");
    }
    assert_eq!((first["model_label"].as_str(), first["model_source"].as_str()), (Some("Sonnet 5.5"), Some("effective")));
    assert!(!first.contains_key("facts"), "no group unless asked for");
    assert_eq!(out[2]["model_source"], "unknown", "an agent whose harness says nothing is unknown, not guessed");
    assert_eq!((page["total"].as_u64(), page["returned"].as_u64(), page["truncated"].as_bool()), (Some(3), Some(3), Some(false)));
    blocks.iter().for_each(|b| crate::backend::agent_runtime_latest::forget(b));
}

#[tokio::test]
async fn not_sonnet_finds_the_agents_on_another_model_and_those_that_cannot_say() {
    let state = test_state();
    let (rows, blocks) = two_models();
    let (out, page) = apply(&state, rows.clone(), &params(&[("model", "!sonnet")]), 1_000);
    assert_eq!(names(&out), ["OnOpus", "InContainer"], "the one on Opus, and the one nothing says is on Sonnet");
    assert_eq!(page["total"], 2);
    let (only, _) = apply(&state, rows.clone(), &params(&[("model", "opus")]), 1_000);
    assert_eq!(names(&only), ["OnOpus"]);
    let (working, _) = apply(&state, rows.clone(), &params(&[("state", "working")]), 1_000);
    assert_eq!(names(&working), ["OnOpus"]);
    let (paged, page) = apply(&state, rows, &params(&[("limit", "1"), ("offset", "1")]), 1_000);
    assert_eq!(names(&paged), ["OnOpus"]);
    assert_eq!((page["total"].as_u64(), page["returned"].as_u64(), page["offset"].as_u64()), (Some(3), Some(1), Some(1)));
    blocks.iter().for_each(|b| crate::backend::agent_runtime_latest::forget(b));
}

#[tokio::test]
async fn the_groups_asked_for_are_read_for_the_page_and_a_container_agent_says_so() {
    let state = test_state();
    let (rows, blocks) = two_models();
    let (out, _) = apply(&state, rows, &params(&[("fields", "model,runtime,host"), ("runtime", "container")]), 1_000);
    assert_eq!(names(&out), ["InContainer"]);
    let facts = &out[0]["facts"];
    assert_eq!(facts["runtime"]["kind"], "container");
    assert_eq!(facts["runtime"]["harness"], "claude");
    assert_eq!(facts["model"]["source"], "unknown");
    assert!(facts["host"]["version"].is_string() && facts["host"]["name"].is_string());
    assert!(facts.get("turn").is_none() && facts.get("context").is_none(), "only the groups named");
    blocks.iter().for_each(|b| crate::backend::agent_runtime_latest::forget(b));
}

#[tokio::test]
async fn the_compact_status_has_a_line_per_agent_counts_and_groups() {
    let state = test_state();
    let (rows, blocks) = two_models();
    let out = status(&state, rows, &params(&[("group_by", "model")]), 1_000);
    let lines: Vec<&str> = out["lines"].as_array().unwrap().iter().map(|l| l.as_str().unwrap()).collect();
    assert_eq!(lines.len(), 3);
    assert!(lines[0].starts_with("OnSonnet | idle | Sonnet 5.5 high"), "{}", lines[0]);
    assert!(lines[1].starts_with("OnOpus | working | Opus 5.5 high"), "{}", lines[1]);
    assert_eq!(out["counts"]["by_state"], json!({ "idle": 2, "working": 1 }));
    assert_eq!(out["counts"]["by_model"]["Sonnet 5.5"], 1);
    assert_eq!(out["counts"]["by_model"]["unknown"], 1);
    let groups = out["groups"].as_array().unwrap();
    assert_eq!(groups.iter().map(|g| g["key"].as_str().unwrap()).collect::<Vec<_>>(), ["Opus 5.5", "Sonnet 5.5", "unknown"]);
    blocks.iter().for_each(|b| crate::backend::agent_runtime_latest::forget(b));
}

#[test]
fn an_account_is_named_by_its_display_name_or_name_and_never_by_an_email() {
    use crate::backend::storage::identities::{IdentityAccount, SecretRef};
    let account = |display: &str, name: &str| IdentityAccount {
        id: "a".into(),
        name: name.into(),
        provider: "claude".into(),
        kind: "oauth".into(),
        display_name: display.into(),
        secret_ref: SecretRef::OAuthConfigDir { dir: String::new() },
        context: json!({ "email": "person@example.com" }),
        status: "valid".into(),
        created_at: 0,
        updated_at: 0,
    };
    assert_eq!(safe_label(&account("Work", "w")), "Work");
    assert_eq!(safe_label(&account("", "Home")), "Home");
    assert_eq!(safe_label(&account("", "person@example.com")), "claude account", "a name that is an email is not shown");
    assert_eq!(safe_label(&account("person@example.com", "")), "claude account", "nor a display name that is one");
    assert_eq!(safe_label(&account("", "")), "claude account", "and the email in its context is never read");
}

#[tokio::test]
async fn no_fact_carries_a_credential_a_prompt_or_a_message() {
    let state = test_state();
    let (rows, blocks) = two_models();
    let (out, _) = apply(&state, rows, &params(&[("detail", "full")]), 1_000);
    let text = serde_json::to_string(&out).unwrap().to_lowercase();
    for forbidden in ["access_token", "refresh_token", "secret", "password", "api_key", "apikey", "@example", "prompt", "bearer"] {
        assert!(!text.contains(forbidden), "a row must not carry {forbidden:?}: {text}");
    }
    blocks.iter().for_each(|b| crate::backend::agent_runtime_latest::forget(b));
}

/// A model switch asked for while a turn runs is carried by the agent's model facts until it applies: the process
/// is still on the old model (the runtime event's `model` and the CLI's report), the pane's meta asks for the new
/// one, and the event says a restart is waiting. Once the replacement runs, nothing is pending.
#[tokio::test]
async fn a_model_switch_asked_for_during_a_turn_is_pending_on_the_agents_model_until_it_applies() {
    let state = test_state();
    let block_id = format!("lf-pending-{}", uuid::Uuid::new_v4());
    let mut block = crate::backend::obj::Block { oid: block_id.clone(), ..Default::default() };
    block.meta = serde_json::from_value(json!({ "agent:runtime": { "model": "opus", "effort": "high" } })).unwrap();
    let event = |restart: bool, model: &str, effective: &str| {
        let mut e = agent_runtime_event(&block_id, None, restart, None);
        e.running = true;
        e.model = Some(model.to_string());
        e.effective_model = Some(effective.to_string());
        crate::backend::agent_runtime_latest::record(&e);
    };
    let ctx = Ctx::new(&state, 1_000, &Select::default(), false);
    let agent = Agent { uid: "u-pending", name: "Switching", provider: "claude", block_id: Some(&block_id), state: "working", container: false };

    event(true, "sonnet", "claude-sonnet-5-5");
    let mid_turn = model_of(&ctx, &agent, Some(&block));
    assert_eq!(mid_turn.label.as_deref(), Some("Sonnet 5.5"), "it still runs the old model");
    assert!(mid_turn.pending_switch);
    assert_eq!(mid_turn.pending_label.as_deref(), Some("Opus"), "and the row says which model it is switching to");

    event(false, "opus", "claude-opus-5-5");
    let applied = model_of(&ctx, &agent, Some(&block));
    assert_eq!(applied.label.as_deref(), Some("Opus 5.5"));
    assert!(!applied.pending_switch && applied.pending_label.is_none());
    crate::backend::agent_runtime_latest::forget(&block_id);
}
