// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Failover (`failover.rs`): an agent out of quota moves itself to an account that can run.

use super::*;
use crate::backend::reactive::held_notes;

fn marked(mut a: IdentityAccount) -> IdentityAccount {
    a.context["never_failover"] = json!(true);
    a
}

fn now() -> u64 {
    agentmux_common::time::now_ms_u64()
}

/// The notes held for a pane's next turn.
fn held_for(block: &str) -> Vec<String> {
    held_notes::attach(block, "").1.note_texts()
}

fn set_failover_setting(state: &AppState, on: bool) {
    let mut settings = state.config_watcher.get_settings();
    settings.extra.insert(failover::SETTING.to_string(), json!(on));
    state.config_watcher.update_settings(settings);
}

/// `a` is the account a limited agent `ag` is on; `others` are the accounts it could go to.
async fn limited_agent(state: &AppState, others: Vec<IdentityAccount>) -> String {
    for a in std::iter::once(mail("a", "valid", "a@x.com")).chain(others) {
        state.id_store.identity_upsert(&a).unwrap();
    }
    agent_def(state, "ag");
    state.identity_store.agent_identity_link("ag", "a", "claude").unwrap();
    let tab = new_tab(state).await;
    pane(state, &tab, "ag", "Ag", "a", Some(failure("usage_limit", Some(in_an_hour()), Some("seven_day"))), false).await
}

/// A usage limit moves the agent to a signed-in account with quota, clears the limit it had, and
/// leaves one note for the agent's next turn; a signed-out account and a marked one are never chosen.
#[tokio::test]
async fn an_agent_out_of_quota_moves_to_an_account_that_can_run_and_is_told() {
    let state = test_state();
    let block = limited_agent(
        &state,
        vec![mail("gone", "expired", "gone@x.com"), marked(mail("no", "valid", "no@x.com")), mail("b", "valid", "b@x.com")],
    )
    .await;

    failover::run(&state, &scan(&state, now()));

    assert_eq!(linked_account(&state, "ag"), ["b"]);
    assert_eq!(stored_failure(&state, &block), None, "the limit of the old account is cleared with the move");
    let notes = held_for(&block);
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].starts_with("[AgentMux] ") && notes[0].contains("b@x.com") && notes[0].contains("usage limit"), "{notes:?}");
    cleanup(&[&block]);
}

/// With nothing but a signed-out account, a marked one and an unchecked one left, the agent stays.
#[tokio::test]
async fn it_never_moves_onto_a_signed_out_marked_or_unchecked_account() {
    let state = test_state();
    let block = limited_agent(
        &state,
        vec![mail("gone", "expired", "gone@x.com"), mail("reauth", "needs_reauth", "re@x.com"), marked(mail("no", "valid", "no@x.com")), mail("new", "unknown", "new@x.com")],
    )
    .await;

    failover::run(&state, &scan(&state, now()));

    assert_eq!(linked_account(&state, "ag"), ["a"]);
    assert!(stored_failure(&state, &block).is_some());
    assert!(held_for(&block).is_empty());
    cleanup(&[&block]);
}

/// A sign-in problem is not a limit: failover moves nothing for it.
#[tokio::test]
async fn a_refused_sign_in_is_not_failed_over() {
    let state = test_state();
    state.id_store.identity_upsert(&mail("a", "valid", "a@x.com")).unwrap();
    state.id_store.identity_upsert(&mail("b", "valid", "b@x.com")).unwrap();
    agent_def(&state, "ag");
    state.identity_store.agent_identity_link("ag", "a", "claude").unwrap();
    let tab = new_tab(&state).await;
    let block = pane(&state, &tab, "ag", "Ag", "a", Some(failure("auth", None, None)), false).await;

    failover::run(&state, &scan(&state, now()));

    assert_eq!(linked_account(&state, "ag"), ["a"]);
    cleanup(&[&block]);
}

/// The one setting turns it off.
#[tokio::test]
async fn the_setting_turns_failover_off() {
    let state = test_state();
    let block = limited_agent(&state, vec![mail("b", "valid", "b@x.com")]).await;
    set_failover_setting(&state, false);

    failover::run(&state, &scan(&state, now()));
    assert_eq!(linked_account(&state, "ag"), ["a"]);

    set_failover_setting(&state, true);
    failover::run(&state, &scan(&state, now()));
    assert_eq!(linked_account(&state, "ag"), ["b"]);
    cleanup(&[&block]);
}

/// A move the agent already has queued (a turn is running) is left alone, and lands with its note
/// when the turn ends.
#[tokio::test]
async fn a_busy_agent_is_moved_when_its_turn_ends() {
    let state = test_state();
    for a in [mail("a", "valid", "a@x.com"), mail("b", "valid", "b@x.com")] {
        state.id_store.identity_upsert(&a).unwrap();
    }
    agent_def(&state, "ag");
    state.identity_store.agent_identity_link("ag", "a", "claude").unwrap();
    let tab = new_tab(&state).await;
    let limited = pane(&state, &tab, "ag", "Ag", "a", Some(failure("usage_limit", Some(in_an_hour()), None)), false).await;
    let working = pane(&state, &tab, "ag", "Ag", "a", None, true).await;

    failover::run(&state, &scan(&state, now()));
    assert_eq!(linked_account(&state, "ag"), ["a"], "queued, not applied under a turn");
    assert_eq!(pending_of(&state, &limited), Some(json!("b")));

    // The next refresh sees the queue and doesn't queue it again.
    failover::run(&state, &scan(&state, now()));
    assert_eq!(pending_of(&state, &limited), Some(json!("b")));

    register_stub(&working, BLOCK_CONTROLLER_PERSISTENT, STATUS_RUNNING, false);
    switch::apply_pending(&state);
    assert_eq!(linked_account(&state, "ag"), ["b"]);
    assert_eq!(held_for(&limited).len(), 1, "the note comes with the move that lands");
    cleanup(&[&limited, &working]);
}

/// The lead of a worker that failed over hears of it in one line.
#[tokio::test]
async fn the_agents_lead_gets_one_line_too() {
    let state = test_state();
    let block = limited_agent(&state, vec![mail("b", "valid", "b@x.com")]).await;
    agent_def(&state, "boss");
    let lead = state.mstore.agent_def_list().unwrap().into_iter().find(|d| d.id == "boss").unwrap();
    state
        .mstore
        .agent_content_set(&crate::backend::storage::store::AgentContent {
            agent_id: "ag".to_string(),
            content_type: crate::server::app_api::agent_spawn::WORKER_OF_CONTENT.into(),
            content: json!({ "agent": lead.name, "id": lead.id }).to_string(),
            updated_at: 1,
        })
        .unwrap();
    let tab = new_tab(&state).await;
    let lead_pane = pane(&state, &tab, "boss", "Boss", "", None, false).await;

    failover::run(&state, &scan(&state, now()));

    let notes = held_for(&lead_pane);
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].contains("Ag") && notes[0].contains("b@x.com"), "{notes:?}");
    cleanup(&[&block, &lead_pane]);
}

/// A re-sign-in that rebuilds the account's context keeps the operator's mark.
#[tokio::test]
async fn a_rewritten_account_keeps_its_mark() {
    let state = test_state();
    state.id_store.identity_upsert(&mail("a", "valid", "a@x.com")).unwrap();
    state.id_store.identity_set_never_failover("a", true).unwrap();

    let mut rebuilt = mail("a", "valid", "a@x.com");
    rebuilt.context = json!({ "email": "a@x.com" });
    state.id_store.identity_upsert(&rebuilt).unwrap();
    assert!(failover::is_never(&state.id_store.identity_get("a").unwrap().unwrap()));

    // A write that says so changes it.
    rebuilt.context = json!({ "email": "a@x.com", "never_failover": false });
    state.id_store.identity_upsert(&rebuilt).unwrap();
    assert!(!failover::is_never(&state.id_store.identity_get("a").unwrap().unwrap()));
}

/// `identity.account.set_failover` marks and unmarks, `identity.accounts` shows the mark, and an
/// unknown account is refused.
#[tokio::test]
async fn the_mark_is_set_and_shown() {
    let state = test_state();
    state.id_store.identity_upsert(&mail("a", "valid", "a@x.com")).unwrap();
    let view = |s: &AppState| accounts::accounts(s, now()).unwrap().accounts.into_iter().find(|a| a.account_id == "a").unwrap();
    assert!(!view(&state).never_failover);

    let r = call(&state, None, COMMAND_IDENTITY_ACCOUNT_SET_FAILOVER, json!({ "account_id": "a", "allow": false })).await.unwrap();
    assert_eq!(r["allow"], json!(false));
    assert!(view(&state).never_failover);
    assert_eq!(state.id_store.identity_get("a").unwrap().unwrap().context["email"], json!("a@x.com"), "the rest of the context stands");

    call(&state, None, COMMAND_IDENTITY_ACCOUNT_SET_FAILOVER, json!({ "account_id": "a", "allow": true })).await.unwrap();
    assert!(!view(&state).never_failover);

    let e = call(&state, None, COMMAND_IDENTITY_ACCOUNT_SET_FAILOVER, json!({ "account_id": "nope", "allow": false })).await.unwrap_err();
    assert!(e.contains("no account"), "{e}");
}
