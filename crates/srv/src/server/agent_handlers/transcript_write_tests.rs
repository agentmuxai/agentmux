// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Who may write an agent's transcript or session state over the WebSocket: its
//! own agent, or the AgentMux window, and nobody else. Every agent holds the
//! instance auth key, so a plain connection that never registered proves nothing.

use std::sync::Arc;

use super::*;
use crate::backend::agent_session::session_io::read_session_state;
use crate::backend::rpc_types::{
    RpcContext, RpcMessage, COMMAND_AGENT_SESSION_APPEND_OUTPUT, COMMAND_AGENT_SESSION_ARCHIVE,
    COMMAND_AGENT_SESSION_WRITE_STATE, COMMAND_APPEND_AGENT_HISTORY,
};
use crate::server::tests::test_state;

struct Harness {
    state: AppState,
    engine: Arc<WshRpcEngine>,
    rx: tokio::sync::mpsc::UnboundedReceiver<RpcMessage>,
    /// Definition ids of the two agents (`a` and `b`).
    a: String,
    b: String,
}

/// A connection described by `ctx(slug of a)`. Two agents exist, `a` and `b`,
/// each with a slug that differs from its definition id. Session state is also
/// mirrored into a store shared by the whole test process, so `tag` makes the
/// ids unique to one test.
fn harness(tag: &str, ctx: impl FnOnce(&str) -> RpcContext) -> Harness {
    let state = test_state();
    let mut ids = Vec::new();
    for who in ["a", "b"] {
        let id = format!("uid-{who}-{tag}");
        let mut def = crate::backend::storage::agents::test_agent_def(&id, &format!("agent {who}"), "claude", "host", 1, "");
        def.slug = format!("agent-{who}-{tag}");
        state.mstore.agent_def_insert(&mut def).unwrap();
        ids.push(id);
    }
    let (engine, rx) = WshRpcEngine::new();
    super::session::register(&engine, &state);
    super::history::register(&engine, &state);
    engine.set_rpc_context(ctx(&format!("agent-a-{tag}")));
    let b = ids.pop().unwrap();
    let a = ids.pop().unwrap();
    Harness { state, engine, rx, a, b }
}

fn as_agent(slug: &str) -> RpcContext {
    RpcContext { agent_id: slug.to_string(), ..Default::default() }
}

impl Harness {
    async fn call(&mut self, command: &str, data: serde_json::Value) -> Result<serde_json::Value, String> {
        self.engine.handle_message(RpcMessage {
            command: command.to_string(),
            reqid: "req-1".to_string(),
            data: Some(data),
            ..Default::default()
        });
        let resp = tokio::time::timeout(std::time::Duration::from_secs(10), self.rx.recv()).await.unwrap().unwrap();
        if resp.error.is_empty() {
            Ok(resp.data.unwrap_or_default())
        } else {
            Err(resp.error)
        }
    }

    fn saved_state(&self, definition_id: &str) -> Option<String> {
        read_session_state(&self.state.filestore, definition_id).ok().and_then(|(content, _)| content)
    }
}

fn write_state(definition_id: &str, content: &str) -> serde_json::Value {
    serde_json::json!({ "definition_id": definition_id, "content": content })
}

fn append_output(definition_id: &str) -> serde_json::Value {
    serde_json::json!({ "definition_id": definition_id, "line": "x" })
}

fn append_history(agent_id: &str) -> serde_json::Value {
    serde_json::json!({ "agent_id": agent_id, "entry": "x" })
}

fn archive(definition_id: &str) -> serde_json::Value {
    serde_json::json!({ "definition_id": definition_id })
}

/// The window may write any agent's session state.
#[tokio::test]
async fn the_window_writes_any_agents_session_state() {
    let mut h = harness("window", |_| RpcContext::operator_for_tests());
    let a = h.a.clone();
    h.call(COMMAND_AGENT_SESSION_WRITE_STATE, write_state(&a, "from the window")).await.unwrap();
    assert_eq!(h.saved_state(&a).as_deref(), Some("from the window"));
}

/// A plain connection (it never registered as an agent, and the host never
/// vouched for it) is refused, and nothing is written.
#[tokio::test]
async fn a_connection_that_is_neither_an_agent_nor_the_window_is_refused() {
    let mut h = harness("plain", |_| RpcContext::default());
    let a = h.a.clone();
    for (command, data) in [
        (COMMAND_AGENT_SESSION_WRITE_STATE, write_state(&a, "forged")),
        (COMMAND_AGENT_SESSION_APPEND_OUTPUT, append_output(&a)),
        (COMMAND_APPEND_AGENT_HISTORY, append_history(&a)),
        (COMMAND_AGENT_SESSION_ARCHIVE, archive(&a)),
    ] {
        let e = h.call(command, data).await.unwrap_err();
        assert!(e.starts_with("FORBIDDEN"), "{command}: {e}");
    }
    assert_eq!(h.saved_state(&a), None);
    assert!(h.state.mstore.agent_history_list(&a, None, 10, 0).unwrap().is_empty());
}

/// An agent's own connection writes its own agent, and no other agent.
#[tokio::test]
async fn an_agent_writes_only_its_own_transcript() {
    let mut h = harness("agent", as_agent);
    let (a, b) = (h.a.clone(), h.b.clone());

    h.call(COMMAND_AGENT_SESSION_WRITE_STATE, write_state(&a, "mine")).await.unwrap();
    assert_eq!(h.saved_state(&a).as_deref(), Some("mine"));
    h.call(COMMAND_APPEND_AGENT_HISTORY, append_history(&a)).await.unwrap();
    h.call(COMMAND_AGENT_SESSION_APPEND_OUTPUT, append_output(&a)).await.unwrap();

    for (command, data) in [
        (COMMAND_AGENT_SESSION_WRITE_STATE, write_state(&b, "not mine")),
        (COMMAND_AGENT_SESSION_APPEND_OUTPUT, append_output(&b)),
        (COMMAND_APPEND_AGENT_HISTORY, append_history(&b)),
        (COMMAND_AGENT_SESSION_ARCHIVE, archive(&b)),
    ] {
        let e = h.call(command, data).await.unwrap_err();
        assert!(e.contains("FORBIDDEN") && e.contains("not by another agent"), "{command}: {e}");
    }
    assert_eq!(h.saved_state(&b), None);
    assert!(h.state.mstore.agent_history_list(&b, None, 10, 0).unwrap().is_empty());
}
