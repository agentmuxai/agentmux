// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The other channels on this computer, asked over loopback for the rows they keep in their own file
//! (SPEC_AUDIT_LIVE_LOG_OF_EVERY_AGENT_ACTION_2026_11_03.md §3.6 point 5). A channel that gave the shared folder up
//! writes its rows to its own file, which this srv cannot read; the channel is asked for them with its `local_url` and
//! key from the host-global registry, exactly as the Swarm asks it for its agents (`swarm_remote::channel_statuses`).
//! A channel that fails, times out, answers badly or predates the route contributes nothing.

use std::time::Duration;

use crate::backend::rpc_types::{AuditChannelRows, AuditQueryReq, AuditQueryResult};
use crate::backend::swarm_remote::ChannelStatusTarget;

/// The route a channel answers on: its own file's rows, as an `audit.query` answer.
pub const OWN_ROWS_PATH: &str = "/agentmux/audit/own";

/// Loopback, so a channel that takes longer is hung, and the Audit shouldn't wait on it.
const TIMEOUT: Duration = Duration::from_millis(2500);
/// A page of 200 rows is well under this; a larger answer is not an answer.
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

/// What to send another channel: the filter of this request, for its first page. Its paging, scope and totals are this
/// srv's own.
pub fn request_for_others(req: &AuditQueryReq) -> AuditQueryReq {
    AuditQueryReq { cursor: None, totals: None, scope: Some("channel".to_string()), ..req.clone() }
}

/// Asks each target concurrently. One entry per channel that answered with rows, in the order of `targets`.
pub async fn ask(http: &reqwest::Client, targets: &[ChannelStatusTarget], req: &AuditQueryReq) -> Vec<AuditChannelRows> {
    let req = request_for_others(req);
    let answers = futures_util::future::join_all(targets.iter().map(|t| {
        let req = &req;
        async move {
            let resp = http
                .post(format!("{}{OWN_ROWS_PATH}", t.local_url))
                .header(agentmux_common::AUTH_KEY_HEADER, &t.auth_key)
                .json(req)
                .timeout(TIMEOUT)
                .send()
                .await
                .ok()
                .filter(|r| r.status().is_success())?;
            let body = crate::backend::lan::discovery::LanDiscovery::read_body_capped(resp, MAX_RESPONSE_BYTES).await.ok()?;
            let result: AuditQueryResult = serde_json::from_slice(&body).ok()?;
            Some(rows_of(&t.channel, result))
        }
    }))
    .await;
    answers.into_iter().flatten().filter(|c| !c.entries.is_empty()).collect()
}

/// A channel's answer as the rows of its section. Each row says which channel it is from and gets an id of its own;
/// a pane link is dropped, since the pane is in the other channel's window.
pub fn rows_of(channel: &str, result: AuditQueryResult) -> AuditChannelRows {
    let mut entries = result.entries;
    for e in &mut entries {
        e.id = format!("{channel}:{}", e.id);
        e.links.pane = None;
        for end in [e.from.as_mut(), e.to.as_mut()].into_iter().flatten() {
            end.links.pane = None;
        }
        let provenance = e.provenance.get_or_insert_with(Default::default);
        if provenance.channel.as_deref().is_none_or(str::is_empty) {
            provenance.channel = Some(channel.to_string());
        }
    }
    AuditChannelRows { channel: channel.to_string(), entries, more: result.next_cursor.is_some() }
}

#[cfg(test)]
#[path = "other_channels_tests.rs"]
mod tests;
