// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A round of sync with one peer against in-memory fake machines: two machines that wrote apart end up holding the
// same in one round, a long backlog comes over in batches, a machine without the lock asks and answers nothing, a
// stranger gets nothing, a removed machine learns who removed it and stops, a lying answer is not taken, and an
// unreachable peer ends the round without harm.

use super::adapter::NewestWins;
use super::log::{self, Refused};
use super::members::Change;
use super::ops::{self, Held, KeysAns, KeysReq, MembersAns, MembersReq, OpError, PullAns, PullReq, PushAns, PushReq};
use super::ops_tests::{ended, node, three, Node, C};
use super::ops::Ctx;
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
