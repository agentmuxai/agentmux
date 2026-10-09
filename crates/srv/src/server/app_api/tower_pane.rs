// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Tower, the read-only task manager pane (`backend::tower_sampler`;
//! SPEC_TOWER_TASK_MANAGER_PANE_2026_10_08.md): `tower.sample` and
//! `tower.command-line`.
//!
//! For the AgentMux window only. An agent's connection is refused: a process
//! list names what every other agent is running, and command lines can hold
//! secrets.

use super::*;
use crate::backend::process_tracker::registry::AgentProcessRegistry;
use crate::backend::tower_sampler::{BlockLabel, Inputs, Tower};

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let (mstore, hostname, tracker) = (state.mstore.clone(), state.hostname.clone(), state.process_tracker.clone());
    engine.register_typed(COMMAND_TOWER_SAMPLE, move |req: TowerSampleReq, ctx| {
        let (mstore, hostname, tracker) = (mstore.clone(), hostname.clone(), tracker.clone());
        async move {
            ui_only(&ctx)?;
            // The process table, and a store read per pane: off the async workers.
            tokio::task::spawn_blocking(move || {
                Tower::global()
                    .sample(req.host.unwrap_or(false), &hostname, || inputs(&tracker), |id| block_label(&mstore, id))
                    .map_err(|e| format!("tower.sample: {e}"))
            })
            .await
            .map_err(|e| format!("tower.sample: {e}"))?
        }
    });
    engine.register_typed(COMMAND_TOWER_COMMAND_LINE, |req: TowerCommandLineReq, ctx| async move {
        ui_only(&ctx)?;
        let key = crate::backend::tower_sampler::parse_proc_id(&req.id).ok_or_else(|| format!("tower.command-line: bad id {:?}", req.id))?;
        tokio::task::spawn_blocking(move || {
            // Only the process the pane showed: a newer one that reused the
            // PID is a different process.
            let alive = agentmux_procstats::snapshot()
                .map_err(|e| format!("tower.command-line: {e}"))?
                .iter()
                .any(|p| p.key() == key);
            Ok(TowerCommandLineResult {
                command_line: if alive { agentmux_procstats::command_line(key.pid) } else { None },
            })
        })
        .await
        .map_err(|e| format!("tower.command-line: {e}"))?
    });
}

fn ui_only(ctx: &RpcContext) -> Result<(), String> {
    if ctx.agent_id.is_empty() {
        Ok(())
    } else {
        Err("FORBIDDEN: Tower is for the AgentMux window, not agents".to_string())
    }
}

fn inputs(tracker: &AgentProcessRegistry) -> Inputs {
    Inputs {
        blocks: tracker.members_by_block(),
        roots: blockcontroller::pidregistry::get_all(),
        own_pid: std::process::id(),
    }
}

/// A pane's name in Tower: the agent's name, else what the terminal runs.
/// `None` for a block that no longer exists.
fn block_label(mstore: &Store, block_id: &str) -> Option<BlockLabel> {
    let block = mstore.get::<Block>(block_id).ok().flatten()?;
    let meta = |key: &str| obj::meta_get_string(&block.meta, key, "");
    let agent_name = [meta("agentName"), meta("agentId")].into_iter().find(|s| !s.is_empty());
    let agent = agent_name.is_some()
        || blockcontroller::get_controller(block_id).is_some_and(|c| c.get_runtime_status().is_agent_pane);
    let label = agent_name.unwrap_or_else(|| {
        let title = meta("frame:title");
        let connection = meta("connection");
        if !title.is_empty() {
            title
        } else if !connection.is_empty() && connection != "local" {
            format!("Terminal ({connection})")
        } else if agent {
            "Agent".to_string()
        } else {
            "Terminal".to_string()
        }
    });
    Some(BlockLabel { label, agent })
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn call(command: &str, data: serde_json::Value, agent_id: &str) -> (serde_json::Value, String) {
        let state = crate::server::tests::test_state();
        let (engine, mut rx) = WshRpcEngine::new();
        register(&engine, &state);
        // An agent's connection carries its id (`bus:register`); the window's
        // carries none.
        if !agent_id.is_empty() {
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
    async fn an_agent_is_refused() {
        let (_, err) = call(COMMAND_TOWER_SAMPLE, json!({}), "AgentX").await;
        assert!(err.contains("FORBIDDEN"), "{err}");
        let (_, err) = call(COMMAND_TOWER_COMMAND_LINE, json!({ "id": "1:1" }), "AgentX").await;
        assert!(err.contains("FORBIDDEN"), "{err}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_command_line_only_for_the_process_the_pane_showed() {
        let me = agentmux_procstats::snapshot().unwrap().into_iter().find(|p| p.pid == std::process::id()).unwrap();
        let (data, err) = call(COMMAND_TOWER_COMMAND_LINE, json!({ "id": format!("{}:{}", me.pid, me.start_key) }), "").await;
        assert_eq!(err, "");
        assert!(data["command_line"].as_str().is_some_and(|s| !s.is_empty()), "{data}");
        // Same PID, another start: a different (newer) process.
        let (data, err) = call(COMMAND_TOWER_COMMAND_LINE, json!({ "id": format!("{}:{}", me.pid, me.start_key + 1) }), "").await;
        assert_eq!(err, "");
        assert!(data.get("command_line").is_none(), "{data}");
        let (_, err) = call(COMMAND_TOWER_COMMAND_LINE, json!({ "id": "nope" }), "").await;
        assert!(err.contains("bad id"), "{err}");
    }
}
