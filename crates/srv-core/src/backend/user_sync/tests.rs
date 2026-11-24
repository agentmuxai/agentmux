// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The layer against in-memory stores of fake machines: convergence, a partition and a rejoin, replay, a forged
// origin, clock skew, a removed member, a skipped sequence, rollback, tombstones, and what is in the clear.

use agentmux_common::jekt_sign::generate_lan_keypair;
use rusqlite::Connection;

use super::adapter::{Adapter, Merge, NewestWins, Side};
use super::hlc::{Clock, Version, MAX_SKEW_MS};
use super::log::{self, Change, Head, Outcome, Pins, Refused, Signer, SyncError, Vector};
use super::record::{BadRecord, Header, Record, CLEAR_FIELDS};

const NOW: i64 = 1_800_000_000_000;
const COLLECTION: &str = "c1";

struct Machine {
    id: String,
    conn: Connection,
    public: [u8; 32],
    private: [u8; 32],
    clock: Clock,
}

fn machine(name: &str, seed: u8) -> Machine {
    let (public, private) = generate_lan_keypair([seed; 32]);
    let conn = Connection::open_in_memory().unwrap();
    log::init(&conn).unwrap();
    log::ensure_collection(&conn, COLLECTION, "kanban", NOW).unwrap();
    Machine { id: name.to_string(), conn, public, private, clock: Clock::new(name) }
}

fn pins(machines: &[&Machine]) -> Pins {
    machines.iter().map(|m| (m.id.clone(), m.public)).collect()
}

impl Machine {
    fn write(&mut self, record: &str, body: &str, at: i64) -> Record {
        let mut signer = Signer { machine: &self.id, private: &self.private, clock: &mut self.clock };
        log::append_local(&self.conn, &mut signer, COLLECTION, record, body.as_bytes().to_vec(), false, at).unwrap()
    }

    fn delete(&mut self, record: &str, at: i64) -> Record {
        let mut signer = Signer { machine: &self.id, private: &self.private, clock: &mut self.clock };
        log::append_local(&self.conn, &mut signer, COLLECTION, record, Vec::new(), true, at).unwrap()
    }

    fn state(&self, record: &str) -> Option<String> {
        let r = log::current(&self.conn, COLLECTION, record).unwrap()?;
        (!r.header.tombstone).then(|| String::from_utf8(r.body).unwrap())
    }

    /// Takes everything `from` holds that this machine lacks.
    fn pull(&mut self, from: &Machine, all: &Pins, at: i64) -> Vec<Result<Outcome, SyncError>> {
        let mine = log::vector(&self.conn, COLLECTION).unwrap();
        let records = log::since(&from.conn, COLLECTION, &mine, 1000).unwrap();
        records.iter().map(|r| log::apply_remote(&self.conn, r, all, &mut self.clock, at, &NewestWins)).collect()
    }
}

fn sync_all(machines: &mut [Machine], at: i64) {
    let all: Pins = machines.iter().map(|m| (m.id.clone(), m.public)).collect();
    for _ in 0..2 {
        for i in 0..machines.len() {
            for j in 0..machines.len() {
                if i != j {
                    let (a, b) = if i < j {
                        let (l, r) = machines.split_at_mut(j);
                        (&mut l[i], &r[0])
                    } else {
                        let (l, r) = machines.split_at_mut(i);
                        (&mut r[0], &l[j])
                    };
                    for outcome in a.pull(b, &all, at) {
                        outcome.unwrap();
                    }
                }
            }
        }
    }
}

#[test]
fn three_machines_converge_and_the_newest_edit_wins() {
    let mut ms = vec![machine("a", 1), machine("b", 2), machine("c", 3)];
    ms[0].write("card", "first", NOW);
    sync_all(&mut ms, NOW + 1);
    ms[1].write("card", "second", NOW + 10);
    ms[2].write("card", "third", NOW + 20);
    ms[0].write("other", "x", NOW + 5);
    sync_all(&mut ms, NOW + 30);
    for m in &ms {
        assert_eq!(m.state("card").as_deref(), Some("third"));
        assert_eq!(m.state("other").as_deref(), Some("x"));
    }
    // Every machine holds the same log.
    let v0 = log::vector(&ms[0].conn, COLLECTION).unwrap();
    assert!(ms.iter().all(|m| log::vector(&m.conn, COLLECTION).unwrap() == v0));
}

#[test]
fn a_partition_heals_and_both_sides_keep_their_history() {
    let mut ms = vec![machine("a", 1), machine("b", 2)];
    ms[0].write("card", "base", NOW);
    sync_all(&mut ms, NOW + 1);
    ms[0].write("card", "from a", NOW + 100);
    ms[1].write("card", "from b", NOW + 200);
    sync_all(&mut ms, NOW + 300);
    assert!(ms.iter().all(|m| m.state("card").as_deref() == Some("from b")));
    let history = log::history(&ms[0].conn, COLLECTION, 10).unwrap();
    assert_eq!(history.len(), 3, "both edits stay in the history");
}

#[test]
fn a_record_applied_twice_changes_nothing() {
    let mut a = machine("a", 1);
    let b = machine("b", 2);
    let rec = a.write("card", "v1", NOW);
    let all = pins(&[&a, &b]);
    let mut clock = Clock::new("b");
    assert_eq!(log::apply_remote(&b.conn, &rec, &all, &mut clock, NOW, &NewestWins).unwrap(), Outcome::Applied);
    assert_eq!(log::apply_remote(&b.conn, &rec, &all, &mut clock, NOW, &NewestWins).unwrap(), Outcome::Duplicate);
    assert_eq!(b.state("card").as_deref(), Some("v1"));
}

fn refused(r: Result<Outcome, SyncError>) -> Refused {
    match r {
        Err(SyncError::Refused(x)) => x,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn a_record_signed_by_another_member_is_refused_as_not_from_its_origin() {
    let mut a = machine("a", 1);
    let c = machine("c", 3);
    let b = machine("b", 2);
    let real = a.write("card", "v1", NOW);
    // C signs a record that says it comes from A.
    let forged = Record::sign(&c.private, Header { origin: "a".into(), version: Version { machine: "a".into(), ..real.header.version.clone() }, ..real.header.clone() }, real.body.clone()).unwrap();
    let all = pins(&[&a, &b, &c]);
    let mut clock = Clock::new("b");
    assert_eq!(refused(log::apply_remote(&b.conn, &forged, &all, &mut clock, NOW, &NewestWins)), Refused::Bad(BadRecord::Signature));
    // A header edited after signing fails too.
    let mut edited = real.clone();
    edited.header.record = "other".into();
    assert_eq!(refused(log::apply_remote(&b.conn, &edited, &all, &mut clock, NOW, &NewestWins)), Refused::Bad(BadRecord::Signature));
    // So does a body swapped under the same header.
    let mut swapped = real;
    swapped.body = b"evil".to_vec();
    assert_eq!(refused(log::apply_remote(&b.conn, &swapped, &all, &mut clock, NOW, &NewestWins)), Refused::Bad(BadRecord::Malformed));
}

#[test]
fn a_version_that_names_another_machine_is_refused() {
    let a = machine("a", 1);
    let header = Header {
        collection: COLLECTION.into(),
        record: "card".into(),
        version: Version { wall_ms: NOW, counter: 0, machine: "b".into() },
        base: None,
        origin: "a".into(),
        seq: 1,
        epoch: 0,
        tombstone: false,
        size: 1,
    };
    assert!(Record::sign(&a.private, header, b"x".to_vec()).is_none());
}

#[test]
fn an_unknown_origin_and_an_unknown_collection_are_refused() {
    let mut a = machine("a", 1);
    let b = machine("b", 2);
    let rec = a.write("card", "v1", NOW);
    let mut clock = Clock::new("b");
    assert_eq!(refused(log::apply_remote(&b.conn, &rec, &Pins::new(), &mut clock, NOW, &NewestWins)), Refused::UnknownOrigin);
    let other = Connection::open_in_memory().unwrap();
    log::init(&other).unwrap();
    assert_eq!(refused(log::apply_remote(&other, &rec, &pins(&[&a]), &mut clock, NOW, &NewestWins)), Refused::UnknownCollection);
}

#[test]
fn a_version_more_than_five_minutes_ahead_is_refused() {
    let mut a = machine("a", 1);
    let b = machine("b", 2);
    let all = pins(&[&a, &b]);
    let mut clock = Clock::new("b");
    let ahead = a.write("card", "v1", NOW + MAX_SKEW_MS + 1000);
    assert!(matches!(refused(log::apply_remote(&b.conn, &ahead, &all, &mut clock, NOW, &NewestWins)), Refused::Skew(_)));
    let near = a.write("card", "v2", NOW + MAX_SKEW_MS - 1000);
    // The first record was refused, so the second is the next after a gap of one.
    assert_eq!(refused(log::apply_remote(&b.conn, &near, &all, &mut clock, NOW, &NewestWins)), Refused::Gap { have: 0 });
}

#[test]
fn a_skipped_sequence_asks_for_a_resend() {
    let mut a = machine("a", 1);
    let b = machine("b", 2);
    a.write("card", "v1", NOW);
    let second = a.write("card", "v2", NOW + 1);
    let all = pins(&[&a, &b]);
    let mut clock = Clock::new("b");
    assert_eq!(refused(log::apply_remote(&b.conn, &second, &all, &mut clock, NOW + 2, &NewestWins)), Refused::Gap { have: 0 });
}

#[test]
fn a_removed_member_writing_at_the_old_epoch_is_refused() {
    let mut a = machine("a", 1);
    let b = machine("b", 2);
    let old = a.write("card", "before", NOW);
    log::set_epoch(&b.conn, COLLECTION, 1, &Vector::new()).unwrap();
    let all = pins(&[&a, &b]);
    let mut clock = Clock::new("b");
    assert_eq!(refused(log::apply_remote(&b.conn, &old, &all, &mut clock, NOW, &NewestWins)), Refused::Epoch { have: 1, got: 0 });
}

#[test]
fn an_older_head_than_the_one_accepted_is_refused() {
    let b = machine("b", 2);
    let head = |g: u64, a: u64| Head { generation: g, vector: [("a".to_string(), a)].into() };
    log::accept_head(&b.conn, COLLECTION, &head(1, 5)).unwrap();
    assert!(matches!(log::accept_head(&b.conn, COLLECTION, &head(0, 9)), Err(SyncError::Refused(Refused::Rollback))));
    assert!(matches!(log::accept_head(&b.conn, COLLECTION, &head(1, 4)), Err(SyncError::Refused(Refused::Rollback))));
    log::accept_head(&b.conn, COLLECTION, &head(1, 5)).unwrap();
    log::accept_head(&b.conn, COLLECTION, &head(2, 0)).unwrap();
    assert_eq!(log::mark(&b.conn, COLLECTION).unwrap().generation, 2);
}

#[test]
fn a_record_shows_only_the_fixed_fields_in_the_clear() {
    let mut a = machine("a", 1);
    let rec = a.write("card", "a secret title", NOW);
    let fields = rec.clear_fields();
    let mut keys: Vec<&str> = fields.keys().map(String::as_str).collect();
    keys.sort_unstable();
    let mut want = CLEAR_FIELDS.to_vec();
    want.sort_unstable();
    assert_eq!(keys, want);
    assert!(!serde_json::Value::Object(fields).to_string().contains("secret"), "the body is not in the clear fields");
}

#[test]
fn a_deletion_is_a_tombstone_with_no_body_that_expires_after_the_horizon() {
    let mut a = machine("a", 1);
    a.write("card", "title", NOW);
    let t = a.delete("card", NOW + 1000);
    assert!(t.header.tombstone && t.body.is_empty());
    assert_eq!(a.state("card"), None);
    assert!(log::expired_tombstones(&a.conn, COLLECTION, NOW + 1000).unwrap().is_empty());
    let after = NOW + 31 * 86_400_000;
    assert_eq!(log::expired_tombstones(&a.conn, COLLECTION, after).unwrap(), vec!["card".to_string()]);
    assert!(log::needs_full_resync(NOW, after, log::HORIZON_BOARD_DAYS));
    assert!(!log::needs_full_resync(NOW, NOW + 86_400_000, log::HORIZON_BOARD_DAYS));
}

#[test]
fn the_history_names_who_and_when_and_never_a_value() {
    let mut ms = vec![machine("a", 1), machine("b", 2)];
    ms[0].write("card", "title one", NOW);
    sync_all(&mut ms, NOW + 1);
    ms[1].write("card", "title two", NOW + 10);
    ms[1].delete("card", NOW + 20);
    let h = log::history(&ms[1].conn, COLLECTION, 10).unwrap();
    let changes: Vec<(&str, Change)> = h.iter().map(|e| (e.machine.as_str(), e.change)).collect();
    assert_eq!(changes, vec![("b", Change::Deleted), ("b", Change::Updated), ("a", Change::Created)]);
}

#[test]
fn a_restarted_machine_still_edits_after_what_it_holds() {
    let mut ms = vec![machine("a", 1), machine("b", 2)];
    ms[1].write("card", "from b, ahead", NOW + 200_000);
    sync_all(&mut ms, NOW + 1);
    // a restarts: a fresh clock, and a wall clock that is behind the version b made.
    ms[0].clock = Clock::new("a");
    let edit = ms[0].write("card", "from a, after restart", NOW + 2);
    let held = log::current(&ms[1].conn, COLLECTION, "card").unwrap().unwrap().header.version;
    assert!(edit.header.version > held, "the edit is after the version it replaces");
    assert_eq!(ms[0].state("card").as_deref(), Some("from a, after restart"));
    sync_all(&mut ms, NOW + 3);
    assert_eq!(ms[1].state("card").as_deref(), Some("from a, after restart"), "the peer takes the edit too");
}

#[test]
fn a_record_made_again_after_a_delete_is_created_not_updated() {
    let mut a = machine("a", 1);
    a.write("card", "one", NOW);
    a.delete("card", NOW + 10);
    a.write("card", "two", NOW + 20);
    a.write("card", "three", NOW + 30);
    let h = log::history(&a.conn, COLLECTION, 10).unwrap();
    let changes: Vec<Change> = h.iter().map(|e| e.change).collect();
    assert_eq!(changes, vec![Change::Updated, Change::Created, Change::Deleted, Change::Created]);
}

/// Keeps both sides of an edit made apart, with the newer one current; an edit made on top of what is held applies.
struct KeepBoth;

impl Adapter for KeepBoth {
    fn merge(&self, local: Option<&Header>, incoming: &Header) -> Merge {
        match local {
            None => Merge::Applied,
            Some(l) if incoming.base.as_ref() == Some(&l.version) => Merge::Applied,
            Some(l) if l.version >= incoming.version => Merge::Conflict { current: Side::Local },
            Some(_) => Merge::Conflict { current: Side::Incoming },
        }
    }
}

/// What `to` takes from `from`, under KeepBoth, and what happened to each record.
fn take_all(from: &Machine, to: &mut Machine, all: &Pins, at: i64) -> Vec<Outcome> {
    let mine = log::vector(&to.conn, COLLECTION).unwrap();
    let records = log::since(&from.conn, COLLECTION, &mine, 100).unwrap();
    records.iter().map(|r| log::apply_remote(&to.conn, r, all, &mut to.clock, at, &KeepBoth).unwrap()).collect()
}

#[test]
fn a_record_names_the_version_it_replaced_and_the_signature_covers_it() {
    let mut a = machine("a", 1);
    let b = machine("b", 2);
    let first = a.write("card", "one", NOW);
    let second = a.write("card", "two", NOW + 1);
    assert_eq!(first.header.base, None);
    assert_eq!(second.header.base, Some(first.header.version.clone()));
    let all = pins(&[&a, &b]);
    let mut clock = Clock::new("b");
    log::apply_remote(&b.conn, &first, &all, &mut clock, NOW, &NewestWins).unwrap();
    let mut edited = second.clone();
    edited.header.base = None;
    assert_eq!(refused(log::apply_remote(&b.conn, &edited, &all, &mut clock, NOW + 2, &NewestWins)), Refused::Bad(BadRecord::Signature));
    assert_eq!(log::current(&a.conn, COLLECTION, "card").unwrap().unwrap().header.base, second.header.base, "stored and read back");
}

#[test]
fn a_follow_on_edit_applies_and_an_edit_made_apart_keeps_both_with_one_current_everywhere() {
    let mut ms = vec![machine("a", 1), machine("b", 2)];
    let all: Pins = ms.iter().map(|m| (m.id.clone(), m.public)).collect();
    ms[0].write("card", "base", NOW);
    let (first, rest) = ms.split_at_mut(1);
    let outcomes = take_all(&first[0], &mut rest[0], &all, NOW + 1);
    assert_eq!(outcomes, vec![Outcome::Applied], "an edit made on top of what is held applies");
    ms[0].write("card", "from a", NOW + 100);
    ms[1].write("card", "from b", NOW + 200);
    let (first, rest) = ms.split_at_mut(1);
    let mut seen = take_all(&first[0], &mut rest[0], &all, NOW + 300);
    seen.extend(take_all(&rest[0], &mut first[0], &all, NOW + 300));
    let conflicts = seen.iter().filter(|o| **o == Outcome::Conflict).count();
    assert_eq!(conflicts, 2, "each machine saw the other's edit as made apart");
    assert!(ms.iter().all(|m| m.state("card").as_deref() == Some("from b")), "the later version is current on both");
    assert_eq!(log::history(&ms[0].conn, COLLECTION, 10).unwrap().len(), 3, "the other side stays in the log");
}

#[test]
fn a_member_that_missed_the_last_record_of_an_epoch_still_catches_up_across_the_change() {
    let mut a = machine("a", 1);
    let b = machine("b", 2);
    let all = pins(&[&a, &b]);
    let old = a.write("card", "before", NOW);
    // Both move to epoch 1; the membership change says a had written 1 record at epoch 0.
    let ended: Vector = [("a".to_string(), 1)].into();
    log::set_epoch(&a.conn, COLLECTION, 1, &ended).unwrap();
    log::set_epoch(&b.conn, COLLECTION, 1, &ended).unwrap();
    let new = a.write("card", "after", NOW + 10);
    assert_eq!((old.header.epoch, new.header.epoch), (0, 1));
    let mut clock = Clock::new("b");
    assert_eq!(log::apply_remote(&b.conn, &old, &all, &mut clock, NOW + 20, &NewestWins).unwrap(), Outcome::Applied);
    assert_eq!(log::apply_remote(&b.conn, &new, &all, &mut clock, NOW + 20, &NewestWins).unwrap(), Outcome::Applied);
    assert_eq!(b.state("card").as_deref(), Some("after"));
    // A record past the end of its origin's old epoch is kept as void (the origin's sequence stays unbroken) and
    // changes nothing; one at an epoch not yet held is refused.
    let signed = |seq: u64, epoch: u32| {
        let header = Header { seq, epoch, ..new.header.clone() };
        Record::sign(&a.private, header, new.body.clone()).unwrap()
    };
    assert_eq!(refused(log::apply_remote(&b.conn, &signed(3, 2), &all, &mut clock, NOW + 20, &NewestWins)), Refused::Epoch { have: 1, got: 2 });
    assert_eq!(log::apply_remote(&b.conn, &signed(3, 0), &all, &mut clock, NOW + 20, &NewestWins).unwrap(), Outcome::Void);
    assert_eq!(b.state("card").as_deref(), Some("after"));
    assert_eq!(log::vector(&b.conn, COLLECTION).unwrap().get("a"), Some(&3));
}

#[test]
fn a_void_record_dated_far_ahead_is_refused_and_does_not_move_this_machines_clock() {
    let mut a = machine("a", 1);
    let b = machine("b", 2);
    let all = pins(&[&a, &b]);
    a.write("card", "before", NOW);
    let ended: Vector = [("a".to_string(), 1)].into();
    log::set_epoch(&b.conn, COLLECTION, 1, &ended).unwrap();
    let mut clock = Clock::new("b");
    let first = log::since(&a.conn, COLLECTION, &Vector::new(), 10).unwrap();
    assert_eq!(log::apply_remote(&b.conn, &first[0], &all, &mut clock, NOW, &NewestWins).unwrap(), Outcome::Applied);
    // A write of a at the old epoch, past its end, dated a day ahead: void, but its date is not taken in.
    let far = Version { wall_ms: NOW + 24 * 60 * 60 * 1000, counter: 0, machine: "a".into() };
    let header = Header { seq: 2, epoch: 0, version: far, ..first[0].header.clone() };
    let late = Record::sign(&a.private, header, first[0].body.clone()).unwrap();
    assert!(matches!(refused(log::apply_remote(&b.conn, &late, &all, &mut clock, NOW, &NewestWins)), Refused::Skew(_)));
    assert_eq!(log::vector(&b.conn, COLLECTION).unwrap().get("a"), Some(&1), "not stored");
    let mut b = b;
    let mine = b.write("card", "mine", NOW + 1);
    assert!(mine.header.version.wall_ms < NOW + 60_000, "this machine's next write is not dated a day ahead: {:?}", mine.header.version);
}

#[test]
fn a_write_stored_as_void_under_a_removal_that_loses_its_fork_becomes_the_records_state() {
    let mut a = machine("a", 1);
    let b = machine("b", 2);
    let all = pins(&[&a, &b]);
    a.write("card", "before", NOW);
    a.write("card", "late", NOW + 10);
    // b learns of a removal that says a had written 1 record at epoch 0: a's second write is void on b.
    let ended: Vector = [("a".to_string(), 1)].into();
    log::set_epoch(&b.conn, COLLECTION, 1, &ended).unwrap();
    let mut b = b;
    let outcomes = b.pull(&a, &all, NOW + 20);
    assert_eq!(outcomes.into_iter().map(Result::unwrap).collect::<Vec<_>>(), vec![Outcome::Applied, Outcome::Void]);
    assert_eq!(b.state("card").as_deref(), Some("before"));
    // The removal then loses a fork: no epoch and no end are left, so the write was within the epoch all along.
    log::reset_epochs(&b.conn, COLLECTION, 0, &[]).unwrap();
    log::recheck(&b.conn, COLLECTION, "b", &NewestWins).unwrap();
    assert_eq!(b.state("card").as_deref(), Some("late"), "the write is applied now that it is no longer void");
    assert_eq!(log::recheck(&b.conn, COLLECTION, "b", &NewestWins).unwrap(), 0, "nothing is left to settle");
}

#[test]
fn a_record_that_a_removal_newly_makes_void_stops_being_the_state_of_its_record() {
    let mut a = machine("a", 1);
    let b = machine("b", 2);
    let all = pins(&[&a, &b]);
    a.write("card", "before", NOW);
    a.write("card", "late", NOW + 10);
    let mut b = b;
    b.pull(&a, &all, NOW + 20);
    assert_eq!(b.state("card").as_deref(), Some("late"));
    // A removal now says a had written only 1 record at epoch 0: the second write was never within the epoch.
    let ended: Vector = [("a".to_string(), 1)].into();
    log::reset_epochs(&b.conn, COLLECTION, 1, &[(0, &ended)]).unwrap();
    log::recheck(&b.conn, COLLECTION, "b", &NewestWins).unwrap();
    assert_eq!(b.state("card").as_deref(), Some("before"), "what the others stopped at is the state again");
    // Its own writes past an end are kept as they are: they are written again, not undone.
    let mut own = machine("b", 3);
    own.write("card", "mine, one", NOW);
    own.write("card", "mine, two", NOW + 10);
    let cut: Vector = [("b".to_string(), 1)].into();
    log::reset_epochs(&own.conn, COLLECTION, 1, &[(0, &cut)]).unwrap();
    log::recheck(&own.conn, COLLECTION, "b", &NewestWins).unwrap();
    assert_eq!(own.state("card").as_deref(), Some("mine, two"));
}

#[test]
fn the_counter_carries_into_the_wall_time_instead_of_overflowing() {
    let mut clock = Clock::new("a");
    let seen = Version { wall_ms: NOW, counter: u32::MAX, machine: "b".into() };
    clock.observe(&seen, NOW).unwrap();
    let next = clock.tick(NOW - 5);
    assert!(next > seen, "the next version is after the one seen");
    assert_eq!(next.wall_ms, NOW + 1);
    assert_eq!(next.counter, 0);
}

#[test]
fn a_deletion_and_a_concurrent_replacement_at_the_same_instant_are_labelled_in_version_order() {
    let mut ms = vec![machine("a", 1), machine("b", 2)];
    ms[0].write("card", "one", NOW);
    sync_all(&mut ms, NOW + 1);
    ms[0].delete("card", NOW + 10);
    ms[1].write("card", "two", NOW + 10);
    sync_all(&mut ms, NOW + 20);
    let h = log::history(&ms[0].conn, COLLECTION, 10).unwrap();
    let changes: Vec<(&str, Change)> = h.iter().map(|e| (e.machine.as_str(), e.change)).collect();
    assert_eq!(changes, vec![("b", Change::Created), ("a", Change::Deleted), ("a", Change::Created)]);
}

#[test]
fn a_catch_up_brings_each_record_after_the_ones_it_replaced_and_a_cut_keeps_a_prefix_of_each_origin() {
    let mut ms = vec![machine("a", 1), machine("z", 2)];
    ms[1].write("card", "from z", NOW);
    sync_all(&mut ms, NOW + 1);
    ms[0].delete("card", NOW + 10);
    ms[1].write("other", "later z", NOW + 20);
    // Alphabetically "a" comes first, but z's earlier version must arrive before a's deletion.
    let all = log::since(&ms[0].conn, COLLECTION, &Vector::new(), 100).unwrap();
    assert!(all.windows(2).all(|w| w[0].header.version < w[1].header.version), "oldest version first");
    assert_eq!(all.first().map(|r| r.header.origin.as_str()), Some("z"));
    // A cut at two records keeps the two oldest, which are a prefix of each origin's sequence.
    let cut = log::since(&ms[0].conn, COLLECTION, &Vector::new(), 2).unwrap();
    let seqs: Vec<(&str, u64)> = cut.iter().map(|r| (r.header.origin.as_str(), r.header.seq)).collect();
    assert_eq!(seqs, vec![("z", 1), ("a", 1)]);
}

#[test]
fn a_record_over_the_size_cap_cannot_be_made() {
    let a = machine("a", 1);
    let header = Header {
        collection: COLLECTION.into(),
        record: "card".into(),
        version: Version { wall_ms: NOW, counter: 0, machine: "a".into() },
        base: None,
        origin: "a".into(),
        seq: 1,
        epoch: 0,
        tombstone: false,
        size: super::record::MAX_RECORD_BYTES + 1,
    };
    assert!(Record::sign(&a.private, header, vec![0; 8]).is_none());
}
