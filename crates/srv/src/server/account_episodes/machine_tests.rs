// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;

fn pane(block: &str, name: &str, account: &str, state: AgentState, resets: Option<u64>, window: Option<&str>) -> BlockedPane {
    BlockedPane {
        block_id: block.to_string(),
        agent_name: name.to_string(),
        account_id: account.to_string(),
        provider: "claude".to_string(),
        state,
        resets_at_ms: resets,
        limit_window: window.map(str::to_string),
        signature: format!("{state:?}|{resets:?}"),
    }
}

fn weekly(block: &str, name: &str, account: &str) -> BlockedPane {
    pane(block, name, account, AgentState::Limited, Some(10_000), Some("seven_day"))
}

fn session(block: &str, name: &str, account: &str) -> BlockedPane {
    pane(block, name, account, AgentState::Limited, Some(5_000), Some(SESSION_WINDOW))
}

fn kinds(effects: &[Effect]) -> Vec<&'static str> {
    effects
        .iter()
        .map(|e| match e {
            Effect::Changed(_) => "changed",
            Effect::Toast(_) => "toast",
            Effect::Announce { .. } => "announce",
            Effect::Closed { .. } => "closed",
        })
        .collect()
}

#[test]
fn a_second_agent_on_the_account_joins_the_open_episode() {
    let mut eps = Episodes::default();
    let first = eps.update(vec![weekly("b1", "Naki-2", "acct")], 1);
    assert_eq!(kinds(&first), ["changed", "toast", "announce"]);
    let joined = eps.update(vec![weekly("b1", "Naki-2", "acct"), weekly("b2", "Loap-3", "acct")], 2);
    assert_eq!(kinds(&joined), ["changed", "announce"], "one toast per episode");
    let Effect::Announce { panes, .. } = &joined[1] else { panic!() };
    assert_eq!(panes.iter().map(|p| p.block_id.as_str()).collect::<Vec<_>>(), ["b2"], "only the newcomer is announced");
    assert_eq!(eps.list().count(), 1);
    let ep = eps.get("acct").unwrap();
    assert_eq!(ep.opened_at_ms, 1);
    assert_eq!(ep.panes.iter().map(|p| p.agent_name.as_str()).collect::<Vec<_>>(), ["Loap-3", "Naki-2"]);
    assert!(eps.update(vec![weekly("b1", "Naki-2", "acct"), weekly("b2", "Loap-3", "acct")], 3).is_empty(), "nothing new");
}

#[test]
fn the_episode_closes_when_no_pane_is_blocked_and_a_new_hit_opens_another() {
    let mut eps = Episodes::default();
    eps.update(vec![weekly("b1", "A", "acct"), weekly("b2", "B", "acct")], 1);
    let left = eps.update(vec![weekly("b2", "B", "acct")], 2);
    assert_eq!(kinds(&left), ["changed"], "one agent moved or recovered");
    let closed = eps.update(vec![], 3);
    assert_eq!(closed, vec![Effect::Closed { account_id: "acct".into(), provider: "claude".into() }]);
    assert_eq!(eps.list().count(), 0);
    let again = eps.update(vec![weekly("b1", "A", "acct")], 4);
    assert_eq!(kinds(&again), ["changed", "toast", "announce"], "a new episode toasts again");
}

#[test]
fn accounts_are_separate_episodes() {
    let mut eps = Episodes::default();
    let effects = eps.update(vec![weekly("b1", "A", "one"), weekly("b2", "B", "two")], 1);
    assert_eq!(effects.iter().filter(|e| matches!(e, Effect::Toast(_))).count(), 2);
    assert_eq!(eps.list().count(), 2);
}

#[test]
fn a_session_limit_raises_no_toast_but_is_announced() {
    let mut eps = Episodes::default();
    assert_eq!(kinds(&eps.update(vec![session("b1", "A", "acct")], 1)), ["changed", "announce"]);
    // A weekly limit on the same account later is news worth a toast.
    let weekly_too = eps.update(vec![session("b1", "A", "acct"), weekly("b2", "B", "acct")], 2);
    assert_eq!(kinds(&weekly_too), ["changed", "toast", "announce"]);
}

#[test]
fn a_sign_in_problem_or_an_unknown_window_toasts() {
    for p in [
        pane("b", "A", "acct", AgentState::SignedOut, None, None),
        pane("b", "A", "acct", AgentState::Limited, None, None),
        pane("b", "A", "acct", AgentState::Limited, Some(9), Some("seven_day_opus")),
    ] {
        let mut eps = Episodes::default();
        assert!(eps.update(vec![p.clone()], 1).iter().any(|e| matches!(e, Effect::Toast(_))), "{p:?}");
    }
}

#[test]
fn an_episode_rebuilt_after_a_restart_is_not_news() {
    let p = weekly("b1", "A", "acct");
    let mut eps = Episodes::new(HashMap::from([("b1".to_string(), p.signature.clone())]));
    assert_eq!(kinds(&eps.update(vec![p.clone()], 1)), ["changed"], "the item is back, with no toast and no notes");
    // The same pane hitting a new limit later is news.
    let renewed = BlockedPane { resets_at_ms: Some(99_000), signature: "new".into(), ..p };
    let effects = eps.update(vec![renewed], 2);
    assert_eq!(kinds(&effects), ["changed"], "still announced once per episode");
    eps.update(vec![], 3);
    assert_eq!(kinds(&eps.update(vec![weekly("b1", "A", "acct")], 4)), ["changed", "toast", "announce"]);
}

#[test]
fn signed_out_wins_and_the_reset_is_the_latest() {
    let mut eps = Episodes::default();
    eps.update(vec![weekly("b1", "A", "acct"), pane("b2", "B", "acct", AgentState::Limited, Some(20_000), Some("seven_day"))], 1);
    let ep = eps.get("acct").unwrap();
    assert_eq!((ep.state(), ep.resets_at_ms(), ep.limit_window().as_deref()), (AgentState::Limited, Some(20_000), Some("seven_day")));
    eps.update(vec![weekly("b1", "A", "acct"), pane("b3", "C", "acct", AgentState::SignedOut, None, None)], 2);
    let ep = eps.get("acct").unwrap();
    assert_eq!((ep.state(), ep.resets_at_ms()), (AgentState::SignedOut, None));
}

#[test]
fn the_toast_body_counts_agents_and_never_names_the_account() {
    let mut eps = Episodes::default();
    eps.update(vec![pane("b1", "A", "acct-secret", AgentState::Limited, None, None)], 1);
    assert_eq!(toast_body(eps.get("acct-secret").unwrap()), "1 agent is out of quota");
    eps.update(vec![pane("b1", "A", "acct-secret", AgentState::SignedOut, None, None), pane("b2", "B", "acct-secret", AgentState::SignedOut, None, None)], 2);
    assert_eq!(toast_body(eps.get("acct-secret").unwrap()), "2 agents need you to sign in again");
    eps.update(vec![weekly("b1", "A", "acct-secret"), weekly("b2", "B", "acct-secret")], 3);
    let body = toast_body(eps.get("acct-secret").unwrap());
    assert!(body.starts_with("2 agents are out of quota until "), "{body}");
    assert!(!body.contains("acct-secret"));
}

#[test]
fn the_leader_note_names_the_agent_and_its_tasks() {
    let p = pane("b1", "Naki-2", "acct", AgentState::Limited, None, None);
    assert_eq!(leader_note(&p, &[163]), "Naki-2 is out of quota and holds #163. Until then, a message to it starts a turn that fails; move it to another account or wait for the reset.");
    let p = pane("b1", "Naki-2", "acct", AgentState::SignedOut, None, None);
    assert_eq!(
        leader_note(&p, &[1, 2, 3]),
        "Naki-2 needs the user to sign in again and holds #1, #2 and #3. Until then, a message to it starts a turn that fails; move it to another account or wait for the reset."
    );
    assert!(leader_note(&weekly("b", "X", "a"), &[4, 5]).contains("is out of quota until "));
}
