// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The messages srv holds for agents, as counts: `jekt.held.summary` answers per agent how many and why and since when
//! the oldest was sent, and the `jektheld` event tells a window the counts changed, so the status bar's summary
//! (`frontend/app/statusbar`) shows them without polling. Counts and reasons only: the hold table's bodies, sender names
//! and message ids stay where they are (SPEC_JEKT_DELIVERY_STATES_AND_MAILBOX_2026_10_01.md, §5.1 for the reasons).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::backend::agent_runtime_state::AgentRuntimeState;
use crate::backend::cold_hold::{REASON_IDLE, REASON_QUOTA};
use crate::backend::jekt_held_summary::{
    JektHeldAgent, JektHeldEvent, JektHeldSummaryResult, REASON_NEEDS_LOGIN, REASON_NOT_RUNNING, REASON_STARTING,
    REASON_USAGE_LIMIT,
};
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{NoArgsReq, COMMAND_JEKT_HELD_SUMMARY};
use crate::backend::storage::jekt_held::HeldSummaryRow;

use super::AppState;

/// How often the hold table is looked at for a change in its counts. A read of a few grouped rows.
const WATCH_EVERY: Duration = Duration::from_secs(5);

/// Why a message is held now: the reason stored with it (idle, quota), else what the agent's own state says. An agent
/// this machine has no definition of is a peer's, whose machine is not answering.
fn reason_for(stored: &str, state: Option<AgentRuntimeState>) -> &'static str {
    if stored == REASON_IDLE {
        return REASON_IDLE;
    }
    if stored == REASON_QUOTA {
        return REASON_QUOTA;
    }
    match state {
        Some(AgentRuntimeState::Limited) => REASON_USAGE_LIMIT,
        Some(AgentRuntimeState::SignedOut) => REASON_NEEDS_LOGIN,
        Some(AgentRuntimeState::Stopped | AgentRuntimeState::Error) => REASON_NOT_RUNNING,
        // Running or coming up: the replay delivers it within its next pass.
        Some(_) => REASON_STARTING,
        None => "peer_offline",
    }
}

/// The held counts as the RPC answers them. `known`: each local agent's UID with its name and state.
pub(crate) fn summarize(rows: &[HeldSummaryRow], known: &HashMap<String, (String, AgentRuntimeState)>) -> JektHeldSummaryResult {
    let mut agents: Vec<JektHeldAgent> = Vec::new();
    for r in rows {
        let local = known.get(&r.target_uid);
        let reason = reason_for(&r.hold_reason, local.map(|(_, s)| *s));
        let name = local.map(|(n, _)| n.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| r.target_agent.clone());
        let since_ms = r.oldest_sent_at_ms.max(0) as u64;
        let count = r.count.max(0) as u32;
        // Two stored reasons of one agent can read as one now (the agent stopped under both): one line.
        match agents.iter_mut().find(|a| a.agent_id == r.target_uid && a.reason == reason) {
            Some(a) => {
                a.count += count;
                a.since_ms = a.since_ms.min(since_ms);
            }
            None => agents.push(JektHeldAgent { agent_id: r.target_uid.clone(), agent_name: name, reason: reason.to_string(), count, since_ms }),
        }
    }
    agents.sort_by_key(|a| a.since_ms);
    JektHeldSummaryResult { agents }
}

/// The totals the event carries.
fn totals(rows: &[HeldSummaryRow]) -> JektHeldEvent {
    let mut uids: Vec<&str> = rows.iter().map(|r| r.target_uid.as_str()).collect();
    uids.sort_unstable();
    uids.dedup();
    JektHeldEvent { messages: rows.iter().map(|r| r.count.max(0) as u32).sum(), agents: uids.len() as u32 }
}

/// The answer, from the hold table and the agents' states. BLOCKING.
fn read(state: &AppState) -> JektHeldSummaryResult {
    let rows = state.mstore.jekt_held_summary().unwrap_or_default();
    if rows.is_empty() {
        return JektHeldSummaryResult { agents: Vec::new() };
    }
    let now = agentmux_common::time::now_ms_u64();
    let known: HashMap<String, (String, AgentRuntimeState)> = super::agent_runtime::list(state, now)
        .into_iter()
        .map(|a| (a.agent_id, (a.name, a.state)))
        .collect();
    summarize(&rows, &known)
}

/// `jekt.held.summary`: the operator's window and any registered agent may read it: it names agents and counts, never a
/// sender, a body or an id. A connection that is neither (it holds only the instance key) is refused, as for
/// `identity.account.availability`.
pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let s = state.clone();
    engine.register_typed(COMMAND_JEKT_HELD_SUMMARY, move |_req: Option<NoArgsReq>, ctx| {
        let s = s.clone();
        async move {
            if ctx.agent_id.is_empty() {
                ctx.operator_only("only the AgentMux window or an agent sees which agents have messages held")?;
            }
            tokio::task::spawn_blocking(move || read(&s)).await.map_err(|e| e.to_string())
        }
    });
}

/// Publish `jektheld` whenever the hold table's counts change. Call once at startup.
pub(crate) fn install(state: &AppState) {
    if tokio::runtime::Handle::try_current().is_err() {
        return;
    }
    let state = state.clone();
    tokio::spawn(async move {
        let mut last: Option<Vec<HeldSummaryRow>> = None;
        loop {
            let mstore = state.mstore.clone();
            if let Ok(rows) = tokio::task::spawn_blocking(move || mstore.jekt_held_summary().unwrap_or_default()).await {
                // Nothing held at the start is no change; every change after is.
                let changed = match &last {
                    Some(prev) => *prev != rows,
                    None => !rows.is_empty(),
                };
                if changed {
                    state.broker.publish_event::<crate::backend::mps_events::JektHeld>(&totals(&rows), vec![], 0);
                }
                last = Some(rows);
            }
            tokio::time::sleep(WATCH_EVERY).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::rpc_types::{RpcContext, RpcMessage};

    fn row(uid: &str, reason: &str, count: i64, oldest: i64) -> HeldSummaryRow {
        HeldSummaryRow { target_uid: uid.into(), target_agent: format!("name-of-{uid}"), hold_reason: reason.into(), count, oldest_sent_at_ms: oldest }
    }

    fn known(list: &[(&str, &str, AgentRuntimeState)]) -> HashMap<String, (String, AgentRuntimeState)> {
        list.iter().map(|(u, n, s)| (u.to_string(), (n.to_string(), *s))).collect()
    }

    #[test]
    fn the_reason_is_what_the_agents_state_says_unless_the_hold_stored_one() {
        let k = known(&[
            ("u-stopped", "Stopped", AgentRuntimeState::Stopped),
            ("u-out", "Out", AgentRuntimeState::SignedOut),
            ("u-lim", "Limited", AgentRuntimeState::Limited),
            ("u-run", "Running", AgentRuntimeState::Working),
            ("u-cold", "Cold", AgentRuntimeState::Idle),
        ]);
        let got = summarize(
            &[row("u-stopped", "", 2, 10), row("u-out", "", 1, 20), row("u-lim", "", 3, 30), row("u-run", "", 1, 40), row("u-cold", REASON_IDLE, 1, 50), row("u-peer", "", 4, 60)],
            &k,
        );
        let by: Vec<(&str, &str, u32)> = got.agents.iter().map(|a| (a.agent_id.as_str(), a.reason.as_str(), a.count)).collect();
        assert_eq!(
            by,
            vec![
                ("u-stopped", "not_running", 2),
                ("u-out", "needs_login", 1),
                ("u-lim", "usage_limit", 3),
                ("u-run", "starting", 1),
                ("u-cold", "idle", 1),
                ("u-peer", "peer_offline", 4),
            ]
        );
        assert_eq!(got.agents[0].agent_name, "Stopped", "the local agent's own name wins");
        assert_eq!(got.agents[5].agent_name, "name-of-u-peer", "an agent of another machine keeps the name the hold recorded");
    }

    #[test]
    fn two_stored_reasons_that_read_the_same_now_are_one_line_with_the_older_since() {
        let k = known(&[("u", "Agent", AgentRuntimeState::Stopped)]);
        let got = summarize(&[row("u", "", 2, 300), row("u", "other", 1, 100)], &k);
        assert_eq!(got.agents.len(), 1);
        assert_eq!((got.agents[0].count, got.agents[0].since_ms), (3, 100));
    }

    #[test]
    fn the_event_carries_only_totals() {
        let e = totals(&[row("a", "", 2, 1), row("a", REASON_QUOTA, 1, 2), row("b", "", 4, 3)]);
        assert_eq!((e.messages, e.agents), (7, 2));
        assert_eq!((totals(&[]).messages, totals(&[]).agents), (0, 0));
    }

    async fn call(state: &AppState, ctx: RpcContext) -> Result<serde_json::Value, String> {
        let (engine, mut rx) = WshRpcEngine::new();
        register(&engine, state);
        engine.set_rpc_context(ctx);
        engine.handle_message(RpcMessage { command: COMMAND_JEKT_HELD_SUMMARY.to_string(), reqid: "req-1".to_string(), data: Some(serde_json::json!({})), ..Default::default() });
        let resp = tokio::time::timeout(Duration::from_secs(10), rx.recv()).await.unwrap().unwrap();
        if resp.error.is_empty() { Ok(resp.data.unwrap_or_default()) } else { Err(resp.error) }
    }

    #[tokio::test]
    async fn the_rpc_answers_counts_and_reasons_and_never_a_body_a_sender_or_an_id() {
        let state = crate::server::tests::test_state();
        assert_eq!(call(&state, RpcContext::operator_for_tests()).await.unwrap(), serde_json::json!({"agents": []}));
        assert!(call(&state, RpcContext::default()).await.is_err(), "a connection that is neither the window nor an agent is refused");
        let req = crate::backend::reactive::types::InjectionRequest {
            target_agent: "ghost".to_string(),
            message: "a secret body".to_string(),
            source_agent: Some("a-secret-sender".to_string()),
            request_id: Some("secret-request-id".to_string()),
            ..Default::default()
        };
        let mut held = crate::server::jekt_hold::held_row(&req, "secret-request-id".to_string(), 1_000);
        held.target_uid = "uid-ghost".to_string();
        state.mstore.jekt_held_insert(&held).unwrap();
        let mut second = held.clone();
        second.request_id = "second-id".to_string();
        second.sent_at_ms = 2_000;
        state.mstore.jekt_held_insert(&second).unwrap();
        for ctx in [RpcContext::operator_for_tests(), RpcContext { agent_id: "some-agent".to_string(), ..Default::default() }] {
            let answer = call(&state, ctx).await.unwrap();
            let text = answer.to_string();
            for secret in ["a secret body", "a-secret-sender", "secret-request-id", "second-id"] {
                assert!(!text.contains(secret), "{secret} left the hold table: {text}");
            }
            let agents = answer["agents"].as_array().unwrap();
            assert_eq!(agents.len(), 1, "{text}");
            assert_eq!(agents[0]["agent_id"], "uid-ghost");
            assert_eq!(agents[0]["count"], 2);
            assert_eq!(agents[0]["since_ms"], 1_000);
            assert_eq!(agents[0]["reason"], "peer_offline", "an agent this machine has no definition of is a peer's");
        }
    }
}
