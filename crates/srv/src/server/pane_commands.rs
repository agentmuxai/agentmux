// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The window's own calls on agent panes: `agent.pane.unregister` and `agent.takeover`.
//! Over HTTP an agent can do both for its own panes with its token (`pane_binding`); the
//! window has no token, so it asks over its socket, and srv answers only a connection the
//! host has proven is the AgentMux window.

use std::sync::Arc;

use axum::body::to_bytes;
use axum::response::Response;
use serde_json::Value;

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{
    AgentPaneUnregisterResult, AgentTakeoverResult, CommandAgentPaneUnregisterData, CommandAgentTakeoverData,
    COMMAND_AGENT_PANE_UNREGISTER, COMMAND_AGENT_TAKEOVER,
};

use super::agent_takeover::{take_over, TakeoverRequest};
use super::reactive::{unregister_pane, UnregisterRequest};
use super::AppState;

/// What an HTTP handler answered, as the RPC's result: its body when it succeeded, its `error` when it did not.
async fn answer(response: Response) -> Result<Value, String> {
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.map_err(|e| e.to_string())?;
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    if status.is_success() {
        Ok(body)
    } else {
        Err(body["error"].as_str().map(str::to_string).unwrap_or_else(|| format!("HTTP {status}")))
    }
}

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let unregister_state = state.clone();
    engine.register_typed(COMMAND_AGENT_PANE_UNREGISTER, move |cmd: CommandAgentPaneUnregisterData, ctx| {
        let state = unregister_state.clone();
        async move {
            ctx.operator_only("only the operator's window unregisters a pane")?;
            let request = UnregisterRequest { agent_id: cmd.agent_id, block_id: cmd.block_id };
            let body = answer(unregister_pane(&state, request).await).await?;
            Ok(AgentPaneUnregisterResult { success: body["success"].as_bool().unwrap_or(false) })
        }
    });
    let takeover_state = state.clone();
    engine.register_typed(COMMAND_AGENT_TAKEOVER, move |cmd: CommandAgentTakeoverData, ctx| {
        let state = takeover_state.clone();
        async move {
            ctx.operator_only("only the operator's window takes an agent over")?;
            let body = answer(take_over(&state, None, TakeoverRequest { block_id: cmd.block_id }).await).await?;
            Ok(AgentTakeoverResult {
                released: body["released"].as_bool().unwrap_or(false),
                from_channel: body["from_channel"].as_str().map(str::to_string),
            })
        }
    });
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::backend::rpc::engine::WshRpcEngine;
    use crate::backend::rpc_types::{RpcContext, RpcMessage};
    use crate::server::tests::test_state;

    async fn call(ctx: RpcContext, command: &str, data: serde_json::Value) -> Result<serde_json::Value, String> {
        let state = test_state();
        let (engine, mut rx) = WshRpcEngine::new();
        super::register(&engine, &state);
        engine.set_rpc_context(ctx);
        engine.handle_message(RpcMessage { command: command.to_string(), reqid: "r".to_string(), data: Some(data), ..Default::default() });
        let resp = tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv()).await.unwrap().unwrap();
        if resp.error.is_empty() {
            Ok(resp.data.unwrap_or_default())
        } else {
            Err(resp.error)
        }
    }

    /// Only the window the host proved unregisters a pane or takes an agent over; an agent's connection and an unproven one are refused.
    #[tokio::test]
    async fn only_the_operators_window_unregisters_or_takes_over() {
        let agent = RpcContext { agent_id: "pc-alpha".to_string(), ..Default::default() };
        for (command, data) in [
            ("agent.pane.unregister", json!({ "agent_id": "pc-x", "block_id": "pc-block" })),
            ("agent.takeover", json!({ "block_id": "pc-block" })),
        ] {
            for ctx in [agent.clone(), RpcContext::default()] {
                let e = call(ctx, command, data.clone()).await.unwrap_err();
                assert!(e.starts_with("FORBIDDEN: only the operator's window"), "{command}: {e}");
            }
        }
        let done = call(RpcContext::operator_for_tests(), "agent.pane.unregister", json!({ "agent_id": "pc-x", "block_id": "pc-block" })).await.unwrap();
        assert_eq!(done["success"], true, "{done}");
        let e = call(RpcContext::operator_for_tests(), "agent.takeover", json!({ "block_id": "pc-no-such-block" })).await.unwrap_err();
        assert!(e.contains("block not found"), "{e}");
    }
}
