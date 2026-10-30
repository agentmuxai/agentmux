// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Keeps the fleet's Global Memory in step (task #12439, section 4.6): the sending half of
//! `memory.global.put`, `.remove` and `.list`.
//!
//! One loop, no change observer: every few seconds this machine fingerprints what it holds for the
//! fleet (`backend::fleet_memory::fingerprint`) and, for each LAN peer whose last sync had another
//! fingerprint (or that has not been synced since it appeared, which is how a machine that was
//! offline catches up), lists what the peer holds and sends what it lacks or has older. Each side
//! runs the same loop, so what the peer holds newer comes back on its own turn. A peer that is not
//! reachable, is not proven, or does not have the ops is left alone until a retry time. Nothing is
//! sent when nothing changed, so the audit trails only carry real work.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use std::future::Future;

use serde_json::{json, Value};

use agentmux_common::remote_op::{OpStatus, RemoteOpResult};

use super::{enabled, send};
use crate::backend::fleet_memory::{self, FleetMeta, Push};
use crate::backend::storage::store::Store;
use crate::server::AppState;

/// How often this machine checks whether it has something new for its peers.
const TICK: Duration = Duration::from_secs(15);
/// A peer is synced again after this long even if nothing here changed, to heal a lost push.
const RESYNC: Duration = Duration::from_secs(10 * 60);
/// A peer that failed is left alone this long.
const RETRY: Duration = Duration::from_secs(5 * 60);
const REASON: &str = "fleet Global Memory sync";
/// The signed form of params is a little longer than their JSON text.
const SIGNING_SLACK: usize = 256;

/// One machine to sync with: a key that tells it from the others, and the `host` the send takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Peer {
    pub key: String,
    pub host: String,
}

/// What the last sync of one peer saw.
#[derive(Debug, Clone)]
pub(super) struct Synced {
    fingerprint: String,
    at: Instant,
    /// Set after a failure: the peer is skipped until then.
    retry_at: Option<Instant>,
}

/// Start the loop. Does nothing while remote operations are off.
pub(crate) fn install(state: AppState) {
    tokio::spawn(async move {
        let mut seen: HashMap<String, Synced> = HashMap::new();
        let mut tick = tokio::time::interval(TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            if enabled(&state) {
                sync_once(&state, &mut seen).await;
            }
        }
    });
}

fn peers(state: &AppState) -> Vec<Peer> {
    state
        .lan_discovery
        .get_instances()
        .into_iter()
        .map(|p| {
            let name = p.hostname.trim().to_string();
            let host = if p.channel.is_empty() { name } else { format!("{name}/{}", p.channel) };
            let key = if p.instance_id.is_empty() { format!("{}:{}", p.address, p.port) } else { p.instance_id };
            Peer { key, host }
        })
        .collect()
}

/// An agent of this machine to sign with: the requests are `slug@machine` on both audit trails.
fn signer(state: &AppState) -> Option<String> {
    state.fleet_feed.snapshot().agents.iter().find(|a| !a.trim().is_empty()).cloned()
}

async fn sync_once(state: &AppState, seen: &mut HashMap<String, Synced>) {
    let peers = peers(state);
    if peers.is_empty() {
        seen.clear();
        return;
    }
    let Some(slug) = signer(state) else { return };
    let call = |host: String, op: &'static str, params: Value| {
        let slug = slug.clone();
        async move { send::call(state, &slug, &host, op, params, REASON).await }
    };
    sync_peers(state.id_store.clone(), &peers, seen, &call, Instant::now()).await;
}

/// Sync every peer that is due. Separate from [`sync_once`] so a test can give it a fake machine.
pub(super) async fn sync_peers<F, Fut>(store: Arc<Store>, peers: &[Peer], seen: &mut HashMap<String, Synced>, call: &F, now: Instant)
where
    F: Fn(String, &'static str, Value) -> Fut,
    Fut: Future<Output = RemoteOpResult>,
{
    // A peer that went away is synced as new when it comes back.
    seen.retain(|key, _| peers.iter().any(|p| &p.key == key));
    let local = match blocking(store.clone(), fleet_memory::list_meta).await {
        Ok(local) => local,
        Err(e) => {
            tracing::warn!(error = %e, "fleet memory sync: could not read what this machine holds");
            return;
        }
    };
    let fingerprint = fleet_memory::fingerprint(&local);
    for peer in peers {
        let due = match seen.get(&peer.key) {
            None => true,
            Some(s) => match s.retry_at {
                Some(at) => now >= at,
                None => s.fingerprint != fingerprint || now.duration_since(s.at) >= RESYNC,
            },
        };
        if !due {
            continue;
        }
        match sync_peer(&store, peer, &local, call).await {
            Ok(sent) => {
                if sent > 0 {
                    tracing::info!(peer = %peer.host, sent, "fleet memory: sent entries to another machine");
                }
                seen.insert(peer.key.clone(), Synced { fingerprint: fingerprint.clone(), at: now, retry_at: None });
            }
            Err(why) => {
                tracing::debug!(peer = %peer.host, why, "fleet memory sync: left for later");
                seen.insert(peer.key.clone(), Synced { fingerprint: String::new(), at: now, retry_at: Some(now + RETRY) });
            }
        }
    }
}

async fn blocking<T: Send + 'static>(
    store: Arc<Store>,
    f: impl FnOnce(&Store) -> Result<T, crate::backend::storage::error::StoreError> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(move || f(&store))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

fn why_not(answer: &RemoteOpResult) -> String {
    answer
        .refused_reason
        .clone()
        .or_else(|| answer.result.get("error").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_else(|| format!("{:?}", answer.status))
}

/// List what `peer` holds and send it what it lacks. Returns how many entries and removals went.
/// An entry the peer takes badly (one too big to send, say) is skipped and counted, not retried
/// now; the peer is retried later only when the list itself failed or a send could not reach it.
async fn sync_peer<F, Fut>(store: &Arc<Store>, peer: &Peer, local: &[FleetMeta], call: &F) -> Result<usize, String>
where
    F: Fn(String, &'static str, Value) -> Fut,
    Fut: Future<Output = RemoteOpResult>,
{
    let answer = call(peer.host.clone(), "memory.global.list", json!({})).await;
    if answer.status != OpStatus::Done {
        return Err(format!("list: {}", why_not(&answer)));
    }
    let remote: Vec<FleetMeta> =
        serde_json::from_value(answer.result.get("entries").cloned().unwrap_or(Value::Null)).map_err(|e| format!("list: not understood: {e}"))?;
    let mut sent = 0;
    for push in fleet_memory::to_push(local, &remote) {
        let (op, params) = match push {
            Push::Put { id } => match blocking(store.clone(), move |s| fleet_memory::entry_for_push(s, &id)).await? {
                Some(entry) => ("memory.global.put", serde_json::to_value(entry).map_err(|e| e.to_string())?),
                // Removed or made machine-only since the listing: nothing to send.
                None => continue,
            },
            Push::Remove { id, removed_at } => {
                let removal = blocking(store.clone(), move |s| fleet_memory::removal_for_push(s, &id, removed_at)).await?;
                ("memory.global.remove", serde_json::to_value(removal).map_err(|e| e.to_string())?)
            }
        };
        // The transport signs at most MAX_PARAMS_BYTES of params. A bigger entry waits for the
        // per-op limit; it is skipped here, and logged, so the others still go.
        if params.to_string().len() + SIGNING_SLACK > agentmux_common::remote_op::MAX_PARAMS_BYTES {
            tracing::warn!(peer = %peer.host, op, bytes = params.to_string().len(), "fleet memory: an entry is too big to send; skipped");
            continue;
        }
        let answer = call(peer.host.clone(), op, params).await;
        match answer.status {
            OpStatus::Done => sent += 1,
            // The machine answered and said no to this one entry (too big, malformed): go on.
            OpStatus::Refused if matches!(answer.refused_reason.as_deref(), Some("bad_params")) => {
                tracing::warn!(peer = %peer.host, op, "fleet memory: the other machine refused an entry as malformed");
            }
            _ => return Err(format!("{op}: {}", why_not(&answer))),
        }
    }
    Ok(sent)
}

#[cfg(test)]
mod tests;
