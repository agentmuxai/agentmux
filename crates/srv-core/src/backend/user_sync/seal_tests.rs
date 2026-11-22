// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Sealing a body for the place its header names: a peer opens what a member sealed, a store without the key cannot,
// a body does not open at any other place, and the closure is given exactly what the header will carry.

use std::cell::RefCell;

use agentmux_common::jekt_sign::generate_lan_keypair;
use rusqlite::Connection;

use super::adapter::NewestWins;
use super::hlc::Clock;
use super::log::{self, Outcome, Pins, SealAt, Signer, SyncError};
use super::record::Record;
use super::seal::{generate_key, open, sealer, KeyRing, OpenError};

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

fn ring_with_key() -> KeyRing {
    let mut ring = KeyRing::new();
    ring.insert(0, generate_key().unwrap());
    ring
}

impl Machine {
    fn write_sealed(&mut self, ring: &KeyRing, record: &str, plain: &[u8], at: i64) -> Record {
        let mut signer = Signer { machine: &self.id, private: &self.private, clock: &mut self.clock };
        log::append_local_with(&self.conn, &mut signer, COLLECTION, record, false, at, sealer(ring, plain)).unwrap()
    }
}

#[test]
fn a_sealed_body_crosses_to_a_peer_that_opens_it_and_shows_nothing_in_the_clear() {
    let ring = ring_with_key();
    let mut a = machine("a", 1);
    let b = machine("b", 2);
    let rec = a.write_sealed(&ring, "card", b"title one", NOW);
    assert!(!String::from_utf8_lossy(&rec.body).contains("title one"), "the body is sealed");
    let all: Pins = [("a".to_string(), a.public), ("b".to_string(), b.public)].into();
    let mut clock = Clock::new("b");
    assert_eq!(log::apply_remote(&b.conn, &rec, &all, &mut clock, NOW + 1, &NewestWins).unwrap(), Outcome::Applied);
    let held = log::current(&b.conn, COLLECTION, "card").unwrap().unwrap();
    assert_eq!(&open(&ring, &held.header, &held.body).unwrap()[..], b"title one");
}

#[test]
fn a_store_that_holds_every_record_cannot_open_one_without_the_key() {
    let ring = ring_with_key();
    let mut a = machine("a", 1);
    let rec = a.write_sealed(&ring, "card", b"a secret", NOW);
    assert_eq!(open(&KeyRing::new(), &rec.header, &rec.body), Err(OpenError::NoKey));
    let mut other = KeyRing::new();
    other.insert(0, generate_key().unwrap());
    assert!(matches!(open(&other, &rec.header, &rec.body), Err(OpenError::Failed(_))), "another key does not open it");
}

#[test]
fn a_body_does_not_open_at_another_record_version_base_epoch_or_collection() {
    let mut ring = ring_with_key();
    let mut a = machine("a", 1);
    a.write_sealed(&ring, "card", b"first", NOW);
    let rec = a.write_sealed(&ring, "card", b"second", NOW + 10);
    assert!(rec.header.base.is_some());
    assert_eq!(&open(&ring, &rec.header, &rec.body).unwrap()[..], b"second");
    // The same key at epoch 1, so only the associated data can refuse.
    let key = ring.get(0).unwrap().clone();
    ring.insert(1, key);
    let altered = [
        ("record", { let mut h = rec.header.clone(); h.record = "other".into(); h }),
        ("collection", { let mut h = rec.header.clone(); h.collection = "c2".into(); h }),
        ("version", { let mut h = rec.header.clone(); h.version.counter += 1; h }),
        ("base", { let mut h = rec.header.clone(); h.base = None; h }),
        ("epoch", { let mut h = rec.header.clone(); h.epoch = 1; h }),
    ];
    for (what, header) in altered {
        assert!(matches!(open(&ring, &header, &rec.body), Err(OpenError::Failed(_))), "a different {what} does not open");
    }
    let mut tampered = rec.body.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 1;
    assert!(matches!(open(&ring, &rec.header, &tampered), Err(OpenError::Failed(_))));
}

#[test]
fn the_closure_is_given_exactly_what_the_header_will_carry() {
    let ring = ring_with_key();
    let mut a = machine("a", 1);
    let first = a.write_sealed(&ring, "card", b"one", NOW);
    let seen: RefCell<Option<SealAt>> = RefCell::new(None);
    let mut signer = Signer { machine: &a.id, private: &a.private, clock: &mut a.clock };
    let second = log::append_local_with(&a.conn, &mut signer, COLLECTION, "card", false, NOW + 5, |at| {
        *seen.borrow_mut() = Some(at.clone());
        Ok(b"two".to_vec())
    })
    .unwrap();
    let at = seen.into_inner().unwrap();
    assert_eq!(
        (at.collection.as_str(), at.record.as_str(), &at.version, &at.base, at.epoch),
        (COLLECTION, "card", &second.header.version, &Some(first.header.version), 0)
    );
}

#[test]
fn a_deletion_has_no_sealed_body_and_a_failed_seal_writes_nothing() {
    let ring = ring_with_key();
    let mut a = machine("a", 1);
    a.write_sealed(&ring, "card", b"one", NOW);
    let mut signer = Signer { machine: &a.id, private: &a.private, clock: &mut a.clock };
    let gone = log::append_local_with(&a.conn, &mut signer, COLLECTION, "card", true, NOW + 1, |_| panic!("a tombstone is not sealed")).unwrap();
    assert!(gone.header.tombstone && gone.body.is_empty());
    let before = log::vector(&a.conn, COLLECTION).unwrap();
    let mut signer = Signer { machine: &a.id, private: &a.private, clock: &mut a.clock };
    let failed = log::append_local_with(&a.conn, &mut signer, COLLECTION, "card", false, NOW + 2, |_| Err("no key".to_string()));
    assert!(matches!(failed, Err(SyncError::Seal(_))));
    assert_eq!(log::vector(&a.conn, COLLECTION).unwrap(), before, "nothing was written");
    // An id the seal does not take (a separator that could run two parts into one) is refused the same way.
    let mut signer = Signer { machine: &a.id, private: &a.private, clock: &mut a.clock };
    let odd = log::append_local_with(&a.conn, &mut signer, COLLECTION, "a:b", false, NOW + 3, sealer(&ring, b"x"));
    assert!(matches!(odd, Err(SyncError::Seal(_))));
}
