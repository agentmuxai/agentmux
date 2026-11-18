// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `gpu.sample`: the status bar's GPU panel (`backend::gpu`; SPEC_STATUS_BAR_GPU_PANEL_2026_11_15.md).
//!
//! The window's, not an agent API (a connection registered as an agent is refused): the rows name other programs
//! by their image names. The counters are read by the one GPU sampler (`GpuSampler::global`), and a process is named
//! by Tower's own sample of the process table, the one place that names a process, so a process has one name in
//! both views.

use super::tower_pane::{not_an_agent_api, sample_local};
use super::*;
use crate::backend::gpu::{shape_processes, GpuSampler, Named};
use std::collections::HashMap;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let state = state.clone();
    engine.register_typed(COMMAND_GPU_SAMPLE, move |_req: GpuSampleReq, ctx| {
        let state = state.clone();
        async move {
            not_an_agent_api(&ctx)?;
            sample(&state).await
        }
    });
}

async fn sample(state: &AppState) -> Result<GpuSampleResult, String> {
    // The counters are read on a blocking thread: a read enumerates every process's engines.
    let (sample, interval, widened) = tokio::task::spawn_blocking(|| GpuSampler::global().sample_for_panel())
        .await
        .map_err(|e| format!("gpu.sample: {e}"))?;
    // Names only when there is someone to name: Tower's process list is a sample of its own.
    let names: HashMap<u32, Named> = if sample.reading.processes.is_empty() {
        HashMap::new()
    } else {
        let req = TowerSampleReq { host: Some(true), ..Default::default() };
        match sample_local(state, &req).await {
            Ok(snap) => snap
                .host
                .map(|h| h.processes)
                .unwrap_or_default()
                .into_iter()
                .map(|p| (p.pid, Named { name: p.name, label: p.detail, kind: p.kind }))
                .collect(),
            Err(e) => {
                tracing::debug!(error = %e, "[gpu] couldn't name the processes");
                HashMap::new()
            }
        }
    };
    let (processes, others) = shape_processes(&sample.reading.processes, |pid| names.get(&pid).cloned());
    Ok(GpuSampleResult {
        ts_ms: agentmux_common::time::now_ms_u64(),
        os: std::env::consts::OS.to_string(),
        state: sample.state,
        adapters: sample.reading.adapters,
        processes,
        others,
        per_process: cfg!(any(windows, target_os = "linux")),
        interval_ms: interval.as_millis() as u32,
        sample_ms: sample.took.as_millis() as u32,
        widened,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn call(agent_id: &str) -> (serde_json::Value, String) {
        let state = crate::server::tests::test_state();
        let (engine, mut rx) = WshRpcEngine::new();
        register(&engine, &state);
        if agent_id.is_empty() {
            engine.set_rpc_context(RpcContext::operator_for_tests());
        } else {
            engine.set_rpc_context(RpcContext { agent_id: agent_id.to_string(), ..Default::default() });
        }
        engine.handle_message(RpcMessage {
            command: COMMAND_GPU_SAMPLE.to_string(),
            reqid: "req-1".to_string(),
            data: Some(serde_json::json!({})),
            ..Default::default()
        });
        let resp = tokio::time::timeout(std::time::Duration::from_secs(20), rx.recv()).await.unwrap().unwrap();
        (resp.data.unwrap_or_default(), resp.error)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_window_gets_an_answer_whatever_this_machine_has() {
        let (data, err) = call("").await;
        assert_eq!(err, "");
        let r: GpuSampleResult = serde_json::from_value(data).unwrap();
        assert_eq!(r.os, std::env::consts::OS);
        assert!(r.interval_ms >= 2000);
        // Nothing is invented for a computer with no counters: no adapters, no rows.
        if !matches!(r.state, GpuSampleState::Ok) {
            assert!(r.adapters.is_empty() && r.processes.is_empty());
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_registered_agent_is_refused() {
        let (_, err) = call("AgentX").await;
        assert!(err.contains("FORBIDDEN"), "{err}");
    }
}
