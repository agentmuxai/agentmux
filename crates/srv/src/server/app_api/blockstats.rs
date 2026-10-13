// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `blockstats.detail`: a terminal's stats badge tooltip
//! (`backend::blockstats_detail`). The window's, not an agent API: a
//! connection registered as an agent is refused.

use super::*;
use super::tower_pane::not_an_agent_api;
use crate::backend::blockcontroller::process_tree::MAX_PIDS_PER_BLOCK;
use crate::backend::blockstats_detail;
use crate::backend::remote::address as remote_address;
use crate::backend::remote::ConnTarget;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let mstore = state.mstore.clone();
    engine.register_typed(COMMAND_BLOCKSTATS_DETAIL, move |req: BlockStatsDetailReq, ctx| {
        let mstore = mstore.clone();
        async move {
            not_an_agent_api(&ctx)?;
            let root = blockcontroller::pidregistry::get_all()
                .into_iter()
                .find(|(id, _)| *id == req.block_id)
                .map(|(_, pid)| pid)
                .ok_or("no process for this pane")?;
            let conn = mstore
                .get::<Block>(&req.block_id)
                .ok()
                .flatten()
                .map(|b| obj::meta_get_string(&b.meta, "connection", ""))
                .unwrap_or_default();
            // The process table (off the async workers) and, for a remote
            // pane, its machine's name and address, side by side.
            let (detail, connection) = tokio::join!(
                tokio::task::spawn_blocking(move || {
                    let snap = agentmux_procstats::snapshot().map_err(|e| format!("blockstats.detail: {e}"))?;
                    blockstats_detail::detail(&snap, root, MAX_PIDS_PER_BLOCK)
                        .ok_or_else(|| "the pane's shell has exited".to_string())
                }),
                connection(&conn),
            );
            let mut detail = detail.map_err(|e| format!("blockstats.detail: {e}"))??;
            detail.connection = connection;
            Ok(detail)
        }
    });
}

/// A remote pane's machine; `None` for a local pane.
async fn connection(name: &str) -> Option<BlockStatsConnection> {
    let kind = match ConnTarget::parse(name).ok()? {
        ConnTarget::Local => return None,
        ConnTarget::Ssh(_) => "ssh",
        ConnTarget::Wsl(_) => "wsl",
    };
    let found = remote_address::of(name).await.unwrap_or_default();
    Some(BlockStatsConnection {
        name: name.trim().to_string(),
        kind: kind.to_string(),
        host: found.host,
        address: found.address,
    })
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
    async fn a_local_pane_has_no_connection_and_a_wsl_pane_names_its_distro() {
        assert_eq!(connection("").await, None);
        assert_eq!(connection("local").await, None);
        let wsl = connection("wsl://NoSuchDistro").await.unwrap();
        assert_eq!((wsl.kind.as_str(), wsl.host.as_deref()), ("wsl", Some("NoSuchDistro")));
        assert_eq!(wsl.address, None, "a distro that doesn't exist has no address");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_registered_agent_is_refused() {
        let (_, err) = call(json!({ "block_id": "x" }), "AgentX").await;
        assert!(err.contains("FORBIDDEN"), "{err}");
    }
}
