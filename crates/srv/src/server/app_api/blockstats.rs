// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `blockstats.detail`: a terminal's stats badge tooltip
//! (`backend::blockstats_detail`). The window's, not an agent API: a
//! connection registered as an agent is refused.

use super::*;
use super::tower_pane::not_an_agent_api;
use crate::backend::blockcontroller::process_tree::MAX_PIDS_PER_BLOCK;
use crate::backend::blockstats_detail;

pub fn register(engine: &Arc<WshRpcEngine>, _state: &AppState) {
    engine.register_typed(COMMAND_BLOCKSTATS_DETAIL, |req: BlockStatsDetailReq, ctx| async move {
        not_an_agent_api(&ctx)?;
        let root = blockcontroller::pidregistry::get_all()
            .into_iter()
            .find(|(id, _)| *id == req.block_id)
            .map(|(_, pid)| pid)
            .ok_or("no process for this pane")?;
        // The process table: off the async workers.
        tokio::task::spawn_blocking(move || {
            let snap = agentmux_procstats::snapshot().map_err(|e| format!("blockstats.detail: {e}"))?;
            blockstats_detail::detail(&snap, root, MAX_PIDS_PER_BLOCK).ok_or_else(|| "the pane's shell has exited".to_string())
        })
        .await
        .map_err(|e| format!("blockstats.detail: {e}"))?
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn call(data: serde_json::Value, agent_id: &str) -> (serde_json::Value, String) {
        let state = crate::server::tests::test_state();
        let (engine, mut rx) = WshRpcEngine::new();
        register(&engine, &state);
        if !agent_id.is_empty() {
            engine.set_rpc_context(RpcContext { agent_id: agent_id.to_string(), ..Default::default() });
        }
        engine.handle_message(RpcMessage {
            command: COMMAND_BLOCKSTATS_DETAIL.to_string(),
            reqid: "req-1".to_string(),
            data: Some(data),
            ..Default::default()
        });
        let resp = tokio::time::timeout(std::time::Duration::from_secs(20), rx.recv()).await.unwrap().unwrap();
        (resp.data.unwrap_or_default(), resp.error)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_registered_pane_gets_its_process_tree() {
        // This test process stands in for the pane's shell.
        let block_id = "blockstats-detail-test-block";
        blockcontroller::pidregistry::register(block_id, std::process::id());
        let (data, err) = call(json!({ "block_id": block_id }), "").await;
        blockcontroller::pidregistry::unregister(block_id);
        assert_eq!(err, "");
        let d: BlockStatsDetail = serde_json::from_value(data).unwrap();
        assert!(d.processes >= 1);
        assert!(d.started_at_ms.is_some());
        assert!(d.mem_private.unwrap_or(0) > 0, "{d:?}");
        assert_eq!(d.mem_commit.is_some(), cfg!(windows), "{d:?}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_unknown_pane_is_an_error() {
        let (_, err) = call(json!({ "block_id": "no-such-block" }), "").await;
        assert!(err.contains("no process"), "{err}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_registered_agent_is_refused() {
        let (_, err) = call(json!({ "block_id": "x" }), "AgentX").await;
        assert!(err.contains("FORBIDDEN"), "{err}");
    }
}
