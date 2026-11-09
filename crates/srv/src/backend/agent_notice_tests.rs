// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::server::tests::test_state;
use crate::server::HostIpc;

/// A notice from a requester no other test shares.
fn notice(kind: Kind) -> Notice {
    let unique = uuid::Uuid::new_v4().simple().to_string();
    Notice {
        kind,
        block_id: format!("block-{unique}"),
        agent_id: format!("agent-{unique}"),
        agent: "Posa".to_string(),
        connection: String::new(),
        headline: "Posa is starting a new agent".to_string(),
        body: "Korp-2 on sonnet.".to_string(),
        quote: "Implement SPEC_X.".to_string(),
        always_label: "Always allow Posa to start agents".to_string(),
        waived: None,
    }
}

/// A state with a window connected: someone who could stop it.
async fn with_window() -> AppState {
    let state = test_state();
    *state.host_ipc.lock().await = Some(HostIpc { port: 1, token: "t".to_string() });
    state
}

async fn card_for(block_id: &str) -> NoticeView {
    for _ in 0..400 {
        if let Some(v) = pending().into_iter().find(|v| v.block_id == block_id) {
            return v;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("no card for {block_id}");
}

fn spawn_announce(state: &AppState, n: Notice, window: Duration) -> tokio::task::JoinHandle<Outcome> {
    let state = state.clone();
    tokio::spawn(async move { announce_for(&state, n, || true, window).await })
}

#[tokio::test]
async fn the_persons_own_turn_goes_ahead_with_a_line_and_no_card() {
    let state = with_window().await;
    let mut n = notice(Kind::Spawn);
    n.waived = Some(Waiver::UserTurn);
    let block = n.block_id.clone();
    assert_eq!(announce_for(&state, n, || true, Duration::from_secs(60)).await, Outcome::Go(Go::Waived(Waiver::UserTurn)));
    assert!(pending().iter().all(|v| v.block_id != block), "no card to answer");
}

#[tokio::test]
async fn with_no_window_connected_it_goes_ahead_at_once() {
    let state = test_state();
    let n = notice(Kind::Spawn);
    let block = n.block_id.clone();
    assert_eq!(announce_for(&state, n, || true, Duration::from_secs(60)).await, Outcome::Go(Go::Headless));
    assert!(pending().iter().all(|v| v.block_id != block));
}

#[tokio::test]
async fn nobody_acting_lets_it_go_ahead_when_the_countdown_ends() {
    let state = with_window().await;
    let n = notice(Kind::Spawn);
    let task = spawn_announce(&state, n.clone(), Duration::from_millis(80));
    let card = card_for(&n.block_id).await;
    assert_eq!((card.status, card.kind, card.agent.as_str(), card.quote.as_str()), ("pending", "spawn", "Posa", "Implement SPEC_X."));
    assert_eq!(card.always_label, "Always allow Posa to start agents");
    assert!(card.deadline_ms > card.opened_ms);
    assert_eq!(task.await.unwrap(), Outcome::Go(Go::Deadline));
    assert!(pending().iter().all(|v| v.block_id != n.block_id), "the card goes with the countdown");
}

#[tokio::test]
async fn stop_it_refuses_once_and_a_second_stop_is_too_late() {
    let state = with_window().await;
    let n = notice(Kind::Spawn);
    let task = spawn_announce(&state, n.clone(), Duration::from_secs(60));
    let card = card_for(&n.block_id).await;
    assert_eq!(decide(&state, &card.id, "veto").unwrap(), "stopped");
    assert_eq!(task.await.unwrap(), Outcome::Stopped);
    assert_eq!(decide(&state, &card.id, "veto").unwrap(), "too_late");
    assert_eq!(decide(&state, &card.id, "now").unwrap(), "too_late", "it ended");
}

#[tokio::test]
async fn start_now_skips_the_wait() {
    let state = with_window().await;
    let n = notice(Kind::Spawn);
    let task = spawn_announce(&state, n.clone(), Duration::from_secs(60));
    let card = card_for(&n.block_id).await;
    assert_eq!(decide(&state, &card.id, "now").unwrap(), "started");
    assert_eq!(task.await.unwrap(), Outcome::Go(Go::Now));
}

#[tokio::test]
async fn always_starts_it_and_remembers_the_agent_so_it_is_not_asked_again() {
    let state = with_window().await;
    let n = notice(Kind::Spawn);
    assert!(!agent_grants::allowed(Topic::Spawn, &n.agent_id));
    let task = spawn_announce(&state, n.clone(), Duration::from_secs(60));
    let card = card_for(&n.block_id).await;
    assert_eq!(decide(&state, &card.id, "always").unwrap(), "always");
    assert_eq!(task.await.unwrap(), Outcome::Go(Go::Always));
    assert!(agent_grants::allowed(Topic::Spawn, &n.agent_id));

    let again = Notice { block_id: format!("{}-again", n.block_id), ..n.clone() };
    assert_eq!(announce_for(&state, again.clone(), || true, Duration::from_secs(60)).await, Outcome::Go(Go::Granted), "no card, even with a window connected");
    assert!(pending().iter().all(|v| v.block_id != again.block_id));

    let other = Notice { agent_id: format!("{}-other", n.agent_id), ..notice(Kind::Spawn) };
    let task = spawn_announce(&state, other.clone(), Duration::from_millis(50));
    card_for(&other.block_id).await;
    assert_eq!(task.await.unwrap(), Outcome::Go(Go::Deadline), "another agent is still shown the card");

    assert!(agent_grants::revoke(Topic::Spawn, &n.agent_id).unwrap());
}

#[tokio::test]
async fn an_always_that_comes_too_late_saves_nothing() {
    let state = with_window().await;
    let n = notice(Kind::Spawn);
    let task = spawn_announce(&state, n.clone(), Duration::from_millis(40));
    let card = card_for(&n.block_id).await;
    assert_eq!(task.await.unwrap(), Outcome::Go(Go::Deadline));
    assert_eq!(decide(&state, &card.id, "always").unwrap(), "too_late");
    assert!(!agent_grants::allowed(Topic::Spawn, &n.agent_id), "the person was told too late, so the agent is still shown the card");
}

#[tokio::test]
async fn a_notice_that_offers_no_always_refuses_one() {
    let state = with_window().await;
    let n = Notice { always_label: String::new(), ..notice(Kind::Spawn) };
    let task = spawn_announce(&state, n.clone(), Duration::from_secs(60));
    let card = card_for(&n.block_id).await;
    assert_eq!(card.always_label, "");
    let err = decide(&state, &card.id, "always").unwrap_err();
    assert!(err.starts_with("INVALID_ARG"), "{err}");
    assert!(!agent_grants::allowed(Topic::Spawn, &n.agent_id));
    assert_eq!(decide(&state, &card.id, "veto").unwrap(), "stopped");
    task.await.unwrap();
}

#[tokio::test]
async fn a_requester_that_is_gone_ends_it_without_starting_anything() {
    let state = with_window().await;
    let n = notice(Kind::Spawn);
    let alive = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let flag = alive.clone();
    let task = {
        let (state, n) = (state.clone(), n.clone());
        tokio::spawn(async move { announce_for(&state, n, move || flag.load(std::sync::atomic::Ordering::SeqCst), Duration::from_secs(60)).await })
    };
    card_for(&n.block_id).await;
    alive.store(false, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(task.await.unwrap(), Outcome::Gone);
    assert!(pending().iter().all(|v| v.block_id != n.block_id));
}

#[tokio::test]
async fn a_request_dropped_while_it_waits_takes_its_card_with_it() {
    let state = with_window().await;
    let n = notice(Kind::Spawn);
    let task = spawn_announce(&state, n.clone(), Duration::from_secs(60));
    let card = card_for(&n.block_id).await;
    task.abort();
    let _ = task.await;
    assert!(pending().iter().all(|v| v.block_id != n.block_id), "the card is cleared");
    assert_eq!(decide(&state, &card.id, "veto").unwrap(), "too_late");
}

#[tokio::test]
async fn the_choice_is_refused_unless_it_is_one_of_the_three() {
    let state = test_state();
    assert!(decide(&state, "nothing", "maybe").unwrap_err().starts_with("INVALID_ARG"));
    assert_eq!(decide(&state, "nothing", "veto").unwrap(), "too_late");
}

mod rpc {
    use super::*;
    use crate::backend::rpc_types::{RpcContext, RpcMessage};

    struct Harness {
        engine: Arc<WshRpcEngine>,
        rx: tokio::sync::mpsc::UnboundedReceiver<RpcMessage>,
        state: AppState,
    }

    async fn harness(ctx: RpcContext) -> Harness {
        let state = with_window().await;
        let (engine, rx) = WshRpcEngine::new();
        register(&engine, &state);
        engine.set_rpc_context(ctx);
        Harness { engine, rx, state }
    }

    impl Harness {
        async fn call(&mut self, command: &str, data: serde_json::Value) -> Result<serde_json::Value, String> {
            self.engine.handle_message(RpcMessage { command: command.to_string(), reqid: "req-1".to_string(), data: Some(data), ..Default::default() });
            let resp = tokio::time::timeout(Duration::from_secs(10), self.rx.recv()).await.unwrap().unwrap();
            if resp.error.is_empty() {
                Ok(resp.data.unwrap_or_default())
            } else {
                Err(resp.error)
            }
        }
    }

    #[tokio::test]
    async fn the_proven_window_stops_and_grants() {
        let mut h = harness(RpcContext::operator_for_tests()).await;
        let n = notice(Kind::Spawn);
        let task = spawn_announce(&h.state, n.clone(), Duration::from_secs(60));
        let card = card_for(&n.block_id).await;
        let out = h.call(COMMAND_AGENT_NOTICE_DECIDE, serde_json::json!({ "request_id": card.id, "choice": "veto" })).await.unwrap();
        assert_eq!(out["outcome"], "stopped");
        assert_eq!(task.await.unwrap(), Outcome::Stopped);

        let out = h.call(COMMAND_AGENT_NOTICE_SET_GRANT, serde_json::json!({ "topic": "spawn", "agent_id": n.agent_id, "allowed": true })).await.unwrap();
        assert!(out["agents"].as_array().unwrap().iter().any(|a| a == &n.agent_id.to_ascii_lowercase()));
        let out = h.call(COMMAND_AGENT_NOTICE_GRANTS, serde_json::json!({ "topic": "spawn" })).await.unwrap();
        assert!(out["agents"].as_array().unwrap().iter().any(|a| a == &n.agent_id.to_ascii_lowercase()));
        h.call(COMMAND_AGENT_NOTICE_SET_GRANT, serde_json::json!({ "topic": "spawn", "agent_id": n.agent_id, "allowed": false })).await.unwrap();
        assert!(!agent_grants::allowed(Topic::Spawn, &n.agent_id));
    }

    #[tokio::test]
    async fn an_agent_connection_can_neither_stop_hurry_nor_grant() {
        let mut h = harness(RpcContext { agent_id: "posa".to_string(), ..Default::default() }).await;
        let n = notice(Kind::Spawn);
        let task = spawn_announce(&h.state, n.clone(), Duration::from_secs(60));
        let card = card_for(&n.block_id).await;
        for choice in ["veto", "now", "always"] {
            let err = h.call(COMMAND_AGENT_NOTICE_DECIDE, serde_json::json!({ "request_id": card.id, "choice": choice })).await.unwrap_err();
            assert!(err.contains("FORBIDDEN"), "{err}");
        }
        let err = h.call(COMMAND_AGENT_NOTICE_SET_GRANT, serde_json::json!({ "topic": "spawn", "agent_id": n.agent_id, "allowed": true })).await.unwrap_err();
        assert!(err.contains("FORBIDDEN"), "{err}");
        assert!(!agent_grants::allowed(Topic::Spawn, &n.agent_id), "an agent can't grant itself or another");
        assert!(pending().iter().any(|v| v.id == card.id), "the countdown is untouched");
        decide(&h.state, &card.id, "veto").unwrap();
        task.await.unwrap();
    }

    #[tokio::test]
    async fn a_connection_the_host_never_proved_is_refused() {
        let mut h = harness(RpcContext::default()).await;
        let err = h.call(COMMAND_AGENT_NOTICE_DECIDE, serde_json::json!({ "request_id": "x", "choice": "veto" })).await.unwrap_err();
        assert!(err.contains("FORBIDDEN"), "{err}");
    }
}
