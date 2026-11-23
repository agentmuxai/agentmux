// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! `sync.hello`, `sync.pull`, `sync.push`, `sync.members`, `sync.join` and `sync.keys`: the transport's thin handlers over the sync layer's op
//! bodies (`backend::user_sync::ops`, SPEC_USER_DATA_SYNC_LAYER). Each is a write op, so the transport has already
//! required a certificate from an install this machine trusts. The caller the layer sees is that install's id: the
//! member of a collection is the install that holds this computer's sync lock, so a request is answered only while this
//! install holds it, and only for a caller that is a current member of the collection it names. Nothing here
//! sends anything or reaches the relay. A join waits for a person unless the collection allows it; the collection's keys
//! reach a member sealed to its box key and never otherwise.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{AppState, RemoteCaller, RemoteRefusal, RemoteResult};
use crate::backend::storage::store::Store;
use crate::backend::storage::wan_identity::{self, WanIdentityStore};
use crate::backend::user_sync::adapter::Adapter;
use crate::backend::user_sync::hlc::Clock;
use crate::backend::user_sync::kanban::KanbanAdapter;
use crate::backend::user_sync::lock::{self, Lock, LEASE_MS};
use crate::backend::user_sync::keys::{SqlKeys, StoreKeys};
use crate::backend::user_sync::ops::{self, Ctx, Held, JoinReq, Joined, KeysAns, KeysReq, Me, MembersAns, MembersReq, OpError, Proof, PullAns, PullReq, PushAns, PushReq};
use crate::backend::user_sync::{chain_store, log};

/// How long a peer's pin must have been held here before a join may go ahead without a person.
pub(super) const MIN_PIN_AGE_MS: i64 = 10 * 60 * 1000;

/// What the transport proved about the caller, as the layer's join check needs it.
struct LanProof(bool);

impl Proof for LanProof {
    fn auto_proven(&self, _machine: &str) -> bool {
        self.0
    }
}

#[derive(Deserialize)]
struct HelloReq {
    collections: Vec<String>,
}

#[derive(Serialize)]
struct HelloAns {
    held: Vec<Held>,
}

fn parse<T: DeserializeOwned>(op: &str, params: Value) -> Result<T, RemoteRefusal> {
    serde_json::from_value(params).map_err(|e| RemoteRefusal::new("bad_params", format!("{op}: {e}")))
}

fn refusal(e: OpError) -> RemoteRefusal {
    let message = match &e {
        OpError::NotActive => "this install does not hold the computer's sync lock".to_string(),
        OpError::NotMember => "not a member of that collection".to_string(),
        OpError::TooBig => "over the size a sync op takes or gives".to_string(),
        OpError::Bad => "not a request this machine can take".to_string(),
        OpError::NoKey => "no key to give".to_string(),
        OpError::Db(m) => m.clone(),
    };
    RemoteRefusal::new(e.code(), message)
}

fn failed(what: impl std::fmt::Display) -> RemoteRefusal {
    RemoteRefusal::failed(what.to_string())
}

/// Everything an op needs from this machine, then the op: this install's identity, the computer's lock, the chains
/// read back from the store and the caller's proof.
fn exec<Req, Ans>(
    store: &Store,
    wan: &WanIdentityStore,
    machine: &str,
    pinned_since: Option<i64>,
    req: Req,
    run: impl FnOnce(&mut Ctx, &str, Req) -> Result<Ans, OpError>,
) -> Result<Ans, RemoteRefusal> {
    let me = wan.instance_ensure(&crate::backend::reactive::registry::local_host_label()).map_err(failed)?;
    let now = agentmux_common::time::now_ms();
    let conn = store.conn().lock().map_err(|_| failed("the store is busy"))?;
    log::init(&conn).map_err(failed)?;
    let active = matches!(lock::acquire(&conn, &me.instance_id, now, LEASE_MS).map_err(failed)?, Lock::Held { .. });
    let mut chains = chain_store::load_all(&conn).map_err(|e| failed(format!("a stored membership chain is not sound: {e:?}")))?;
    let mut clock = Clock::new(&me.instance_id);
    let mut pending = chain_store::load_pending(&conn).map_err(|e| failed(format!("the waiting joins are not readable: {e:?}")))?;
    let keystore = SqlKeys(&conn);
    let keys = StoreKeys { store: &keystore, wan_public: me.public_key, wan_seed: me.private_key };
    let proof = LanProof(pinned_since.is_some_and(|since| now - since >= MIN_PIN_AGE_MS));
    let mut ctx = Ctx {
        conn: &conn,
        chains: &mut chains,
        clock: &mut clock,
        pending: &mut pending,
        me: Me { id: &me.instance_id, private: &me.private_key },
        proof: &proof,
        keys: &keys,
        now_ms: now,
        chain_cap: ops::MAX_ENTRIES,
        active,
    };
    let answer = run(&mut ctx, machine, req).map_err(refusal);
    chain_store::save_pending(&conn, &pending).map_err(failed)?;
    answer
}

async fn run<Req, Ans>(
    state: &AppState,
    caller: &RemoteCaller,
    op: &'static str,
    params: Value,
    f: impl FnOnce(&mut Ctx, &str, Req) -> Result<Ans, OpError> + Send + 'static,
) -> RemoteResult
where
    Req: DeserializeOwned + Send + 'static,
    Ans: Serialize + Send + 'static,
{
    if caller.instance_id.is_empty() {
        return Err(RemoteRefusal::new("needs_certificate", format!("{op} is answered only for an install that showed a certificate")));
    }
    let req: Req = parse(op, params)?;
    let store = state.id_store.clone();
    let wan = state.mstore.wan_identity().or_else(wan_identity::global).ok_or_else(|| failed("this install has no identity yet"))?;
    let pinned_since = state.mstore.lan_peer_pubkey_pin_since(&caller.slug).ok().flatten().map(|(_, at)| at);
    let machine = caller.instance_id.clone();
    let answer = tokio::task::spawn_blocking(move || exec(&store, &wan, &machine, pinned_since, req, f))
        .await
        .map_err(|e| failed(format!("{op} did not finish: {e}")))??;
    serde_json::to_value(answer).map_err(failed)
}

/// The adapter for a collection's kind, or none for a kind this build does not merge.
fn adapter_for(kind: &str) -> Option<Box<dyn Adapter>> {
    match kind {
        "kanban" => Some(Box::new(KanbanAdapter)),
        _ => None,
    }
}

pub(super) async fn hello(state: &AppState, caller: &RemoteCaller, params: Value) -> RemoteResult {
    run(state, caller, "sync.hello", params, |ctx, who, req: HelloReq| Ok(HelloAns { held: ops::hello(ctx, who, &req.collections)? })).await
}

pub(super) async fn pull(state: &AppState, caller: &RemoteCaller, params: Value) -> RemoteResult {
    run(state, caller, "sync.pull", params, |ctx, who, req: PullReq| -> Result<PullAns, OpError> { ops::pull(ctx, who, &req) }).await
}

pub(super) async fn push(state: &AppState, caller: &RemoteCaller, params: Value) -> RemoteResult {
    run(state, caller, "sync.push", params, |ctx, who, req: PushReq| -> Result<PushAns, OpError> {
        let kind = ops::kind_of(ctx.conn, &req.collection).map_err(|_| OpError::NotMember)?;
        let adapter = adapter_for(&kind).ok_or(OpError::Bad)?;
        ops::push(ctx, who, &req, adapter.as_ref())
    })
    .await
}

pub(super) async fn members(state: &AppState, caller: &RemoteCaller, params: Value) -> RemoteResult {
    run(state, caller, "sync.members", params, |ctx, who, req: MembersReq| -> Result<MembersAns, OpError> { ops::members(ctx, who, &req) }).await
}

pub(super) async fn join(state: &AppState, caller: &RemoteCaller, params: Value) -> RemoteResult {
    run(state, caller, "sync.join", params, |ctx, who, req: JoinReq| -> Result<Joined, OpError> { ops::join(ctx, who, &req) }).await
}

pub(super) async fn keys(state: &AppState, caller: &RemoteCaller, params: Value) -> RemoteResult {
    run(state, caller, "sync.keys", params, |ctx, who, req: KeysReq| -> Result<KeysAns, OpError> { ops::keys(ctx, who, &req) }).await
}

#[cfg(test)]
mod tests;
