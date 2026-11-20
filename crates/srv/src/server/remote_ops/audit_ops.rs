// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `audit.query`: a paired computer reads this computer's Audit, and this computer reads theirs
//! (SPEC_AUDIT_LIVE_LOG_OF_EVERY_AGENT_ACTION_2026_11_03.md §3.6 point 5). A read, answered only when the owner turned
//! on `audit:sharewithpaired` (off unless set, like `tower:sharewithpaired`), and only to a peer whose install this
//! computer has approved: the rows name agents and what they did. The answer is the first page of the `machine`
//! scope, the same rows the window lists, with no message text, and without the links that only work here (a pane, a
//! history, a Kanban task). The asking computer shows the rows and forgets them: nothing is stored, and the cloud never
//! carries them.

use std::collections::HashSet;
use std::time::Duration;

use serde_json::Value;

use super::op::{OpStatus, RemoteOpResult};
use super::{AppState, RemoteCaller, RemoteRefusal, RemoteResult};
use crate::backend::audit_query::peers::{self, QUERY_OP, SETTING_AUDIT_SHARE};
use crate::backend::lan::discovery::LanInstance;
use crate::backend::rpc_types::{AuditPeerRows, AuditQueryReq, AuditQueryResult};

/// How long the Audit waits for one computer: it is a list on screen, and the others should not wait on a slow one.
const ASK_FOR: Duration = Duration::from_secs(4);
const REASON: &str = "list its Audit";

fn shares(state: &AppState) -> bool {
    state.config_watcher.get_settings().extra.get(SETTING_AUDIT_SHARE).and_then(Value::as_bool).unwrap_or(false)
}

/// The receiving half: this computer's page, when it shares.
pub(super) async fn query(state: &AppState, _caller: &RemoteCaller, params: Value) -> RemoteResult {
    if !shares(state) {
        return Err(RemoteRefusal::new("not_shared", format!("this computer does not share its Audit with paired computers ({SETTING_AUDIT_SHARE})")));
    }
    let asked: AuditQueryReq = serde_json::from_value(params).map_err(|e| RemoteRefusal::failed(format!("audit.query: {e}")))?;
    let req = peers::request_for_peer(&asked);
    let mut page = crate::server::app_api::audit_query::machine_page(state, &req).await.map_err(RemoteRefusal::failed)?;
    peers::without_local_links(&mut page.entries);
    for channel in page.other_channels.iter_mut().flatten() {
        peers::without_local_links(&mut channel.entries);
    }
    serde_json::to_value(&page).map_err(|e| RemoteRefusal::failed(format!("audit.query: {e}")))
}

/// The machines to ask, as `(name, host)`: `name` for the section, `host` as `send` takes it. One per computer, the
/// first instance announced, named as `send::resolve_peer` takes it: `name`, or `name/channel`, or
/// `name/channel/install` when the computer runs several, so the name picks exactly one. That instance answers for the
/// computer's other channels too, which it asks over loopback.
fn lan_hosts(instances: &[LanInstance]) -> Vec<(String, String)> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for p in instances {
        let name = p.hostname.trim();
        if name.is_empty() || !seen.insert(name.to_ascii_lowercase()) {
            continue;
        }
        let same_name: Vec<&LanInstance> = instances.iter().filter(|q| q.hostname.trim().eq_ignore_ascii_case(name)).collect();
        let same_channel = same_name.iter().filter(|q| q.channel.eq_ignore_ascii_case(&p.channel)).count();
        let host = match (same_name.len(), same_channel) {
            (1, _) => name.to_string(),
            (_, 1) => format!("{name}/{}", p.channel),
            _ => format!("{name}/{}/{}", p.channel, p.install),
        };
        out.push((name.to_string(), host));
    }
    out
}

/// A computer's answer as its section: the rows, or why there are none.
fn rows_from(name: &str, answer: RemoteOpResult) -> AuditPeerRows {
    if answer.status != OpStatus::Done {
        let error = answer.result.get("error").and_then(Value::as_str);
        return peers::no_rows(name, &peers::note_for(answer.refused_reason.as_deref(), error));
    }
    match serde_json::from_value::<AuditQueryResult>(answer.result) {
        Ok(page) => peers::rows_of(name, page),
        Err(e) => peers::no_rows(name, &format!("its answer was not understood: {e}")),
    }
}

/// The asking half: every paired computer on the LAN, concurrently, signed as one of this computer's agents. Empty
/// while remote operations are off or no agent is running to sign with.
pub(crate) async fn ask_peers(state: &AppState, req: &AuditQueryReq) -> Vec<AuditPeerRows> {
    if !super::enabled(state) {
        return Vec::new();
    }
    let Some(slug) = super::fleet_sync::signer(state) else { return Vec::new() };
    let params = serde_json::to_value(peers::request_for_peer(req)).unwrap_or(Value::Null);
    let hosts = lan_hosts(&state.lan_discovery.get_instances());
    futures_util::future::join_all(hosts.into_iter().map(|(name, host)| {
        let (slug, params) = (slug.clone(), params.clone());
        async move {
            match tokio::time::timeout(ASK_FOR, super::send::call(state, &slug, &host, QUERY_OP, params, REASON)).await {
                Ok(answer) => rows_from(&name, answer),
                Err(_) => peers::no_rows(&name, "it did not answer in time"),
            }
        }
    }))
    .await
}

#[cfg(test)]
#[path = "audit_ops_tests.rs"]
mod tests;
