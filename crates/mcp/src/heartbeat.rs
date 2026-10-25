// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The heartbeat to srv: every few seconds this process says it is alive, how
//! old its oldest unanswered call is, and how far behind its async runtime
//! runs. srv's self-heal (`backend::mcp_health`, #2614) restarts the server
//! when the beats stop, or the runtime has stopped turning, or a call has
//! outlived every deadline.
//!
//! It runs on a thread of its own, with a runtime of its own: a process
//! whose main runtime is starved or stuck still beats, and says so, instead
//! of going quiet in a way no one can tell from a closed pane.

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;

use crate::serve::Health;

const EVERY: Duration = Duration::from_secs(15);
const FIRST: Duration = Duration::from_secs(5);

/// Start the beats. Does nothing without the agent's token and slug (a
/// process started outside AgentMux's own spawn, a test).
pub(crate) fn start(health: Arc<Health>, local_url: String, auth_key: String) {
    let var = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
    let (Some(token), Some(slug)) = (var("AGENTMUX_AGENT_TOKEN"), var("AGENTMUX_AGENT_SLUG"))
    else {
        return;
    };
    if local_url.is_empty() || auth_key.is_empty() {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("mcp-heartbeat".into())
        .spawn(move || {
            let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            // Everything, the client included, inside this runtime's own context.
            rt.block_on(async move {
                let Ok(client) = reqwest::Client::builder()
                    .timeout(Duration::from_secs(10))
                    .build()
                else {
                    return;
                };
                let url = format!(
                    "{}/api/v1/agent/self/mcp-beat",
                    local_url.trim_end_matches('/')
                );
                tokio::time::sleep(FIRST).await;
                loop {
                    let oldest = health.oldest_unanswered();
                    let body = json!({
                        "agent": slug,
                        "pid": std::process::id(),
                        "inflight": health.inflight(),
                        "oldest_tool": oldest.as_ref().map(|(t, _)| t.clone()),
                        "oldest_s": oldest.map(|(_, s)| s),
                        "runtime_lag_ms": health.runtime_lag_ms(),
                        "call_threads": health.call_threads(),
                    });
                    // A failure is not news: srv may be restarting, and the next beat tries again.
                    let _ = client
                        .post(&url)
                        .header(agentmux_common::AUTH_KEY_HEADER, &auth_key)
                        .header("X-Agent-Token", token.trim())
                        .json(&body)
                        .send()
                        .await;
                    tokio::time::sleep(EVERY).await;
                }
            });
        });
    if let Err(e) = spawned {
        eprintln!("agentmux-mcp: couldn't start the heartbeat thread: {e}");
    }
}
