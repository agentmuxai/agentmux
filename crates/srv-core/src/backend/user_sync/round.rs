// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! One round of sync with one peer, for one collection (SPEC_USER_DATA_SYNC_LAYER §4.2, the loop's body): the asking
//! side of the `sync.*` ops. A round learns the membership changes each side lacks (`sync.members`), takes the keys of
//! any epoch this machine lacks (`sync.keys`), pulls the records it lacks, and pushes the ones the peer lacks. It is
//! pure over a [`PeerCalls`] (the calls to the peer) and a [`Session`] (this machine's own state): a transport hands
//! it both and it never opens a connection, so it is tested against in-memory machines. A [`Session`] lends the
//! round this machine's [`Ctx`] only for the length of one closure, so a transport that keeps the store locked while
//! a closure runs never keeps it locked across a call to the peer (which is itself asking this machine for the same
//! store). Which peers, when, and what wakes the loop belong to the transport.
//!
//! A round never gives up the others' records because one is refused: each record is checked and applied on its own,
//! the refusals are counted by kind, and a refusal that is the peer's news (an epoch this machine has not reached, a
//! sequence skipped) ends the pull so the next round takes it again from the membership step.

use super::adapter::Adapter;
use super::chain_store;
use super::log::{self, Outcome, Refused, SyncError};
use super::members::{Change, Entry, Kind, MemberError, Taken};
use super::ops::{self, Ctx, Held, KeysAns, KeysReq, MembersAns, MembersReq, OpError, PullAns, PullReq, PushAns, PushReq};

/// The most batches one round pulls or pushes: a peer with more waits for the next round.
pub const MAX_BATCHES: usize = 64;
/// The most times one round asks for membership entries (a longer chain than one answer carries).
pub const MAX_MEMBER_ASKS: usize = 8;

/// Why a call to the peer did not give an answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallError {
    /// The peer could not be reached or did not answer.
    Unreachable(String),
    /// The peer answered with a refusal; the code is the op's (`OpError::code`) or the transport's.
    Refused(String),
}

/// This machine's side: its sync state, lent to one closure at a time.
pub trait Session {
    fn with<R>(&mut self, f: impl FnOnce(&mut Ctx) -> R) -> R;
}

/// The calls to one peer. Each is one remote op; the transport signs and carries it.
pub trait PeerCalls {
    fn hello(&mut self, collections: &[String]) -> Result<Vec<Held>, CallError>;
    fn members(&mut self, req: &MembersReq) -> Result<MembersAns, CallError>;
    fn keys(&mut self, req: &KeysReq) -> Result<KeysAns, CallError>;
    fn pull(&mut self, req: &PullReq) -> Result<PullAns, CallError>;
    fn push(&mut self, req: &PushReq) -> Result<PushAns, CallError>;
}

/// How a round ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ended {
    /// Both sides now hold the same, as far as this round could tell.
    Synced,
    /// This install does not hold the computer's sync lock: nothing was asked.
    NotActive,
    /// The peer's install does not hold its lock right now: try again later.
    PeerIdle,
    /// The peer could not be reached.
    Unreachable(String),
    /// This machine does not hold the collection, or is not in it.
    NotHere,
    /// The peer does not list this machine as a member (it was removed there, or never added): nothing was exchanged
    /// beyond what a former member may learn.
    NotAMemberThere,
    /// This machine learned it was removed from the collection, and by whom. Nothing after that was asked.
    Removed { by: String },
    /// The peer sent something this machine will not take (an answer over a cap, an unsound chain).
    Distrusted(String),
    /// A local store error: the round stopped.
    Failed(String),
}

/// What a round did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Round {
    pub ended: Ended,
    /// Membership entries taken from the peer, and how many of ours the peer took.
    pub entries_taken: usize,
    pub entries_given: usize,
    pub keys_taken: usize,
    /// Records pulled and applied (not counting ones already held), kept as the older side, or in conflict.
    pub pulled: usize,
    pub kept_local: usize,
    pub conflicts: usize,
    /// Records the peer took from this machine.
    pub pushed: usize,
    /// Records refused on either side, by kind (the refusal's name).
    pub refused: Vec<String>,
}

impl Round {
    fn new() -> Round {
        Round { ended: Ended::Synced, entries_taken: 0, entries_given: 0, keys_taken: 0, pulled: 0, kept_local: 0, conflicts: 0, pushed: 0, refused: Vec::new() }
    }

    fn end(mut self, ended: Ended) -> Round {
        self.ended = ended;
        self
    }

    fn refuse(&mut self, why: &Refused) {
        let name = match why {
            Refused::UnknownCollection => "unknown_collection",
            Refused::UnknownOrigin => "unknown_origin",
            Refused::Bad(_) => "bad_record",
            Refused::Epoch { .. } => "epoch",
            Refused::Skew(_) => "skew",
            Refused::Gap { .. } => "gap",
            Refused::Rollback => "rollback",
        };
        if !self.refused.iter().any(|r| r == name) {
            self.refused.push(name.to_string());
        }
    }

    /// Whether anything changed on either side.
    pub fn changed(&self) -> bool {
        self.entries_taken + self.entries_given + self.keys_taken + self.pulled + self.kept_local + self.conflicts + self.pushed > 0
    }
}

fn stop(e: CallError) -> Ended {
    match e {
        CallError::Unreachable(why) => Ended::Unreachable(why),
        CallError::Refused(code) if code == OpError::NotActive.code() => Ended::PeerIdle,
        CallError::Refused(code) if code == OpError::NotMember.code() => Ended::NotAMemberThere,
        CallError::Refused(code) => Ended::Distrusted(format!("the peer refused: {code}")),
    }
}

fn db(e: impl std::fmt::Display) -> Ended {
    Ended::Failed(e.to_string())
}

/// The machine that signed this machine's removal, when the chain removes it.
fn removed_by(entries: &[Entry], me: &str) -> Option<String> {
    entries.iter().find(|e| matches!(&e.change, Change::Remove { id, .. } if id == me)).map(|e| e.signer.clone())
}

fn is_member(ctx: &Ctx, collection: &str) -> bool {
    ctx.chains.get(collection).and_then(|c| c.state(collection).ok()).is_some_and(|s| s.members.get(ctx.me.id).is_some_and(|m| m.kind == Kind::Machine))
}

fn taken_count(taken: &[Result<Taken, MemberError>]) -> usize {
    taken.iter().filter(|t| matches!(t, Ok(Taken::Extended | Taken::Replaced(_)))).count()
}

/// What to say to the peer next, read from this machine's state.
struct Ask {
    have: u64,
    offered: Vec<Entry>,
}

/// Learns the membership entries each side lacks. A removal of this machine ends the round.
fn exchange_members<S: Session, P: PeerCalls>(s: &mut S, peer: &mut P, collection: &str, mut peer_n: u64, out: &mut Round) -> Result<(), Ended> {
    for _ in 0..MAX_MEMBER_ASKS {
        let ask = s.with(|ctx| {
            let chain = ctx.chains.get(collection).ok_or(Ended::NotHere)?;
            let offered = chain.entries.iter().skip(peer_n as usize).take(ops::MAX_ENTRIES).cloned().collect();
            Ok::<_, Ended>(Ask { have: chain.entries.len() as u64, offered })
        })?;
        let ans = peer.members(&MembersReq { collection: collection.to_string(), have_n: ask.have, offered: ask.offered }).map_err(stop)?;
        if ans.entries.len() > ops::MAX_ENTRIES {
            return Err(Ended::Distrusted("a membership answer over the cap".into()));
        }
        out.entries_given += taken_count(&ans.taken);
        let (took, now, removed) = s.with(|ctx| {
            let chain = ctx.chains.get_mut(collection).ok_or(Ended::NotHere)?;
            let mut taken: Vec<Result<Taken, MemberError>> = Vec::new();
            for entry in ans.entries.iter().cloned() {
                taken.push(Ok(chain.take(collection, entry).map_err(|e| Ended::Distrusted(format!("a membership entry that does not verify: {e:?}")))?));
            }
            ops::discard_lost_keys(ctx.keys, collection, chain, &taken);
            ops::sync_epoch(ctx.conn, collection, chain).map_err(|e| Ended::Failed(format!("{e:?}")))?;
            chain_store::save(ctx.conn, collection, chain).map_err(db)?;
            let removed = if is_member(ctx, collection) { None } else { Some(removed_by(&ctx.chains[collection].entries, ctx.me.id).unwrap_or_default()) };
            Ok((taken_count(&taken), ctx.chains[collection].entries.len() as u64, removed))
        })?;
        out.entries_taken += took;
        if let Some(by) = removed {
            return Err(Ended::Removed { by });
        }
        // Another ask only when the peer has more of its chain than it sent and this machine advanced.
        if ans.entries.is_empty() || now >= ans.n {
            break;
        }
        peer_n = peer_n.max(now.min(ans.n));
    }
    Ok(())
}

/// Takes the keys of epochs this machine lacks.
fn take_keys<S: Session, P: PeerCalls>(s: &mut S, peer: &mut P, collection: &str, out: &mut Round) -> Result<(), Ended> {
    let have = s.with(|ctx| {
        let epoch = ctx.chains.get(collection).and_then(|c| c.state(collection).ok()).map_or(0, |st| st.epoch);
        let have = ctx.keys.held(collection);
        if (0..=epoch).all(|e| have.contains(&e)) {
            None
        } else {
            Some(have)
        }
    });
    let Some(have) = have else { return Ok(()) };
    let ans = peer.keys(&KeysReq { collection: collection.to_string(), have }).map_err(stop)?;
    if ans.keys.len() > ops::MAX_NAMES {
        return Err(Ended::Distrusted("a key answer over the cap".into()));
    }
    out.keys_taken += s.with(|ctx| ctx.keys.take(collection, &ans.keys));
    Ok(())
}

/// Pulls and applies, batch by batch, until the peer has no more or a batch changes nothing.
fn pull_all<S: Session, P: PeerCalls>(s: &mut S, peer: &mut P, collection: &str, adapter: &dyn Adapter, out: &mut Round) -> Result<(), Ended> {
    for _ in 0..MAX_BATCHES {
        let before = s.with(|ctx| log::vector(ctx.conn, collection)).map_err(db)?;
        let req = PullReq { collection: collection.to_string(), vector: before.clone(), limit: ops::MAX_BATCH_RECORDS };
        let ans = peer.pull(&req).map_err(stop)?;
        ops::check_pull_answer(&ans, req.limit).map_err(|_| Ended::Distrusted("a pull answer over the cap".into()))?;
        let (after, stalled) = s.with(|ctx| {
            let state = ctx.chains.get(collection).and_then(|c| c.state(collection).ok()).ok_or(Ended::NotHere)?;
            let pins = state.pins();
            let mut stalled = false;
            for rec in &ans.records {
                match log::apply_remote(ctx.conn, rec, &pins, ctx.clock, ctx.now_ms, adapter) {
                    Ok(Outcome::Applied) => out.pulled += 1,
                    Ok(Outcome::KeptLocal) => out.kept_local += 1,
                    Ok(Outcome::Conflict) => out.conflicts += 1,
                    Ok(Outcome::Duplicate | Outcome::Void) => {}
                    Err(SyncError::Refused(why)) => {
                        out.refuse(&why);
                        // A skipped sequence or an epoch not reached yet: what follows from that origin would be
                        // refused the same way, so the round leaves it for the next one.
                        stalled |= matches!(why, Refused::Gap { .. } | Refused::Epoch { .. });
                    }
                    Err(SyncError::Db(e)) => return Err(db(e)),
                    Err(SyncError::Seal(e)) => return Err(Ended::Failed(e)),
                }
            }
            Ok((log::vector(ctx.conn, collection).map_err(db)?, stalled))
        })?;
        if !ans.more || after == before || stalled {
            break;
        }
    }
    Ok(())
}

/// Pushes what the peer lacks, batch by batch, until it holds all of it or a batch moves nothing.
fn push_all<S: Session, P: PeerCalls>(s: &mut S, peer: &mut P, collection: &str, mut theirs: log::Vector, out: &mut Round) -> Result<(), Ended> {
    for _ in 0..MAX_BATCHES {
        let records = s.with(|ctx| {
            let mut records = log::since(ctx.conn, collection, &theirs, ops::MAX_BATCH_RECORDS).map_err(db)?;
            ops::cap_bytes(&mut records);
            Ok::<_, Ended>(records)
        })?;
        if records.is_empty() {
            break;
        }
        let ans = peer.push(&PushReq { collection: collection.to_string(), records: records.clone() }).map_err(stop)?;
        if ans.results.len() != records.len() {
            return Err(Ended::Distrusted("a push answer that does not match the request".into()));
        }
        for result in &ans.results {
            match result {
                Ok(Outcome::Applied | Outcome::KeptLocal | Outcome::Conflict) => out.pushed += 1,
                Ok(Outcome::Duplicate | Outcome::Void) => {}
                Err(why) => out.refuse(why),
            }
        }
        let moved = ans.vector != theirs;
        theirs = ans.vector;
        if !moved {
            break;
        }
    }
    Ok(())
}

/// One round with one peer for one collection.
pub fn sync_collection<S: Session, P: PeerCalls>(s: &mut S, peer: &mut P, collection: &str, adapter: &dyn Adapter) -> Round {
    let mut out = Round::new();
    let start = s.with(|ctx| {
        if !ctx.active {
            Err(Ended::NotActive)
        } else if !is_member(ctx, collection) {
            Err(Ended::NotHere)
        } else {
            Ok(ctx.chains[collection].entries.len() as u64)
        }
    });
    let mine = match start {
        Ok(n) => n,
        Err(ended) => return out.end(ended),
    };
    let held = match peer.hello(&[collection.to_string()]) {
        Ok(held) => held,
        Err(e) => return out.end(stop(e)),
    };
    let Some(held) = held.into_iter().find(|h| h.collection == collection) else {
        // The peer leaves this machine out of what it answers, which is also how it answers a machine it removed. A
        // former member may still ask for the membership entries up to its own removal, so ask: that is how a removed
        // machine learns it was, and by whom.
        return match exchange_members(s, peer, collection, mine, &mut out) {
            Err(Ended::Removed { by }) => out.end(Ended::Removed { by }),
            _ => out.end(Ended::NotAMemberThere),
        };
    };
    let steps = (|| {
        exchange_members(s, peer, collection, held.n, &mut out)?;
        take_keys(s, peer, collection, &mut out)?;
        pull_all(s, peer, collection, adapter, &mut out)?;
        push_all(s, peer, collection, held.vector.clone(), &mut out)
    })();
    match steps {
        Ok(()) => out,
        Err(ended) => out.end(ended),
    }
}
