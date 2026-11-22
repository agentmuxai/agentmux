// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The Vault's adapter on the real sync layer, against in-memory stores of fake machines: follow-on edits, a partition,
//! a fast clock, a delete that did not see an edit, a rotation in progress, and a base changed on the way. The bodies
//! are plain text here, because the adapter reads headers only. Every value is invented.

use agentmux_common::jekt_sign::generate_lan_keypair;
use rusqlite::Connection;

use super::*;
use crate::backend::user_sync::hlc::Clock;
use crate::backend::user_sync::log::{self, Outcome, Pins, Refused, Signer, SyncError, Vector};
use crate::backend::user_sync::record::{BadRecord, Record};

const NOW: i64 = 1_800_000_000_000;
const VAULT: &str = "vault-1";

struct Machine {
    id: String,
    conn: Connection,
    public: [u8; 32],
    private: [u8; 32],
    clock: Clock,
}

struct Never;
impl Rotations for Never {
    fn rotating(&self, _: &str) -> bool {
        false
    }
}

struct Rotating(&'static str);
impl Rotations for Rotating {
    fn rotating(&self, record: &str) -> bool {
        record == self.0
    }
}

fn machine(name: &str, seed: u8) -> Machine {
    let (public, private) = generate_lan_keypair([seed; 32]);
    let conn = Connection::open_in_memory().unwrap();
    log::init(&conn).unwrap();
    log::ensure_collection(&conn, VAULT, "vault", NOW).unwrap();
    Machine { id: name.to_string(), conn, public, private, clock: Clock::new(name) }
}

fn pins(machines: &[&Machine]) -> Pins {
    machines.iter().map(|m| (m.id.clone(), m.public)).collect()
}

impl Machine {
    fn write(&mut self, record: &str, body: &str, at: i64) -> Record {
        let mut signer = Signer { machine: &self.id, private: &self.private, clock: &mut self.clock };
        log::append_local(&self.conn, &mut signer, VAULT, record, body.as_bytes().to_vec(), false, at).unwrap()
    }

    fn delete(&mut self, record: &str, at: i64) -> Record {
        let mut signer = Signer { machine: &self.id, private: &self.private, clock: &mut self.clock };
        log::append_local(&self.conn, &mut signer, VAULT, record, Vec::new(), true, at).unwrap()
    }

    fn state(&self, record: &str) -> Option<String> {
        let r = log::current(&self.conn, VAULT, record).unwrap()?;
        (!r.header.tombstone).then(|| String::from_utf8(r.body).unwrap())
    }

    /// How many versions of `record` this machine's log holds: the current one and every one it kept.
    fn versions(&self, record: &str) -> usize {
        log::since(&self.conn, VAULT, &Vector::new(), 1000).unwrap().iter().filter(|r| r.header.record == record).count()
    }

    fn pull_with(&mut self, from: &Machine, all: &Pins, at: i64, rotations: &dyn Rotations) -> Vec<Result<Outcome, SyncError>> {
        let mine = log::vector(&self.conn, VAULT).unwrap();
        let records = log::since(&from.conn, VAULT, &mine, 1000).unwrap();
        let adapter = VaultAdapter { conn: &self.conn, rotations };
        records.iter().map(|r| log::apply_remote(&self.conn, r, all, &mut self.clock, at, &adapter)).collect()
    }

    fn pull(&mut self, from: &Machine, all: &Pins, at: i64) -> Vec<Outcome> {
        self.pull_with(from, all, at, &Never).into_iter().map(|o| o.unwrap()).collect()
    }
}

#[test]
fn an_edit_made_on_top_of_what_the_other_machine_holds_is_applied_not_a_conflict() {
    let (mut a, mut b) = (machine("a", 1), machine("b", 2));
    let all = pins(&[&a, &b]);
    a.write("entry-1", "first", NOW);
    assert_eq!(b.pull(&a, &all, NOW + 1), [Outcome::Applied]);
    b.write("entry-1", "second", NOW + 10);
    assert_eq!(a.pull(&b, &all, NOW + 20), [Outcome::Applied], "it follows on from what a holds");
    assert_eq!((a.state("entry-1").as_deref(), b.state("entry-1").as_deref()), (Some("second"), Some("second")));
    // Hearing it all again changes nothing.
    assert!(a.pull(&b, &all, NOW + 30).is_empty());
}

#[test]
fn two_machines_that_changed_a_secret_apart_keep_both_and_agree_which_is_current() {
    let (mut a, mut b) = (machine("a", 1), machine("b", 2));
    let all = pins(&[&a, &b]);
    a.write("entry-1", "base", NOW);
    b.pull(&a, &all, NOW + 1);
    a.write("entry-1", "from a", NOW + 100);
    b.write("entry-1", "from b", NOW + 200);
    assert_eq!(a.pull(&b, &all, NOW + 300), [Outcome::Conflict]);
    assert_eq!(b.pull(&a, &all, NOW + 300), [Outcome::Conflict]);
    // The later version is current on both, and each keeps the other as a version.
    assert_eq!((a.state("entry-1").as_deref(), b.state("entry-1").as_deref()), (Some("from b"), Some("from b")));
    assert_eq!((a.versions("entry-1"), b.versions("entry-1")), (3, 3));
}

#[test]
fn a_fast_clock_wins_the_choice_but_the_other_change_is_kept() {
    let (mut a, mut b) = (machine("a", 1), machine("b", 2));
    let all = pins(&[&a, &b]);
    a.write("entry-1", "base", NOW);
    b.pull(&a, &all, NOW + 1);
    // b's clock is four minutes ahead, inside what the layer accepts; a changes it later by the clock on the wall.
    b.write("entry-1", "from b", NOW + 4 * 60 * 1000);
    a.write("entry-1", "from a", NOW + 100_000);
    assert_eq!(a.pull(&b, &all, NOW + 110_000), [Outcome::Conflict]);
    assert_eq!(b.pull(&a, &all, NOW + 110_000), [Outcome::Conflict]);
    assert_eq!((a.state("entry-1").as_deref(), b.state("entry-1").as_deref()), (Some("from b"), Some("from b")));
    assert_eq!(a.versions("entry-1"), 3, "a's later change in real time is still there");
}

#[test]
fn an_edit_made_without_seeing_a_delete_wins_over_it_on_both_machines() {
    let (mut a, mut b) = (machine("a", 1), machine("b", 2));
    let all = pins(&[&a, &b]);
    a.write("entry-1", "base", NOW);
    b.pull(&a, &all, NOW + 1);
    b.write("entry-1", "edited", NOW + 50);
    a.delete("entry-1", NOW + 100);
    // The delete is the later version, and the edit still wins: nothing is lost to a delete the editor did not see.
    assert_eq!(a.pull(&b, &all, NOW + 200), [Outcome::Applied]);
    assert_eq!(b.pull(&a, &all, NOW + 200), [Outcome::KeptLocal]);
    assert_eq!((a.state("entry-1").as_deref(), b.state("entry-1").as_deref()), (Some("edited"), Some("edited")));
}

#[test]
fn a_delete_made_on_top_of_what_the_other_holds_is_applied() {
    let (mut a, mut b) = (machine("a", 1), machine("b", 2));
    let all = pins(&[&a, &b]);
    a.write("entry-1", "base", NOW);
    b.pull(&a, &all, NOW + 1);
    b.delete("entry-1", NOW + 10);
    assert_eq!(a.pull(&b, &all, NOW + 20), [Outcome::Applied]);
    assert_eq!((a.state("entry-1"), b.state("entry-1")), (None, None));
}

#[test]
fn a_rotation_in_progress_is_not_overwritten_by_a_change_from_another_machine() {
    let (mut a, mut b) = (machine("a", 1), machine("b", 2));
    let all = pins(&[&a, &b]);
    a.write("entry-1", "before rotation", NOW);
    b.pull(&a, &all, NOW + 1);
    b.write("entry-1", "edited on b", NOW + 10);
    let got = a.pull_with(&b, &all, NOW + 20, &Rotating("entry-1"));
    assert!(matches!(got.as_slice(), [Ok(Outcome::Conflict)]), "{got:?}");
    assert_eq!(a.state("entry-1").as_deref(), Some("before rotation"), "the entry stays as it was until the rotation ends");
    assert_eq!(a.versions("entry-1"), 2, "and b's change is kept as a version");
    // Another entry on the same machine is not held back.
    b.write("entry-2", "other", NOW + 30);
    assert!(matches!(a.pull_with(&b, &all, NOW + 40, &Rotating("entry-1")).as_slice(), [Ok(Outcome::Applied)]));
}

#[test]
fn a_record_whose_base_was_changed_on_the_way_is_refused() {
    let (mut a, mut b) = (machine("a", 1), machine("b", 2));
    let all = pins(&[&a, &b]);
    a.write("entry-1", "base", NOW);
    b.pull(&a, &all, NOW + 1);
    a.write("entry-1", "from a", NOW + 100);
    b.write("entry-1", "from b", NOW + 200);
    // A relay rewrites b's record to say it followed on from a's, to hide that the two were made apart.
    let mut forged = log::since(&b.conn, VAULT, &Vector::new(), 1000).unwrap().into_iter().find(|r| r.header.origin == "b").unwrap();
    forged.header.base = log::current(&a.conn, VAULT, "entry-1").unwrap().map(|r| r.header.version);
    let said = log::apply_remote(&a.conn, &forged, &all, &mut a.clock, NOW + 300, &VaultAdapter { conn: &a.conn, rotations: &Never });
    assert!(matches!(said, Err(SyncError::Refused(Refused::Bad(BadRecord::Signature)))), "{said:?}");
    assert_eq!(a.state("entry-1").as_deref(), Some("from a"));
}
