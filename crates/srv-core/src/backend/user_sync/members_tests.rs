// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The membership chain: a change that no member signed is refused, an entry that does not extend the chain is
// refused, a removed member can sign nothing, a recovery subject never signs or anchors, and a fork settles by hash
// to the same head on every machine.

use std::collections::BTreeMap;

use super::members::{Change, Chain, Entry, Kind, MemberError, Policy, State, Subject, Taken};

const C: &str = "c1";

/// A machine: the seed of its signing key and the subject it offers.
fn machine(seed: u8) -> ([u8; 32], Subject) {
    let private = [seed; 32];
    let subject = Subject::machine(&private, &[seed.wrapping_add(100); 32], "sealed-label").unwrap();
    (private, subject)
}

/// A chain founded by machine 1.
fn founded() -> (Chain, [u8; 32], Subject) {
    let (private, a) = machine(1);
    let mut chain = Chain::default();
    let found = Entry::sign(&private, C, 1, None, Change::Found { member: a.clone(), policy: Policy::Approve }, &a.id).unwrap();
    assert_eq!(chain.take(C, found), Ok(Taken::Extended));
    (chain, private, a)
}

fn add(chain: &mut Chain, by: &[u8; 32], signer: &Subject, member: &Subject) {
    let entry = chain.next(by, C, &signer.id, Change::Add { member: member.clone() }).unwrap();
    assert_eq!(chain.take(C, entry), Ok(Taken::Extended));
}

#[test]
fn members_are_machines_and_recovery_subjects_and_only_machines_are_pinned_or_anchor_a_join() {
    let (mut chain, ka, a) = founded();
    let (kb, b) = machine(2);
    add(&mut chain, &ka, &a, &b);
    let recovery = Subject::recovery(&[9; 32], "sealed");
    add(&mut chain, &kb, &b, &recovery);
    let state = chain.state(C);
    assert_eq!(state.members.len(), 3);
    assert_eq!(state.epoch, 0);
    let pins = state.pins();
    assert_eq!(pins.keys().cloned().collect::<Vec<_>>(), { let mut v = vec![a.id.clone(), b.id.clone()]; v.sort(); v });
    assert!(!state.anchored(&|id| id == recovery.id), "a recovery subject is never an anchor");
    assert!(state.anchored(&|id| id == b.id));
    assert!(!state.anchored(&|_| false));
    assert_eq!(recovery.kind, Kind::Recovery);
}

#[test]
fn a_change_that_no_member_signed_is_refused() {
    let (mut chain, ka, a) = founded();
    let (kb, b) = machine(2);
    let (kc, c) = machine(3);
    let prev = chain.entries.last().map(Entry::hash);
    // Signed by a machine that is not a member.
    let stranger = Entry::sign(&kc, C, 2, prev.clone(), Change::Add { member: c.clone() }, &c.id).unwrap();
    assert_eq!(chain.take(C, stranger), Err(MemberError::NotAMember));
    // Names a member as the signer but is signed with another key.
    let forged = Entry::sign(&kb, C, 2, prev.clone(), Change::Add { member: b.clone() }, &a.id).unwrap();
    assert_eq!(chain.take(C, forged), Err(MemberError::BadSignature));
    // Altered after it was signed.
    let mut altered = chain.next(&ka, C, &a.id, Change::Add { member: b.clone() }).unwrap();
    altered.change = Change::Add { member: c.clone() };
    assert_eq!(chain.take(C, altered), Err(MemberError::BadSignature));
    // A recovery subject signs nothing.
    let recovery = Subject::recovery(&[9; 32], "sealed");
    add(&mut chain, &ka, &a, &recovery);
    let by_recovery = chain.next(&ka, C, &recovery.id, Change::Add { member: b.clone() }).unwrap();
    assert_eq!(chain.take(C, by_recovery), Err(MemberError::NotAMember));
    assert_eq!(chain.state(C).members.len(), 2, "nothing refused was taken");
    let _ = kb;
}

#[test]
fn an_entry_that_does_not_extend_the_chain_is_refused() {
    let (mut chain, ka, a) = founded();
    let (_, b) = machine(2);
    let (_, c) = machine(3);
    let good = chain.next(&ka, C, &a.id, Change::Add { member: b.clone() }).unwrap();
    let wrong_prev = Entry::sign(&ka, C, 2, Some("00".repeat(32)), Change::Add { member: b.clone() }, &a.id).unwrap();
    let wrong_n = Entry::sign(&ka, C, 5, good.prev.clone(), Change::Add { member: b.clone() }, &a.id).unwrap();
    let other_collection = Entry::sign(&ka, "c2", 2, good.prev.clone(), Change::Add { member: b.clone() }, &a.id).unwrap();
    for bad in [wrong_prev, wrong_n, other_collection] {
        assert_eq!(chain.take(C, bad), Err(MemberError::DoesNotExtend));
    }
    assert_eq!(chain.take(C, good.clone()), Ok(Taken::Extended));
    assert_eq!(chain.take(C, good), Ok(Taken::Duplicate), "an entry already held changes nothing");
    // Only the first entry founds, and the first entry must found.
    let second_found = chain.next(&ka, C, &a.id, Change::Found { member: c.clone(), policy: Policy::AutoProven }).unwrap();
    assert_eq!(chain.take(C, second_found), Err(MemberError::Founding));
    let mut empty = Chain::default();
    let first_add = Entry::sign(&ka, C, 1, None, Change::Add { member: b }, &a.id).unwrap();
    assert_eq!(empty.take(C, first_add), Err(MemberError::Founding));
}

#[test]
fn a_subject_must_be_what_it_says() {
    let (mut chain, ka, a) = founded();
    let (_, b) = machine(2);
    let try_add = |chain: &mut Chain, member: Subject| {
        let entry = chain.next(&ka, C, &a.id, Change::Add { member }).unwrap();
        chain.take(C, entry)
    };
    let mut other_id = b.clone();
    other_id.id = "someoneelse".into();
    assert_eq!(try_add(&mut chain, other_id), Err(MemberError::BadSubject), "the id is made from the signing key");
    let mut swapped_box = b.clone();
    swapped_box.box_key = "ab".repeat(32);
    assert_eq!(try_add(&mut chain, swapped_box), Err(MemberError::BadSubject), "the box key is signed by the signing key");
    let mut no_key = b.clone();
    no_key.signing_key = None;
    assert_eq!(try_add(&mut chain, no_key), Err(MemberError::BadSubject));
    let mut recovery_with_key = Subject::recovery(&[9; 32], "s");
    recovery_with_key.signing_key = b.signing_key.clone();
    assert_eq!(try_add(&mut chain, recovery_with_key), Err(MemberError::BadSubject));
    assert_eq!(try_add(&mut chain, b.clone()), Ok(Taken::Extended));
    assert_eq!(try_add(&mut chain, b), Err(MemberError::AlreadyMember));
}

#[test]
fn a_removal_moves_the_epoch_and_a_removed_member_signs_nothing() {
    let (mut chain, ka, a) = founded();
    let (kb, b) = machine(2);
    let (kc, c) = machine(3);
    add(&mut chain, &ka, &a, &b);
    add(&mut chain, &ka, &a, &c);
    let ended: BTreeMap<String, u64> = [(a.id.clone(), 7), (c.id.clone(), 3)].into();
    let remove = chain.next(&ka, C, &a.id, Change::Remove { id: b.id.clone(), ended: ended.clone() }).unwrap();
    assert_eq!(chain.take(C, remove), Ok(Taken::Extended));
    let state = chain.state(C);
    assert_eq!((state.epoch, state.members.contains_key(&b.id)), (1, false));
    assert!(!state.pins().contains_key(&b.id));
    // The removed member can no longer change the list.
    let late = chain.next(&kb, C, &b.id, Change::SetPolicy { policy: Policy::AutoProven }).unwrap();
    assert_eq!(chain.take(C, late), Err(MemberError::NotAMember));
    // `ended` may only name machines that remain.
    let names_removed: BTreeMap<String, u64> = [(b.id.clone(), 1)].into();
    let bad = chain.next(&ka, C, &a.id, Change::Remove { id: c.id.clone(), ended: names_removed }).unwrap();
    assert_eq!(chain.take(C, bad), Err(MemberError::BadRemoval));
    // A machine may remove itself, but the last machine cannot go, and a stranger is not removable.
    let leave = chain.next(&kc, C, &c.id, Change::Remove { id: c.id.clone(), ended: [(a.id.clone(), 7)].into() }).unwrap();
    assert_eq!(chain.take(C, leave), Ok(Taken::Extended));
    assert_eq!(chain.state(C).epoch, 2);
    let last = chain.next(&ka, C, &a.id, Change::Remove { id: a.id.clone(), ended: BTreeMap::new() }).unwrap();
    assert_eq!(chain.take(C, last), Err(MemberError::BadRemoval));
    let stranger = chain.next(&ka, C, &a.id, Change::Remove { id: "nobody".into(), ended: BTreeMap::new() }).unwrap();
    assert_eq!(chain.take(C, stranger), Err(MemberError::BadRemoval));
}

#[test]
fn how_machines_join_changes_only_by_a_signed_entry() {
    let (mut chain, ka, a) = founded();
    assert_eq!(chain.state(C).policy, Policy::Approve);
    let (kc, c) = machine(3);
    let by_stranger = Entry::sign(&kc, C, 2, chain.entries.last().map(Entry::hash), Change::SetPolicy { policy: Policy::AutoProven }, &c.id).unwrap();
    assert_eq!(chain.take(C, by_stranger), Err(MemberError::NotAMember));
    assert_eq!(chain.state(C).policy, Policy::Approve, "a peer cannot quietly relax approve");
    let signed = chain.next(&ka, C, &a.id, Change::SetPolicy { policy: Policy::AutoProven }).unwrap();
    assert_eq!(chain.take(C, signed), Ok(Taken::Extended));
    assert_eq!(chain.state(C).policy, Policy::AutoProven);
}

#[test]
fn a_fork_settles_by_hash_to_the_same_head_on_every_machine() {
    let (base, ka, a) = founded();
    let (_, b) = machine(2);
    let (_, c) = machine(3);
    let x = base.next(&ka, C, &a.id, Change::Add { member: b.clone() }).unwrap();
    let y = base.next(&ka, C, &a.id, Change::Add { member: c.clone() }).unwrap();
    let (mut p, mut q) = (base.clone(), base.clone());
    let first_p = p.take(C, x.clone()).and_then(|_| p.take(C, y.clone()));
    let first_q = q.take(C, y.clone()).and_then(|_| q.take(C, x.clone()));
    assert_eq!(p, q, "both machines hold the same entry");
    let winner = if x.hash() < y.hash() { &x } else { &y };
    assert_eq!(p.entries.last(), Some(winner));
    // The machine that held the loser first replaced it; the other refused the loser.
    let results = [first_p, first_q];
    assert!(results.iter().any(|r| matches!(r, Ok(Taken::Replaced(_)))));
    assert!(results.iter().any(|r| matches!(r, Err(MemberError::ForkLost))));
    // The winning branch goes on from either machine.
    let next = p.next(&ka, C, &a.id, Change::SetPolicy { policy: Policy::AutoProven }).unwrap();
    assert_eq!(q.take(C, next), Ok(Taken::Extended));
}

#[test]
fn a_chain_received_whole_is_checked_entry_by_entry() {
    let (mut chain, ka, a) = founded();
    let (_, b) = machine(2);
    add(&mut chain, &ka, &a, &b);
    assert!(Chain::from_entries(C, chain.entries.clone()).is_ok());
    let mut broken = chain.entries.clone();
    broken[1].sig = broken[0].sig.clone();
    assert_eq!(Chain::from_entries(C, broken), Err(MemberError::BadSignature));
    let state: State = Chain::from_entries(C, chain.entries.clone()).unwrap().state(C);
    assert_eq!(state.n, 2);
}
