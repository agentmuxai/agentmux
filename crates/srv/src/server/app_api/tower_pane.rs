// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Tower, the read-only task manager pane (`backend::tower_sampler`;
//! SPEC_TOWER_TASK_MANAGER_PANE_2026_10_08.md): `tower.sample`.
//!
//! Tower's RPCs are the window's, not an agent API: a connection registered
//! as an agent (`bus:register`) is refused.
//!
//! With a `connection` it samples another machine instead
//! (`backend::tower_remote`): an SSH host or a WSL distribution.

use super::*;
use crate::backend::process_tracker::registry::AgentProcessRegistry;
use crate::backend::{tower_memory_ledger, tower_peers, tower_remote};
use crate::backend::tower_agentmux::{own_view_pid, parse_renderer_map, renderer_serves, RendererEntry};
use crate::backend::tower_sampler::{block_label, Inputs, Tower};

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    register_peers(engine, state);
    let auth_key = state.auth_key.clone();
    let lan = state.lan_discovery.clone();
    let local = state.clone();
    spawn_ledger_timer(&local);
    engine.register_typed(COMMAND_TOWER_SAMPLE, move |req: TowerSampleReq, ctx| {
        let (local, auth_key) = (local.clone(), auth_key.clone());
        let lan = lan.clone();
        async move {
            not_an_agent_api(&ctx)?;
            // A paired AgentMux computer: its viewer listener (`tower_peers`).
            if let Some(id) = req.connection.as_deref().and_then(|c| c.strip_prefix(tower_peers::PEER_PREFIX)) {
                let peers = tower_peers::Peers::global();
                let hostname = peers.get(id).map(|p| p.hostname).unwrap_or_default();
                let moved: Vec<String> = lan
                    .get_instances()
                    .into_iter()
                    .filter(|i| !hostname.is_empty() && i.hostname.eq_ignore_ascii_case(&hostname))
                    .map(|i| i.address)
                    .collect();
                // A paired computer's AgentMux may predate kinds: fill them by name.
                let mut snap = tower_peers::sample(peers, id, req.filter.as_deref().unwrap_or(""), moved).await?;
                crate::backend::tower_kind::fill_missing(&mut snap);
                return Ok(snap);
            }
            // Another machine: its helper samples it (`tower_remote`).
            if let Some(conn) = req.connection.as_deref().filter(|c| !tower_remote::is_local(c)) {
                let block_id = req.block_id.clone().unwrap_or_default();
                let ask = (!block_id.is_empty()).then(|| crate::backend::remote::sessions::AskIn {
                    block_id: &block_id,
                    auth_key: &auth_key,
                });
                // The helper's frame carries names only: kinds by name.
                let mut snap = tower_remote::sample(conn, req.filter.as_deref().unwrap_or(""), ask).await?;
                crate::backend::tower_kind::fill_missing(&mut snap);
                return Ok(snap);
            }
            sample_local(&local, &req).await
        }
    });
}

/// This computer's sample, with the views the request asked for. Also what
/// keeps the memory ledger: every sample that carries memory goes in it.
async fn sample_local(state: &AppState, req: &TowerSampleReq) -> Result<TowerSnapshot, String> {
    let renderers = renderer_map(&state.http_client, &state.host_ipc).await;
    let (mstore, hostname, tracker) = (state.mstore.clone(), state.hostname.clone(), state.process_tracker.clone());
    let (want_host, want_panes, want_memory, want_history) = (
        req.host.unwrap_or(false),
        req.panes.unwrap_or(false),
        req.memory.unwrap_or(false),
        req.memory_history.unwrap_or(false),
    );
    // The process table, and a store read per pane: off the async workers.
    tokio::task::spawn_blocking(move || {
        let page = |id: &str| browser_page(&mstore, id);
        let workspace = |id: &str| window_workspace(&mstore, id);
        let mut snap = Tower::global()
            .sample(
                want_host,
                &hostname,
                || inputs(&tracker),
                |id| block_label(&mstore, id),
                // A renderer reported the PID it sees for itself (namespaced on Linux).
                |pid| renderer_serves(&renderers, own_view_pid(pid), &page, &workspace),
            )
            .map_err(|e| format!("tower.sample: {e}"))?;
        // The Panes view: windows, tabs and panes (SPEC_TOWER_PANES_VIEW_2026_10_21.md).
        if want_panes {
            let layout = crate::backend::tower_panes::read_layout(&mstore);
            snap.panes = Some(crate::backend::tower_panes::build(&layout, &snap, &renderers, &own_view_pid));
        }
        // The Memory view: AgentMux's total and its parts, and the ledger's hour
        // (SPEC_TOWER_MEMORY_FIRST_AND_AGENTMUX_BREAKDOWN_2026_10_25.md).
        if want_memory {
            let mut memory = crate::backend::tower_memory::build(&snap);
            tower_memory_ledger::feed(&mut memory, snap.ts_ms, want_history);
            snap.memory = Some(memory);
        }
        Ok(snap)
    })
    .await
    .map_err(|e| format!("tower.sample: {e}"))?
}

/// The ledger keeps its hour with no Tower open (spec D7): every 30 s it
/// samples if no Tower did. One timer per process.
fn spawn_ledger_timer(state: &AppState) {
    static STARTED: std::sync::Once = std::sync::Once::new();
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return;
    };
    let state = state.clone();
    STARTED.call_once(|| {
        runtime.spawn(async move {
            let step = std::time::Duration::from_millis(tower_memory_ledger::STEP_MS * tower_memory_ledger::IDLE_EVERY_STEPS);
            let mut tick = tokio::time::interval(step);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            // The first tick is now, with nothing to read yet: skip it.
            tick.tick().await;
            loop {
                tick.tick().await;
                if tower_memory_ledger::fed_within(step * 4 / 5) {
                    continue;
                }
                let req = TowerSampleReq { memory: Some(true), ..Default::default() };
                if let Err(e) = sample_local(&state, &req).await {
                    tracing::debug!(error = %e, "[tower] the memory ledger couldn't sample");
                }
            }
        });
    });
}

/// `tower.peers`, `tower.pair` and `tower.forget`: the AgentMux computers this
/// one is paired with (`backend::tower_peers`).
fn register_peers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    engine.register_typed(COMMAND_TOWER_PEERS, |_req: Option<NoArgsReq>, ctx| async move {
        not_an_agent_api(&ctx)?;
        Ok(TowerPeersResult { peers: tower_peers::Peers::global().list() })
    });
    let hostname = state.hostname.clone();
    engine.register_typed(COMMAND_TOWER_PAIR, move |req: TowerPairReq, ctx| {
        let device_name = format!("{hostname} (Tower)");
        async move {
            not_an_agent_api(&ctx)?;
            tower_peers::pair(tower_peers::Peers::global(), &req.link, &device_name).await
        }
    });
    engine.register_typed(COMMAND_TOWER_FORGET, |req: TowerForgetReq, ctx| async move {
        not_an_agent_api(&ctx)?;
        let id = req.connection.strip_prefix(tower_peers::PEER_PREFIX).ok_or("not a paired computer")?;
        tokio::task::spawn_blocking({
            let id = id.to_string();
            move || tower_peers::Peers::global().forget(&id)
        })
        .await
        .map_err(|e| e.to_string())??;
        Ok(TowerPeersResult { peers: tower_peers::Peers::global().list() })
    });
}

/// Tower's RPCs are the AgentMux window's, not an agent API: only a
/// connection the host proved is the window's gets them.
pub(super) fn not_an_agent_api(ctx: &RpcContext) -> Result<(), String> {
    ctx.operator_only("Tower is the AgentMux window's, not an agent API")
}

pub(crate) fn inputs(tracker: &AgentProcessRegistry) -> Inputs {
    Inputs {
        blocks: tracker.members_by_block(),
        roots: blockcontroller::pidregistry::get_all(),
        own_pid: std::process::id(),
    }
}

/// Which renderer serves which window or pane: only the window host knows
/// (`crates/cef/src/renderer_map.rs`). Asked with a short timeout: a slow or
/// absent host leaves renderers unnamed rather than holding up the sample.
async fn renderer_map(
    http_client: &reqwest::Client,
    host_ipc: &tokio::sync::Mutex<Option<crate::server::HostIpc>>,
) -> Vec<RendererEntry> {
    let Some(host) = host_ipc.lock().await.clone() else {
        return Vec::new();
    };
    let timeout = Some(std::time::Duration::from_millis(500));
    match crate::server::host_http::post_to_host(http_client, &host, "renderer_map", serde_json::json!({}), timeout).await {
        Ok(answer) => parse_renderer_map(&answer),
        Err(e) => {
            tracing::debug!(error = %e, "[tower] couldn't ask the host which renderer serves what");
            Vec::new()
        }
    }
}

/// A browser pane's page, for its renderer's row: its title, else its URL's
/// host.
fn browser_page(mstore: &Store, block_id: &str) -> Option<String> {
    let block = mstore.get::<Block>(block_id).ok().flatten()?;
    let meta = |key: &str| obj::meta_get_string(&block.meta, key, "");
    let title = meta("frame:title");
    if !title.is_empty() {
        return Some(title);
    }
    crate::backend::tower_agentmux::page_host(&meta("url"))
}

/// A window's workspace name, for its renderer's row.
fn window_workspace(mstore: &Store, window_id: &str) -> Option<String> {
    let window = mstore.get::<obj::Window>(window_id).ok().flatten()?;
    let workspace = mstore.get::<obj::Workspace>(&window.workspaceid).ok().flatten()?;
    Some(workspace.name).filter(|n| !n.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn call(command: &str, data: serde_json::Value, agent_id: &str) -> (serde_json::Value, String) {
        let state = crate::server::tests::test_state();
        let (engine, mut rx) = WshRpcEngine::new();
        register(&engine, &state);
        // An agent's connection carries its id (`bus:register`); the window's
        // carries none and has the host's proof.
        if agent_id.is_empty() {
            engine.set_rpc_context(RpcContext::operator_for_tests());
        } else {
            engine.set_rpc_context(RpcContext { agent_id: agent_id.to_string(), ..Default::default() });
        }
        engine.handle_message(RpcMessage {
            command: command.to_string(),
            reqid: "req-1".to_string(),
            data: Some(data),
            ..Default::default()
        });
        let resp = tokio::time::timeout(std::time::Duration::from_secs(20), rx.recv()).await.unwrap().unwrap();
        (resp.data.unwrap_or_default(), resp.error)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_window_gets_a_sample_with_the_host_list() {
        let (data, err) = call(COMMAND_TOWER_SAMPLE, json!({ "host": true }), "").await;
        assert_eq!(err, "");
        let snap: TowerSnapshot = serde_json::from_value(data).unwrap();
        assert!(snap.cpu_count >= 1);
        assert_eq!(snap.os, std::env::consts::OS);
        let host = snap.host.expect("host list asked for");
        assert!(host.processes.iter().any(|p| p.pid == std::process::id()));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_registered_agent_is_refused() {
        let (_, err) = call(COMMAND_TOWER_SAMPLE, json!({}), "AgentX").await;
        assert!(err.contains("FORBIDDEN"), "{err}");
    }

    #[test]
    fn a_connection_the_host_never_proved_is_refused() {
        let err = not_an_agent_api(&RpcContext::default()).unwrap_err();
        assert!(err.starts_with("FORBIDDEN") && err.contains("isn't the AgentMux window"), "{err}");
        assert!(not_an_agent_api(&RpcContext::operator_for_tests()).is_ok());
    }
}
