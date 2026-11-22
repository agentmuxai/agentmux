// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The layer against in-memory stores of fake machines: convergence, a partition and a rejoin, replay, a forged
// origin, clock skew, a removed member, a skipped sequence, rollback, tombstones, and what is in the clear.

use agentmux_common::jekt_sign::generate_lan_keypair;
use rusqlite::Connection;

use super::adapter::NewestWins;
use super::hlc::{Clock, Version, MAX_SKEW_MS};
use super::log::{self, Change, Head, Outcome, Pins, Refused, Signer, SyncError};
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
    log::set_epoch(&b.conn, COLLECTION, 1).unwrap();
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
fn a_record_over_the_size_cap_cannot_be_made() {
    let a = machine("a", 1);
    let header = Header {
        collection: COLLECTION.into(),
        record: "card".into(),
        version: Version { wall_ms: NOW, counter: 0, machine: "a".into() },
        origin: "a".into(),
        seq: 1,
        epoch: 0,
        tombstone: false,
        size: super::record::MAX_RECORD_BYTES + 1,
    };
    assert!(Record::sign(&a.private, header, vec![0; 8]).is_none());
}
