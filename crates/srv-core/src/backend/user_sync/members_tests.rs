// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The membership chain: a change that no member signed is refused, an entry that does not extend the chain is
// refused, a removed member can sign nothing, a recovery subject never signs or anchors, and a fork settles by hash
// to the same head on every machine.

use std::collections::BTreeMap;

use super::members::{Change, Chain, Entry, Kind, MemberError, Policy, State, Subject, Taken, MAX_SIDE};

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
    let state = chain.state(C).unwrap();
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
    assert_eq!(chain.state(C).unwrap().members.len(), 2, "nothing refused was taken");
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
    let state = chain.state(C).unwrap();
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
    assert_eq!(chain.state(C).unwrap().epoch, 2);
    let last = chain.next(&ka, C, &a.id, Change::Remove { id: a.id.clone(), ended: BTreeMap::new() }).unwrap();
    assert_eq!(chain.take(C, last), Err(MemberError::BadRemoval));
    let stranger = chain.next(&ka, C, &a.id, Change::Remove { id: "nobody".into(), ended: BTreeMap::new() }).unwrap();
    assert_eq!(chain.take(C, stranger), Err(MemberError::BadRemoval));
}

#[test]
fn how_machines_join_changes_only_by_a_signed_entry() {
    let (mut chain, ka, a) = founded();
    assert_eq!(chain.state(C).unwrap().policy, Policy::Approve);
    let (kc, c) = machine(3);
    let by_stranger = Entry::sign(&kc, C, 2, chain.entries.last().map(Entry::hash), Change::SetPolicy { policy: Policy::AutoProven }, &c.id).unwrap();
    assert_eq!(chain.take(C, by_stranger), Err(MemberError::NotAMember));
    assert_eq!(chain.state(C).unwrap().policy, Policy::Approve, "a peer cannot quietly relax approve");
    let signed = chain.next(&ka, C, &a.id, Change::SetPolicy { policy: Policy::AutoProven }).unwrap();
    assert_eq!(chain.take(C, signed), Ok(Taken::Extended));
    assert_eq!(chain.state(C).unwrap().policy, Policy::AutoProven);
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
    let state: State = Chain::from_entries(C, chain.entries.clone()).unwrap().state(C).unwrap();
    assert_eq!(state.n, 2);
}

/// A chain founded by machine 1 with machines 2 and 3 added: (chain, keys, subjects).
fn three() -> (Chain, [[u8; 32]; 3], [Subject; 3]) {
    let (mut chain, ka, a) = founded();
    let (kb, b) = machine(2);
    let (kc, c) = machine(3);
    add(&mut chain, &ka, &a, &b);
    add(&mut chain, &ka, &a, &c);
    (chain, [ka, kb, kc], [a, b, c])
}

fn ended(pairs: &[(&Subject, u64)]) -> BTreeMap<String, u64> {
    pairs.iter().map(|(s, n)| (s.id.clone(), *n)).collect()
}

/// An `Add` of a fresh machine signed by `by`, with its label varied until the entry's hash is below `below`.
fn grind_add(chain: &Chain, by: &[u8; 32], signer: &Subject, below: &str) -> Entry {
    for i in 0..500 {
        let (_, d) = machine(40);
        let d = Subject { sealed_label: format!("label-{i}"), ..d };
        let entry = chain.next(by, C, &signer.id, Change::Add { member: d }).unwrap();
        if entry.hash().as_str() < below {
            return entry;
        }
    }
    panic!("no lower hash found");
}

#[test]
fn a_member_cannot_undo_its_own_removal_by_racing_it() {
    let (chain, [ka, kb, _], [a, b, c]) = three();
    let removal = chain.next(&ka, C, &a.id, Change::Remove { id: b.id.clone(), ended: ended(&[(&a, 4), (&c, 2)]) }).unwrap();
    // b signs a competing entry at the same position and varies it until its hash is the lower one.
    let rival = grind_add(&chain, &kb, &b, &removal.hash());
    assert!(rival.hash() < removal.hash());
    let (mut p, mut q) = (chain.clone(), chain.clone());
    assert_eq!(p.take(C, removal.clone()), Ok(Taken::Extended));
    assert_eq!(p.take(C, rival.clone()), Err(MemberError::ForkLost));
    assert_eq!(q.take(C, rival), Ok(Taken::Extended));
    assert!(matches!(q.take(C, removal.clone()), Ok(Taken::Replaced(_))));
    for chain in [&p, &q] {
        let state = chain.state(C).unwrap();
        assert_eq!((chain.entries.last(), state.epoch, state.members.contains_key(&b.id)), (Some(&removal), 1, false));
    }
}

#[test]
fn two_members_removing_each_other_at_once_are_settled_by_who_joined_first_whatever_the_hashes() {
    let (chain, [ka, kb, _], [a, b, c]) = three();
    let by_a = chain.next(&ka, C, &a.id, Change::Remove { id: b.id.clone(), ended: ended(&[(&a, 1), (&c, 1)]) }).unwrap();
    // b's removal of a is varied (the sequence numbers it names) until its hash is lower than a's.
    let by_b = (0..500)
        .map(|i| chain.next(&kb, C, &b.id, Change::Remove { id: a.id.clone(), ended: ended(&[(&b, i), (&c, 1)]) }).unwrap())
        .find(|e| e.hash() < by_a.hash())
        .unwrap();
    let (mut p, mut q) = (chain.clone(), chain.clone());
    let _ = (p.take(C, by_a.clone()), p.take(C, by_b.clone()));
    let _ = (q.take(C, by_b), q.take(C, by_a.clone()));
    assert_eq!((p.entries.last(), q.entries.last()), (Some(&by_a), Some(&by_a)), "the founder's removal stands on both");
}

#[test]
fn a_cycle_of_removals_at_one_position_settles_the_same_way_in_every_order() {
    let (chain, [ka, kb, kc], [a, b, c]) = three();
    let ra = chain.next(&ka, C, &a.id, Change::Remove { id: b.id.clone(), ended: ended(&[(&a, 1), (&c, 1)]) }).unwrap();
    let rb = chain.next(&kb, C, &b.id, Change::Remove { id: c.id.clone(), ended: ended(&[(&a, 1), (&b, 1)]) }).unwrap();
    let rc = chain.next(&kc, C, &c.id, Change::Remove { id: a.id.clone(), ended: ended(&[(&b, 1), (&c, 1)]) }).unwrap();
    let orders = [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]];
    let entries = [ra, rb, rc];
    let heads: Vec<Option<Entry>> = orders
        .iter()
        .map(|order| {
            let mut p = chain.clone();
            for i in order {
                let _ = p.take(C, entries[*i].clone());
            }
            p.entries.last().cloned()
        })
        .collect();
    assert!(heads.iter().all(|h| *h == heads[0]), "every order ends at the same head");
    assert_eq!(heads[0].as_ref().map(|e| e.signer.clone()), Some(a.id.clone()), "the earliest member's entry stands");
}

#[test]
fn the_founding_entry_is_never_replaced() {
    let (mut chain, _, a) = founded();
    let held = chain.entries[0].clone();
    // Anyone can sign a founding entry naming themselves, so a second one is a different collection, however low its hash.
    for seed in 50..80 {
        let (private, s) = machine(seed);
        let rival = Entry::sign(&private, C, 1, None, Change::Found { member: s.clone(), policy: Policy::AutoProven }, &s.id).unwrap();
        assert_eq!(chain.take(C, rival), Err(MemberError::ForkLost));
    }
    assert_eq!(chain.entries, vec![held]);
    assert!(chain.state(C).unwrap().members.contains_key(&a.id));
}

#[test]
fn a_removal_names_every_machine_that_remains_and_only_those() {
    let (chain, [ka, _, _], [a, b, c]) = three();
    let remove = |ended: BTreeMap<String, u64>| chain.next(&ka, C, &a.id, Change::Remove { id: b.id.clone(), ended }).unwrap();
    let mut p = chain.clone();
    assert_eq!(p.take(C, remove(BTreeMap::new())), Err(MemberError::BadRemoval), "empty");
    assert_eq!(p.take(C, remove(ended(&[(&a, 1)]))), Err(MemberError::BadRemoval), "one left out");
    assert_eq!(p.take(C, remove(ended(&[(&a, 1), (&c, 1), (&b, 1)]))), Err(MemberError::BadRemoval), "the removed named");
    let stranger: BTreeMap<String, u64> = [(a.id.clone(), 1), (c.id.clone(), 1), ("nobody".to_string(), 1)].into();
    assert_eq!(p.take(C, remove(stranger)), Err(MemberError::BadRemoval), "a non-member named");
    assert_eq!(p.take(C, remove(ended(&[(&a, 1), (&c, 1)]))), Ok(Taken::Extended));
}

#[test]
fn a_recovery_subject_has_the_id_its_box_key_makes() {
    let (mut chain, ka, a) = founded();
    let (_, b) = machine(2);
    let mut taking_a_machines_id = Subject::recovery(&[9; 32], "s");
    taking_a_machines_id.id = b.id.clone();
    let entry = chain.next(&ka, C, &a.id, Change::Add { member: taking_a_machines_id }).unwrap();
    assert_eq!(chain.take(C, entry), Err(MemberError::BadSubject));
    let mut arbitrary = Subject::recovery(&[9; 32], "s");
    arbitrary.id = "recovery-anything".into();
    let entry = chain.next(&ka, C, &a.id, Change::Add { member: arbitrary }).unwrap();
    assert_eq!(chain.take(C, entry), Err(MemberError::BadSubject));
}

#[test]
fn a_chain_that_was_altered_after_it_was_taken_in_reads_as_an_error_not_an_empty_collection() {
    let (mut chain, _, _) = founded();
    chain.entries[0].sig = "AAAA".into();
    assert_eq!(chain.state(C), Err(MemberError::BadSignature));
}


/// Two branches off a chain of three machines: the entries `[lo, lo2]` (the one with the lower hash, and one that goes on
/// from it) and `[hi, hi2]`, all signed by the founder at positions 4 and 5. The lower hash wins at position 4.
fn two_branches() -> (Chain, [Entry; 2], [Entry; 2], [[u8; 32]; 3], [Subject; 3]) {
    let (chain, keys, subjects) = three();
    let [ka, ..] = keys;
    let [a, ..] = &subjects;
    let hi = chain.next(&ka, C, &a.id, Change::SetPolicy { policy: Policy::Approve }).unwrap();
    let lo = grind_add(&chain, &ka, a, &hi.hash());
    let after = |first: &Entry, policy: Policy| {
        let mut c = chain.clone();
        assert_eq!(c.take(C, first.clone()), Ok(Taken::Extended));
        c.next(&ka, C, &a.id, Change::SetPolicy { policy }).unwrap()
    };
    let (lo2, hi2) = (after(&lo, Policy::AutoProven), after(&hi, Policy::AutoProven));
    (chain, [lo, lo2], [hi, hi2], keys, subjects)
}

/// Delivers `entries` in order, again and again until a pass takes nothing more (an entry whose predecessor has not
/// come yet is refused, and offered again by its sender).
fn deliver(chain: &mut Chain, entries: &[&Entry]) {
    for _ in 0..entries.len() + 1 {
        for e in entries {
            let _ = chain.take(C, (*e).clone());
        }
    }
}

#[test]
fn a_fork_further_back_than_the_head_settles_after_one_branch_was_extended() {
    let (base, [lo, lo2], [hi, hi2], ..) = two_branches();
    assert!(lo.hash() < hi.hash());
    // The machine that went on from `hi` is told of `lo`: `lo` wins at position 4, so `hi2` leaves the chain with `hi`.
    let mut p = base.clone();
    assert_eq!(p.take(C, hi.clone()), Ok(Taken::Extended));
    assert_eq!(p.take(C, hi2.clone()), Ok(Taken::Extended));
    assert_eq!(p.take(C, lo.clone()), Ok(Taken::Replaced(vec![hi.clone(), hi2.clone()])));
    assert_eq!(p.entries[3..], [lo.clone()]);
    assert_eq!(p.side().cloned().collect::<Vec<_>>(), vec![hi.clone(), hi2.clone()], "both are held");
    // The machine that went on from `lo` is told of `hi` and `hi2`: neither displaces anything.
    let mut q = base.clone();
    assert_eq!(q.take(C, lo.clone()), Ok(Taken::Extended));
    assert_eq!(q.take(C, lo2.clone()), Ok(Taken::Extended));
    assert_eq!(q.take(C, hi.clone()), Err(MemberError::ForkLost));
    assert_eq!(q.take(C, hi2.clone()), Err(MemberError::ForkLost), "on a branch that lost");
    assert_eq!(q.entries[3..], [lo.clone(), lo2.clone()]);
    // `p` goes on from `lo` as `q` did.
    assert_eq!(p.take(C, lo2.clone()), Ok(Taken::Extended));
    assert_eq!(p.entries, q.entries);
    assert_eq!(p, q, "the same entries are held on both, in the chain and beside it");
}

#[test]
fn a_branch_that_was_extended_wins_whole_when_its_first_entry_has_the_lower_hash() {
    let (base, [lo, lo2], [hi, hi2], ..) = two_branches();
    // The machine that held `hi` and `hi2` is told of `lo` and `lo2` and takes them over its own.
    let mut p = base.clone();
    deliver(&mut p, &[&hi, &hi2]);
    assert_eq!(p.take(C, lo.clone()), Ok(Taken::Replaced(vec![hi, hi2])));
    assert_eq!(p.take(C, lo2.clone()), Ok(Taken::Extended));
    assert_eq!(p.entries[3..], [lo, lo2]);
}

#[test]
fn a_fork_two_places_back_settles_the_same_in_every_order_of_delivery() {
    let (base, [lo, lo2], [hi, hi2], ..) = two_branches();
    let all = [&lo, &lo2, &hi, &hi2];
    let mut ends = Vec::new();
    // Every order of the four entries, each given again until nothing more is taken.
    let mut order = [0usize, 1, 2, 3];
    let mut orders = Vec::new();
    fn permute(k: usize, a: &mut [usize; 4], out: &mut Vec<[usize; 4]>) {
        if k == a.len() {
            out.push(*a);
            return;
        }
        for i in k..a.len() {
            a.swap(k, i);
            permute(k + 1, a, out);
            a.swap(k, i);
        }
    }
    permute(0, &mut order, &mut orders);
    assert_eq!(orders.len(), 24);
    for order in orders {
        let mut p = base.clone();
        deliver(&mut p, &order.map(|i| all[i]));
        ends.push(p);
    }
    assert!(ends.iter().all(|p| *p == ends[0]), "every order ends in the same chain and the same side entries");
    assert_eq!(ends[0].entries[3..], [lo, lo2], "the lower hash won, and its branch went on");
    assert_eq!(ends[0].side().count(), 2);
}

#[test]
fn a_removal_on_a_branch_that_lost_is_returned_with_what_followed_it() {
    let (chain, [ka, kb, _], [a, b, c]) = three();
    let ra = chain.next(&ka, C, &a.id, Change::Remove { id: c.id.clone(), ended: ended(&[(&a, 1), (&b, 1)]) }).unwrap();
    let rival = grind_add(&chain, &ka, &a, &ra.hash());
    // `b` went on from the removal with a second one (of itself, signed by `a`), then `rival` arrives.
    let mut p = chain.clone();
    assert_eq!(p.take(C, ra.clone()), Ok(Taken::Extended));
    let second = p.next(&ka, C, &a.id, Change::Remove { id: b.id.clone(), ended: ended(&[(&a, 1)]) }).unwrap();
    assert_eq!(p.take(C, second.clone()), Ok(Taken::Extended));
    assert_eq!(p.take(C, rival.clone()), Ok(Taken::Replaced(vec![ra, second])));
    assert_eq!(p.entries[3..], [rival]);
    let _ = kb;
}

#[test]
fn the_entries_held_beside_the_chain_are_kept_to_a_limit_and_read_back() {
    let (base, [ka, ..], [a, ..]) = three();
    let mut p = base.clone();
    for i in 0..(MAX_SIDE + 8) {
        let (_, d) = machine(60);
        let d = Subject { sealed_label: format!("rival-{i}"), ..d };
        let e = base.next(&ka, C, &a.id, Change::Add { member: d }).unwrap();
        let _ = p.take(C, e);
    }
    assert!(p.side().count() <= MAX_SIDE, "{}", p.side().count());
    let back = Chain::restore(C, p.entries.clone(), p.side().cloned().collect()).unwrap();
    assert_eq!(back, p, "what is held reads back as it was");
    // An entry that follows nothing held is not held.
    let stray = Entry::sign(&ka, C, 7, Some("ab".repeat(32)), Change::SetPolicy { policy: Policy::Approve }, &a.id).unwrap();
    assert_eq!(Chain::restore(C, p.entries.clone(), vec![stray]), Err(MemberError::DoesNotExtend));
}
