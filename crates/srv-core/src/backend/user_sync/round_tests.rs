// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A round of sync with one peer against in-memory fake machines: two machines that wrote apart end up holding the
// same in one round, a long backlog comes over in batches, a machine without the lock asks and answers nothing, a
// stranger gets nothing, a removed machine learns who removed it and stops, a lying answer is not taken, and an
// unreachable peer ends the round without harm.

use super::adapter::NewestWins;
use super::log::{self, Refused};
use super::chain_store;
use super::members::{Change, Entry, Policy};
use super::ops::{self, Held, KeysAns, KeysReq, MembersAns, MembersReq, OpError, PullAns, PullReq, PushAns, PushReq};
use super::ops_tests::{ended, node, three, Node, C};
use super::ops::Ctx;
use super::record::Record;
use super::round::{sync_collection, CallError, Ended, PeerCalls, Round, Session};

/// The calls of `asker` to `host`, answered by the real ops of `host`.
struct Link<'a> {
    asker: String,
    host: &'a mut Node,
    down: bool,
    /// Answers a pull with more records than were asked for.
    pad_pull: bool,
    keys_asked: usize,
}

fn link<'a>(asker: &Node, host: &'a mut Node) -> Link<'a> {
    Link { asker: asker.id.clone(), host, down: false, pad_pull: false, keys_asked: 0 }
}

fn refused(e: OpError) -> CallError {
    CallError::Refused(e.code().to_string())
}

impl PeerCalls for Link<'_> {
    fn hello(&mut self, collections: &[String]) -> Result<Vec<Held>, CallError> {
        if self.down {
            return Err(CallError::Unreachable("no route".into()));
        }
        ops::hello(&self.host.ctx(), &self.asker, collections).map_err(refused)
    }
    fn members(&mut self, req: &MembersReq) -> Result<MembersAns, CallError> {
        ops::members(&mut self.host.ctx(), &self.asker, req).map_err(refused)
    }
    fn keys(&mut self, req: &KeysReq) -> Result<KeysAns, CallError> {
        self.keys_asked += 1;
        ops::keys(&self.host.ctx(), &self.asker, req).map_err(refused)
    }
    fn pull(&mut self, req: &PullReq) -> Result<PullAns, CallError> {
        let mut ans = ops::pull(&self.host.ctx(), &self.asker, req).map_err(refused)?;
        if self.pad_pull {
            let first = ans.records[0].clone();
            ans.records = vec![first; ops::MAX_BATCH_RECORDS + 1];
        }
        Ok(ans)
    }
    fn push(&mut self, req: &PushReq) -> Result<PushAns, CallError> {
        ops::push(&mut self.host.ctx(), &self.asker, req, &NewestWins).map_err(refused)
    }
}

/// A node as the asking side of a round: its state, lent for one closure.
struct Mine<'a>(&'a mut Node);

impl Session for Mine<'_> {
    fn with<R>(&mut self, f: impl FnOnce(&mut Ctx) -> R) -> Result<R, String> {
        Ok(f(&mut self.0.ctx()))
    }
}

fn round(asker: &mut Node, host: &mut Node) -> Round {
    let mut l = link(asker, host);
    sync_collection(&mut Mine(asker), &mut l, C, &NewestWins)
}

#[test]
fn machines_that_wrote_apart_hold_the_same_after_one_round() {
    let (mut a, mut b, _) = three("kanban");
    a.write(C, "task.1", "from a");
    b.write(C, "task.2", "from b");
    let r = round(&mut b, &mut a);
    assert_eq!(r.ended, Ended::Synced);
    assert_eq!((r.pulled, r.pushed), (1, 1));
    assert_eq!(a.vector(C), b.vector(C));
    assert_eq!(log::current(&a.conn, C, "task.2").unwrap().unwrap().header.origin, b.id);
    assert_eq!(log::current(&b.conn, C, "task.1").unwrap().unwrap().header.origin, a.id);
    let again = round(&mut b, &mut a);
    assert!(!again.changed(), "a second round with nothing new changes nothing: {again:?}");
}

#[test]
fn a_backlog_longer_than_one_batch_comes_over_in_one_round() {
    let (mut a, mut b, _) = three("kanban");
    for i in 0..(ops::MAX_BATCH_RECORDS * 2 + 50) {
        a.write(C, &format!("task.{i}"), "x");
    }
    let r = round(&mut b, &mut a);
    assert_eq!(r.ended, Ended::Synced);
    assert_eq!(r.pulled, ops::MAX_BATCH_RECORDS * 2 + 50);
    assert_eq!(a.vector(C), b.vector(C));
}

#[test]
fn an_install_without_the_lock_neither_asks_nor_answers() {
    let (mut a, mut b, _) = three("kanban");
    a.write(C, "task.1", "x");
    b.active = false;
    assert_eq!(round(&mut b, &mut a).ended, Ended::NotActive);
    b.active = true;
    a.active = false;
    assert_eq!(round(&mut b, &mut a).ended, Ended::PeerIdle);
    assert!(b.vector(C).is_empty(), "nothing was taken from a peer that does not hold its lock");
}

#[test]
fn a_peer_that_does_not_list_this_machine_is_reported_and_nothing_is_taken() {
    let (mut a, b, _) = three("kanban");
    let mut stranger = node(9);
    a.write(C, "task.1", "x");
    // The stranger holds the collection by name (it was never added), and the host answers it as for one that does not exist.
    stranger.found(C, "kanban", super::members::Policy::Approve);
    let r = round(&mut stranger, &mut a);
    assert_eq!(r.ended, Ended::NotAMemberThere);
    assert!(!r.changed());
    assert!(b.vector(C).is_empty());
}

#[test]
fn a_machine_removed_while_away_learns_who_removed_it_and_gets_nothing_after_it() {
    let (mut a, b, mut c) = three("kanban");
    let gone = ended(&[(&a, 0), (&b, 0)]);
    ops::change(&mut a.ctx(), C, Change::Remove { id: c.id.clone(), ended: gone }).unwrap();
    a.write(C, "task.after", "never for c");
    c.write(C, "task.mine", "kept here");
    let r = round(&mut c, &mut a);
    assert_eq!(r.ended, Ended::Removed { by: a.id.clone() });
    assert_eq!(r.entries_taken, 1);
    assert_eq!((r.pulled, r.pushed), (0, 0), "nothing after the removal is read or written");
    assert!(log::current(&c.conn, C, "task.mine").unwrap().is_some(), "its own records stay readable here");
    assert!(log::current(&a.conn, C, "task.mine").unwrap().is_none());
    assert!(log::current(&c.conn, C, "task.after").unwrap().is_none());
    let again = round(&mut c, &mut a);
    assert_eq!(again.ended, Ended::NotHere, "once removed, the machine no longer syncs the collection");
}

#[test]
fn a_removed_machine_hears_it_from_a_member_that_is_not_the_remover() {
    let (mut a, mut b, mut c) = three("kanban");
    let gone = ended(&[(&a, 0), (&b, 0)]);
    let remove = ops::change(&mut a.ctx(), C, Change::Remove { id: c.id.clone(), ended: gone }).unwrap();
    let ans = ops::members(&mut b.ctx(), &a.id, &MembersReq { collection: C.into(), have_n: 3, offered: vec![remove] }).unwrap();
    assert_eq!(ans.taken.len(), 1);
    let r = round(&mut c, &mut b);
    assert_eq!(r.ended, Ended::Removed { by: a.id.clone() });
}

#[test]
fn an_answer_over_the_cap_is_not_taken() {
    let (mut a, mut b, _) = three("kanban");
    a.write(C, "task.1", "x");
    let mut l = link(&b, &mut a);
    l.pad_pull = true;
    let r = sync_collection(&mut Mine(&mut b), &mut l, C, &NewestWins);
    assert!(matches!(r.ended, Ended::Distrusted(_)), "{:?}", r.ended);
    assert!(b.vector(C).is_empty(), "nothing from a padded answer was applied");
}

#[test]
fn an_unreachable_peer_ends_the_round_and_changes_nothing() {
    let (mut a, mut b, _) = three("kanban");
    b.write(C, "task.1", "x");
    let mut l = link(&b, &mut a);
    l.down = true;
    let r = sync_collection(&mut Mine(&mut b), &mut l, C, &NewestWins);
    assert_eq!(r.ended, Ended::Unreachable("no route".into()));
    assert!(!r.changed());
    assert!(a.vector(C).is_empty());
}

#[test]
fn a_machine_that_lacks_a_key_asks_for_it_once() {
    let (mut a, mut b, _) = three("kanban");
    // The fake key store holds nothing, so the round asks once.
    let mut l = link(&b, &mut a);
    let r = sync_collection(&mut Mine(&mut b), &mut l, C, &NewestWins);
    assert_eq!(r.ended, Ended::Synced);
    assert_eq!(l.keys_asked, 1);
}

#[test]
fn a_record_for_another_collection_is_refused_alone_and_the_one_beside_it_is_taken() {
    let (mut a, mut b, _) = three("kanban");
    a.write(C, "task.1", "x");
    let mut o = node(9);
    o.found("board.other", "kanban", super::members::Policy::Approve);
    let rec = o.write("board.other", "task.9", "y");
    // A record for another collection in a push to a: refused on its own, and the valid one beside it is taken.
    let valid = b.write(C, "task.2", "z");
    let ans = ops::push(&mut a.ctx(), &b.id, &PushReq { collection: C.into(), records: vec![rec, valid] }, &NewestWins).unwrap();
    assert_eq!(ans.results[0], Err(Refused::UnknownCollection));
    assert!(ans.results[1].is_ok());
}

/// A peer whose answer to a membership ask is the given entries, as when its chain moved between this machine's ask and
/// the answer; every other call goes to the real ops.
struct Answering<'a> {
    inner: Link<'a>,
    entries: Vec<Entry>,
    /// What it answers a pull with, in place of the real answer.
    pulled: Option<Vec<Record>>,
}

impl PeerCalls for Answering<'_> {
    fn hello(&mut self, collections: &[String]) -> Result<Vec<Held>, CallError> {
        self.inner.hello(collections)
    }
    fn members(&mut self, _: &MembersReq) -> Result<MembersAns, CallError> {
        Ok(MembersAns { entries: self.entries.clone(), n: 4, taken: vec![] })
    }
    fn keys(&mut self, req: &KeysReq) -> Result<KeysAns, CallError> {
        self.inner.keys(req)
    }
    fn pull(&mut self, req: &PullReq) -> Result<PullAns, CallError> {
        match &self.pulled {
            Some(records) => Ok(PullAns { records: records.clone(), more: false, epoch: 0, n: 3 }),
            None => self.inner.pull(req),
        }
    }
    fn push(&mut self, req: &PushReq) -> Result<PushAns, CallError> {
        self.inner.push(req)
    }
}

#[test]
fn an_entry_that_loses_a_fork_is_a_result_of_the_round_not_a_reason_to_distrust_the_peer() {
    let (mut a, mut b, c) = three("kanban");
    ops::change(&mut a.ctx(), C, Change::SetPolicy { policy: Policy::Approve }).unwrap();
    let gone = ended(&[(&a, 0), (&b, 0)]);
    let remove = ops::change(&mut b.ctx(), C, Change::Remove { id: c.id.clone(), ended: gone }).unwrap();
    // The peer's answer holds an entry beside this machine's head that loses to it.
    let mut peer = Answering { inner: link(&a, &mut b), entries: vec![remove.clone()], pulled: None };
    let r = sync_collection(&mut Mine(&mut a), &mut peer, C, &NewestWins);
    assert_eq!(r.ended, Ended::Synced, "{:?}", r.ended);
    assert_eq!(a.chains[C].forks(), &[remove], "the fork is kept, unsettled");
    assert_eq!(chain_store::load_all(&a.conn).unwrap(), a.chains, "and stored");
}

#[test]
fn entries_taken_before_one_that_does_not_verify_are_stored() {
    let (mut a, mut b, _) = three("kanban");
    let good = ops::change(&mut b.ctx(), C, Change::SetPolicy { policy: Policy::Approve }).unwrap();
    let mut bad = good.clone();
    bad.n = 5;
    let mut peer = Answering { inner: link(&a, &mut b), entries: vec![good, bad], pulled: None };
    let r = sync_collection(&mut Mine(&mut a), &mut peer, C, &NewestWins);
    assert!(matches!(r.ended, Ended::Distrusted(_)), "{:?}", r.ended);
    assert_eq!(a.chains[C].entries.len(), 4, "the entry before it was taken");
    assert_eq!(chain_store::load_all(&a.conn).unwrap(), a.chains, "and stored, with its epoch");
    assert_eq!(r.entries_taken, 1);
}

#[test]
fn machines_that_appended_different_entries_at_the_same_position_settle_on_one_chain() {
    let (mut a, mut b, c) = three("kanban");
    ops::change(&mut a.ctx(), C, Change::SetPolicy { policy: Policy::Approve }).unwrap();
    let gone = ended(&[(&a, 0), (&b, 0)]);
    ops::change(&mut b.ctx(), C, Change::Remove { id: c.id.clone(), ended: gone }).unwrap();
    assert_eq!(a.chains[C].entries.len(), b.chains[C].entries.len());
    assert_ne!(a.chains[C].entries, b.chains[C].entries, "each holds its own entry at the fourth place");
    // The chains are as long as each other, so what separates them is the head: it is what the round compares.
    let r = round(&mut b, &mut a);
    assert_eq!(r.ended, Ended::Synced, "{:?}", r.ended);
    assert_eq!(a.chains[C].entries, b.chains[C].entries, "one of the two entries won on both machines");
    assert_eq!(chain_store::load_all(&b.conn).unwrap(), b.chains);
    let again = round(&mut b, &mut a);
    assert!(!again.changed(), "{again:?}");
}

#[test]
fn a_machine_whose_chain_is_shorter_settles_a_fork_with_the_longer_chain_that_won_it() {
    let (mut a, mut b, c) = three("kanban");
    ops::change(&mut a.ctx(), C, Change::SetPolicy { policy: Policy::Approve }).unwrap();
    let gone = ended(&[(&a, 0), (&b, 0)]);
    let remove = ops::change(&mut b.ctx(), C, Change::Remove { id: c.id.clone(), ended: gone }).unwrap();
    // Which of the two entries at the fourth place wins is the same everywhere: the machine that wrote it goes on from
    // there, and the other is left holding a chain one entry shorter than the winner's.
    let mut probe = a.chains[C].clone();
    let b_wins = matches!(probe.take(C, remove), Ok(super::members::Taken::Replaced(_)));
    let (mut winner, mut loser, next) = if b_wins { (b, a, Policy::Approve) } else { (a, b, Policy::AutoProven) };
    ops::change(&mut winner.ctx(), C, Change::SetPolicy { policy: next }).unwrap();
    assert_eq!(winner.chains[C].entries.len(), loser.chains[C].entries.len() + 1);
    let r = round(&mut loser, &mut winner);
    assert_eq!(r.ended, Ended::Synced, "{:?}", r.ended);
    assert_eq!(winner.chains[C].entries, loser.chains[C].entries);
    assert_eq!(chain_store::load_all(&loser.conn).unwrap(), loser.chains);
}

#[test]
fn a_peer_that_cannot_be_asked_for_membership_is_not_reported_as_one_that_does_not_list_this_machine() {
    let (mut a, _, _) = three("kanban");
    let mut stranger = node(9);
    stranger.found(C, "kanban", Policy::Approve);
    // The peer leaves the stranger out of its hello and then goes silent: the round says what happened.
    struct Silent<'a>(Link<'a>);
    impl PeerCalls for Silent<'_> {
        fn hello(&mut self, c: &[String]) -> Result<Vec<Held>, CallError> {
            self.0.hello(c)
        }
        fn members(&mut self, _: &MembersReq) -> Result<MembersAns, CallError> {
            Err(CallError::Unreachable("gone quiet".into()))
        }
        fn keys(&mut self, r: &KeysReq) -> Result<KeysAns, CallError> {
            self.0.keys(r)
        }
        fn pull(&mut self, r: &PullReq) -> Result<PullAns, CallError> {
            self.0.pull(r)
        }
        fn push(&mut self, r: &PushReq) -> Result<PushAns, CallError> {
            self.0.push(r)
        }
    }
    let mut peer = Silent(link(&stranger, &mut a));
    let r = sync_collection(&mut Mine(&mut stranger), &mut peer, C, &NewestWins);
    assert_eq!(r.ended, Ended::Unreachable("gone quiet".into()));
}

#[test]
fn a_record_for_another_collection_in_a_pull_is_refused_even_when_this_machine_holds_that_collection() {
    let (mut a, mut b, _) = three("kanban");
    // Both machines hold a second collection; b is in the first (the one being synced) and writes into the second.
    a.found("board.other", "kanban", Policy::Approve);
    b.found("board.other", "kanban", Policy::Approve);
    let rec = b.write("board.other", "task.9", "y");
    let mut peer = Answering { inner: link(&a, &mut b), entries: vec![], pulled: Some(vec![rec]) };
    let r = sync_collection(&mut Mine(&mut a), &mut peer, C, &NewestWins);
    assert!(log::current(&a.conn, "board.other", "task.9").unwrap().is_none(), "not applied to a collection the peer was not asked about");
    assert!(r.refused.iter().any(|x| x == "unknown_collection"), "{:?}", r.refused);
}
