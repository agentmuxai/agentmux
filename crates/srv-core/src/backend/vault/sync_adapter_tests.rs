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
fn a_delete_and_an_edit_made_apart_end_as_the_later_of_the_two_with_the_other_kept() {
    let (mut a, mut b) = (machine("a", 1), machine("b", 2));
    let all = pins(&[&a, &b]);
    a.write("entry-1", "base", NOW);
    b.pull(&a, &all, NOW + 1);
    b.write("entry-1", "edited", NOW + 50);
    a.delete("entry-1", NOW + 100);
    // The delete is the later of the two: whichever side hears first, both end with the entry deleted, and the edit is a
    // version each still holds.
    assert_eq!(a.pull(&b, &all, NOW + 200), [Outcome::Conflict]);
    assert_eq!(b.pull(&a, &all, NOW + 200), [Outcome::Conflict]);
    assert_eq!((a.state("entry-1"), b.state("entry-1")), (None, None));
    assert_eq!((a.versions("entry-1"), b.versions("entry-1")), (3, 3));
}

#[test]
fn an_edit_later_than_a_delete_made_apart_from_it_ends_current_on_both_machines() {
    let (mut a, mut b) = (machine("a", 1), machine("b", 2));
    let all = pins(&[&a, &b]);
    a.write("entry-1", "base", NOW);
    b.pull(&a, &all, NOW + 1);
    a.delete("entry-1", NOW + 50);
    b.write("entry-1", "edited", NOW + 100);
    assert_eq!(a.pull(&b, &all, NOW + 200), [Outcome::Conflict]);
    assert_eq!(b.pull(&a, &all, NOW + 200), [Outcome::Conflict]);
    assert_eq!((a.state("entry-1").as_deref(), b.state("entry-1").as_deref()), (Some("edited"), Some("edited")));
    assert_eq!((a.versions("entry-1"), b.versions("entry-1")), (3, 3), "the delete is kept as a version");
}

#[test]
fn two_deletes_and_an_edit_between_them_end_deleted_whatever_order_a_machine_hears_them_in() {
    let (mut a, mut x, mut y, mut c) = (machine("a", 1), machine("x", 2), machine("y", 3), machine("c", 4));
    let (mut m, mut n) = (machine("m", 5), machine("n", 6));
    let all = pins(&[&a, &x, &y, &c, &m, &n]);
    a.write("entry-1", "base", NOW);
    for peer in [&mut x, &mut y, &mut c, &mut m, &mut n] {
        peer.pull(&a, &all, NOW + 1);
    }
    // x and y delete the entry apart, and c edits it between the two deletes.
    x.delete("entry-1", NOW + 100);
    c.write("entry-1", "edited", NOW + 120);
    y.delete("entry-1", NOW + 150);
    // m hears the earlier delete first, n the later one; both are then sent the edit.
    for (peer, order) in [(&mut m, [&x, &y, &c]), (&mut n, [&y, &x, &c])] {
        for from in order {
            peer.pull(from, &all, NOW + 200);
        }
    }
    assert_eq!((m.state("entry-1"), n.state("entry-1")), (None, None), "the later delete is current on both");
    assert_eq!((m.versions("entry-1"), n.versions("entry-1")), (4, 4));
}

#[test]
fn two_machines_that_made_the_same_entry_apart_keep_both_and_agree_which_is_current() {
    // The Vault names an entry from its kind and label, so two machines can each make "the same" entry before they meet.
    let (mut a, mut b) = (machine("a", 1), machine("b", 2));
    let all = pins(&[&a, &b]);
    a.write("entry-1", "made on a", NOW + 10);
    b.write("entry-1", "made on b", NOW + 20);
    assert_eq!(a.pull(&b, &all, NOW + 100), [Outcome::Conflict]);
    assert_eq!(b.pull(&a, &all, NOW + 100), [Outcome::Conflict]);
    assert_eq!((a.state("entry-1").as_deref(), b.state("entry-1").as_deref()), (Some("made on b"), Some("made on b")));
    assert_eq!((a.versions("entry-1"), b.versions("entry-1")), (2, 2));
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

#[test]
fn a_machine_sent_a_delete_before_the_entry_it_deleted_does_not_bring_the_entry_back() {
    // z makes an entry and edits it twice; a takes all three and deletes it; c, new, takes a's log, which lists a's own
    // records first, so the delete reaches c before the three it came after.
    let (mut z, mut a, mut c) = (machine("z", 3), machine("a", 1), machine("c", 4));
    let all = pins(&[&z, &a, &c]);
    z.write("entry-1", "created", NOW);
    z.write("entry-1", "edited", NOW + 10);
    z.write("entry-1", "edited again", NOW + 20);
    a.pull(&z, &all, NOW + 30);
    a.delete("entry-1", NOW + 40);
    let got = c.pull(&a, &all, NOW + 50);
    // The delete, then each earlier version: none brings the entry back (the first two cannot be told from changes made
    // apart, so they are kept as versions; the last is known to be the one the delete followed).
    assert_eq!(got, [Outcome::Applied, Outcome::Conflict, Outcome::Conflict, Outcome::KeptLocal]);
    assert_eq!(c.state("entry-1"), None, "still deleted");
    assert_eq!(a.state("entry-1"), None);
}

#[test]
fn a_restore_and_its_later_edit_delivered_out_of_order_end_with_the_edit_on_every_machine() {
    // m deletes an entry; z, sent the delete, restores it; a, sent the restore, edits it. m is then sent the edit before
    // the restore, because a's own records are listed first.
    let (mut m, mut z, mut a) = (machine("m", 1), machine("z", 3), machine("a", 2));
    let all = pins(&[&m, &z, &a]);
    m.write("entry-1", "made", NOW);
    m.delete("entry-1", NOW + 10);
    z.pull(&m, &all, NOW + 15);
    z.write("entry-1", "restored", NOW + 20);
    a.pull(&z, &all, NOW + 25);
    a.write("entry-1", "edited again", NOW + 30);
    assert_eq!(m.pull(&a, &all, NOW + 40), [Outcome::Conflict, Outcome::KeptLocal], "the edit takes the place of the delete; the restore it is on top of is then known to be older");
    assert_eq!(z.pull(&a, &all, NOW + 40), [Outcome::Applied]);
    for machine in [&m, &z, &a] {
        assert_eq!(machine.state("entry-1").as_deref(), Some("edited again"), "{}", machine.id);
    }
}
