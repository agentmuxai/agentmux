// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A login that expired (`login_expiry.rs`): its agents move to a valid login by themselves, the
//! one that was mid-turn is told to carry on, and the person hears of it once.

use super::*;
use crate::backend::reactive::held_notes;

fn now() -> u64 {
    agentmux_common::time::now_ms_u64()
}

/// The tests share the process's record of logins told about.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    let guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    login_expiry::TOLD.lock().unwrap_or_else(|e| e.into_inner()).clear();
    guard
}

fn held_for(block: &str) -> Vec<String> {
    held_notes::attach(block, "").1.note_texts()
}

/// `ag` ran on `a`, a login that is lost; `others` are the accounts it could go to. With `failed` its
/// pane's turn died on a refused sign-in (which is what makes an `expired` login lost); without, the
/// provider refused the login itself (`needs_reauth`).
async fn on_an_expired_login(state: &AppState, others: Vec<IdentityAccount>, failed: bool) -> String {
    let status = if failed { "expired" } else { "needs_reauth" };
    for a in std::iter::once(mail("a", status, "me@x.com")).chain(others) {
        state.id_store.identity_upsert(&a).unwrap();
    }
    agent_def(state, "ag");
    state.identity_store.agent_identity_link("ag", "a", "claude").unwrap();
    let tab = new_tab(state).await;
    pane(state, &tab, "ag", "Ag", "a", failed.then(|| failure("auth", None, None)), false).await
}

/// A valid login of the same account wins over an older account of another one, and the
/// agent whose turn died there is cleared of the failure.
#[tokio::test]
async fn the_agents_move_to_a_valid_login_of_the_same_account_first() {
    let _serial = serial();
    let state = test_state();
    // `aa` is older by id than `b` and has quota, but it is another account.
    let block = on_an_expired_login(&state, vec![mail("aa", "valid", "other@x.com"), mail("b", "valid", "me@x.com")], true).await;

    login_expiry::run(&state, &scan(&state, now()));

    assert_eq!(linked_account(&state, "ag"), ["b"]);
    assert_eq!(stored_failure(&state, &block), None, "the refused sign-in of the old login is cleared with the move");
    cleanup(&[&block]);
}

/// With no other login of the account, another signed-in account of the provider takes them.
#[tokio::test]
async fn else_to_another_signed_in_account_that_can_run() {
    let _serial = serial();
    let state = test_state();
    let block = on_an_expired_login(&state, vec![mail("gone", "needs_reauth", "gone@x.com"), mail("c", "valid", "c@x.com")], false).await;

    login_expiry::run(&state, &scan(&state, now()));

    assert_eq!(linked_account(&state, "ag"), ["c"]);
    cleanup(&[&block]);
}

/// A marked account, an unchecked one, a signed-out one and one a pane found out of quota are never
/// chosen, so the agent stays and nobody is told.
#[tokio::test]
async fn it_never_picks_a_marked_unchecked_signed_out_or_blocked_login() {
    let _serial = serial();
    let state = test_state();
    let mut marked = mail("no", "valid", "me@x.com");
    marked.context["never_failover"] = json!(true);
    let block = on_an_expired_login(&state, vec![marked, mail("new", "unknown", "me@x.com"), mail("dead", "expired", "me@x.com"), mail("full", "valid", "full@x.com")], false).await;
    let tab = new_tab(&state).await;
    let full = pane(&state, &tab, "other", "Other", "full", Some(failure("usage_limit", Some(in_an_hour()), None)), false).await;

    login_expiry::run(&state, &scan(&state, now()));

    assert_eq!(linked_account(&state, "ag"), ["a"]);
    assert!(login_expiry::rows(&state, &[]).is_empty(), "nothing moved, so the episode's own notice stands");
    cleanup(&[&block, &full]);
}

/// A valid login with a refused sign-in on one pane is not an expired login: nothing moves.
#[tokio::test]
async fn a_valid_login_is_left_alone() {
    let _serial = serial();
    let state = test_state();
    for a in [mail("a", "valid", "me@x.com"), mail("b", "valid", "me@x.com")] {
        state.id_store.identity_upsert(&a).unwrap();
    }
    agent_def(&state, "ag");
    state.identity_store.agent_identity_link("ag", "a", "claude").unwrap();
    let tab = new_tab(&state).await;
    let block = pane(&state, &tab, "ag", "Ag", "a", Some(failure("auth", None, None)), false).await;

    login_expiry::run(&state, &scan(&state, now()));

    assert_eq!(linked_account(&state, "ag"), ["a"]);
    cleanup(&[&block]);
}

/// The setting turns it off.
#[tokio::test]
async fn the_setting_turns_it_off() {
    let _serial = serial();
    let state = test_state();
    let block = on_an_expired_login(&state, vec![mail("b", "valid", "me@x.com")], false).await;
    let set = |on: bool| {
        let mut settings = state.config_watcher.get_settings();
        settings.extra.insert(failover::SETTING.to_string(), json!(on));
        state.config_watcher.update_settings(settings);
    };
    set(false);
    login_expiry::run(&state, &scan(&state, now()));
    assert_eq!(linked_account(&state, "ag"), ["a"]);
    set(true);
    login_expiry::run(&state, &scan(&state, now()));
    assert_eq!(linked_account(&state, "ag"), ["b"]);
    cleanup(&[&block]);
}

/// The interrupted pane gets the note that starts its turn (not the held one); an agent that was
/// idle gets the one-line held note. Neither is told twice.
#[tokio::test]
async fn the_agent_that_was_cut_off_is_told_to_carry_on_and_the_idle_one_gets_a_line() {
    let _serial = serial();
    let state = test_state();
    let cut_off = on_an_expired_login(&state, vec![mail("b", "valid", "me@x.com")], true).await;
    agent_def(&state, "idle");
    state.identity_store.agent_identity_link("idle", "a", "claude").unwrap();
    let tab = new_tab(&state).await;
    let idle = pane(&state, &tab, "idle", "Idle", "a", None, false).await;

    login_expiry::run(&state, &scan(&state, now()));

    assert_eq!(linked_account(&state, "ag"), ["b"]);
    assert_eq!(linked_account(&state, "idle"), ["b"]);
    assert!(held_for(&cut_off).is_empty(), "its note is the turn it starts, not a held line");
    let notes = held_for(&idle);
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].starts_with("[AgentMux] Your login expired") && notes[0].contains("me@x.com"), "{notes:?}");
    let resume = login_expiry::resume_note("me@x.com ·2");
    assert!(resume.contains("moved you to") && resume.contains("check git status and your PR first"), "{resume}");
    cleanup(&[&cut_off, &idle]);
}

/// The person is told once per login: a row in the strip with no agents (they moved), until the
/// login is signed in again.
#[tokio::test]
async fn the_person_is_told_once_until_the_login_is_signed_in_again() {
    let _serial = serial();
    let state = test_state();
    let block = on_an_expired_login(&state, vec![mail("b", "valid", "me@x.com")], true).await;

    login_expiry::run(&state, &scan(&state, now()));
    login_expiry::run(&state, &scan(&state, now()));

    let rows = login_expiry::rows(&state, &[]);
    assert_eq!(rows.len(), 1);
    assert_eq!((rows[0].account_id.as_str(), rows[0].state.as_str()), ("a", "signed_out"));
    assert!(rows[0].agents.is_empty() && rows[0].block_ids.is_empty() && rows[0].candidates.is_empty());
    assert!(login_expiry::rows(&state, &["a".to_string()]).is_empty(), "an open episode for it shows it already");

    // Signed in again: the row ends.
    state.id_store.identity_set_status("a", "valid").unwrap();
    login_expiry::run(&state, &scan(&state, now()));
    assert!(login_expiry::rows(&state, &[]).is_empty());
    assert!(login_expiry::TOLD.lock().unwrap().is_empty());
    cleanup(&[&block]);
}

/// A login with the choice made on the pure side: the same account wins, then the oldest.
#[test]
fn the_choice_prefers_the_same_account_then_the_oldest() {
    let from = mail("a", "expired", "me@x.com");
    let mut old = mail("zz", "valid", "other@x.com");
    old.created_at = 1;
    let mut same = mail("b", "valid", "me@x.com");
    same.created_at = 9;
    let all = [from.clone(), old.clone(), same.clone()];
    let none = HashMap::new();
    let never = |_: &str| false;
    assert_eq!(login_expiry::choose(&all, &none, &never, &from).map(|a| a.id.as_str()), Some("b"));
    // The same account is blocked: the other account.
    let blocked = HashMap::from([("b".to_string(), "out of quota".to_string())]);
    assert_eq!(login_expiry::choose(&all, &blocked, &never, &from).map(|a| a.id.as_str()), Some("zz"));
    // An account an agent was just moved off over its limit, which no pane has re-tested, is skipped too.
    let left = |id: &str| id == "b";
    assert_eq!(login_expiry::choose(&all, &none, &left, &from).map(|a| a.id.as_str()), Some("zz"));
    assert!(login_expiry::choose(&[from.clone()], &none, &never, &from).is_none());
}

/// An `expired` status alone can be an access token the CLI renews by itself: with no pane refused on
/// the login, the healthy agents on it stay and nobody is told.
#[tokio::test]
async fn a_renewable_expiry_with_no_refused_pane_moves_nobody() {
    let _serial = serial();
    let state = test_state();
    for a in [mail("a", "expired", "me@x.com"), mail("b", "valid", "me@x.com")] {
        state.id_store.identity_upsert(&a).unwrap();
    }
    agent_def(&state, "ag");
    state.identity_store.agent_identity_link("ag", "a", "claude").unwrap();
    let tab = new_tab(&state).await;
    let block = pane(&state, &tab, "ag", "Ag", "a", None, false).await;

    login_expiry::run(&state, &scan(&state, now()));

    assert_eq!(linked_account(&state, "ag"), ["a"]);
    assert!(login_expiry::rows(&state, &[]).is_empty());
    cleanup(&[&block]);
}

/// An agent with no links of its own runs on its seeded template's account: it moves too, though only the
/// template is linked.
#[tokio::test]
async fn an_agent_that_inherits_the_expired_login_from_its_template_moves() {
    let _serial = serial();
    let state = test_state();
    for a in [mail("a", "needs_reauth", "me@x.com"), mail("b", "valid", "me@x.com")] {
        state.id_store.identity_upsert(&a).unwrap();
    }
    let mut template = crate::backend::storage::agents::test_agent_def("tpl", "Template", "claude", "host", 1, "");
    template.is_seeded = 1;
    state.mstore.agent_def_insert(&mut template).unwrap();
    state.identity_store.agent_identity_link("tpl", "a", "claude").unwrap();
    let mut child = crate::backend::storage::agents::test_agent_def("kid", "Kid", "claude", "host", 1, "");
    child.parent_id = "tpl".to_string();
    state.mstore.agent_def_insert(&mut child).unwrap();
    let tab = new_tab(&state).await;
    let block = pane(&state, &tab, "kid", "Kid", "a", None, false).await;

    login_expiry::run(&state, &scan(&state, now()));

    assert_eq!(linked_account(&state, "kid"), ["b"]);
    cleanup(&[&block]);
}
