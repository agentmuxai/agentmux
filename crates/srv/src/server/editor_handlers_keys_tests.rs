// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{RpcContext, RpcMessage};

/// Run `writeagentconfig` for an `.mcp.json` naming `slug`, as `ctx`, and return the file as
/// written (or the refusal).
async fn write_mcp_json(ctx: RpcContext, slug: &str) -> Result<serde_json::Value, String> {
    let state = crate::server::tests::test_state();
    let (engine, mut rx) = WshRpcEngine::new();
    super::register_editor_handlers(&engine, &state);
    engine.set_rpc_context(ctx);
    let dir = tempfile::tempdir().unwrap();
    let mcp = serde_json::json!({
        "mcpServers": { "agentmux": { "type": "stdio", "command": "agentmux-mcp", "env": { "AGENTMUX_AGENT_ID": slug } } }
    });
    engine.handle_message(RpcMessage {
        command: "writeagentconfig".to_string(),
        reqid: "req-1".to_string(),
        data: Some(serde_json::json!({
            "working_dir": dir.path().to_string_lossy(),
            "files": [{ "path": ".mcp.json", "content": mcp.to_string() }],
        })),
        ..Default::default()
    });
    let resp = tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv()).await.unwrap().unwrap();
    if !resp.error.is_empty() {
        assert!(!dir.path().join(".mcp.json").exists(), "a refusal writes nothing");
        return Err(resp.error);
    }
    Ok(serde_json::from_str(&std::fs::read_to_string(dir.path().join(".mcp.json")).unwrap()).unwrap())
}

#[tokio::test]
async fn only_the_window_is_handed_an_agents_signing_keys() {
    let window = write_mcp_json(RpcContext::operator_for_tests(), "victim").await.unwrap();
    let env = &window["mcpServers"]["agentmux"]["env"];
    assert!(env["AGENTMUX_JEKT_KEY"].as_str().is_some_and(|k| !k.is_empty()), "the window's launch carries the keys: {window}");

    // A connection that is neither the window nor an agent (an agent using the instance key without registering)
    // writes nothing at all, so there is no file for keys to be left out of.
    let refused = write_mcp_json(RpcContext::default(), "victim").await.unwrap_err();
    assert!(refused.starts_with("FORBIDDEN"), "{refused}");
}

/// A connection registered as an agent writes nothing outside its own workspace, so it can neither
/// plant files in another agent's folder nor sign a `.mcp.json` as another agent; and one that never
/// registered (it only holds the instance key) cannot either.
#[tokio::test]
async fn an_agent_cannot_write_config_for_another_agent_or_into_another_folder() {
    for ctx in [RpcContext { agent_id: "attacker".into(), ..Default::default() }, RpcContext::default()] {
        planted_files_are_refused(ctx).await;
    }
}

async fn planted_files_are_refused(ctx: RpcContext) {
    let state = crate::server::tests::test_state();
    let (engine, mut rx) = WshRpcEngine::new();
    super::register_editor_handlers(&engine, &state);
    engine.set_rpc_context(ctx);
    let dir = tempfile::tempdir().unwrap();
    for agent_id in [Some("victim"), None] {
        engine.handle_message(RpcMessage {
            command: "writeagentconfig".to_string(),
            reqid: "req-2".to_string(),
            data: Some(serde_json::json!({
                "working_dir": dir.path().to_string_lossy(),
                "agent_id": agent_id,
                "files": [{ "path": "CLAUDE.md", "content": "planted" }],
            })),
            ..Default::default()
        });
        let resp = tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv()).await.unwrap().unwrap();
        assert!(resp.error.starts_with("FORBIDDEN"), "{agent_id:?}: {}", resp.error);
        assert!(!dir.path().join("CLAUDE.md").exists(), "nothing was written");
    }
}
