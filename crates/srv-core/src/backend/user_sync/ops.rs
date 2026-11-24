// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! The bodies of the `sync.*` ops as pure functions (SPEC_USER_DATA_SYNC_LAYER §3.1, §4): `hello`, `pull`, `push`,
//! `members` and `join`, over the log, the membership chains and the caller's proven machine id. Nothing here opens a
//! connection or sends anything: a request comes in, an answer goes out, and what a function may touch is what
//! [`Ctx`] hands it. The transport (the remote-ops route on the LAN) is wired to these separately.
//!
//! Every op starts with the same two checks: this install holds the computer's sync lock (otherwise it answers
//! nothing), and the caller is a current member of the collection it names (a stranger, and a collection this
//! machine does not hold, get the same refusal). Both answers are capped: at the handler on the way out, and by
//! [`check_pull_answer`] on the way in.

use std::collections::BTreeMap;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use super::adapter::Adapter;
use super::chain_store;
use super::hlc::Clock;
use super::log::{self, Outcome, Refused, Signer, SyncError, Vector};
use super::members::{Chain, Change, Entry, Kind, MemberError, Policy, State, Subject, Taken};
use super::record::{Record, MAX_RECORD_BYTES};
use super::seal::{self, KeyRing};

/// Most collections one `hello` may ask about, and most origins one vector may name.
pub const MAX_NAMES: usize = 64;
/// Most records in one `pull` answer or `push` request.
pub const MAX_BATCH_RECORDS: usize = 200;
/// Most bytes in one batch, counting each record's body and a fixed amount for its header. One record larger than
/// this goes alone (up to `MAX_RECORD_BYTES`).
pub const MAX_BATCH_BYTES: usize = 256 * 1024;
/// Most entries of a membership chain one answer carries; a longer chain is not served until it is compacted.
pub const MAX_ENTRIES: usize = 256;
/// Most join requests one collection keeps waiting for a person.
pub const MAX_PENDING: usize = 16;
/// How long a join waits for a person before it is dropped (the machine asks again).
pub const PENDING_TTL_MS: i64 = 24 * 60 * 60 * 1000;
/// Kinds of collection whose joins may go ahead without a person confirming (a Vault never does yet).
pub const AUTO_JOIN_KINDS: [&str; 1] = ["kanban"];

/// The most bytes of JSON params a sync op takes: a batch at its largest as base64, its headers, and what is around
/// them. A record too large for a push is pulled by the peer instead (an answer is not held to this).
pub const MAX_WIRE_BYTES: usize = MAX_BATCH_BYTES / 3 * 4 + MAX_BATCH_RECORDS * 1024 + 64 * 1024;

const HEADER_WEIGHT: usize = 512;

/// Why an op did not run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpError {
    /// This install does not hold the computer's sync lock.
    NotActive,
    /// The caller is not a member of the collection, or this machine does not hold it.
    NotMember,
    /// A request or an answer over a cap.
    TooBig,
    /// A request that is not what it says (a join for another machine, an entry that does not sign or extend).
    Bad,
    /// A member was added but no key could be sealed to it: nothing was done.
    NoKey,
    Db(String),
}

impl OpError {
    /// The code an op's refusal carries to the asker.
    pub fn code(&self) -> &'static str {
        match self {
            OpError::NotActive => "not_active",
            OpError::NotMember => "not_member",
            OpError::TooBig => "too_big",
            OpError::Bad => "bad_request",
            OpError::NoKey => "no_key",
            OpError::Db(_) => "failed",
        }
    }
}

impl From<rusqlite::Error> for OpError {
    fn from(e: rusqlite::Error) -> Self {
        OpError::Db(e.to_string())
    }
}

/// This machine as a member: its id and its signing key's seed (held only while it holds the lock).
pub struct Me<'a> {
    pub id: &'a str,
    pub private: &'a [u8; 32],
}

/// Whether a machine asking to join is proven enough for a join without a person: pinned here for at least a minimum
/// age, listed in the account's directory, and hosted by an install whose certificate chain is the one remote writes
/// already require. The transport supplies it.
pub trait Proof {
    fn auto_proven(&self, machine: &str) -> bool;
}

/// Seals the collection's keys to a new member's box key: one sealed key for each epoch the member needs.
pub trait KeyDelivery {
    fn keys_for(&self, collection: &str, to: &Subject) -> Vec<(u32, String)>;
    /// Makes and keeps the key of an epoch before a change that starts it is signed: false if it could not be made.
    fn mint(&self, _collection: &str, _epoch: u32) -> bool {
        true
    }
    /// Opens keys another member sealed to this machine and keeps them: how many were taken.
    fn take(&self, _collection: &str, _keys: &[(u32, String)]) -> usize {
        0
    }
    /// Forgets the key this machine made for an epoch whose removal lost a fork to another entry.
    fn discard(&self, _collection: &str, _epoch: u32) {}
    /// The epochs this machine holds a key for.
    fn held(&self, _collection: &str) -> Vec<u32> {
        Vec::new()
    }
}

/// A machine waiting for a person to confirm its join on this member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingJoin {
    pub collection: String,
    pub subject: Subject,
    pub asked_at: i64,
}

/// Everything an op may touch.
pub struct Ctx<'a> {
    pub conn: &'a Connection,
    pub chains: &'a mut BTreeMap<String, Chain>,
    pub clock: &'a mut Clock,
    pub pending: &'a mut Vec<PendingJoin>,
    pub me: Me<'a>,
    pub proof: &'a dyn Proof,
    pub keys: &'a dyn KeyDelivery,
    pub now_ms: i64,
    /// The longest chain this machine hands over (`MAX_ENTRIES` unless a caller sets less).
    pub chain_cap: usize,
    /// Whether this install holds the computer's sync lock right now.
    pub active: bool,
}

enum Standing {
    Current(State),
    /// Was a member and is not now: may learn how it was removed, and nothing else.
    Former,
    Stranger,
}

fn standing(ctx: &Ctx, collection: &str, caller: &str) -> Standing {
    let Some(chain) = ctx.chains.get(collection) else { return Standing::Stranger };
    let Ok(state) = chain.state(collection) else { return Standing::Stranger };
    if state.members.get(caller).is_some_and(|m| m.kind == Kind::Machine) {
        return Standing::Current(state);
    }
    let was = chain.entries.iter().any(|e| matches!(&e.change, Change::Found { member, .. } | Change::Add { member } if member.id == caller));
    if was {
        Standing::Former
    } else {
        Standing::Stranger
    }
}

fn gate(ctx: &Ctx) -> Result<(), OpError> {
    if ctx.active {
        Ok(())
    } else {
        Err(OpError::NotActive)
    }
}

/// The state of a collection whose current member the caller is: the check made before any op that reads or writes.
fn member(ctx: &Ctx, collection: &str, caller: &str) -> Result<State, OpError> {
    gate(ctx)?;
    match standing(ctx, collection, caller) {
        Standing::Current(state) => Ok(state),
        _ => Err(OpError::NotMember),
    }
}

fn weight(r: &Record) -> usize {
    r.body.len() + HEADER_WEIGHT
}

fn within_caps(records: &[Record]) -> bool {
    let bytes: usize = records.iter().map(weight).sum();
    records.len() <= MAX_BATCH_RECORDS
        && (records.len() <= 1 || bytes <= MAX_BATCH_BYTES)
        && records.iter().all(|r| r.body.len() as u64 <= MAX_RECORD_BYTES)
}

/// What a machine holds of one collection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Held {
    pub collection: String,
    pub n: u64,
    pub head: Option<String>,
    pub epoch: u32,
    pub vector: Vector,
}

/// `sync.hello`: for each named collection the caller is a member of, the chain position, the key epoch and what this
/// machine holds. A collection the caller is not a member of is left out, as if it did not exist.
pub fn hello(ctx: &Ctx, caller: &str, collections: &[String]) -> Result<Vec<Held>, OpError> {
    gate(ctx)?;
    if collections.len() > MAX_NAMES {
        return Err(OpError::TooBig);
    }
    let mut out = Vec::new();
    for c in collections {
        if let Standing::Current(state) = standing(ctx, c, caller) {
            out.push(Held { collection: c.clone(), n: state.n, head: state.head.clone(), epoch: state.epoch, vector: log::vector(ctx.conn, c)? });
        }
    }
    Ok(out)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullReq {
    pub collection: String,
    pub vector: Vector,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullAns {
    pub records: Vec<Record>,
    /// More remain: ask again with the vector after applying these.
    pub more: bool,
    pub epoch: u32,
    pub n: u64,
}

/// `sync.pull`: the records the caller lacks, capped in count and bytes.
pub fn pull(ctx: &Ctx, caller: &str, req: &PullReq) -> Result<PullAns, OpError> {
    let state = member(ctx, &req.collection, caller)?;
    if req.vector.len() > MAX_NAMES {
        return Err(OpError::TooBig);
    }
    let cap = req.limit.clamp(1, MAX_BATCH_RECORDS);
    let mut records = log::since(ctx.conn, &req.collection, &req.vector, cap + 1)?;
    let mut more = records.len() > cap;
    records.truncate(cap);
    more |= cap_bytes(&mut records);
    Ok(PullAns { records, more, epoch: state.epoch, n: state.n })
}

/// Cuts a batch to the byte cap (a first record always stays): whether anything was cut.
pub fn cap_bytes(records: &mut Vec<Record>) -> bool {
    let (mut bytes, mut keep) = (0, 0);
    for r in records.iter() {
        if keep > 0 && bytes + weight(r) > MAX_BATCH_BYTES {
            break;
        }
        bytes += weight(r);
        keep += 1;
    }
    let cut = keep < records.len();
    records.truncate(keep);
    cut
}

/// The asker's side of the cap: an answer over what was asked, or over the byte cap, is refused whole.
pub fn check_pull_answer(ans: &PullAns, asked: usize) -> Result<(), OpError> {
    if ans.records.len() > asked.clamp(1, MAX_BATCH_RECORDS) || !within_caps(&ans.records) {
        return Err(OpError::TooBig);
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushReq {
    pub collection: String,
    pub records: Vec<Record>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushAns {
    /// One result per record, in order.
    pub results: Vec<Result<Outcome, Refused>>,
    /// What this machine holds now, so the pusher sends what is still lacking (after a `Gap`, or a cut-off write
    /// re-issued at the new epoch).
    pub vector: Vector,
    pub epoch: u32,
}

/// `sync.push`: records from the caller (its own, or ones it forwards). A request over the caps is refused before
/// anything is applied; each record is checked and applied on its own.
pub fn push(ctx: &mut Ctx, caller: &str, req: &PushReq, adapter: &dyn Adapter) -> Result<PushAns, OpError> {
    let state = member(ctx, &req.collection, caller)?;
    if !within_caps(&req.records) {
        return Err(OpError::TooBig);
    }
    let pins = state.pins();
    let mut results = Vec::with_capacity(req.records.len());
    for rec in &req.records {
        if rec.header.collection != req.collection {
            results.push(Err(Refused::UnknownCollection));
            continue;
        }
        match log::apply_remote(ctx.conn, rec, &pins, ctx.clock, ctx.now_ms, adapter) {
            Ok(o) => results.push(Ok(o)),
            Err(SyncError::Refused(r)) => results.push(Err(r)),
            Err(SyncError::Db(e)) => return Err(e.into()),
            Err(SyncError::Seal(e)) => return Err(OpError::Db(e)),
        }
    }
    Ok(PushAns { results, vector: log::vector(ctx.conn, &req.collection)?, epoch: state.epoch })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MembersReq {
    pub collection: String,
    /// How many entries the caller holds.
    pub have_n: u64,
    /// Entries the caller offers (a member only).
    pub offered: Vec<Entry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MembersAns {
    pub entries: Vec<Entry>,
    pub n: u64,
    /// One result per offered entry, in order.
    pub taken: Vec<Result<Taken, MemberError>>,
}

/// Puts the log's key epoch, and the end each remaining member had at each epoch left, exactly where the chain says
/// (one removal moves it one epoch): in either direction, so a removal that lost a fork to another entry leaves
/// nothing behind. Called after every change to a chain.
pub fn sync_epoch(conn: &Connection, collection: &str, chain: &Chain) -> Result<(), OpError> {
    let removals: Vec<&BTreeMap<String, u64>> = chain.entries.iter().filter_map(|e| if let Change::Remove { ended, .. } = &e.change { Some(ended) } else { None }).collect();
    let cuts: Vec<(u32, &log::Vector)> = removals.iter().enumerate().map(|(i, ended)| (i as u32, *ended)).collect();
    log::reset_epochs(conn, collection, removals.len() as u32, &cuts)?;
    Ok(())
}

/// A removal that lost a fork to another entry leaves no key behind for its epoch, whichever machine signed it or made
/// the key: the winner's key is the one that counts, and is taken when it is offered.
pub(super) fn discard_lost_keys(keys: &dyn KeyDelivery, collection: &str, chain: &Chain, taken: &[Result<Taken, MemberError>]) {
    for t in taken {
        if let Ok(Taken::Replaced(old)) = t {
            if matches!(old.change, Change::Remove { .. }) {
                let before = chain.entries.iter().take(old.n.saturating_sub(1) as usize).filter(|e| matches!(e.change, Change::Remove { .. })).count();
                keys.discard(collection, before as u32 + 1);
            }
        }
    }
}

/// A former member is told how it was removed and nothing that came after.
fn visible_to_former(entries: &[Entry], caller: &str) -> usize {
    entries.iter().position(|e| matches!(&e.change, Change::Remove { id, .. } if id == caller)).map_or(0, |i| i + 1)
}

/// `sync.members`: takes the entries a member offers (each one verified against the chain, so an entry no member
/// signed is refused), and answers with the entries after the caller's position. A machine that was removed may ask
/// too, and learns up to its own removal.
pub fn members(ctx: &mut Ctx, caller: &str, req: &MembersReq) -> Result<MembersAns, OpError> {
    gate(ctx)?;
    if req.offered.len() > MAX_ENTRIES {
        return Err(OpError::TooBig);
    }
    let was_current = match standing(ctx, &req.collection, caller) {
        Standing::Current(_) => true,
        Standing::Former => false,
        Standing::Stranger => return Err(OpError::NotMember),
    };
    let mut taken = Vec::new();
    if was_current {
        let chain = ctx.chains.get_mut(&req.collection).ok_or(OpError::NotMember)?;
        taken = req.offered.iter().map(|e| chain.take(&req.collection, e.clone())).collect();
        discard_lost_keys(ctx.keys, &req.collection, chain, &taken);
        sync_epoch(ctx.conn, &req.collection, chain)?;
        chain_store::save(ctx.conn, &req.collection, chain)?;
    }
    // What the caller may read is decided after its offers are taken: one that removes the caller itself ends what it sees.
    let still_current = matches!(standing(ctx, &req.collection, caller), Standing::Current(_));
    let chain = &ctx.chains[&req.collection];
    let upto = if still_current { chain.entries.len() } else { visible_to_former(&chain.entries, caller) };
    let from = (req.have_n as usize).min(upto);
    let entries: Vec<Entry> = chain.entries[from..upto].iter().take(MAX_ENTRIES).cloned().collect();
    Ok(MembersAns { entries, n: chain.entries.len() as u64, taken })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeysReq {
    pub collection: String,
    /// The epochs the caller already holds a key for.
    pub have: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeysAns {
    /// The keys the caller lacks, each sealed to the caller's box key as the chain lists it, for its epoch only.
    pub keys: Vec<(u32, String)>,
}

/// `sync.keys`: the collection's keys a current member lacks (it learned of a removal, or lost one), sealed to it. A
/// machine that is not a current member is given none, and never a key of an epoch the chain has not reached.
pub fn keys(ctx: &Ctx, caller: &str, req: &KeysReq) -> Result<KeysAns, OpError> {
    let state = member(ctx, &req.collection, caller)?;
    if req.have.len() > MAX_NAMES {
        return Err(OpError::TooBig);
    }
    let to = state.members.get(caller).ok_or(OpError::NotMember)?;
    let keys = ctx.keys.keys_for(&req.collection, to).into_iter().filter(|(epoch, _)| *epoch <= state.epoch && !req.have.contains(epoch)).collect();
    Ok(KeysAns { keys })
}

/// Whether this machine may seal a new record for the collection. Not while a removal at the chain's head is
/// contested: the key epoch it would seal at may be replaced by another's.
pub fn may_seal(ctx: &Ctx, collection: &str) -> bool {
    ctx.chains.get(collection).is_some_and(|c| !c.epoch_contested())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinReq {
    pub collection: String,
    pub subject: Subject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Joined {
    /// The caller is a member: the whole chain, and the collection's keys sealed to it.
    Added { entries: Vec<Entry>, keys: Vec<(u32, String)> },
    /// A person must confirm on a member before it is added.
    Pending,
}

pub fn kind_of(conn: &Connection, collection: &str) -> Result<String, OpError> {
    Ok(conn.query_row("SELECT kind FROM db_sync_collections WHERE id = ?1", [collection], |r| r.get(0))?)
}

/// A membership change signed by this machine and taken into its own chain (the key epoch moves too, for a removal):
/// the entry to hand to the other members. Nothing is signed unless this install holds the lock.
pub fn change(ctx: &mut Ctx, collection: &str, change: Change) -> Result<Entry, OpError> {
    gate(ctx)?;
    // A removal starts the next key epoch: its key is made first, fresh (one left by an earlier attempt is never
    // reused), so no member is removed that cannot be given one. If the removal is not then signed and taken, the key
    // is forgotten again, so no key exists for an epoch that has not started.
    let starts = if matches!(change, Change::Remove { .. }) { Some(chain_epoch(ctx, collection)? + 1) } else { None };
    if let Some(next) = starts {
        ctx.keys.discard(collection, next);
        if !ctx.keys.mint(collection, next) {
            return Err(OpError::NoKey);
        }
    }
    let done = sign_and_take(ctx, collection, change);
    if let (Err(_), Some(next)) = (&done, starts) {
        ctx.keys.discard(collection, next);
    }
    done
}

fn chain_epoch(ctx: &Ctx, collection: &str) -> Result<u32, OpError> {
    let chain = ctx.chains.get(collection).ok_or(OpError::NotMember)?;
    Ok(chain.state(collection).map_err(|_| OpError::Bad)?.epoch)
}

fn sign_and_take(ctx: &mut Ctx, collection: &str, change: Change) -> Result<Entry, OpError> {
    let chain = ctx.chains.get_mut(collection).ok_or(OpError::NotMember)?;
    let entry = chain.next(ctx.me.private, collection, ctx.me.id, change).ok_or(OpError::Bad)?;
    chain.take(collection, entry.clone()).map_err(|_| OpError::Bad)?;
    sync_epoch(ctx.conn, collection, chain)?;
    chain_store::save(ctx.conn, collection, chain)?;
    Ok(entry)
}

fn added(ctx: &Ctx, collection: &str, keys: Vec<(u32, String)>) -> Result<Joined, OpError> {
    let chain = &ctx.chains[collection];
    if chain.entries.len() > ctx.chain_cap {
        return Err(OpError::TooBig);
    }
    Ok(Joined::Added { entries: chain.entries.clone(), keys })
}

/// The keys to hand a machine that is about to be a member: none means nothing is done.
fn keys_or_refuse(ctx: &Ctx, collection: &str, subject: &Subject) -> Result<Vec<(u32, String)>, OpError> {
    // Only the epochs the chain has reached: a key for one that has not started is never handed out.
    let reached = chain_epoch(ctx, collection)?;
    let keys: Vec<(u32, String)> = ctx.keys.keys_for(collection, subject).into_iter().filter(|(epoch, _)| *epoch <= reached).collect();
    if keys.is_empty() {
        return Err(OpError::NoKey);
    }
    Ok(keys)
}

/// `sync.join`: a machine asks to be added. The caller must be the machine the subject names. If it is already a
/// member it is answered again (a retry after a person confirmed). Otherwise it joins at once only when the
/// collection's policy is `auto_proven`, the collection is of a kind that allows it and the caller is proven; any
/// other join waits for a person to confirm on a member ([`approve`]).
pub fn join(ctx: &mut Ctx, caller: &str, req: &JoinReq) -> Result<Joined, OpError> {
    gate(ctx)?;
    let state = ctx.chains.get(&req.collection).and_then(|c| c.state(&req.collection).ok()).ok_or(OpError::NotMember)?;
    if !state.members.contains_key(ctx.me.id) {
        return Err(OpError::NotMember);
    }
    if req.subject.kind != Kind::Machine || req.subject.id != caller {
        return Err(OpError::Bad);
    }
    if !req.subject.valid() {
        return Err(OpError::Bad);
    }
    let kind = kind_of(ctx.conn, &req.collection)?;
    let already = state.members.get(caller) == Some(&req.subject);
    let at_once = state.policy == Policy::AutoProven && AUTO_JOIN_KINDS.contains(&kind.as_str()) && ctx.proof.auto_proven(caller);
    if !already && !at_once {
        return wait_for_person(ctx, &req.collection, &req.subject);
    }
    let keys = keys_or_refuse(ctx, &req.collection, &req.subject)?;
    if !already {
        room_for_one_more(ctx, &req.collection)?;
        change(ctx, &req.collection, Change::Add { member: req.subject.clone() })?;
        ctx.pending.retain(|p| !(p.collection == req.collection && p.subject.id == caller));
    }
    added(ctx, &req.collection, keys)
}

/// Refuses before anything is signed when one more entry would make the chain longer than an answer may carry (a
/// member added whose chain could then not be handed over is never left behind).
fn room_for_one_more(ctx: &Ctx, collection: &str) -> Result<(), OpError> {
    let len = ctx.chains.get(collection).map_or(0, |c| c.entries.len());
    if len + 1 > ctx.chain_cap {
        return Err(OpError::TooBig);
    }
    Ok(())
}

fn wait_for_person(ctx: &mut Ctx, collection: &str, subject: &Subject) -> Result<Joined, OpError> {
    let now = ctx.now_ms;
    ctx.pending.retain(|p| now - p.asked_at < PENDING_TTL_MS);
    let waiting = ctx.pending.iter().filter(|p| p.collection == collection).count();
    let already = ctx.pending.iter().any(|p| p.collection == collection && p.subject.id == subject.id);
    if !already {
        if waiting >= MAX_PENDING {
            return Err(OpError::TooBig);
        }
        ctx.pending.push(PendingJoin { collection: collection.to_string(), subject: subject.clone(), asked_at: ctx.now_ms });
    }
    Ok(Joined::Pending)
}

/// A person confirmed, on this member, that a waiting machine may join: the signed `Add` (the member then answers the
/// machine's next `join` with the chain and keys).
pub fn approve(ctx: &mut Ctx, collection: &str, machine: &str) -> Result<Entry, OpError> {
    gate(ctx)?;
    let at = ctx.pending.iter().position(|p| p.collection == collection && p.subject.id == machine).ok_or(OpError::Bad)?;
    let subject = ctx.pending[at].subject.clone();
    keys_or_refuse(ctx, collection, &subject)?;
    ctx.pending.remove(at);
    room_for_one_more(ctx, collection)?;
    change(ctx, collection, Change::Add { member: subject })
}

/// Why a joiner did not take the chain it was given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JoinNo {
    /// A chain that does not verify entry by entry.
    Unsound(MemberError),
    /// It does not list this machine.
    NotInIt,
    /// No member of it is one this machine itself trusts (its own LAN pin and account check).
    NoAnchor,
}

/// The joiner's side: takes a chain only if every entry verifies, it lists this machine and at least one of its
/// machine members is one this machine itself trusts. The peer that sent it is not taken on its word.
pub fn take_join(collection: &str, me: &str, entries: Vec<Entry>, trusted: &dyn Fn(&str) -> bool) -> Result<Chain, JoinNo> {
    let chain = Chain::from_entries(collection, entries).map_err(JoinNo::Unsound)?;
    let state = chain.state(collection).map_err(JoinNo::Unsound)?;
    if !state.members.contains_key(me) {
        return Err(JoinNo::NotInIt);
    }
    if !state.anchored(trusted) {
        return Err(JoinNo::NoAnchor);
    }
    Ok(chain)
}

/// Founds a collection here: its first entry, signed by this machine, and the collection in the log.
pub fn found(conn: &Connection, chains: &mut BTreeMap<String, Chain>, keys: &dyn KeyDelivery, collection: &str, kind: &str, me: &Subject, private: &[u8; 32], policy: Policy, now_ms: i64) -> Result<(), OpError> {
    if !keys.mint(collection, 0) {
        return Err(OpError::NoKey);
    }
    log::ensure_collection(conn, collection, kind, now_ms)?;
    let first = Entry::sign(private, collection, 1, None, Change::Found { member: me.clone(), policy }, &me.id).ok_or(OpError::Bad)?;
    let chain = Chain::from_entries(collection, vec![first]).map_err(|_| OpError::Bad)?;
    chain_store::save(conn, collection, &chain)?;
    chains.insert(collection.to_string(), chain);
    Ok(())
}

/// A joiner keeps the chain it took ([`take_join`]): the collection in the log, the chain in the store, and the log's
/// key epoch where the chain says it is.
pub fn adopt(conn: &Connection, chains: &mut BTreeMap<String, Chain>, collection: &str, kind: &str, chain: Chain, now_ms: i64) -> Result<(), OpError> {
    log::ensure_collection(conn, collection, kind, now_ms)?;
    sync_epoch(conn, collection, &chain)?;
    chain_store::save(conn, collection, &chain)?;
    chains.insert(collection.to_string(), chain);
    Ok(())
}

/// Writes again, at the collection's current key epoch, what this machine wrote at an older epoch beyond the end its
/// removal-of-another entry named (those writes are refused by the others): each record whose current state is one of
/// them is opened with the key of its epoch and sealed with the current one. Returns how many were re-issued.
pub fn reissue(conn: &Connection, signer: &mut Signer, ring: &KeyRing, collection: &str, now_ms: i64) -> Result<usize, SyncError> {
    let cut = log::cut_off(conn, collection, signer.machine)?;
    for rec in &cut {
        if rec.header.tombstone {
            log::append_local(conn, signer, collection, &rec.header.record, Vec::new(), true, now_ms)?;
        } else {
            let plain = seal::open(ring, &rec.header, &rec.body).map_err(|e| SyncError::Seal(format!("{e:?}")))?;
            log::append_local_with(conn, signer, collection, &rec.header.record, false, now_ms, seal::sealer(ring, &plain))?;
        }
    }
    Ok(cut.len())
}
