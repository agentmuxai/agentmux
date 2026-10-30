// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Who may archive or restore a pane's transcript: the agent running on that pane, or the AgentMux
//! window, and nobody else. Restore replaces the transcript with a file whose path block meta holds,
//! so letting any connection call it would let one agent put words in another's record.

use super::*;
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{RpcContext, RpcMessage, COMMAND_SESSION_ARCHIVE, COMMAND_SESSION_RESTORE};
use crate::backend::storage::agents::test_agent_def;
use crate::server::tests::test_state;

/// A state with agents `own-a` and `own-b`, and a pane running `own-a`.
fn state_with_a_pane() -> (AppState, String) {
    let state = test_state();
    for (id, slug) in [("uid-own-a", "own-a"), ("uid-own-b", "own-b")] {
        let mut def = test_agent_def(id, slug, "claude", "host", 1, "");
        def.slug = slug.to_string();
        state.mstore.agent_def_insert(&mut def).unwrap();
    }
    let mut block = crate::backend::obj::Block {
        oid: "own-block".to_string(),
        parentoref: String::new(),
        version: 0,
        runtimeopts: None,
        stickers: None,
        meta: {
            let mut m = crate::backend::obj::MetaMapType::new();
            m.insert("view".to_string(), serde_json::json!("agent"));
            m.insert("agentId".to_string(), serde_json::json!("uid-own-a"));
            m
        },
        subblockids: None,
    };
    state.mstore.insert(&mut block).unwrap();
    (state, "own-block".to_string())
}

async fn call(state: &AppState, ctx: RpcContext, command: &str, block_id: &str) -> Result<serde_json::Value, String> {
    let (engine, mut rx) = WshRpcEngine::new();
    super::session::register(&engine, state);
    engine.set_rpc_context(ctx);
    engine.handle_message(RpcMessage {
        command: command.to_string(),
        reqid: "req-1".to_string(),
        data: Some(serde_json::json!({ "block_id": block_id })),
        ..Default::default()
    });
    let resp = tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv()).await.unwrap().unwrap();
    if resp.error.is_empty() {
        Ok(resp.data.unwrap_or_default())
    } else {
        Err(resp.error)
    }
}

fn as_agent(slug: &str) -> RpcContext {
    RpcContext { agent_id: slug.to_string(), ..Default::default() }
}

/// Neither another agent, nor a connection that never registered and was never proven, may touch a
/// pane's transcript; and no block (not even a made-up one) lets them in.
#[tokio::test]
async fn only_the_panes_own_agent_or_the_window_archives_or_restores_its_transcript() {
    let (state, block) = state_with_a_pane();
    for command in [COMMAND_SESSION_ARCHIVE, COMMAND_SESSION_RESTORE] {
        for (ctx, target) in [
            (as_agent("own-b"), block.as_str()),
            (RpcContext::default(), block.as_str()),
            (as_agent("own-a"), "own-no-such-block"),
        ] {
            let e = call(&state, ctx, command, target).await.unwrap_err();
            assert!(e.starts_with("FORBIDDEN"), "{command} on {target}: {e}");
        }
    }
}

/// The pane's own agent, in any letter case, and the window, get past the check (and then meet the
/// pane having no archive to restore, which is not a refusal).
#[tokio::test]
async fn the_panes_own_agent_and_the_window_are_not_refused() {
    let (state, block) = state_with_a_pane();
    for ctx in [as_agent("own-a"), as_agent("OWN-A"), RpcContext::operator_for_tests()] {
        let e = call(&state, ctx, COMMAND_SESSION_RESTORE, &block).await.unwrap_err();
        assert!(!e.starts_with("FORBIDDEN"), "{e}");
    }
}
