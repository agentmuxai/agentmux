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
/// A beat this long after the last one means the machine slept (or this
/// thread was starved): the clocks the lag and the ages are made from jumped,
/// so that beat doesn't speak to either.
const LATE_AFTER: Duration = Duration::from_secs(45);

/// The beat's body. After a sleep (`late`) the runtime lag and the age of the
/// oldest call are left out for this one beat: the tick task hasn't had a
/// chance to run yet, and srv would take the gap for a stuck runtime.
pub(crate) fn beat_body(health: &Health, slug: &str, late: bool) -> serde_json::Value {
    let oldest = if late {
        None
    } else {
        health.oldest_unanswered()
    };
    json!({
        "agent": slug,
        "pid": std::process::id(),
        "inflight": health.inflight(),
        "oldest_tool": oldest.as_ref().map(|(t, _)| t.clone()),
        "oldest_s": oldest.map(|(_, s)| s),
        "runtime_lag_ms": if late { 0 } else { health.runtime_lag_ms() },
        "call_threads": health.call_threads(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_beat_after_a_sleep_reports_no_lag_and_no_call_age() {
        let h = Health::new();
        h.begin("1".into(), "UI".into());
        std::thread::sleep(Duration::from_millis(1100));
        let ordinary = beat_body(&h, "a", false);
        assert!(ordinary["runtime_lag_ms"].as_u64().unwrap() >= 1000);
        assert_eq!(ordinary["oldest_tool"], "UI");
        let late = beat_body(&h, "a", true);
        assert_eq!(late["runtime_lag_ms"], 0);
        assert!(late["oldest_tool"].is_null() && late["oldest_s"].is_null());
        assert_eq!(late["inflight"], 1);
    }
}

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
                let mut last = std::time::Instant::now();
                loop {
                    let late = last.elapsed() > LATE_AFTER;
                    last = std::time::Instant::now();
                    let body = beat_body(&health, &slug, late);
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
