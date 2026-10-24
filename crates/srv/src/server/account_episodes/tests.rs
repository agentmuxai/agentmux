// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The scan, the rebind and the move, against a real store
//! (`docs/specs/SPEC_UNRESPONSIVE_AGENTS_2026_10_23.md` §6, §10).

use super::*;
use crate::backend::agent_state::test_support::register_stub;
use crate::backend::blockcontroller::{self, BLOCK_CONTROLLER_PERSISTENT, STATUS_RUNNING};
use crate::backend::storage::identities::{IdentityAccount, SecretRef};
use crate::backend::storage::work_board::{requested, NewCard};
use crate::backend::storage::work_queue::{ClaimFilter, WorkItem};
use crate::server::tests::test_state;
use agentmux_common::ipc::{Command, Event};
use serde_json::json;

async fn dispatch_apply(state: &AppState, cmd: Command) -> Vec<Event> {
    let evs = crate::server::service::dispatch_to_reducer(state, cmd).await;
    for ev in &evs {
        crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore).unwrap();
    }
    evs
}

async fn new_tab(state: &AppState) -> String {
    let ws = dispatch_apply(state, Command::CreateWorkspace { name: "w".into() })
        .await
        .iter()
        .find_map(|e| match e {
            Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
            _ => None,
        })
        .unwrap();
    dispatch_apply(state, Command::CreateTab { workspace_id: ws, name: "t".into() })
        .await
        .iter()
        .find_map(|e| match e {
            Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
            _ => None,
        })
        .unwrap()
}

/// An agent pane of `agent` that ran on `account` (none when empty), with `failure` stored, and
/// a stub controller (`turn_active` as given). No `controller` meta, so a
/// restart leaves the stub alone.
async fn pane(state: &AppState, tab: &str, agent: &str, name: &str, account: &str, failure: Option<serde_json::Value>, turn_active: bool) -> String {
    let run_account = if account.is_empty() { serde_json::Value::Null } else { json!(account) };
    let mut meta = json!({ "view": "agent", "agentId": agent, "agentName": name, "agentProvider": "claude", "agent:run_account": run_account });
    if let Some(f) = failure {
        meta[blockcontroller::core::META_LAST_FAILURE] = f;
    }
    let block = dispatch_apply(state, Command::CreateBlock { tab_id: tab.to_string(), meta })
        .await
        .iter()
        .find_map(|e| match e {
            Event::BlockCreated { block_id, .. } => Some(block_id.clone()),
            _ => None,
        })
        .unwrap();
    register_stub(&block, BLOCK_CONTROLLER_PERSISTENT, STATUS_RUNNING, turn_active);
    block
}

fn failure(code: &str, resets_at_ms: Option<u64>, window: Option<&str>) -> serde_json::Value {
    let mut f = json!({ "code": code, "title": "Usage limit reached", "detail": "", "retryable": false });
    if let Some(r) = resets_at_ms {
        f["resetsAtMs"] = json!(r);
    }
    if let Some(w) = window {
        f["limitWindow"] = json!(w);
    }
    f
}

fn account(id: &str, provider: &str, status: &str) -> IdentityAccount {
    IdentityAccount {
        id: id.to_string(),
        name: format!("{id}-name"),
        provider: provider.to_string(),
        kind: "oauth".to_string(),
        display_name: String::new(),
        secret_ref: SecretRef::OAuthConfigDir { dir: format!("/tmp/{id}") },
        context: json!({}),
        status: status.to_string(),
        created_at: 0,
        updated_at: 0,
    }
}

/// A definition the identity links can point at.
fn agent_def(state: &AppState, id: &str) {
    let mut def = crate::backend::storage::agents::test_agent_def(id, &format!("Agent {id}"), "claude", "host", 1, "");
    state.mstore.agent_def_insert(&mut def).unwrap();
}

fn stored_failure(state: &AppState, block: &str) -> Option<serde_json::Value> {
    state.mstore.get::<Block>(block).unwrap().unwrap().meta.get(blockcontroller::core::META_LAST_FAILURE).cloned()
}

fn cleanup(blocks: &[&String]) {
    for b in blocks {
        blockcontroller::delete_controller(b);
    }
}

#[tokio::test]
async fn the_scan_groups_limited_and_signed_out_panes_by_their_run_account() {
    let state = test_state();
    state.id_store.identity_upsert(&account("acct-1", "claude-code", "valid")).unwrap();
    let tab = new_tab(&state).await;
    let now = agentmux_common::time::now_ms_u64();
    let weekly = pane(&state, &tab, "a1", "Naki-2", "acct-1", Some(failure("usage_limit", Some(now + 60_000), Some("seven_day"))), false).await;
    let auth = pane(&state, &tab, "a2", "Loap-3", "acct-1", Some(failure("auth", None, None)), false).await;
    let reset = pane(&state, &tab, "a3", "Old", "acct-1", Some(failure("usage_limit", Some(now - 1), None)), false).await;
    let transient = pane(&state, &tab, "a4", "Busy", "acct-1", Some(failure("rate_limited", None, None)), false).await;
    let mid_turn = pane(&state, &tab, "a5", "Mid", "acct-1", Some(failure("usage_limit", None, None)), true).await;
    let healthy = pane(&state, &tab, "a6", "Fine", "acct-1", None, false).await;

    let mut panes = scan(&state, now);
    panes.sort_by(|a, b| a.agent_name.cmp(&b.agent_name));
    let got: Vec<(&str, AgentState, Option<&str>, &str, &str)> =
        panes.iter().map(|p| (p.agent_name.as_str(), p.state, p.limit_window.as_deref(), p.account_id.as_str(), p.provider.as_str())).collect();
    assert_eq!(
        got,
        vec![
            ("Loap-3", AgentState::SignedOut, None, "acct-1", "claude"),
            ("Mid", AgentState::Limited, None, "acct-1", "claude"),
            ("Naki-2", AgentState::Limited, Some("seven_day"), "acct-1", "claude"),
        ],
        "a pane retrying (a turn in flight) stays; a passed reset, a transient failure and a clean pane are not blocked"
    );
    let base = baseline(&state.mstore);
    assert!(base.contains_key(&weekly) && base.contains_key(&auth) && base.contains_key(&reset) && base.contains_key(&mid_turn));
    assert!(!base.contains_key(&transient) && !base.contains_key(&healthy));
    cleanup(&[&weekly, &auth, &reset, &transient, &mid_turn, &healthy]);
}

#[tokio::test]
async fn the_strip_lists_the_agents_their_tasks_and_where_they_can_move() {
    let state = test_state();
    for a in [account("acct-1", "claude", "valid"), account("spare-unknown", "claude", "unknown"), account("spare", "claude", "valid"), account("gh", "github", "valid"), account("out", "claude", "needs_reauth")] {
        state.id_store.identity_upsert(&a).unwrap();
    }
    let tab = new_tab(&state).await;
    let now = agentmux_common::time::now_ms_u64();
    let block = pane(&state, &tab, "a1", "Naki-r4test", "acct-1", Some(failure("usage_limit", Some(now + 60_000), Some("seven_day"))), false).await;
    state.reactive_handler.register_agent("Naki-r4test", &block, None).unwrap();
    let mut item = WorkItem {
        id: "w-r4".into(),
        title: "Write the spec".into(),
        payload: String::new(),
        kind: String::new(),
        target_agent: String::new(),
        target_group: String::new(),
        priority: 0,
        state: String::new(),
        claimed_by: String::new(),
        target_agent_uid: String::new(),
        claimed_by_uid: String::new(),
        claim_expires: None,
        attempts: 0,
        max_attempts: 3,
        created_by: "Lead-r4test".into(),
        created_by_uid: String::new(),
        created_at: now as i64,
        updated_at: now as i64,
        not_before: None,
        result: String::new(),
    };
    state.identity_store.work_queue_enqueue(&item).unwrap();
    let card = NewCard { requested_by: "Lead-r4test".into(), requested_by_kind: requested::AGENT.into(), ..Default::default() };
    let num = state.identity_store.work_card_create(&item.id, &card, now as i64).unwrap();
    let claim = ClaimFilter { kind: None, agent_id: "Naki-r4test".into(), agent_uid: String::new(), groups: vec![] };
    assert_eq!(state.identity_store.work_queue_claim(&claim, now as i64, 60_000).unwrap().unwrap().id, "w-r4");
    // A task nobody holds isn't the episode's.
    item.id = "w-r4-open".into();
    state.identity_store.work_queue_enqueue(&item).unwrap();

    let mut eps = Episodes::default();
    eps.update(scan(&state, now), now);
    let v = view(&state, eps.get("acct-1").unwrap());
    assert_eq!((v.state.as_str(), v.resets_at_ms, v.limit_window.as_deref()), ("limited", Some(now + 60_000), Some("seven_day")));
    assert_eq!(v.agents, vec![AccountEpisodeAgent { block_id: block.clone(), agent_id: "a1".into(), agent_name: "Naki-r4test".into() }]);
    assert_eq!((v.agent_ids.clone(), v.block_ids.clone()), (vec!["a1".to_string()], vec![block.clone()]), "who it covers");
    assert_eq!(v.tasks, vec![AccountEpisodeTask { id: "w-r4".into(), number: num, title: "Write the spec".into() }], "only what it holds");
    let to: Vec<&str> = v.candidates.iter().map(|c| c.account_id.as_str()).collect();
    assert_eq!(to, ["spare", "spare-unknown"], "same provider, not blocked, not the account itself; signed in first");
    let held = held_tasks(&state, &[block.as_str()]);
    assert_eq!(held[&block][0].1, "Lead-r4test", "the requester hears about it");
    state.reactive_handler.unregister_block(&block);
    cleanup(&[&block]);
}

#[tokio::test]
async fn a_rebind_clears_only_the_failures_of_the_old_account() {
    let state = test_state();
    let tab = new_tab(&state).await;
    let f = || Some(failure("usage_limit", None, None));
    let old = pane(&state, &tab, "agent-x", "X", "old", f(), false).await;
    let already = pane(&state, &tab, "agent-x", "X", "new", f(), false).await;
    let other_agent = pane(&state, &tab, "agent-y", "Y", "old", f(), false).await;
    let before = failed_panes(&state.mstore, &state.identity_store, "agent-x", "claude-code");
    assert_eq!(before.len(), 2, "alias and canonical provider are the same");
    assert!(failed_panes(&state.mstore, &state.identity_store, "agent-x", "github").is_empty(), "another provider's link changes nothing");
    assert_eq!(after_rebind(&state.mstore, &state.event_bus, before, "new"), 1);
    assert_eq!(stored_failure(&state, &old), None, "its run was on the old account");
    assert!(stored_failure(&state, &already).is_some(), "it failed on the new account itself");
    assert!(stored_failure(&state, &other_agent).is_some(), "another agent");
    cleanup(&[&old, &already, &other_agent]);
}

/// Whether `held_jekt_wake` was notified since it was last drained.
async fn held_wake_notified() -> bool {
    tokio::time::timeout(Duration::from_millis(200), crate::backend::reactive::held_jekt_wake().notified()).await.is_ok()
}

/// Signing in again to the account a pane was refused on ends its "signed
/// out" (#2237): the failure is cleared, the episode closes and held
/// messages are woken. Only the operator's window may report it.
#[tokio::test]
async fn a_sign_in_to_the_same_account_ends_its_signed_out_episode() {
    let state = test_state();
    state.id_store.identity_upsert(&account("acct-a", "claude", "valid")).unwrap();
    let tab = new_tab(&state).await;
    let now = agentmux_common::time::now_ms_u64();
    let block = pane(&state, &tab, "s1", "Signed-out", "acct-a", Some(failure("auth", None, None)), false).await;
    let mut eps = Episodes::default();
    eps.update(scan(&state, now), now);
    assert_eq!(eps.get("acct-a").map(|e| e.state()), Some(AgentState::SignedOut));

    let e = call(&state, Some("loap-3"), COMMAND_IDENTITY_ACCOUNT_SIGNED_IN, json!({ "account_id": "acct-a" })).await.unwrap_err();
    assert!(e.starts_with("FORBIDDEN"), "{e}");
    assert!(stored_failure(&state, &block).is_some(), "an agent can't clear it");

    held_wake_notified().await;
    let r = call(&state, None, COMMAND_IDENTITY_ACCOUNT_SIGNED_IN, json!({ "account_id": "acct-a" })).await.unwrap();
    assert_eq!(r["cleared"], json!(1));
    assert_eq!(stored_failure(&state, &block), None);
    assert!(held_wake_notified().await, "held messages replay");
    let closed = eps.update(scan(&state, now + 1_000), now + 1_000);
    assert!(closed.iter().any(|e| matches!(e, Effect::Closed { account_id, .. } if account_id == "acct-a")), "{closed:?}");
    cleanup(&[&block]);
}

/// A usage limit isn't a sign-in problem: it ends at the reset or a move.
#[tokio::test]
async fn a_sign_in_leaves_a_usage_limit_alone() {
    let state = test_state();
    state.id_store.identity_upsert(&account("acct-a", "claude", "valid")).unwrap();
    let tab = new_tab(&state).await;
    let now = agentmux_common::time::now_ms_u64();
    let block = pane(&state, &tab, "u1", "Limited", "acct-a", Some(failure("usage_limit", Some(now + 60_000), Some("five_hour"))), false).await;
    let mut eps = Episodes::default();
    eps.update(scan(&state, now), now);

    assert_eq!(clear_signed_out(&state.mstore, &state.identity_store, &state.event_bus, "acct-a"), 0);
    assert!(stored_failure(&state, &block).is_some());
    assert!(eps.update(scan(&state, now + 1_000), now + 1_000).is_empty(), "its episode stays open");
    assert_eq!(eps.get("acct-a").map(|e| e.state()), Some(AgentState::Limited));
    cleanup(&[&block]);
}

/// A sign-in to one account says nothing about another.
#[tokio::test]
async fn a_sign_in_leaves_another_accounts_panes_alone() {
    let state = test_state();
    let tab = new_tab(&state).await;
    let on_b = pane(&state, &tab, "b1", "On-b", "acct-b", Some(failure("auth", None, None)), false).await;
    assert_eq!(clear_signed_out(&state.mstore, &state.identity_store, &state.event_bus, "acct-a"), 0);
    assert!(stored_failure(&state, &on_b).is_some());
    cleanup(&[&on_b]);
}

#[tokio::test]
async fn the_move_relinks_idle_agents_and_leaves_busy_ones() {
    let state = test_state();
    for a in [account("old", "claude", "valid"), account("spare", "claude", "valid"), account("gh", "github", "valid")] {
        state.id_store.identity_upsert(&a).unwrap();
    }
    let tab = new_tab(&state).await;
    let f = || Some(failure("usage_limit", None, None));
    // An idle limited agent with a stale alias row, and one mid-turn.
    agent_def(&state, "idle");
    agent_def(&state, "busy");
    state.identity_store.agent_identity_link("idle", "old", "claude-code").unwrap();
    state.identity_store.agent_identity_link("idle", "gh", "github").unwrap();
    state.identity_store.agent_identity_link("busy", "old", "claude").unwrap();
    let idle = pane(&state, &tab, "idle", "Idle", "old", f(), false).await;
    let busy = pane(&state, &tab, "busy", "Busy", "old", f(), true).await;

    let req = IdentityAccountMoveAgentsReq { from: "old".into(), to: "spare".into(), block_ids: vec![idle.clone(), busy.clone(), "gone".into()] };
    let r = moves::move_agents(&state, &req).unwrap();
    assert_eq!(r.moved, vec![AccountEpisodeAgent { block_id: idle.clone(), agent_id: "idle".into(), agent_name: "Idle".into() }]);
    assert_eq!(r.busy, vec![AccountEpisodeAgent { block_id: busy.clone(), agent_id: "busy".into(), agent_name: "Busy".into() }]);
    assert_eq!(r.failed.iter().map(|f| f.block_id.as_str()).collect::<Vec<_>>(), ["gone"]);

    let mut links: Vec<(String, String)> =
        state.identity_store.agent_identity_list_for_agent("idle").unwrap().into_iter().map(|l| (l.provider, l.account_id)).collect();
    links.sort();
    assert_eq!(links, vec![("claude".into(), "spare".into()), ("github".into(), "gh".into())], "the alias row went; other providers stay");
    assert_eq!(stored_failure(&state, &idle), None, "its failure was the old account's");
    assert!(stored_failure(&state, &busy).is_some());
    assert_eq!(state.identity_store.agent_identity_list_for_agent("busy").unwrap()[0].account_id, "old");

    // Never across providers, never to the same account, never to one that can't run.
    let bad = |to: &str| moves::move_agents(&state, &IdentityAccountMoveAgentsReq { from: "old".into(), to: to.into(), block_ids: vec![idle.clone()] });
    assert!(bad("gh").unwrap_err().contains("never move across providers"));
    assert!(bad("old").is_err());
    assert!(bad("nope").is_err());
    let tired = pane(&state, &tab, "z", "Z", "spare", f(), false).await;
    assert!(bad("spare").unwrap_err().contains("out of quota"), "spare is blocked now");
    cleanup(&[&idle, &busy, &tired]);
}

/// The per-pane toasts an episode replaces: its own failure toast and its
/// "finished" (spec §6.2). A pane with no account keeps them.
#[tokio::test]
async fn an_episode_speaks_for_a_panes_quota_or_sign_in_failure_on_an_account() {
    let state = test_state();
    let tab = new_tab(&state).await;
    let now = agentmux_common::time::now_ms_u64();
    let session = pane(&state, &tab, "a1", "A", "acct", Some(failure("usage_limit", Some(now + 1_000), Some("five_hour"))), false).await;
    let auth = pane(&state, &tab, "a2", "B", "acct", Some(failure("auth", None, None)), false).await;
    let reset = pane(&state, &tab, "a3", "C", "acct", Some(failure("usage_limit", Some(now), None)), false).await;
    let transient = pane(&state, &tab, "a4", "D", "acct", Some(failure("network", None, None)), false).await;
    let no_account = pane(&state, &tab, "a5", "E", "", Some(failure("usage_limit", None, None)), false).await;
    let got: Vec<bool> = [&session, &auth, &reset, &transient, &no_account].iter().map(|b| covers(&state.mstore, &state.identity_store, b, now)).collect();
    assert_eq!(got, [true, true, false, false, false]);
    assert!(!covers(&state.mstore, &state.identity_store, "no-such-block", now));
    cleanup(&[&session, &auth, &reset, &transient, &no_account]);
}

/// One call on an engine with this module's RPCs; `as_agent` registers the
/// connection as an agent, as `bus:register` does.
async fn call(state: &AppState, as_agent: Option<&str>, command: &str, data: serde_json::Value) -> Result<serde_json::Value, String> {
    use crate::backend::rpc_types::{RpcContext, RpcMessage};
    let (engine, mut rx) = WshRpcEngine::new();
    register(&engine, state);
    if let Some(agent) = as_agent {
        engine.set_rpc_context(RpcContext { agent_id: agent.to_string(), ..Default::default() });
    }
    engine.handle_message(RpcMessage { command: command.to_string(), reqid: "req-1".to_string(), data: Some(data), ..Default::default() });
    let resp = tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv()).await.unwrap().unwrap();
    if resp.error.is_empty() {
        Ok(resp.data.unwrap_or_default())
    } else {
        Err(resp.error)
    }
}

/// Both RPCs are the operator's: an agent connection can neither read which
/// accounts are blocked (their labels can be emails) nor move other agents.
#[tokio::test]
async fn only_the_operators_client_may_list_episodes_or_move_agents() {
    let state = test_state();
    for a in [account("old", "claude", "valid"), account("spare", "claude", "valid")] {
        state.id_store.identity_upsert(&a).unwrap();
    }
    agent_def(&state, "idle");
    state.identity_store.agent_identity_link("idle", "old", "claude").unwrap();
    let tab = new_tab(&state).await;
    let idle = pane(&state, &tab, "idle", "Idle", "old", Some(failure("usage_limit", None, None)), false).await;
    let move_req = json!({ "from": "old", "to": "spare", "block_ids": [idle] });

    let e = call(&state, Some("loap-3"), COMMAND_IDENTITY_ACCOUNT_AVAILABILITY, json!({})).await.unwrap_err();
    assert!(e.starts_with("FORBIDDEN"), "{e}");
    let e = call(&state, Some("loap-3"), COMMAND_IDENTITY_ACCOUNT_MOVE_AGENTS, move_req.clone()).await.unwrap_err();
    assert!(e.starts_with("FORBIDDEN"), "{e}");
    assert_eq!(state.identity_store.agent_identity_list_for_agent("idle").unwrap()[0].account_id, "old", "nothing moved");
    assert!(stored_failure(&state, &idle).is_some());

    let listed = call(&state, None, COMMAND_IDENTITY_ACCOUNT_AVAILABILITY, json!({})).await.unwrap();
    assert!(listed["episodes"].is_array());
    let moved = call(&state, None, COMMAND_IDENTITY_ACCOUNT_MOVE_AGENTS, move_req).await.unwrap();
    assert_eq!(moved["moved"][0]["block_id"], json!(idle));
    assert_eq!(state.identity_store.agent_identity_list_for_agent("idle").unwrap()[0].account_id, "spare");
    cleanup(&[&idle]);
}

/// The link is the agent's: a pane of it in a turn keeps the agent where it
/// is, even when that pane isn't the episode's; once all are idle, every pane
/// restarts on the new link.
#[tokio::test]
async fn an_agent_moves_only_when_none_of_its_panes_is_in_a_turn() {
    let state = test_state();
    for a in [account("old", "claude", "valid"), account("spare", "claude", "valid")] {
        state.id_store.identity_upsert(&a).unwrap();
    }
    agent_def(&state, "twin");
    state.identity_store.agent_identity_link("twin", "old", "claude").unwrap();
    let tab = new_tab(&state).await;
    let limited = pane(&state, &tab, "twin", "Twin", "old", Some(failure("usage_limit", None, None)), false).await;
    let working = pane(&state, &tab, "twin", "Twin", "old", None, true).await;
    let req = IdentityAccountMoveAgentsReq { from: "old".into(), to: "spare".into(), block_ids: vec![limited.clone()] };

    let r = moves::move_agents(&state, &req).unwrap();
    assert!(r.moved.is_empty());
    let mut busy: Vec<&str> = r.busy.iter().map(|a| a.block_id.as_str()).collect();
    busy.sort();
    let mut both = vec![limited.as_str(), working.as_str()];
    both.sort();
    assert_eq!(busy, both, "listed, with the pane in a turn");
    assert_eq!(state.identity_store.agent_identity_list_for_agent("twin").unwrap()[0].account_id, "old", "not relinked");

    blockcontroller::delete_controller(&working);
    register_stub(&working, BLOCK_CONTROLLER_PERSISTENT, STATUS_RUNNING, false);
    let r = moves::move_agents(&state, &req).unwrap();
    let mut moved: Vec<&str> = r.moved.iter().map(|a| a.block_id.as_str()).collect();
    moved.sort();
    assert_eq!(moved, both, "both panes restart on the new link");
    assert_eq!(state.identity_store.agent_identity_list_for_agent("twin").unwrap()[0].account_id, "spare");
    cleanup(&[&limited, &working]);
}

/// One note per requester per episode, naming every agent of theirs that is
/// blocked, and none for an agent that joins later.
#[tokio::test]
async fn a_requester_of_two_blocked_agents_gets_one_note() {
    let state = test_state();
    state.id_store.identity_upsert(&account("acct-1", "claude", "valid")).unwrap();
    let tab = new_tab(&state).await;
    let now = agentmux_common::time::now_ms_u64();
    let claim = |agent: &str, id: &str, title: &str| {
        let item = WorkItem {
            id: id.into(),
            title: title.into(),
            payload: String::new(),
            kind: String::new(),
            target_agent: String::new(),
            target_group: String::new(),
            priority: 0,
            state: String::new(),
            claimed_by: String::new(),
            target_agent_uid: String::new(),
            claimed_by_uid: String::new(),
            claim_expires: None,
            attempts: 0,
            max_attempts: 3,
            created_by: "Lead-r4note".into(),
            created_by_uid: String::new(),
            created_at: now as i64,
            updated_at: now as i64,
            not_before: None,
            result: String::new(),
        };
        state.identity_store.work_queue_enqueue(&item).unwrap();
        let card = NewCard { requested_by: "Lead-r4note".into(), requested_by_kind: requested::AGENT.into(), ..Default::default() };
        let num = state.identity_store.work_card_create(id, &card, now as i64).unwrap();
        let f = ClaimFilter { kind: None, agent_id: agent.into(), agent_uid: String::new(), groups: vec![] };
        assert_eq!(state.identity_store.work_queue_claim(&f, now as i64, 60_000).unwrap().unwrap().id, id);
        num
    };
    let f = || Some(failure("usage_limit", Some(now + 60_000), Some("seven_day")));
    let a = pane(&state, &tab, "n1", "Note-a-r4", "acct-1", f(), false).await;
    let b = pane(&state, &tab, "n2", "Note-b-r4", "acct-1", f(), false).await;
    let c = pane(&state, &tab, "n3", "Note-c-r4", "acct-1", f(), false).await;
    for (name, block) in [("Note-a-r4", &a), ("Note-b-r4", &b), ("Note-c-r4", &c)] {
        state.reactive_handler.register_agent(name, block, None).unwrap();
    }
    let na = claim("Note-a-r4", "w-na", "First");
    let nb = claim("Note-b-r4", "w-nb", "Second");
    claim("Note-c-r4", "w-nc", "Third");

    let episodes = Mutex::new(Episodes::default());
    let panes = scan(&state, now);
    let (first, later): (Vec<BlockedPane>, Vec<BlockedPane>) = panes.into_iter().partition(|p| p.block_id != c);
    episodes.lock().unwrap().update(first.clone(), now);
    let notes = requester_notes(&state, &episodes, "acct-1", &first);
    assert_eq!(notes.len(), 1, "two agents, one requester: one note");
    let (to, text) = &notes[0];
    assert_eq!(to, "Lead-r4note");
    assert!(text.starts_with("[AgentMux] ") && text.contains("Note-a-r4") && text.contains("Note-b-r4"), "{text}");
    assert!(text.contains(&format!("#{na}")) && text.contains(&format!("#{nb}")), "{text}");
    episodes.lock().unwrap().update([first, later.clone()].concat(), now);
    assert!(requester_notes(&state, &episodes, "acct-1", &later).is_empty(), "a later agent tells the same requester nothing more");
    for block in [&a, &b, &c] {
        state.reactive_handler.unregister_block(block);
    }
    cleanup(&[&a, &b, &c]);
}

/// A retry turn doesn't close the episode: the pane stays in it while the
/// turn runs, with no second toast or note, and failing again is the same
/// episode.
#[tokio::test]
async fn a_retry_turn_keeps_the_pane_in_its_episode() {
    let state = test_state();
    state.id_store.identity_upsert(&account("acct-1", "claude", "valid")).unwrap();
    let tab = new_tab(&state).await;
    let now = agentmux_common::time::now_ms_u64();
    let limit = failure("usage_limit", Some(now + 600_000), Some("seven_day"));
    let block = pane(&state, &tab, "r1", "Retry", "acct-1", Some(limit.clone()), false).await;
    let mut eps = Episodes::default();
    assert_eq!(eps.update(scan(&state, now), now).iter().filter(|e| matches!(e, Effect::Toast(_))).count(), 1);

    // The operator retries: a turn runs on the same stored failure.
    blockcontroller::delete_controller(&block);
    register_stub(&block, BLOCK_CONTROLLER_PERSISTENT, STATUS_RUNNING, true);
    assert!(eps.update(scan(&state, now + 1_000), now + 1_000).is_empty(), "still in its episode: nothing closes, nothing is told again");
    assert!(covers(&state.mstore, &state.identity_store, &block, now + 1_000));

    // The turn fails again on the same limit.
    blockcontroller::delete_controller(&block);
    register_stub(&block, BLOCK_CONTROLLER_PERSISTENT, STATUS_RUNNING, false);
    let mut meta = crate::backend::obj::MetaMapType::new();
    meta.insert(blockcontroller::core::META_LAST_FAILURE.to_string(), limit);
    crate::server::service::update_object_meta(&state.mstore, &format!("block:{block}"), &meta).unwrap();
    assert!(eps.update(scan(&state, now + 2_000), now + 2_000).is_empty(), "the same episode");
    assert_eq!(eps.get("acct-1").unwrap().opened_at_ms, now);
    cleanup(&[&block]);
}
