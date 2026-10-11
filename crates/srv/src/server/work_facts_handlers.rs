// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! HTTP side of agents' work facts
//! (docs/specs/SPEC_AGENT_OVERLAP_AWARENESS_2026_10_10.md §3.1, §3.2):
//!
//! - `GET /api/v1/work-facts`: this srv's own agents' facts. Other channels
//!   on this computer call it; it never fans out itself.
//! - `GET /api/v1/work-facts/who`: the `WhoIsWorkingOn` MCP tool. This srv's
//!   agents plus every other channel's (through the host-global registry,
//!   the same reach `ListConversations` has), matched against the question.
//!   LAN and WAN agents are listed by name only: their facts stay on their
//!   own computer.
//!
//! Also adds the facts to `ListConversations` entries ([`annotate_conversations`]).

use std::collections::HashMap;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use serde::Deserialize;
use serde_json::json;

use crate::backend::reactive::types::AgentRegistration;
use crate::backend::work_facts::cross_channel::{self, own_channel};
use crate::backend::work_facts::{self, matching, WorkFacts};

use super::caller::Caller;
use super::AppState;

/// Shown with every answer, so an agent reads it as information.
const NOTE: &str = "Informational only: nothing is locked or blocked. If someone else is changing the same \
                    thing, message them (SendMessage) before you change it.";

/// This srv's own agents' facts. Store reads, so off the async workers.
async fn local_facts(state: &AppState) -> Vec<WorkFacts> {
    let regs = state.reactive_handler.list_agents();
    let store = state.mstore.clone();
    let channel = own_channel();
    tokio::task::spawn_blocking(move || work_facts::collect_local(&store, &regs, &channel))
        .await
        .unwrap_or_default()
}

/// Every other channel's agents' facts, plus the channels that didn't answer.
async fn cross_channel_facts(state: &AppState) -> (Vec<WorkFacts>, Vec<String>) {
    cross_channel::fetch(&state.http_client, &state.local_web_url).await
}

/// `GET /api/v1/work-facts`: this srv's own agents' work facts.
pub async fn handle_work_facts(State(state): State<AppState>) -> impl IntoResponse {
    Json(json!({ "channel": own_channel(), "agents": local_facts(&state).await }))
}

/// `WhoIsWorkingOn`'s arguments, plus who is asking (`agent`, `block`: the
/// MCP's own `AGENTMUX_AGENT_ID` and block id; the request's agent token wins
/// over both).
#[derive(Debug, Default, Deserialize)]
pub struct WhoParams {
    pub path: Option<String>,
    pub repo: Option<String>,
    pub branch: Option<String>,
    pub query: Option<String>,
    pub agent: Option<String>,
    pub block: Option<String>,
}

/// The caller's own block: by its token's UID, else the block id it sent,
/// else its name when exactly one registered agent has it. Only used to
/// leave the caller out and to read its own repository, so a name is enough.
fn caller_block(regs: &[AgentRegistration], uid: Option<&str>, block: Option<&str>, agent: Option<&str>) -> Option<String> {
    if let Some(uid) = uid {
        if let Some(r) = regs.iter().find(|r| r.uid.as_deref() == Some(uid)) {
            return Some(r.block_id.clone());
        }
    }
    let block = block.map(str::trim).filter(|b| !b.is_empty());
    if let Some(r) = block.and_then(|b| regs.iter().find(|r| r.block_id == b)) {
        return Some(r.block_id.clone());
    }
    let agent = agent.map(str::trim).filter(|a| !a.is_empty())?;
    let mut named = regs.iter().filter(|r| r.agent_id.eq_ignore_ascii_case(agent));
    match (named.next(), named.next()) {
        (Some(r), None) => Some(r.block_id.clone()),
        _ => None,
    }
}

/// Take the caller's own facts out of `facts`.
fn split_caller(facts: Vec<WorkFacts>, caller_block: Option<&str>) -> (Option<WorkFacts>, Vec<WorkFacts>) {
    let (mine, others): (Vec<_>, Vec<_>) =
        facts.into_iter().partition(|f| caller_block.is_some_and(|b| f.block_id == b));
    (mine.into_iter().next(), others)
}

/// LAN and WAN agents: name and liveness only.
fn remote_agents(state: &AppState) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    for lan in state.lan_discovery.get_instances() {
        for name in &lan.agents {
            out.push(json!({ "agent": name, "channel": "lan", "status": "online" }));
        }
    }
    if let Some(sub) = crate::muxbus::cloud_subscriber::get_global_subscriber() {
        for name in sub.subscribed_agents() {
            out.push(json!({ "agent": name, "channel": "wan", "status": "online" }));
        }
    }
    out
}

/// `GET /api/v1/work-facts/who`: `WhoIsWorkingOn`.
pub(crate) async fn handle_who_is_working_on(
    State(state): State<AppState>,
    caller: Option<axum::Extension<Caller>>,
    Query(p): Query<WhoParams>,
) -> Response {
    let regs = state.reactive_handler.list_agents();
    let uid = caller.as_ref().and_then(|c| c.0.uid().map(str::to_string));
    let me_block = caller_block(&regs, uid.as_deref(), p.block.as_deref(), p.agent.as_deref());
    let (local, (cross, unreachable)) = tokio::join!(local_facts(&state), cross_channel_facts(&state));
    let (me, mut others) = split_caller(local, me_block.as_deref());
    others.extend(cross);

    let q = matching::WhoQuery { path: p.path, repo: p.repo, branch: p.branch, query: p.query };
    let target = match matching::resolve(&q, me.as_ref(), &others) {
        Ok(t) => t,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response(),
    };
    let agents = matching::who_is_working_on(&target, &others);
    Json(json!({
        "asked": target,
        "caller": me.as_ref().map(|m| json!({ "agent": m.agent, "repo": m.repo, "branch": m.branch })),
        "agents": agents,
        "remote_agents": remote_agents(&state),
        "unreachable_channels": unreachable,
        "note": NOTE,
    }))
    .into_response()
}

/// Start gathering every agent's facts for `ListConversations`, this srv's
/// and the other channels' at once, so the handler can run its own preview
/// fan-out meanwhile rather than paying a second round after it.
pub(super) fn start_conversation_facts(state: &AppState) -> tokio::task::JoinHandle<Vec<WorkFacts>> {
    let state = state.clone();
    tokio::spawn(async move {
        let (mut local, (cross, _)) = tokio::join!(local_facts(&state), cross_channel_facts(&state));
        local.extend(cross);
        local
    })
}

/// Add a `work` field (see `work_facts::conversation_summary`) to every
/// host and cross-channel `ListConversations` entry that names its block,
/// from the facts [`start_conversation_facts`] gathered. If gathering failed
/// the entries are left as they were.
pub(super) async fn annotate_conversations(
    facts: tokio::task::JoinHandle<Vec<WorkFacts>>,
    entries: &mut [serde_json::Value],
) {
    let facts = facts.await.unwrap_or_default();
    let by_block: HashMap<&str, &WorkFacts> = facts.iter().map(|f| (f.block_id.as_str(), f)).collect();
    for e in entries.iter_mut() {
        let Some(f) = e.get("block_id").and_then(|b| b.as_str()).and_then(|b| by_block.get(b)) else {
            continue;
        };
        let summary = work_facts::conversation_summary(f);
        if let Some(obj) = e.as_object_mut() {
            obj.insert("work".to_string(), summary);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg(agent: &str, block: &str, uid: Option<&str>) -> AgentRegistration {
        AgentRegistration {
            agent_id: agent.into(),
            block_id: block.into(),
            tab_id: None,
            uid: uid.map(Into::into),
            registered_at: 0,
            last_seen: 0,
            registration_nonce: 0,
        }
    }

    #[test]
    fn the_caller_is_found_by_token_then_block_then_unique_name() {
        let regs = vec![reg("AgentY", "b1", Some("u1")), reg("Agent2", "b2", None), reg("Agent2", "b3", None)];
        assert_eq!(caller_block(&regs, Some("u1"), Some("b2"), None).as_deref(), Some("b1"));
        assert_eq!(caller_block(&regs, None, Some("b2"), Some("AgentY")).as_deref(), Some("b2"));
        assert_eq!(caller_block(&regs, None, None, Some("agenty")).as_deref(), Some("b1"));
        assert_eq!(caller_block(&regs, None, None, Some("Agent2")), None, "an ambiguous name names no one");
        assert_eq!(caller_block(&regs, None, Some("gone"), None), None);
    }

    /// The caller is left out of the answer, and its own facts are what
    /// a relative path resolves against.
    #[test]
    fn the_caller_is_excluded_and_its_repository_is_assumed() {
        let fact = |agent: &str, block: &str, root: &str| WorkFacts {
            agent: agent.into(),
            block_id: block.into(),
            repo: Some("o/r".into()),
            repo_root: Some(root.into()),
            dirty_files: vec!["a.rs".into()],
            ..Default::default()
        };
        let (me, others) = split_caller(vec![fact("Me", "b1", "/me"), fact("Other", "b2", "/o")], Some("b1"));
        assert_eq!(me.as_ref().map(|m| m.agent.as_str()), Some("Me"));
        let q = matching::WhoQuery { path: Some("a.rs".into()), ..Default::default() };
        let t = matching::resolve(&q, me.as_ref(), &others).unwrap();
        let names: Vec<String> = matching::who_is_working_on(&t, &others).into_iter().map(|r| r.agent).collect();
        assert_eq!(names, vec!["Other".to_string()]);

        let (none, all) = split_caller(vec![fact("A", "b1", "/a")], None);
        assert!(none.is_none());
        assert_eq!(all.len(), 1, "an unknown caller excludes no one");
    }

    #[tokio::test]
    async fn the_facts_endpoint_lists_this_srvs_agents() {
        let state = crate::server::tests::test_state();
        let unique = uuid::Uuid::new_v4();
        let (agent, block) = (format!("facts-agent-{unique}"), format!("facts-block-{unique}"));
        state.reactive_handler.register_agent(&agent, &block, None).unwrap();
        let resp = handle_work_facts(State(state)).await.into_response();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let agents: Vec<WorkFacts> = serde_json::from_value(body["agents"].clone()).unwrap();
        let mine = agents.iter().find(|f| f.agent == agent).expect("registered agent listed");
        assert_eq!(mine.block_id, block);
        assert_eq!(mine.status, "stopped", "no controller in a test");
    }

    #[tokio::test]
    async fn who_explains_a_question_it_cannot_answer() {
        let state = crate::server::tests::test_state();
        let resp = handle_who_is_working_on(
            State(state),
            None,
            Query(WhoParams { path: Some("relative/x.rs".into()), ..Default::default() }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn conversations_entries_gain_a_work_field_by_block() {
        let state = crate::server::tests::test_state();
        let unique = uuid::Uuid::new_v4();
        let (agent, block) = (format!("conv-agent-{unique}"), format!("conv-block-{unique}"));
        state.reactive_handler.register_agent(&agent, &block, None).unwrap();
        let mut entries = vec![
            json!({ "name": agent, "tier": "host", "block_id": block }),
            json!({ "name": "remote", "tier": "lan" }),
        ];
        annotate_conversations(start_conversation_facts(&state), &mut entries).await;
        assert!(entries[0]["work"].is_object(), "{}", entries[0]);
        assert_eq!(entries[0]["work"]["dirty_count"], 0);
        assert!(entries[1].get("work").is_none(), "LAN entries carry no facts");
    }
}
