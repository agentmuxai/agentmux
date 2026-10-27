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
async fn the_move_relinks_idle_agents_and_queues_busy_ones() {
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
    assert_eq!(pending_of(&state, &busy), Some(json!("spare")), "its move waits for the turn to end");
    assert_eq!(pending_of(&state, &idle), None);

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
/// connection as an agent, as `bus:register` does, and `None` is the
/// operator's window, proven by the host.
async fn call(state: &AppState, as_agent: Option<&str>, command: &str, data: serde_json::Value) -> Result<serde_json::Value, String> {
    use crate::backend::rpc_types::RpcContext;
    let ctx = match as_agent {
        Some(agent) => RpcContext { agent_id: agent.to_string(), ..Default::default() },
        None => RpcContext::operator_for_tests(),
    };
    call_with(state, ctx, command, data).await
}

async fn call_with(
    state: &AppState,
    ctx: crate::backend::rpc_types::RpcContext,
    command: &str,
    data: serde_json::Value,
) -> Result<serde_json::Value, String> {
    use crate::backend::rpc_types::RpcMessage;
    let (engine, mut rx) = WshRpcEngine::new();
    register(&engine, state);
    engine.set_rpc_context(ctx);
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

    // Nor can a connection the host never proved is the AgentMux window's.
    for (command, data) in [
        (COMMAND_IDENTITY_ACCOUNT_AVAILABILITY, json!({})),
        (COMMAND_IDENTITY_ACCOUNT_MOVE_AGENTS, move_req.clone()),
        (COMMAND_IDENTITY_ACCOUNT_SIGNED_IN, json!({ "account_id": "old" })),
    ] {
        let e = call_with(&state, Default::default(), command, data).await.unwrap_err();
        assert!(e.starts_with("FORBIDDEN") && e.contains("isn't the AgentMux window"), "{command}: {e}");
    }
    assert_eq!(state.identity_store.agent_identity_list_for_agent("idle").unwrap()[0].account_id, "old", "nothing moved");
    assert!(stored_failure(&state, &idle).is_some(), "nothing cleared");

    let listed = call(&state, None, COMMAND_IDENTITY_ACCOUNT_AVAILABILITY, json!({})).await.unwrap();
    assert!(listed["episodes"].is_array());
    let moved = call(&state, None, COMMAND_IDENTITY_ACCOUNT_MOVE_AGENTS, move_req).await.unwrap();
    assert_eq!(moved["moved"][0]["block_id"], json!(idle));
    assert_eq!(state.identity_store.agent_identity_list_for_agent("idle").unwrap()[0].account_id, "spare");
    cleanup(&[&idle]);
}

/// The link is the agent's: a pane of it in a turn queues the move, even when that pane isn't
/// the episode's; once all are idle, every pane restarts on the new link.
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
    assert_eq!(pending_of(&state, &limited), Some(json!("spare")), "queued on the episode's pane too");

    blockcontroller::delete_controller(&working);
    register_stub(&working, BLOCK_CONTROLLER_PERSISTENT, STATUS_RUNNING, false);
    let r = moves::move_agents(&state, &req).unwrap();
    let mut moved: Vec<&str> = r.moved.iter().map(|a| a.block_id.as_str()).collect();
    moved.sort();
    assert_eq!(moved, both, "both panes restart on the new link");
    assert_eq!(state.identity_store.agent_identity_list_for_agent("twin").unwrap()[0].account_id, "spare");
    assert_eq!(pending_of(&state, &limited), None, "applied, so no longer queued");
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

/// Runs `command` on an engine with the agent handlers (link, unlink) registered.
async fn call_agent_handlers(state: &AppState, command: &str, data: serde_json::Value) -> Result<serde_json::Value, String> {
    use crate::backend::rpc_types::RpcMessage;
    let (engine, mut rx) = WshRpcEngine::new();
    crate::server::agent_handlers::register_agent_handlers(&engine, state);
    engine.set_rpc_context(crate::backend::rpc_types::RpcContext::operator_for_tests());
    engine.handle_message(RpcMessage { command: command.to_string(), reqid: "req-1".to_string(), data: Some(data), ..Default::default() });
    let resp = tokio::time::timeout(Duration::from_secs(10), rx.recv()).await.unwrap().unwrap();
    if resp.error.is_empty() {
        Ok(resp.data.unwrap_or_default())
    } else {
        Err(resp.error)
    }
}

/// Moving an agent to another account must not leave a stored usage limit
/// behind (#4936 §3.5), including the limit of a pane from before
/// `agent:run_account` was stamped, which counts against the agent's links.
#[tokio::test]
async fn a_link_to_another_account_clears_an_unstamped_panes_limit() {
    let state = test_state();
    for id in ["old", "new"] {
        state.id_store.identity_upsert(&account(id, "claude", "valid")).unwrap();
    }
    agent_def(&state, "agent-x");
    state.identity_store.agent_identity_link("agent-x", "old", "claude").unwrap();
    let tab = new_tab(&state).await;
    let limited = pane(&state, &tab, "agent-x", "X", "", Some(failure("usage_limit", None, None)), false).await;
    call_agent_handlers(&state, "linkagentidentity", json!({ "agent_id": "agent-x", "account_id": "new", "provider": "claude" })).await.unwrap();
    assert_eq!(stored_failure(&state, &limited), None, "the limit was the old account's");
    cleanup(&[&limited]);
}

/// An unlink leaves the agent with no account, so its panes' limits go too;
/// a silent one is the first half of a rebind and clears nothing itself.
#[tokio::test]
async fn an_unlink_clears_the_limit_unless_it_is_the_silent_half_of_a_rebind() {
    let state = test_state();
    state.id_store.identity_upsert(&account("old", "claude", "valid")).unwrap();
    agent_def(&state, "agent-x");
    state.identity_store.agent_identity_link("agent-x", "old", "claude").unwrap();
    let tab = new_tab(&state).await;
    let limited = pane(&state, &tab, "agent-x", "X", "old", Some(failure("usage_limit", None, None)), false).await;
    call_agent_handlers(&state, "unlinkagentidentity", json!({ "agent_id": "agent-x", "provider": "claude", "silent": true })).await.unwrap();
    assert!(stored_failure(&state, &limited).is_some(), "silent: the link that follows decides");
    state.identity_store.agent_identity_link("agent-x", "old", "claude").unwrap();
    call_agent_handlers(&state, "unlinkagentidentity", json!({ "agent_id": "agent-x", "provider": "claude" })).await.unwrap();
    assert_eq!(stored_failure(&state, &limited), None);
    cleanup(&[&limited]);
}

/// Listing the accounts numbers a group that shares a provider and an email, in
/// the order its accounts were made, keeps a number once given, and drops it when
/// the email changes (#4936 §3.3).
#[tokio::test]
async fn listing_accounts_numbers_same_email_groups_and_keeps_the_numbers() {
    let state = test_state();
    let with_email = |id: &str, email: &str, created_at: i64| {
        let mut a = account(id, "claude", "valid");
        a.context = json!({ "email": email });
        a.created_at = created_at;
        a
    };
    for a in [with_email("second", "Me@example.com", 20), with_email("first", "me@example.com", 10), with_email("other", "x@example.com", 5)] {
        state.id_store.identity_upsert(&a).unwrap();
    }
    let list = || async { call_agent_handlers(&state, "listidentityaccounts", json!({})).await.unwrap() };
    let seq = |v: &serde_json::Value, id: &str| v.as_array().unwrap().iter().find(|a| a["id"] == id).map(|a| a["context"]["seq"].clone());
    let r = list().await;
    assert_eq!((seq(&r, "first"), seq(&r, "second"), seq(&r, "other")), (Some(json!(1)), Some(json!(2)), Some(json!(1))));

    // `first` is removed and a third sign-in arrives: `second` stays 2, the new one is 3.
    state.id_store.identity_delete("first").unwrap();
    state.id_store.identity_upsert(&with_email("third", "me@example.com", 30)).unwrap();
    let r = list().await;
    assert_eq!((seq(&r, "second"), seq(&r, "third")), (Some(json!(2)), Some(json!(3))));

    // A provider filter doesn't renumber the group.
    let r = call_agent_handlers(&state, "listidentityaccounts", json!({ "provider": "claude" })).await.unwrap();
    assert_eq!(seq(&r, "third"), Some(json!(3)));

    // A different address is a different group: the old number goes.
    state.id_store.identity_set_context_email("third", "new@example.com").unwrap();
    let r = list().await;
    assert_eq!(seq(&r, "third"), Some(json!(1)), "numbered afresh in its new group");

    // A change of case only is the same group and keeps its number.
    state.id_store.identity_set_context_email("second", "ME@Example.com ").unwrap();
    let r = list().await;
    assert_eq!(seq(&r, "second"), Some(json!(2)));

    // The highest number is deleted: the next sign-in is not given it again.
    for (id, at) in [("zed1", 1), ("zed2", 2)] {
        state.id_store.identity_upsert(&with_email(id, "z@example.com", at)).unwrap();
    }
    let r = list().await;
    assert_eq!((seq(&r, "zed1"), seq(&r, "zed2")), (Some(json!(1)), Some(json!(2))));
    state.id_store.identity_delete("zed2").unwrap();
    state.id_store.identity_upsert(&with_email("zed3", "z@example.com", 3)).unwrap();
    let r = list().await;
    assert_eq!(seq(&r, "zed3"), Some(json!(3)), "2 was handed out and is not reused");

    // A re-login that rebuilds the context from the email alone keeps it too.
    state.id_store.identity_upsert(&with_email("second", "me@example.com", 20)).unwrap();
    let r = list().await;
    assert_eq!(seq(&r, "second"), Some(json!(2)));
}

fn switch_req(agent: &str, account: &str, when: switch::AccountSwitchWhen) -> switch::AgentAccountSwitchReq {
    switch::AgentAccountSwitchReq { agent_id: Some(agent.to_string()), account: account.to_string(), when }
}

fn linked_account(state: &AppState, agent: &str) -> Vec<String> {
    state.identity_store.agent_identity_list_for_agent(agent).unwrap().into_iter().map(|l| l.account_id).collect()
}

fn pending_of(state: &AppState, block: &str) -> Option<serde_json::Value> {
    state.mstore.get::<Block>(block).unwrap().unwrap().meta.get(switch::META_PENDING_ACCOUNT).cloned().filter(|v| !v.is_null())
}

/// What the strip queued lands when the turn ends, even on an account not checked yet; and a bind
/// from the failure row replaces an alias row that points elsewhere instead of calling it unchanged.
#[tokio::test]
async fn a_queued_strip_move_to_an_unchecked_account_lands_and_a_stale_alias_row_is_replaced() {
    use switch::AccountSwitchWhen::AfterTurn;
    let state = test_state();
    for (id, status) in [("old", "valid"), ("fresh", "unknown"), ("a", "valid"), ("b", "valid")] {
        state.id_store.identity_upsert(&account(id, "claude", status)).unwrap();
    }
    agent_def(&state, "busy");
    agent_def(&state, "dup");
    state.identity_store.agent_identity_link("busy", "old", "claude").unwrap();
    let tab = new_tab(&state).await;
    let busy = pane(&state, &tab, "busy", "Busy", "old", None, true).await;
    let req = IdentityAccountMoveAgentsReq { from: "old".into(), to: "fresh".into(), block_ids: vec![busy.clone()] };
    assert_eq!(moves::move_agents(&state, &req).unwrap().busy.len(), 1, "queued");
    register_stub(&busy, BLOCK_CONTROLLER_PERSISTENT, STATUS_RUNNING, false);
    switch::apply_pending(&state);
    assert_eq!(linked_account(&state, "busy"), ["fresh"]);

    // `claude -> a` and a stale `claude-code -> b`: the effective account is b, so a is a real move.
    state.identity_store.agent_identity_link("dup", "a", "claude").unwrap();
    state.identity_store.agent_identity_link("dup", "b", "claude-code").unwrap();
    let r = switch::switch(&state, "", &switch_req("dup", "a", AfterTurn)).unwrap();
    assert_eq!(r.status, switch::AccountSwitchStatus::Switched);
    assert_eq!(linked_account(&state, "dup"), ["a"], "the alias row went");
    cleanup(&[&busy]);
}

/// An idle agent moves at once; one in a turn waits and moves when the turn ends;
/// "now" does not wait; the same account changes nothing and cancels a wait (#4936 §3.2).
#[tokio::test]
async fn a_switch_applies_at_once_when_idle_waits_for_a_turn_and_cancels_on_the_current_account() {
    use switch::AccountSwitchWhen::{AfterTurn, Now};
    let state = test_state();
    for id in ["old", "new"] {
        state.id_store.identity_upsert(&account(id, "claude", "valid")).unwrap();
    }
    agent_def(&state, "idle");
    agent_def(&state, "busy");
    for a in ["idle", "busy"] {
        state.identity_store.agent_identity_link(a, "old", "claude").unwrap();
    }
    let tab = new_tab(&state).await;
    let lim = || Some(failure("usage_limit", None, None));
    let idle = pane(&state, &tab, "idle", "Idle", "old", lim(), false).await;
    let busy = pane(&state, &tab, "busy", "Busy", "old", lim(), true).await;

    let r = switch::switch(&state, "", &switch_req("idle", "new", AfterTurn)).unwrap();
    assert_eq!((r.status.clone(), r.restarted), (switch::AccountSwitchStatus::Switched, 1));
    assert_eq!(linked_account(&state, "idle"), ["new"]);
    assert_eq!(stored_failure(&state, &idle), None, "the old account's limit goes with the move");
    let env = state.mstore.get::<Block>(&idle).unwrap().unwrap().meta.get("cmd:env").cloned();
    assert_eq!(env.and_then(|e| e.get("CLAUDE_CONFIG_DIR").cloned()), Some(json!("/tmp/new")), "the pane's own auth env follows the link");

    let r = switch::switch(&state, "busy", &switch_req("busy", "new", AfterTurn)).unwrap();
    assert_eq!(r.status, switch::AccountSwitchStatus::Queued);
    assert_eq!(linked_account(&state, "busy"), ["old"], "not moved under a turn");
    assert_eq!(pending_of(&state, &busy), Some(json!("new")));
    switch::apply_pending(&state);
    assert_eq!(linked_account(&state, "busy"), ["old"], "still in its turn");

    // Choosing the account it is on cancels the wait.
    let r = switch::switch(&state, "", &switch_req("busy", "old", AfterTurn)).unwrap();
    assert_eq!(r.status, switch::AccountSwitchStatus::Unchanged);
    assert_eq!(pending_of(&state, &busy), None);

    // Queue again; the turn ends; the queued switch applies.
    switch::switch(&state, "", &switch_req("busy", "new", AfterTurn)).unwrap();
    register_stub(&busy, BLOCK_CONTROLLER_PERSISTENT, STATUS_RUNNING, false);
    switch::apply_pending(&state);
    assert_eq!(linked_account(&state, "busy"), ["new"]);
    assert_eq!(pending_of(&state, &busy), None, "the wait is cleared once applied");

    // "now" doesn't wait for a turn.
    state.identity_store.agent_identity_link("busy", "old", "claude").unwrap();
    register_stub(&busy, BLOCK_CONTROLLER_PERSISTENT, STATUS_RUNNING, true);
    let r = switch::switch(&state, "", &switch_req("busy", "new", Now)).unwrap();
    assert_eq!(r.status, switch::AccountSwitchStatus::Switched);
    assert!(
        blockcontroller::get_controller(&busy).is_none_or(|c| !c.get_runtime_status().turn_active),
        "now ends the turn: a forced resync alone would only defer the restart to its end"
    );
    cleanup(&[&idle, &busy]);
}

/// An agent with no provider of its own launches with its bundle's, so its accounts are that provider's.
#[tokio::test]
async fn an_agent_with_no_provider_switches_among_its_bundles_accounts() {
    let state = test_state();
    for id in ["old", "new"] {
        state.id_store.identity_upsert(&account(id, "claude", "valid")).unwrap();
    }
    state
        .mstore
        .bundle_upsert(&crate::backend::storage::bundles::Bundle {
            id: "b1".into(),
            name: "b1".into(),
            description: String::new(),
            is_blank: false,
            is_global: false,
            provider: "claude".into(),
            model: String::new(),
            instructions: String::new(),
            instructions_by_provider: "{}".into(),
            context_files: "[]".into(),
            mcp_servers: "[]".into(),
            skills: "[]".into(),
            sort_order: 0,
            created_at: 0,
            updated_at: 0,
            is_system: false,
        })
        .unwrap();
    let mut def = crate::backend::storage::agents::test_agent_def("inherits", "Agent inherits", "", "host", 1, "");
    def.memory_id = "b1".into();
    state.mstore.agent_def_insert(&mut def).unwrap();
    state.identity_store.agent_identity_link("inherits", "old", "claude").unwrap();
    let r = switch::switch(&state, "", &switch_req("inherits", "new", switch::AccountSwitchWhen::AfterTurn)).unwrap();
    assert_eq!(r.status, switch::AccountSwitchStatus::Switched);
    assert_eq!(linked_account(&state, "inherits"), ["new"]);
}

/// A connection that is neither a registered agent nor the proven window can't switch anyone
/// (it would be recorded as the operator); an agent and the window can.
#[tokio::test]
async fn an_unproven_connection_cannot_switch_an_agents_account() {
    let state = test_state();
    for id in ["old", "new"] {
        state.id_store.identity_upsert(&account(id, "claude", "valid")).unwrap();
    }
    agent_def(&state, "idle");
    state.identity_store.agent_identity_link("idle", "old", "claude").unwrap();
    let req = json!({ "agent_id": "idle", "account": "new" });
    let e = call_with(&state, Default::default(), COMMAND_AGENT_ACCOUNT_SWITCH, req.clone()).await.unwrap_err();
    assert!(e.starts_with("FORBIDDEN") && e.contains("isn't the AgentMux window"), "{e}");
    assert_eq!(linked_account(&state, "idle"), ["old"], "nothing moved");
    call(&state, None, COMMAND_AGENT_ACCOUNT_SWITCH, req.clone()).await.unwrap();
    assert_eq!(linked_account(&state, "idle"), ["new"]);
    state.identity_store.agent_identity_link("idle", "old", "claude").unwrap();
    call(&state, Some("idle"), COMMAND_AGENT_ACCOUNT_SWITCH, req).await.unwrap();
    assert_eq!(linked_account(&state, "idle"), ["new"]);
}

/// The account must be a signed-in one of the agent's provider, named by id or by
/// the label the pane shows; a bare email two accounts share is refused (#4936 §3.3).
#[tokio::test]
async fn a_switch_names_the_account_by_id_or_numbered_label_and_refuses_what_cant_run() {
    use switch::AccountSwitchWhen::AfterTurn;
    let state = test_state();
    let with_email = |id: &str, status: &str, seq: Option<u32>| {
        let mut a = account(id, "claude", status);
        a.context = json!({ "email": "me@example.com" });
        if let Some(n) = seq {
            a.context["seq"] = json!(n);
        }
        a
    };
    for a in [with_email("one", "valid", Some(1)), with_email("two", "valid", Some(2)), with_email("out", "expired", Some(3)), account("gh", "github", "valid")] {
        state.id_store.identity_upsert(&a).unwrap();
    }
    agent_def(&state, "ag");
    state.identity_store.agent_identity_link("ag", "one", "claude").unwrap();

    let e = switch::switch(&state, "", &switch_req("ag", "me@example.com", AfterTurn)).unwrap_err();
    assert!(e.starts_with("UNKNOWN_ACCOUNT") && e.contains("me@example.com \u{b7}2"), "{e}");
    let e = switch::switch(&state, "", &switch_req("ag", "out", AfterTurn)).unwrap_err();
    assert!(e.starts_with("ACCOUNT_CANT_RUN"), "{e}");
    let e = switch::switch(&state, "", &switch_req("ag", "gh", AfterTurn)).unwrap_err();
    assert!(e.starts_with("UNKNOWN_ACCOUNT"), "another provider's account isn't offered: {e}");
    let e = switch::switch(&state, "", &switch::AgentAccountSwitchReq { agent_id: None, account: "two".into(), when: AfterTurn }).unwrap_err();
    assert!(e.contains("say which agent"), "{e}");

    let r = switch::switch(&state, "", &switch_req("ag", "me@example.com \u{b7}2", AfterTurn)).unwrap();
    assert_eq!((r.account_id.as_str(), r.status), ("two", switch::AccountSwitchStatus::Switched));
    assert_eq!(r.account_label, "me@example.com \u{b7}2");
    assert_eq!(linked_account(&state, "ag"), ["two"]);
}
