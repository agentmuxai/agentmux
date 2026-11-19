// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Keeps the fleet's Global Memory in step (task #12439, section 4.6): the sending half of
//! `memory.global.put`, `.remove` and `.list`.
//!
//! One loop, no change observer: every few seconds this machine fingerprints what it holds for the
//! fleet (`backend::memory::fleet::fingerprint`) and, for each LAN peer whose last sync had another
//! fingerprint (or that has not been synced since it appeared, which is how a machine that was
//! offline catches up), lists what the peer holds and sends what it lacks or has older. Each side
//! runs the same loop, so what the peer holds newer comes back on its own turn. A peer that is not
//! reachable, is not proven, or does not have the ops is left alone until a retry time. Nothing is
//! sent when nothing changed, so the audit trails only carry real work. What each peer answered
//! (applied, or kept what it had and why) is kept in `status` for the Memory list, so the writer can
//! see which entries did not arrive.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use std::future::Future;

use serde_json::{json, Value};

use crate::server::remote_ops::op::{OpStatus, RemoteOpResult};

use super::{enabled, send};
use crate::backend::memory::fleet::{self, FleetMeta, Push};
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
    sync_peers_with(store, peers, seen, call, now, status::global()).await;
}

/// [`sync_peers`], with the status it reports to given, so a test reads its own.
pub(super) async fn sync_peers_with<F, Fut>(
    store: Arc<Store>,
    peers: &[Peer],
    seen: &mut HashMap<String, Synced>,
    call: &F,
    now: Instant,
    report: &status::Status,
) where
    F: Fn(String, &'static str, Value) -> Fut,
    Fut: Future<Output = RemoteOpResult>,
{
    // A peer that went away is synced as new when it comes back.
    seen.retain(|key, _| peers.iter().any(|p| &p.key == key));
    report.keep_only(&peers.iter().map(|p| p.host.as_str()).collect::<Vec<_>>());
    let local = match blocking(store.clone(), fleet::list_meta).await {
        Ok(local) => local,
        Err(e) => {
            tracing::warn!(error = %e, "fleet memory sync: could not read what this machine holds");
            return;
        }
    };
    let fingerprint = fleet::fingerprint(&local);
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
            Ok((sent, not_applied)) => {
                if sent > 0 {
                    tracing::info!(peer = %peer.host, sent, "fleet memory: sent entries to another machine");
                }
                for (id, why) in report.synced(&peer.host, agentmux_common::time::now_ms(), sent, not_applied) {
                    tracing::warn!(peer = %peer.host, entry = %id, why, "fleet memory: the other machine did not apply an entry");
                }
                seen.insert(peer.key.clone(), Synced { fingerprint: fingerprint.clone(), at: now, retry_at: None });
            }
            Err(why) => {
                tracing::debug!(peer = %peer.host, why, "fleet memory sync: left for later");
                report.failed(&peer.host, &why);
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

/// How the peer answered one entry: `Applied` when it changed what it holds, `Skipped(why)` when
/// it answered and kept what it had for a reason the writer should hear about, `Nothing` when
/// there was nothing to do (it already holds it, or holds a newer one, which comes back on its own
/// turn).
enum Answered {
    Applied,
    Skipped(String),
    Nothing,
}

fn answered(result: &Value) -> Answered {
    match result.get("outcome").and_then(Value::as_str) {
        Some("skipped") => match result.get("reason").and_then(Value::as_str).unwrap_or("not_applied") {
            "same" | "older" | "already_removed" | "unknown" => Answered::Nothing,
            why => Answered::Skipped(why.to_string()),
        },
        // An answer from a build that does not say what it did is counted as before.
        _ => Answered::Applied,
    }
}

/// List what `peer` holds and send it what it lacks. Returns how many entries and removals the peer
/// applied, and which ones it answered but did not apply (an owner's `machine` entry, say) with why.
/// An entry the peer takes badly (one too big to send, say) is skipped and counted, not retried
/// now; the peer is retried later only when the list itself failed or a send could not reach it.
async fn sync_peer<F, Fut>(
    store: &Arc<Store>,
    peer: &Peer,
    local: &[FleetMeta],
    call: &F,
) -> Result<(usize, std::collections::BTreeMap<String, String>), String>
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
    let mut not_applied = std::collections::BTreeMap::new();
    for push in fleet::to_push(local, &remote) {
        let entry_id = match &push {
            Push::Put { id } | Push::Remove { id, .. } => id.clone(),
        };
        let (op, params) = match push {
            Push::Put { id } => match blocking(store.clone(), move |s| fleet::entry_for_push(s, &id)).await? {
                Some(entry) => ("memory.global.put", serde_json::to_value(entry).map_err(|e| e.to_string())?),
                // Removed or made machine-only since the listing: nothing to send.
                None => continue,
            },
            Push::Remove { id, removed_at } => {
                let removal = blocking(store.clone(), move |s| fleet::removal_for_push(s, &id, removed_at)).await?;
                ("memory.global.remove", serde_json::to_value(removal).map_err(|e| e.to_string())?)
            }
        };
        // The transport signs at most the operation's params limit. A bigger entry is skipped here,
        // and logged, so the others still go.
        if params.to_string().len() + SIGNING_SLACK > super::max_params_of(op) {
            tracing::warn!(peer = %peer.host, op, bytes = params.to_string().len(), "fleet memory: an entry is too big to send; skipped");
            not_applied.insert(entry_id, "too_big".to_string());
            continue;
        }
        let answer = call(peer.host.clone(), op, params).await;
        match answer.status {
            OpStatus::Done => match answered(&answer.result) {
                Answered::Applied => sent += 1,
                Answered::Skipped(why) => {
                    not_applied.insert(entry_id, why);
                }
                Answered::Nothing => {}
            },
            // The machine answered and said no to this one entry (too big, malformed): go on.
            OpStatus::Refused if matches!(answer.refused_reason.as_deref(), Some("bad_params")) => {
                tracing::warn!(peer = %peer.host, op, "fleet memory: the other machine refused an entry as malformed");
                not_applied.insert(entry_id, "bad_params".to_string());
            }
            // The machine ran this one entry and it failed there (a store error on its side, from an
            // older build than one that answers `name_taken`): say so for the entry and go on. One entry
            // the peer cannot take must not stop the rest. An answer with no audit id never ran.
            OpStatus::Failed if !answer.audit_id.is_empty() => {
                let why = why_not(&answer);
                tracing::warn!(peer = %peer.host, op, entry = %entry_id, why, "fleet memory: the other machine failed on an entry");
                not_applied.insert(entry_id, format!("failed: {}", why.chars().take(120).collect::<String>()));
            }
            _ => return Err(format!("{op}: {}", why_not(&answer))),
        }
    }
    Ok((sent, not_applied))
}

pub(crate) mod status;

#[cfg(test)]
mod tests;
