// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `blockstats.detail`: a terminal's stats badge and its tooltip
//! (`backend::blockstats_detail`). The window's, not an agent API: a
//! connection registered as an agent is refused.
//!
//! - A local pane: its processes here, from one snapshot.
//! - A remote pane (SSH, WSL): its processes on that machine, asked of
//!   AgentMux's helper there, and the machine's name and address.

use super::*;
use super::tower_pane::not_an_agent_api;
use crate::backend::blockcontroller::durable_ssh::META_KEY_SESSION_ID;
use crate::backend::blockcontroller::process_tree::MAX_PIDS_PER_BLOCK;
use crate::backend::blockstats_detail;
use crate::backend::remote::address as remote_address;
use crate::backend::remote::ConnTarget;

/// How long asking a remote pane's helper may take before the badge reads
/// "not measured" for this round.
const REMOTE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let mstore = state.mstore.clone();
    engine.register_typed(COMMAND_BLOCKSTATS_DETAIL, move |req: BlockStatsDetailReq, ctx| {
        let mstore = mstore.clone();
        async move {
            not_an_agent_api(&ctx)?;
            let meta = mstore.get::<Block>(&req.block_id).ok().flatten().map(|b| b.meta);
            let meta_str = |key: &str| meta.as_ref().map(|m| obj::meta_get_string(m, key, "")).unwrap_or_default();
            let conn = meta_str("connection");
            // Only a terminal has a remote shell to measure; any other pane on
            // a connection (Files, preview) is not asked of the machine.
            let remote_terminal = meta_str("view") == "term";
            if remote_terminal && matches!(ConnTarget::parse(&conn), Ok(ConnTarget::Ssh(_) | ConnTarget::Wsl(_))) {
                // The machine's name and address, and the pane's processes
                // there, side by side.
                let session = meta_str(META_KEY_SESSION_ID);
                let (connection, remote) = tokio::join!(connection(&conn), remote(&conn, &req.block_id, &session));
                return Ok(BlockStatsDetail { local: None, connection, remote: Some(remote) });
            }
            let root = blockcontroller::pidregistry::get_all()
                .into_iter()
                .find(|(id, _)| *id == req.block_id)
                .map(|(_, pid)| pid)
                .ok_or("no process for this pane")?;
            // The process table: off the async workers.
            let local = tokio::task::spawn_blocking(move || {
                let snap = agentmux_procstats::snapshot().map_err(|e| format!("blockstats.detail: {e}"))?;
                blockstats_detail::detail(&snap, root, MAX_PIDS_PER_BLOCK)
                    .ok_or_else(|| "the pane's shell has exited".to_string())
            })
            .await
            .map_err(|e| format!("blockstats.detail: {e}"))??;
            Ok(BlockStatsDetail { local: Some(local), connection: None, remote: None })
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

/// A remote pane's processes on its machine. WSL: the helper Tower runs in
/// the distribution. SSH: the host's helper, only over a connection that
/// needs no prompt and no install (`files::connect_quiet`): this is asked
/// every few seconds, never on the user's word.
async fn remote(conn: &str, block_id: &str, session: &str) -> BlockStatsRemote {
    let ask = async {
        let helper = match ConnTarget::parse(conn)? {
            ConnTarget::Wsl(distro) => crate::backend::tower_remote::wsl_helper(&distro).await?,
            ConnTarget::Ssh(_) => crate::backend::remote::files::connect_quiet(conn).await.map_err(|e| e.message)?,
            ConnTarget::Local => return Err("not a remote pane".to_string()),
        };
        helper.pane_procs(block_id, session).await.map_err(|e| e.message)
    };
    match tokio::time::timeout(REMOTE_TIMEOUT, ask).await {
        Ok(Ok(frame)) => blockstats_detail::remote(block_id, frame),
        Ok(Err(why)) => blockstats_detail::remote_unavailable(why),
        Err(_) => blockstats_detail::remote_unavailable("the machine didn't answer in time".to_string()),
    }
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
        assert!(d.connection.is_none() && d.remote.is_none(), "{d:?}");
        let d = d.local.expect("a local pane's figures");
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

    /// An SSH pane's machine answers through its helper's connection (here
    /// the real request handling, in process): this test process carries no
    /// pane's marker, so the host's figures come back without the pane's.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_remote_pane_is_measured_by_its_machines_helper() {
        let home = tempfile::tempdir().unwrap();
        crate::backend::remote::files::connect_in_process("me@stats-test-host", home.path().to_path_buf()).await;
        let r = remote("me@stats-test-host", "no-pane-has-this-id", "").await;
        assert_eq!(r.found, "none", "{r:?}");
        let host = r.host.expect("the host's figures");
        assert!(host.processes > 1 && host.cpu_count >= 1, "{host:?}");
        assert!(r.figures.is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_remote_pane_whose_helper_isnt_there_is_unavailable_not_an_error() {
        let r = remote("me@no-helper-here.invalid", "b", "").await;
        assert_eq!(r.found, "unavailable", "{r:?}");
        assert!(r.reason.is_some());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_registered_agent_is_refused() {
        let (_, err) = call(json!({ "block_id": "x" }), "AgentX").await;
        assert!(err.contains("FORBIDDEN"), "{err}");
    }
}
