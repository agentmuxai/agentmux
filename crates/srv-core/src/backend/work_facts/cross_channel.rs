// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Other AgentMux channels' agents' work facts, on this computer: every other
//! channel's srv in the host-global registry is asked for its own agents'
//! facts (`GET /api/v1/work-facts`), concurrently, each with its own timeout,
//! so one dead channel never stalls the caller
//! (docs/specs/SPEC_AGENT_OVERLAP_AWARENESS_2026_10_10.md §3.2, §3.3). Used by
//! `WhoIsWorkingOn`, `ListConversations` and the overlap notes.

use std::collections::HashMap;
use std::time::Duration;

use super::WorkFacts;

/// Per-channel timeout: one dead channel must not stall the answer.
pub const TIMEOUT: Duration = Duration::from_millis(1500);

/// The AgentMux channel this srv runs in (`stable`, `dev`, ...).
pub fn own_channel() -> String {
    std::env::var("AGENTMUX_CHANNEL").unwrap_or_else(|_| "stable".to_string())
}

/// Another AgentMux channel's srv on this computer.
struct Peer {
    local_url: String,
    auth_key: String,
    channel: String,
}

/// Every other channel's srv in the host-global registry, once each.
/// `own_url` is this srv's own local URL, never asked.
fn other_channels(own_url: &str) -> Vec<Peer> {
    let own = own_channel();
    let Some(shared_dir) = crate::registry::resolve_shared_reactive_dir() else {
        return Vec::new();
    };
    let mut seen: HashMap<String, Peer> = HashMap::new();
    for e in crate::backend::reactive::registry::list_all_shared(&shared_dir) {
        if e.channel == own || e.local_url == own_url || e.auth_key.is_empty() {
            continue;
        }
        seen.entry(e.local_url.clone()).or_insert(Peer {
            local_url: e.local_url,
            auth_key: e.auth_key,
            channel: e.channel,
        });
    }
    seen.into_values().collect()
}

/// Every other channel's agents' facts, fetched concurrently, plus the
/// channels that didn't answer in time.
pub async fn fetch(client: &reqwest::Client, own_url: &str) -> (Vec<WorkFacts>, Vec<String>) {
    let mut set = tokio::task::JoinSet::new();
    for peer in other_channels(own_url) {
        let client = client.clone();
        set.spawn(async move {
            let fetch = client
                .get(format!("{}/api/v1/work-facts", peer.local_url.trim_end_matches('/')))
                .header(agentmux_common::AUTH_KEY_HEADER, &peer.auth_key)
                .send();
            let body = match tokio::time::timeout(TIMEOUT, fetch).await {
                Ok(Ok(resp)) if resp.status().is_success() => {
                    tokio::time::timeout(TIMEOUT, resp.json::<serde_json::Value>()).await.ok().and_then(Result::ok)
                }
                _ => None,
            };
            let agents: Option<Vec<WorkFacts>> =
                body.and_then(|b| serde_json::from_value(b.get("agents")?.clone()).ok());
            (peer.channel, agents)
        });
    }
    let (mut facts, mut unreachable) = (Vec::new(), Vec::new());
    while let Some(res) = set.join_next().await {
        match res {
            Ok((_, Some(agents))) => facts.extend(agents),
            Ok((channel, None)) => unreachable.push(channel),
            Err(_) => {}
        }
    }
    unreachable.sort();
    (facts, unreachable)
}
