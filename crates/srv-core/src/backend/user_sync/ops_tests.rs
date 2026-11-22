// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The sync ops against in-memory fake machines: nothing is answered by an install without the lock, nothing to a
// caller who is not a member, answers and requests are capped, an unsigned membership change is refused, a join
// waits for a person unless the collection allows it, a removed member learns only of its removal, and a write cut
// off by a removal is written again at the new epoch and reaches the others.

use std::collections::BTreeMap;

use rusqlite::Connection;
use zeroize::Zeroizing;

use super::adapter::NewestWins;
use super::chain_store::{self, LoadError};
use super::hlc::Clock;
use super::log::{self, Outcome, Refused, Signer};
use super::members::{Change, Chain, Entry, MemberError, Policy, Subject, Taken};
use super::ops::{self, Ctx, JoinNo, JoinReq, Joined, KeyDelivery, Me, MembersReq, OpError, PendingJoin, Proof, PullReq, PushReq};
use super::record::Record;
use super::seal::KeyRing;

const NOW: i64 = 1_800_000_000_000;
const C: &str = "board.1";

struct Fake(bool);
impl Proof for Fake {
    fn auto_proven(&self, _: &str) -> bool {
        self.0
    }
}
impl KeyDelivery for Fake {
    fn keys_for(&self, _: &str, _: &Subject) -> Vec<(u32, String)> {
        if self.0 {
            vec![(0, "sealed-key".to_string())]
        } else {
            Vec::new()
        }
    }
}
static YES: Fake = Fake(true);
static NO: Fake = Fake(false);

struct Node {
    id: String,
    private: [u8; 32],
    subject: Subject,
    conn: Connection,
    chains: BTreeMap<String, Chain>,
    clock: Clock,
    pending: Vec<PendingJoin>,
    ring: KeyRing,
    active: bool,
    proven: bool,
    keys: bool,
    now: i64,
    chain_cap: usize,
}

fn node(seed: u8) -> Node {
    let private = [seed; 32];
    let subject = Subject::machine(&private, &[seed.wrapping_add(100); 32], "label").unwrap();
    let conn = Connection::open_in_memory().unwrap();
    log::init(&conn).unwrap();
    let mut ring = KeyRing::new();
    ring.insert(0, Zeroizing::new([7u8; 32]));
    ring.insert(1, Zeroizing::new([8u8; 32]));
    Node {
        id: subject.id.clone(),
        clock: Clock::new(&subject.id),
        private,
        subject,
        conn,
        chains: BTreeMap::new(),
        pending: Vec::new(),
        ring,
        active: true,
        proven: true,
        keys: true,
        now: NOW,
        chain_cap: ops::MAX_ENTRIES,
    }
}

impl Node {
    fn ctx(&mut self) -> Ctx<'_> {
        Ctx {
            conn: &self.conn,
            chains: &mut self.chains,
            clock: &mut self.clock,
            pending: &mut self.pending,
            me: Me { id: &self.id, private: &self.private },
            proof: if self.proven { &YES } else { &NO },
            keys: if self.keys { &YES } else { &NO },
            now_ms: self.now,
            chain_cap: self.chain_cap,
            active: self.active,
        }
    }

    fn found(&mut self, collection: &str, kind: &str, policy: Policy) {
        ops::found(&self.conn, &mut self.chains, collection, kind, &self.subject, &self.private, policy, NOW).unwrap();
    }

    fn write(&mut self, collection: &str, record: &str, body: &str) -> Record {
        self.write_bytes(collection, record, body.as_bytes())
    }

    fn write_bytes(&mut self, collection: &str, record: &str, body: &[u8]) -> Record {
        let mut signer = Signer { machine: &self.id, private: &self.private, clock: &mut self.clock };
        log::append_local_with(&self.conn, &mut signer, collection, record, false, NOW, super::seal::sealer(&self.ring, body)).unwrap()
    }

    fn vector(&self, collection: &str) -> log::Vector {
        log::vector(&self.conn, collection).unwrap()
    }

    fn pull_req(&self, collection: &str, limit: usize) -> PullReq {
        PullReq { collection: collection.to_string(), vector: self.vector(collection), limit }
    }
}

/// `joiner` joins `host`'s collection and takes the chain it is given, trusting only `host` itself.
fn join(host: &mut Node, joiner: &mut Node, collection: &str, kind: &str) {
    let req = JoinReq { collection: collection.to_string(), subject: joiner.subject.clone() };
    let Joined::Added { entries, .. } = ops::join(&mut host.ctx(), &joiner.id, &req).unwrap() else { panic!("joined") };
    let host_id = host.id.clone();
    let chain = ops::take_join(collection, &joiner.id, entries, &|m| m == host_id).unwrap();
    log::ensure_collection(&joiner.conn, collection, kind, NOW).unwrap();
    joiner.chains.insert(collection.to_string(), chain);
}

/// `to` asks `from` for the membership entries it lacks and takes them.
fn catch_up(to: &mut Node, from: &mut Node, collection: &str) {
    let have = to.chains[collection].entries.len() as u64;
    let ans = ops::members(&mut from.ctx(), &to.id, &MembersReq { collection: collection.to_string(), have_n: have, offered: vec![] }).unwrap();
    let chain = to.chains.get_mut(collection).unwrap();
    for e in ans.entries {
        chain.take(collection, e).unwrap();
    }
    ops::sync_epoch(&to.conn, collection, chain).unwrap();
}

/// Everything `to` lacks from `from`, through pull and push of whole answers.
fn sync_records(to: &mut Node, from: &mut Node, collection: &str) {
    loop {
        let req = to.pull_req(collection, 1000);
        let ans = ops::pull(&from.ctx(), &to.id, &req).unwrap();
        let push = PushReq { collection: collection.to_string(), records: ans.records.clone() };
        let done = ops::push(&mut to.ctx(), &from.id, &push, &NewestWins).unwrap();
        assert!(done.results.iter().all(|r| r.is_ok()), "{:?}", done.results);
        if !ans.more {
            break;
        }
    }
}

fn three(kind: &str) -> (Node, Node, Node) {
    let (mut a, mut b, mut c) = (node(1), node(2), node(3));
    a.found(C, kind, Policy::AutoProven);
    join(&mut a, &mut b, C, kind);
    join(&mut a, &mut c, C, kind);
    catch_up(&mut b, &mut a, C);
    (a, b, c)
}

fn ended(pairs: &[(&Node, u64)]) -> BTreeMap<String, u64> {
    pairs.iter().map(|(n, s)| (n.id.clone(), *s)).collect()
}

#[test]
fn an_install_without_the_lock_answers_nothing() {
    let (mut a, b, _) = three("kanban");
    a.active = false;
    let req = b.pull_req(C, 10);
    assert_eq!(ops::hello(&a.ctx(), &b.id, &[C.to_string()]), Err(OpError::NotActive));
    assert_eq!(ops::pull(&a.ctx(), &b.id, &req), Err(OpError::NotActive));
    assert_eq!(ops::push(&mut a.ctx(), &b.id, &PushReq { collection: C.into(), records: vec![] }, &NewestWins), Err(OpError::NotActive));
    assert_eq!(ops::members(&mut a.ctx(), &b.id, &MembersReq { collection: C.into(), have_n: 0, offered: vec![] }), Err(OpError::NotActive));
    let join = JoinReq { collection: C.into(), subject: b.subject.clone() };
    assert_eq!(ops::join(&mut a.ctx(), &b.id, &join), Err(OpError::NotActive));
    assert_eq!(ops::change(&mut a.ctx(), C, Change::SetPolicy { policy: Policy::Approve }), Err(OpError::NotActive), "and signs nothing");
}

#[test]
fn a_caller_that_is_not_a_member_gets_the_same_answer_as_for_a_collection_that_does_not_exist() {
    let (mut a, _, _) = three("kanban");
    let stranger = node(9);
    let req = |c: &str| PullReq { collection: c.into(), vector: log::Vector::new(), limit: 10 };
    assert_eq!(ops::pull(&a.ctx(), &stranger.id, &req(C)), Err(OpError::NotMember));
    assert_eq!(ops::pull(&a.ctx(), &stranger.id, &req("board.nothing")), Err(OpError::NotMember));
    let push = PushReq { collection: C.into(), records: vec![] };
    assert_eq!(ops::push(&mut a.ctx(), &stranger.id, &push, &NewestWins), Err(OpError::NotMember));
    let members = MembersReq { collection: C.into(), have_n: 0, offered: vec![] };
    assert_eq!(ops::members(&mut a.ctx(), &stranger.id, &members), Err(OpError::NotMember));
    assert_eq!(ops::hello(&a.ctx(), &stranger.id, &[C.to_string()]), Ok(vec![]), "hello leaves out what the caller is not in");
}

#[test]
fn hello_lists_only_the_collections_the_caller_is_a_member_of() {
    let (mut a, b, _) = three("kanban");
    a.found("board.2", "kanban", Policy::Approve);
    let held = ops::hello(&a.ctx(), &b.id, &[C.to_string(), "board.2".to_string()]).unwrap();
    assert_eq!(held.iter().map(|h| h.collection.as_str()).collect::<Vec<_>>(), vec![C]);
    assert_eq!((held[0].n, held[0].epoch), (3, 0));
    let names: Vec<String> = (0..65).map(|i| format!("c{i}")).collect();
    assert_eq!(ops::hello(&a.ctx(), &b.id, &names), Err(OpError::TooBig));
}

#[test]
fn a_pull_answer_is_capped_in_records_and_in_bytes_and_the_asker_refuses_one_over_its_caps() {
    let (mut a, b, _) = three("kanban");
    for i in 0..250 {
        a.write(C, &format!("r{i}"), "x");
    }
    let first = ops::pull(&a.ctx(), &b.id, &b.pull_req(C, 10_000)).unwrap();
    assert_eq!((first.records.len(), first.more), (ops::MAX_BATCH_RECORDS, true));
    let small = ops::pull(&a.ctx(), &b.id, &b.pull_req(C, 5)).unwrap();
    assert_eq!((small.records.len(), small.more), (5, true));
    assert_eq!(ops::check_pull_answer(&first, 10_000), Ok(()));
    assert_eq!(ops::check_pull_answer(&first, 5), Err(OpError::TooBig), "more than was asked for");

    let (mut a, b, _) = three("kanban");
    for i in 0..3 {
        a.write_bytes(C, &format!("big{i}"), &vec![1u8; 100 * 1024]);
    }
    let two = ops::pull(&a.ctx(), &b.id, &b.pull_req(C, 10)).unwrap();
    assert_eq!((two.records.len(), two.more), (2, true), "the third would pass the byte cap");
    let mut over = two.clone();
    over.records.push(over.records[0].clone());
    assert_eq!(ops::check_pull_answer(&over, 10), Err(OpError::TooBig));
    let huge = a.write_bytes(C, "huge", &vec![1u8; 1024 * 1024]);
    let alone = super::ops::PullAns { records: vec![huge], more: false, epoch: 0, n: 3 };
    assert_eq!(ops::check_pull_answer(&alone, 10), Ok(()), "one large record goes alone");
}

#[test]
fn a_push_applies_each_record_and_one_over_the_caps_is_refused_before_anything_is_applied() {
    let (mut a, mut b, _) = three("kanban");
    let r1 = b.write(C, "r1", "one");
    let mut other = b.write(C, "r2", "two");
    other.header.collection = "board.2".into();
    let ans = ops::push(&mut a.ctx(), &b.id, &PushReq { collection: C.into(), records: vec![r1, other] }, &NewestWins).unwrap();
    assert_eq!(ans.results, vec![Ok(Outcome::Applied), Err(Refused::UnknownCollection)], "a record for another collection is not applied");
    assert_eq!(ans.vector.get(&b.id), Some(&1));

    let many: Vec<Record> = (0..=ops::MAX_BATCH_RECORDS).map(|i| b.write(C, &format!("m{i}"), "x")).collect();
    let before = a.vector(C);
    assert_eq!(ops::push(&mut a.ctx(), &b.id, &PushReq { collection: C.into(), records: many }, &NewestWins), Err(OpError::TooBig));
    assert_eq!(a.vector(C), before);
}

#[test]
fn a_membership_entry_no_member_signed_is_refused_and_a_member_s_is_taken() {
    let (mut a, mut b, _) = three("kanban");
    let stranger = node(9);
    let chain = &a.chains[C];
    let forged = Entry::sign(&stranger.private, C, chain.entries.len() as u64 + 1, chain.entries.last().map(Entry::hash), Change::Add { member: stranger.subject.clone() }, &stranger.id).unwrap();
    let ans = ops::members(&mut a.ctx(), &b.id, &MembersReq { collection: C.into(), have_n: 3, offered: vec![forged] }).unwrap();
    assert_eq!(ans.taken, vec![Err(MemberError::NotAMember)]);
    assert_eq!(a.chains[C].entries.len(), 3);

    let policy = ops::change(&mut b.ctx(), C, Change::SetPolicy { policy: Policy::Approve }).unwrap();
    let ans = ops::members(&mut a.ctx(), &b.id, &MembersReq { collection: C.into(), have_n: 3, offered: vec![policy] }).unwrap();
    assert_eq!(ans.taken, vec![Ok(Taken::Extended)]);
    assert_eq!(a.chains[C].state(C).unwrap().policy, Policy::Approve);
}

#[test]
fn a_removed_machine_learns_how_it_was_removed_and_nothing_after_it() {
    let (mut a, b, c) = three("kanban");
    let gone = ended(&[(&a, 0), (&b, 0)]);
    let remove = ops::change(&mut a.ctx(), C, Change::Remove { id: c.id.clone(), ended: gone }).unwrap();
    let later = node(4);
    let add = ops::change(&mut a.ctx(), C, Change::Add { member: later.subject.clone() }).unwrap();
    let ask = MembersReq { collection: C.into(), have_n: 3, offered: vec![] };
    let ans = ops::members(&mut a.ctx(), &c.id, &ask).unwrap();
    assert_eq!(ans.entries, vec![remove], "its removal, and not the member added after it");
    assert!(!ans.entries.contains(&add));
    // It cannot read or write the collection, nor offer an entry.
    assert_eq!(ops::pull(&a.ctx(), &c.id, &c.pull_req(C, 10)), Err(OpError::NotMember));
    let policy = Entry::sign(&c.private, C, 4, a.chains[C].entries.get(2).map(Entry::hash), Change::SetPolicy { policy: Policy::AutoProven }, &c.id).unwrap();
    let ans = ops::members(&mut a.ctx(), &c.id, &MembersReq { collection: C.into(), have_n: 3, offered: vec![policy] }).unwrap();
    assert!(ans.taken.is_empty(), "a former member's offer is not taken");
}

#[test]
fn a_join_without_a_person_needs_the_policy_the_kind_and_a_proven_caller() {
    let (mut a, mut b) = (node(1), node(2));
    a.found(C, "kanban", Policy::AutoProven);
    a.proven = false;
    let req = JoinReq { collection: C.into(), subject: b.subject.clone() };
    assert_eq!(ops::join(&mut a.ctx(), &b.id, &req), Ok(Joined::Pending), "not proven: a person confirms");
    a.proven = true;
    // A Vault never joins without a person yet, whatever its policy says.
    a.found("vault.1", "vault", Policy::AutoProven);
    let vault = JoinReq { collection: "vault.1".into(), subject: b.subject.clone() };
    assert_eq!(ops::join(&mut a.ctx(), &b.id, &vault), Ok(Joined::Pending));
    assert_eq!(a.chains["vault.1"].entries.len(), 1);
    // A join for another machine than the caller is refused.
    let c = node(3);
    let other = JoinReq { collection: C.into(), subject: c.subject.clone() };
    assert_eq!(ops::join(&mut a.ctx(), &b.id, &other), Err(OpError::Bad));
    // Approve policy: pending even when proven.
    a.found("board.3", "kanban", Policy::Approve);
    let approve = JoinReq { collection: "board.3".into(), subject: b.subject.clone() };
    assert_eq!(ops::join(&mut a.ctx(), &b.id, &approve), Ok(Joined::Pending));
    join(&mut a, &mut b, C, "kanban");
    assert_eq!(a.chains[C].state(C).unwrap().members.len(), 2);
}

#[test]
fn a_waiting_join_is_added_only_when_a_person_confirms_and_the_machine_then_gets_the_chain_and_keys() {
    let (mut a, b) = (node(1), node(2));
    a.found("vault.1", "vault", Policy::Approve);
    let req = JoinReq { collection: "vault.1".into(), subject: b.subject.clone() };
    assert_eq!(ops::join(&mut a.ctx(), &b.id, &req), Ok(Joined::Pending));
    assert_eq!(ops::join(&mut a.ctx(), &b.id, &req), Ok(Joined::Pending));
    assert_eq!(a.pending.len(), 1, "asking again does not queue twice");
    let entry = ops::approve(&mut a.ctx(), "vault.1", &b.id).unwrap();
    assert_eq!(entry.signer, a.id);
    assert!(a.pending.is_empty());
    assert_eq!(ops::approve(&mut a.ctx(), "vault.1", &b.id), Err(OpError::Bad), "nothing left to confirm");
    let Joined::Added { entries, keys } = ops::join(&mut a.ctx(), &b.id, &req).unwrap() else { panic!("added") };
    assert_eq!((entries.len(), keys.len()), (2, 1));
    let a_id = a.id.clone();
    assert!(ops::take_join("vault.1", &b.id, entries, &|m| m == a_id).is_ok());
}

#[test]
fn the_waiting_list_is_capped_and_a_join_with_no_key_to_give_adds_nobody() {
    let mut a = node(1);
    a.found(C, "kanban", Policy::Approve);
    for seed in 10..(10 + ops::MAX_PENDING as u8) {
        let n = node(seed);
        let req = JoinReq { collection: C.into(), subject: n.subject.clone() };
        assert_eq!(ops::join(&mut a.ctx(), &n.id, &req), Ok(Joined::Pending));
    }
    let n = node(99);
    assert_eq!(ops::join(&mut a.ctx(), &n.id, &JoinReq { collection: C.into(), subject: n.subject.clone() }), Err(OpError::TooBig));

    let mut a = node(1);
    a.found(C, "kanban", Policy::AutoProven);
    a.keys = false;
    let b = node(2);
    assert_eq!(ops::join(&mut a.ctx(), &b.id, &JoinReq { collection: C.into(), subject: b.subject.clone() }), Err(OpError::NoKey));
    assert_eq!(a.chains[C].entries.len(), 1, "no entry was signed");
}

#[test]
fn a_joiner_takes_a_chain_only_when_it_verifies_lists_it_and_has_a_member_it_trusts_itself() {
    let (mut a, b) = (node(1), node(2));
    a.found(C, "kanban", Policy::AutoProven);
    let req = JoinReq { collection: C.into(), subject: b.subject.clone() };
    let Joined::Added { entries, .. } = ops::join(&mut a.ctx(), &b.id, &req).unwrap() else { panic!("added") };
    let a_id = a.id.clone();
    assert!(ops::take_join(C, &b.id, entries.clone(), &|m| m == a_id).is_ok());
    assert_eq!(ops::take_join(C, &b.id, entries.clone(), &|_| false).err(), Some(JoinNo::NoAnchor), "the sender's word is not the anchor");
    assert_eq!(ops::take_join(C, "someone-else", entries.clone(), &|_| true).err(), Some(JoinNo::NotInIt));
    let mut altered = entries.clone();
    altered[1].signer = b.id.clone();
    assert!(matches!(ops::take_join(C, &b.id, altered, &|_| true), Err(JoinNo::Unsound(_))));
}

#[test]
fn a_write_cut_off_by_a_removal_is_written_again_at_the_new_epoch_and_reaches_the_others() {
    let (mut a, mut b, mut c) = three("kanban");
    a.write(C, "card", "first");
    sync_records(&mut b, &mut a, C);
    sync_records(&mut c, &mut a, C);
    // A writes again before it hears of the removal; B removes C having seen only A's first write.
    let lost = a.write(C, "card", "second");
    assert_eq!(lost.header.seq, 2);
    let gone = ended(&[(&a, 1), (&b, 0)]);
    let remove = ops::change(&mut b.ctx(), C, Change::Remove { id: c.id.clone(), ended: gone }).unwrap();
    let ans = ops::members(&mut a.ctx(), &b.id, &MembersReq { collection: C.into(), have_n: 3, offered: vec![remove] }).unwrap();
    assert_eq!(ans.taken, vec![Ok(Taken::Extended)]);
    assert_eq!(log::collection_epoch(&a.conn, C).unwrap(), Some(1));
    // A re-issues it at the new epoch; the cut-off write still goes with it, so A's sequence reaches B unbroken.
    let mut signer = Signer { machine: &a.id, private: &a.private, clock: &mut a.clock };
    assert_eq!(ops::reissue(&a.conn, &mut signer, &a.ring, C, NOW).unwrap(), 1);
    let sent = log::since(&a.conn, C, &b.vector(C), 100).unwrap();
    assert_eq!(sent.iter().map(|r| (r.header.seq, r.header.epoch)).collect::<Vec<_>>(), vec![(2, 0), (3, 1)], "the cut-off write goes too, as a void one");
    let ans = ops::push(&mut b.ctx(), &a.id, &PushReq { collection: C.into(), records: sent }, &NewestWins).unwrap();
    assert_eq!(ans.results, vec![Ok(Outcome::Void), Ok(Outcome::Applied)], "the cut-off write changes nothing; the re-issued one is applied");
    let held = log::current(&b.conn, C, "card").unwrap().unwrap();
    let plain = super::seal::open(&b.ring, &held.header, &held.body).unwrap();
    assert_eq!(&plain[..], b"second");
    // The removed machine's old-epoch write is refused: it is no member.
    let stale = c.write(C, "card", "from the removed one");
    let refused = ops::push(&mut b.ctx(), &c.id, &PushReq { collection: C.into(), records: vec![stale] }, &NewestWins);
    assert_eq!(refused, Err(OpError::NotMember));
}

#[test]
fn a_write_at_the_new_epoch_is_a_gap_until_what_the_cut_owes_is_held() {
    let (mut a, mut b, c) = three("kanban");
    a.write(C, "x", "1");
    a.write(C, "x", "2");
    let gone = ended(&[(&a, 2), (&b, 0)]);
    let remove = ops::change(&mut b.ctx(), C, Change::Remove { id: c.id.clone(), ended: gone }).unwrap();
    ops::members(&mut a.ctx(), &b.id, &MembersReq { collection: C.into(), have_n: 3, offered: vec![remove] }).unwrap();
    // B holds none of A's records yet: A's end at the old epoch (2) is still owed, so a later sequence is a gap.
    let mut signer = Signer { machine: &a.id, private: &a.private, clock: &mut a.clock };
    let later = log::append_local_with(&a.conn, &mut signer, C, "y", false, NOW, super::seal::sealer(&a.ring, b"3")).unwrap();
    assert_eq!(later.header.epoch, 1);
    let ans = ops::push(&mut b.ctx(), &a.id, &PushReq { collection: C.into(), records: vec![later.clone()] }, &NewestWins).unwrap();
    assert_eq!(ans.results, vec![Err(Refused::Gap { have: 0 })]);
    sync_records(&mut b, &mut a, C);
    assert_eq!(b.vector(C).get(&a.id), Some(&3));
}

#[test]
fn nothing_is_sealed_while_a_removal_at_the_head_is_contested() {
    let (mut a, mut b, c) = three("kanban");
    let policy = ops::change(&mut a.ctx(), C, Change::SetPolicy { policy: Policy::Approve }).unwrap();
    let gone = ended(&[(&a, 0), (&b, 0)]);
    let remove = ops::change(&mut b.ctx(), C, Change::Remove { id: c.id.clone(), ended: gone }).unwrap();
    assert!(ops::may_seal(&a.ctx(), C));
    let ans = ops::members(&mut a.ctx(), &b.id, &MembersReq { collection: C.into(), have_n: 3, offered: vec![remove] }).unwrap();
    assert_eq!(ans.taken, vec![Err(MemberError::ForkLost)], "the earlier member's entry stands");
    assert!(!ops::may_seal(&a.ctx(), C), "the removal beside the head is not settled");
    let d = node(4);
    ops::change(&mut a.ctx(), C, Change::Add { member: d.subject.clone() }).unwrap();
    assert!(ops::may_seal(&a.ctx(), C), "once the head is outgrown the fork is over");
    let _ = policy;
}

#[test]
fn the_ops_reach_nothing_but_the_log_and_the_chains() {
    let src = include_str!("ops.rs");
    for word in ["reqwest", "hyper", "TcpStream", "UdpSocket", "std::net", "tokio::net", "relay", "http"] {
        assert!(!src.contains(word), "ops.rs mentions {word}");
    }
}

#[test]
fn a_subject_that_is_not_what_it_says_is_never_queued_and_a_waiting_join_expires_or_is_dropped_when_it_cannot_be_added() {
    let (mut a, b) = (node(1), node(2));
    a.found(C, "kanban", Policy::Approve);
    let mut forged = b.subject.clone();
    forged.box_key = "00".repeat(32);
    assert_eq!(ops::join(&mut a.ctx(), &b.id, &JoinReq { collection: C.into(), subject: forged }), Err(OpError::Bad));
    assert!(a.pending.is_empty(), "a subject whose box key its signing key did not sign is not queued");

    // One that was added by another member meanwhile: approving it fails, and it does not stay in the list.
    let req = JoinReq { collection: C.into(), subject: b.subject.clone() };
    assert_eq!(ops::join(&mut a.ctx(), &b.id, &req), Ok(Joined::Pending));
    ops::change(&mut a.ctx(), C, Change::Add { member: b.subject.clone() }).unwrap();
    assert_eq!(ops::approve(&mut a.ctx(), C, &b.id), Err(OpError::Bad));
    assert!(a.pending.is_empty());

    // A full list frees itself when its entries have waited too long.
    let mut full = node(1);
    full.found(C, "kanban", Policy::Approve);
    for seed in 10..(10 + ops::MAX_PENDING as u8) {
        let n = node(seed);
        ops::join(&mut full.ctx(), &n.id, &JoinReq { collection: C.into(), subject: n.subject.clone() }).unwrap();
    }
    let late = node(99);
    let ask = JoinReq { collection: C.into(), subject: late.subject.clone() };
    assert_eq!(ops::join(&mut full.ctx(), &late.id, &ask), Err(OpError::TooBig));
    full.now = NOW + ops::PENDING_TTL_MS;
    assert_eq!(ops::join(&mut full.ctx(), &late.id, &ask), Ok(Joined::Pending));
    assert_eq!(full.pending.len(), 1);
}

#[test]
fn a_chain_too_long_to_hand_over_adds_nobody() {
    let (mut a, b) = (node(1), node(2));
    a.found(C, "kanban", Policy::AutoProven);
    a.chain_cap = 3;
    ops::change(&mut a.ctx(), C, Change::SetPolicy { policy: Policy::Approve }).unwrap();
    ops::change(&mut a.ctx(), C, Change::SetPolicy { policy: Policy::AutoProven }).unwrap();
    assert_eq!(a.chains[C].entries.len(), 3);
    let req = JoinReq { collection: C.into(), subject: b.subject.clone() };
    assert_eq!(ops::join(&mut a.ctx(), &b.id, &req), Err(OpError::TooBig));
    assert_eq!(a.chains[C].entries.len(), 3, "no entry was signed for it");
    assert!(!a.chains[C].state(C).unwrap().members.contains_key(&b.id));
}

#[test]
fn a_member_whose_removal_is_among_the_entries_it_offers_sees_nothing_after_it() {
    let (mut a, mut b, c) = three("kanban");
    let gone = ended(&[(&a, 0), (&b, 0)]);
    let remove = ops::change(&mut b.ctx(), C, Change::Remove { id: c.id.clone(), ended: gone }).unwrap();
    // The entries a member of another copy of the chain signed on top of it, handed over by the machine they remove.
    let mut other = a.chains[C].clone();
    other.take(C, remove.clone()).unwrap();
    let d = node(4);
    let add = other.next(&a.private, C, &a.id, Change::Add { member: d.subject.clone() }).unwrap();
    let ans = ops::members(&mut a.ctx(), &c.id, &MembersReq { collection: C.into(), have_n: 3, offered: vec![remove.clone(), add] }).unwrap();
    assert_eq!(ans.taken.len(), 2);
    assert_eq!(ans.entries, vec![remove], "its removal and nothing after it, though the offer was taken first");
}

#[test]
fn a_removal_that_loses_a_fork_leaves_neither_its_epoch_nor_its_ends_behind() {
    let (mut a, mut b, c) = three("kanban");
    a.write(C, "x", "1");
    let policy = ops::change(&mut a.ctx(), C, Change::SetPolicy { policy: Policy::Approve }).unwrap();
    let gone = ended(&[(&a, 0), (&b, 0)]);
    ops::change(&mut b.ctx(), C, Change::Remove { id: c.id.clone(), ended: gone }).unwrap();
    assert_eq!(log::collection_epoch(&b.conn, C).unwrap(), Some(1));
    // The earlier member's competing entry wins at B: the removal is gone, and with it its epoch and its ends.
    let ans = ops::members(&mut b.ctx(), &a.id, &MembersReq { collection: C.into(), have_n: 3, offered: vec![policy] }).unwrap();
    assert!(matches!(ans.taken[0], Ok(Taken::Replaced(_))));
    assert_eq!(log::collection_epoch(&b.conn, C).unwrap(), Some(0));
    let pushed = log::since(&a.conn, C, &log::Vector::new(), 10).unwrap();
    let ans = ops::push(&mut b.ctx(), &a.id, &PushReq { collection: C.into(), records: pushed }, &NewestWins).unwrap();
    assert_eq!(ans.results, vec![Ok(Outcome::Applied)], "A's write past the lost removal's end is taken");
}

#[test]
fn a_later_sequence_after_a_cut_is_never_taken_before_the_ones_before_it() {
    let (mut a, mut b, c) = three("kanban");
    a.write(C, "x", "1");
    sync_records(&mut b, &mut a, C);
    a.write(C, "x", "2");
    let gone = ended(&[(&a, 1), (&b, 0)]);
    let remove = ops::change(&mut b.ctx(), C, Change::Remove { id: c.id.clone(), ended: gone }).unwrap();
    ops::members(&mut a.ctx(), &b.id, &MembersReq { collection: C.into(), have_n: 3, offered: vec![remove] }).unwrap();
    let mut signer = Signer { machine: &a.id, private: &a.private, clock: &mut a.clock };
    assert_eq!(ops::reissue(&a.conn, &mut signer, &a.ring, C, NOW).unwrap(), 1);
    a.write(C, "y", "4");
    let all = log::since(&a.conn, C, &b.vector(C), 100).unwrap();
    assert_eq!(all.iter().map(|r| r.header.seq).collect::<Vec<_>>(), vec![2, 3, 4]);
    // The last arrives first: 2 (void) and 3 are still owed, so it is a gap and the vector does not move past them.
    let early = ops::push(&mut b.ctx(), &a.id, &PushReq { collection: C.into(), records: vec![all[2].clone()] }, &NewestWins).unwrap();
    assert_eq!(early.results, vec![Err(Refused::Gap { have: 1 })]);
    assert_eq!(b.vector(C).get(&a.id), Some(&1));
    let in_order = ops::push(&mut b.ctx(), &a.id, &PushReq { collection: C.into(), records: all }, &NewestWins).unwrap();
    assert_eq!(in_order.results, vec![Ok(Outcome::Void), Ok(Outcome::Applied), Ok(Outcome::Applied)]);
    assert_eq!(b.vector(C).get(&a.id), Some(&4));
}

#[test]
fn an_approval_with_no_key_to_give_adds_nobody_and_keeps_the_request_waiting() {
    let (mut a, b) = (node(1), node(2));
    a.found(C, "kanban", Policy::Approve);
    let req = JoinReq { collection: C.into(), subject: b.subject.clone() };
    assert_eq!(ops::join(&mut a.ctx(), &b.id, &req), Ok(Joined::Pending));
    a.keys = false;
    assert_eq!(ops::approve(&mut a.ctx(), C, &b.id), Err(OpError::NoKey));
    assert_eq!((a.pending.len(), a.chains[C].entries.len()), (1, 1));
    a.keys = true;
    assert!(ops::approve(&mut a.ctx(), C, &b.id).is_ok());
}

#[test]
fn the_chains_come_back_after_a_restart_with_their_removals_and_unsettled_forks() {
    let (mut a, mut b, c) = three("kanban");
    a.found("board.2", "kanban", Policy::Approve);
    assert_eq!(chain_store::load_all(&a.conn).unwrap(), a.chains);
    // A fork not yet outgrown, and a removal: both are read back.
    ops::change(&mut a.ctx(), C, Change::SetPolicy { policy: Policy::Approve }).unwrap();
    let gone = ended(&[(&a, 0), (&b, 0)]);
    let remove = ops::change(&mut b.ctx(), C, Change::Remove { id: c.id.clone(), ended: gone }).unwrap();
    ops::members(&mut a.ctx(), &b.id, &MembersReq { collection: C.into(), have_n: 3, offered: vec![remove] }).unwrap();
    assert!(!ops::may_seal(&a.ctx(), C));
    let back = chain_store::load_all(&a.conn).unwrap();
    assert_eq!(back, a.chains);
    assert!(back[C].epoch_contested(), "the fork is still unsettled after a restart");
    let bb = chain_store::load_all(&b.conn).unwrap();
    assert_eq!(bb[C].state(C).unwrap().epoch, 1);
}

#[test]
fn a_stored_chain_that_was_altered_is_an_error_not_a_shorter_collection() {
    let (a, _, _) = three("kanban");
    let altered = |sql: &str| {
        let copy = Connection::open_in_memory().unwrap();
        log::init(&copy).unwrap();
        for (slot, idx, text) in a.conn.prepare("SELECT slot, idx, entry FROM db_sync_chain").unwrap().query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?))).unwrap().map(|r| r.unwrap()) {
            copy.execute("INSERT INTO db_sync_chain (collection, slot, idx, entry) VALUES (?1, ?2, ?3, ?4)", rusqlite::params![C, slot, idx, text]).unwrap();
        }
        copy.execute(sql, []).unwrap();
        chain_store::load(&copy, C)
    };
    assert!(chain_store::load(&a.conn, C).unwrap().is_some());
    assert!(matches!(altered("DELETE FROM db_sync_chain WHERE idx = 1"), Err(LoadError::Unsound(_))), "a middle entry gone");
    assert!(matches!(altered("UPDATE db_sync_chain SET entry = replace(entry, '\"n\":2', '\"n\":3') WHERE idx = 1"), Err(LoadError::Unsound(_))));
    assert!(matches!(altered("UPDATE db_sync_chain SET entry = 'not json' WHERE idx = 2"), Err(LoadError::Unreadable)));
}

#[test]
fn a_joiner_keeps_the_chain_it_took_and_its_epoch_follows_it() {
    let (mut a, mut b, c) = three("kanban");
    let gone = ended(&[(&a, 0), (&b, 0)]);
    ops::change(&mut a.ctx(), C, Change::Remove { id: c.id.clone(), ended: gone }).unwrap();
    let mut d = node(4);
    let req = JoinReq { collection: C.into(), subject: d.subject.clone() };
    let Joined::Added { entries, .. } = ops::join(&mut a.ctx(), &d.id, &req).unwrap() else { panic!("added") };
    let a_id = a.id.clone();
    let chain = ops::take_join(C, &d.id, entries, &|m| m == a_id).unwrap();
    ops::adopt(&d.conn, &mut d.chains, C, "kanban", chain, NOW).unwrap();
    assert_eq!(log::collection_epoch(&d.conn, C).unwrap(), Some(1));
    assert_eq!(chain_store::load_all(&d.conn).unwrap(), d.chains);
    let _ = &mut b;
}
