// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{RpcContext, RpcMessage};

/// Run `writeagentconfig` for an `.mcp.json` naming `slug`, as `ctx`, and return the file as
/// written.
async fn write_mcp_json(ctx: RpcContext, slug: &str) -> serde_json::Value {
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
    assert!(resp.error.is_empty(), "{}", resp.error);
    serde_json::from_str(&std::fs::read_to_string(dir.path().join(".mcp.json")).unwrap()).unwrap()
}

#[tokio::test]
async fn only_the_window_is_handed_an_agents_signing_keys() {
    let window = write_mcp_json(RpcContext::operator_for_tests(), "victim").await;
    let env = &window["mcpServers"]["agentmux"]["env"];
    assert!(env["AGENTMUX_JEKT_KEY"].as_str().is_some_and(|k| !k.is_empty()), "the window's launch carries the keys: {window}");

    // Any other connection, or one registered as an agent: the file is written, without them.
    for ctx in [RpcContext::default(), RpcContext { agent_id: "attacker".into(), ..Default::default() }] {
        let written = write_mcp_json(ctx, "victim").await;
        let env = &written["mcpServers"]["agentmux"]["env"];
        assert_eq!(env["AGENTMUX_AGENT_ID"], "victim", "the rest of the file is as sent");
        for key in ["AGENTMUX_JEKT_KEY", "AGENTMUX_LAN_KEY", "AGENTMUX_WAN_KEY"] {
            assert!(env.get(key).is_none(), "{key} was handed to a caller that isn't the window: {written}");
        }
    }
}
