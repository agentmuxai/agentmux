// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The sync layer's loop on the LAN (SPEC_USER_DATA_SYNC_LAYER §4.2): the sending half of the `sync.*` ops. Every few
//! seconds (or at once when something calls [`wake`]) this machine looks at the collections it is a member of and, for
//! each other member it can see on the LAN, runs one round (`backend::user_sync::round`) when something changed here
//! since the last round with it, when it has not been synced for a while (to heal a lost push), or when it was never
//! synced. A peer that is offline, unproven or without the ops is left alone until a retry time. Nothing is sent when
//! nothing changed, so the audit trails carry only real work.
//!
//! Only an install that holds the computer's sync lock plans anything, and a round lends the store to the sync layer
//! one step at a time (never across a call to the peer), so this machine keeps answering the peer's own requests
//! while its round is running.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use tokio::runtime::Handle;

use super::op::{OpStatus, RemoteOpResult};
use super::sync_ops::{self, HelloAns};
use super::{enabled, fleet_sync, send};
use crate::backend::storage::store::Store;
use crate::backend::storage::wan_identity::{self, WanIdentityStore};
use crate::backend::user_sync::members::Kind;
use crate::backend::user_sync::ops::{self, Ctx, Held, KeysAns, KeysReq, MembersAns, MembersReq, PullAns, PullReq, PushAns, PushReq};
use crate::backend::user_sync::round::{self, CallError, Ended, PeerCalls, Round, Session};
use crate::backend::user_sync::{chain_store, log};
use crate::server::AppState;

/// How often this machine checks whether it has something new for its peers.
const TICK: Duration = Duration::from_secs(15);
/// A peer is synced again after this long even if nothing here changed, to heal a lost push.
const RESYNC: Duration = Duration::from_secs(10 * 60);
/// A peer that failed is left alone this long.
const RETRY: Duration = Duration::from_secs(5 * 60);
const REASON: &str = "sync layer";

static WAKE: tokio::sync::Notify = tokio::sync::Notify::const_new();

/// Wakes the loop now: something here changed a collection, so its peers should hear of it within a second instead of
/// at the next tick.
#[allow(dead_code)]
pub(crate) fn wake() {
    WAKE.notify_one();
}

/// A collection this machine is a member of, the other machines in it and what this machine holds of it now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Plan {
    pub collection: String,
    pub kind: String,
    pub members: Vec<String>,
    /// Changes whenever this machine's chain or records of the collection do.
    pub fingerprint: String,
}

/// A machine seen on the LAN: the install id it advertises (what a chain names it by) and the `host` a send takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Seen {
    pub install_id: String,
    pub host: String,
}

/// One round to run: a collection with one of its members that is reachable now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Target {
    pub collection: String,
    pub kind: String,
    pub machine: String,
    pub host: String,
    pub fingerprint: String,
}

/// What the last round with one peer left behind.
#[derive(Debug, Clone)]
pub(super) struct Last {
    fingerprint: String,
    at: Instant,
    /// Set after a round that did not finish: the peer is skipped until then.
    retry_at: Option<Instant>,
}

/// Start the loop. Does nothing while remote operations are off.
pub(crate) fn install(state: AppState) {
    tokio::spawn(async move {
        let mut last: HashMap<(String, String), Last> = HashMap::new();
        let mut tick = tokio::time::interval(TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = tick.tick() => {}
                _ = WAKE.notified() => {}
            }
            if enabled(&state) {
                sync_once(&state, &mut last).await;
            }
        }
    });
}

/// The collections to sync, read from this machine's state under the lock: none unless this install holds it.
fn plans(store: &Store, wan: &WanIdentityStore) -> Result<Vec<Plan>, String> {
    // An install with no collection never asks for the computer's sync lock: another install of this computer may be
    // the one that holds collections, and the lock is theirs to keep.
    let any = {
        let conn = store.conn().lock().map_err(|_| "the store is busy".to_string())?;
        log::init(&conn).map_err(|e| e.to_string())?;
        !chain_store::load_all(&conn).map_err(|e| format!("{e:?}"))?.is_empty()
    };
    if !any {
        return Ok(Vec::new());
    }
    sync_ops::with_ctx(store, wan, None, |ctx: &mut Ctx| {
        if !ctx.active {
            return Vec::new();
        }
        let mut out = Vec::new();
        for (collection, chain) in ctx.chains.iter() {
            let Ok(state) = chain.state(collection) else { continue };
            if !state.members.get(ctx.me.id).is_some_and(|m| m.kind == Kind::Machine) {
                continue;
            }
            let members: Vec<String> = state.members.values().filter(|m| m.kind == Kind::Machine && m.id != ctx.me.id).map(|m| m.id.clone()).collect();
            if members.is_empty() {
                continue;
            }
            let (Ok(kind), Ok(vector)) = (ops::kind_of(ctx.conn, collection), log::vector(ctx.conn, collection)) else { continue };
            let fingerprint = format!("{}:{}:{}:{}", state.n, state.epoch, state.head.clone().unwrap_or_default(), json!(vector));
            out.push(Plan { collection: collection.clone(), kind, members, fingerprint });
        }
        out
    })
    .map_err(|e| e.message)
}

/// The rounds to run: each collection with each of its members that is on the LAN now.
pub(super) fn targets(plans: &[Plan], seen: &[Seen]) -> Vec<Target> {
    let mut out = Vec::new();
    for plan in plans {
        for member in &plan.members {
            if let Some(peer) = seen.iter().find(|p| !p.install_id.is_empty() && &p.install_id == member) {
                out.push(Target { collection: plan.collection.clone(), kind: plan.kind.clone(), machine: member.clone(), host: peer.host.clone(), fingerprint: plan.fingerprint.clone() });
            }
        }
    }
    out
}

/// Whether a round with this peer is due: it never ran, or its retry time has come, or what this machine holds changed,
/// or it has been a while.
pub(super) fn due(last: Option<&Last>, fingerprint: &str, now: Instant) -> bool {
    match last {
        None => true,
        Some(l) => match l.retry_at {
            Some(at) => now >= at,
            None => l.fingerprint != fingerprint || now.duration_since(l.at) >= RESYNC,
        },
    }
}

/// What to remember of a round: a peer that answered is left alone until something changes; one that did not is left
/// alone for a while. A round that changed what this machine holds is looked at again at the next tick, so what it
/// took goes on to the other members.
pub(super) fn remember(round: &Round, fingerprint: &str, now: Instant) -> Last {
    let finished = matches!(round.ended, Ended::Synced);
    if finished && !round.changed() {
        Last { fingerprint: fingerprint.to_string(), at: now, retry_at: None }
    } else if finished {
        Last { fingerprint: String::new(), at: now, retry_at: None }
    } else {
        Last { fingerprint: String::new(), at: now, retry_at: Some(now + RETRY) }
    }
}

fn seen_on_lan(state: &AppState) -> Vec<Seen> {
    state
        .lan_discovery
        .get_instances()
        .into_iter()
        .map(|p| {
            let name = p.hostname.trim().to_string();
            let host = if p.channel.is_empty() { name } else { format!("{name}/{}", p.channel) };
            Seen { install_id: p.install_id, host }
        })
        .collect()
}

async fn sync_once(state: &AppState, last: &mut HashMap<(String, String), Last>) {
    let Some(slug) = fleet_sync::signer(state) else { return };
    let Some(wan) = state.mstore.wan_identity().or_else(wan_identity::global) else { return };
    let store = state.id_store.clone();
    let planned = {
        let (store, wan) = (store.clone(), wan.clone());
        match tokio::task::spawn_blocking(move || plans(&store, &wan)).await {
            Ok(Ok(planned)) => planned,
            Ok(Err(e)) => {
                tracing::debug!(error = %e, "sync: could not read what this machine holds");
                return;
            }
            Err(_) => return,
        }
    };
    last.retain(|(collection, _), _| planned.iter().any(|p| &p.collection == collection));
    let now = Instant::now();
    for target in targets(&planned, &seen_on_lan(state)) {
        let key = (target.collection.clone(), target.machine.clone());
        if !due(last.get(&key), &target.fingerprint, now) {
            continue;
        }
        let done = run_round(state.clone(), store.clone(), wan.clone(), slug.clone(), target.clone()).await;
        match done {
            Some(r) => {
                report(&target, &r);
                last.insert(key, remember(&r, &target.fingerprint, Instant::now()));
            }
            None => {
                last.insert(key, Last { fingerprint: String::new(), at: now, retry_at: Some(now + RETRY) });
            }
        }
    }
}

fn report(target: &Target, r: &Round) {
    if let Ended::Removed { by } = &r.ended {
        tracing::warn!(collection = %target.collection, peer = %target.host, removed_by = %by, "sync: this machine was removed from a collection");
    } else if r.changed() {
        tracing::info!(collection = %target.collection, peer = %target.host, pulled = r.pulled, pushed = r.pushed, entries = r.entries_taken + r.entries_given, keys = r.keys_taken, "sync: a round with another machine");
    } else if !matches!(r.ended, Ended::Synced) {
        tracing::debug!(collection = %target.collection, peer = %target.host, ended = ?r.ended, "sync: a round left for later");
    }
}

/// One round on a blocking thread: its calls to the peer wait on this runtime, and its steps on this machine's state
/// each take the store for their own length.
async fn run_round(state: AppState, store: Arc<Store>, wan: Arc<WanIdentityStore>, slug: String, target: Target) -> Option<Round> {
    let handle = Handle::current();
    tokio::task::spawn_blocking(move || {
        let adapter = sync_ops::adapter_for(&target.kind)?;
        let mut mine = Mine { store: &store, wan: &wan };
        let mut peer = Peer { state, slug, host: target.host, handle };
        Some(round::sync_collection(&mut mine, &mut peer, &target.collection, adapter.as_ref()))
    })
    .await
    .ok()
    .flatten()
}

struct Mine<'a> {
    store: &'a Store,
    wan: &'a WanIdentityStore,
}

impl Session for Mine<'_> {
    fn with<R>(&mut self, f: impl FnOnce(&mut Ctx) -> R) -> Result<R, String> {
        sync_ops::with_ctx(self.store, self.wan, None, f).map_err(|e| e.message)
    }
}

/// The calls to one peer, as signed remote ops sent by one of this machine's agents.
struct Peer {
    state: AppState,
    slug: String,
    host: String,
    handle: Handle,
}

impl Peer {
    fn call<Ans: DeserializeOwned>(&self, op: &'static str, params: Value) -> Result<Ans, CallError> {
        let answer = self.handle.block_on(send::call(&self.state, &self.slug, &self.host, op, params, REASON));
        answer_of(answer)
    }

    fn params<T: serde::Serialize>(req: &T) -> Result<Value, CallError> {
        serde_json::to_value(req).map_err(|e| CallError::Unreachable(format!("a request that cannot be written: {e}")))
    }
}

impl PeerCalls for Peer {
    fn hello(&mut self, collections: &[String]) -> Result<Vec<Held>, CallError> {
        Ok(self.call::<HelloAns>("sync.hello", json!({ "collections": collections }))?.held)
    }
    fn members(&mut self, req: &MembersReq) -> Result<MembersAns, CallError> {
        self.call("sync.members", Self::params(req)?)
    }
    fn keys(&mut self, req: &KeysReq) -> Result<KeysAns, CallError> {
        self.call("sync.keys", Self::params(req)?)
    }
    fn pull(&mut self, req: &PullReq) -> Result<PullAns, CallError> {
        self.call("sync.pull", Self::params(req)?)
    }
    fn push(&mut self, req: &PushReq) -> Result<PushAns, CallError> {
        self.call("sync.push", Self::params(req)?)
    }
}

/// A peer's answer as the round reads it: what it says, a refusal with its code, or the machine not answering.
pub(super) fn answer_of<Ans: DeserializeOwned>(answer: RemoteOpResult) -> Result<Ans, CallError> {
    match answer.status {
        OpStatus::Done => serde_json::from_value(answer.result).map_err(|e| CallError::Refused(format!("bad_answer: {e}"))),
        OpStatus::Refused => Err(CallError::Refused(answer.refused_reason.unwrap_or_else(|| "refused".to_string()))),
        OpStatus::Failed => {
            let why = answer.result.get("error").and_then(Value::as_str).unwrap_or("failed").to_string();
            Err(CallError::Unreachable(why))
        }
        other => Err(CallError::Refused(format!("not_done: {other:?}"))),
    }
}

#[cfg(test)]
mod tests;
