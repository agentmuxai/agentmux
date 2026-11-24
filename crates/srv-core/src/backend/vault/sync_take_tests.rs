// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A record from another machine, taken into the Vault: two machines, each with a store of its own, write with
//! `sync_capture` and take with [`Taker`]. Every value is invented.

use std::collections::BTreeMap;

use agentmux_common::jekt_sign::generate_lan_keypair;
use zeroize::Zeroizing;

use super::*;
use crate::backend::storage::vault::{VaultInfoRow, VaultRow};
use crate::backend::user_sync::log::{Signer, Vector};
use crate::backend::storage::vault_sync::TakeWrite;
use crate::backend::user_sync::seal::KeyRing;
use crate::backend::vault::sync_capture::{self, capture};
use crate::backend::vault::sync_record::Known;

const NOW: i64 = 1_800_000_000_000;
const VAULT: &str = "vault-a";
const DAY: i64 = 24 * 60 * 60 * 1000;

struct Machine {
    id: String,
    store: Store,
    public: [u8; 32],
    private: [u8; 32],
    clock: Clock,
    ring: KeyRing,
}

fn ring() -> KeyRing {
    let mut ring = KeyRing::new();
    ring.insert(0, Zeroizing::new([7u8; 32]));
    ring
}

fn key() -> Key {
    seal::test_key()
}

fn machine(name: &str, seed: u8) -> Machine {
    let store = Store::open_in_memory().unwrap();
    store.apply_identity_schema_for_tests().unwrap();
    {
        let conn = store.conn().lock().unwrap();
        sync_capture::init(&conn).unwrap();
        log::ensure_collection(&conn, VAULT, sync_capture::KIND, NOW).unwrap();
        log::ensure_collection(&conn, "vault-b", sync_capture::KIND, NOW).unwrap();
    }
    for id in [VAULT, "vault-b"] {
        store.vault_add(&VaultInfoRow { id: id.into(), name: id.into(), agents: "all".into(), agent_ids: "[]".into(), position: 1, created_at: 1 }).ok();
    }
    let (public, private) = generate_lan_keypair([seed; 32]);
    Machine { id: name.to_string(), store, public, private, clock: Clock::new(name), ring: ring() }
}

fn parts(value: &str) -> Parts {
    Parts(BTreeMap::from([("value".to_string(), value.to_string())]))
}

fn entry(id: &str, label: &str, kind: &str, value: &str, tags: &str) -> VaultRow {
    VaultRow {
        id: id.into(),
        kind: kind.into(),
        label: label.into(),
        fields: "{\"service\":\"payments\"}".into(),
        tags: tags.into(),
        sealed: seal::seal(&key(), id, &parts(value)).unwrap(),
        created_at: 1,
        updated_at: 1,
        last_used_at: 0,
        vault_id: VAULT.into(),
    }
}

impl Machine {
    fn capture(&mut self, vault: &str, at: i64) -> sync_capture::Captured {
        let mut signer = Signer { machine: &self.id, private: &self.private, clock: &mut self.clock };
        capture(&self.store, &mut signer, vault, &self.ring, &key(), &Known(&[]), at).unwrap()
    }

    /// Changes an entry here as a person would.
    fn edit(&self, entry_id: &str, value: &str, tags: &str, at: i64) {
        let mut row = self.store.vault_get(entry_id).unwrap().unwrap();
        row.sealed = seal::seal(&key(), entry_id, &parts(value)).unwrap();
        row.tags = tags.into();
        row.updated_at = at;
        self.store.vault_update(&row).unwrap();
    }

    fn value(&self, entry_id: &str) -> Option<String> {
        let row = self.store.vault_get(entry_id).unwrap()?;
        seal::open(&key(), entry_id, &row.sealed).unwrap().get("value").map(str::to_string)
    }

    /// The entry here that has `link` as one of its ids.
    fn entry_of(&self, link: &str) -> Option<String> {
        self.store.vault_entry_of_link(link).unwrap()
    }

    fn link(&self, entry_id: &str) -> String {
        self.store.vault_link_primary(entry_id).unwrap().unwrap()
    }

    /// The versions the entry has here, newest first: state and whether it holds a value.
    fn versions(&self, entry_id: &str) -> Vec<(String, bool)> {
        let row = self.store.vault_get(entry_id).unwrap().unwrap();
        self.store.vault_versions(entry_id, row.created_at).unwrap().into_iter().map(|v| (v.state, !v.sealed.is_empty())).collect()
    }
}

fn pins(a: &Machine, b: &Machine) -> Pins {
    [(a.id.clone(), a.public), (b.id.clone(), b.public)].into_iter().collect()
}

/// Sends what `from` holds and `to` lacks, the way a transport would, and takes each record in.
fn ship(from: &Machine, to: &mut Machine, pins: &Pins, at: i64) -> Vec<Taken> {
    let vector: Vector = log::vector(&to.store.conn().lock().unwrap(), VAULT).unwrap();
    let mut records = log::since(&from.store.conn().lock().unwrap(), VAULT, &vector, 100).unwrap();
    let other: Vector = log::vector(&to.store.conn().lock().unwrap(), "vault-b").unwrap();
    records.extend(log::since(&from.store.conn().lock().unwrap(), "vault-b", &other, 100).unwrap());
    records.sort_by(|x, y| x.header.version.cmp(&y.header.version));
    let own = key();
    let taker = Taker { store: &to.store, ring: &to.ring, key: &own, profiles: &Known(&[]) };
    records.iter().map(|r| taker.receive(r, pins, &mut to.clock, at).unwrap()).collect()
}

/// What a sync round does first: takes again what was deferred in the last one, which settles the deletes that waited.
fn settle(to: &Machine, at: i64) -> Vec<(String, Result<Effect, String>)> {
    let own = key();
    Taker { store: &to.store, ring: &to.ring, key: &own, profiles: &Known(&[]) }.retake(VAULT, at).unwrap()
}

/// Two machines that both hold one entry, written on `a` and taken by `b`.
fn both_hold_one() -> (Machine, Machine, Pins, String, String) {
    let mut a = machine("a", 1);
    let mut b = machine("b", 2);
    let p = pins(&a, &b);
    a.store.vault_insert(&entry("entry-a", "Invoice provider key", "api-key", "sk_test_invented_0001", "[\"prod\"]")).unwrap();
    a.capture(VAULT, NOW);
    let taken = ship(&a, &mut b, &p, NOW + 5);
    assert_eq!(taken.len(), 1);
    let link = a.link("entry-a");
    let on_b = b.entry_of(&link).expect("b has the entry under the record's id");
    (a, b, p, "entry-a".to_string(), on_b)
}

#[test]
fn a_new_entry_is_made_here_with_the_record_s_id_and_is_not_written_back() {
    let (a, mut b, _p, _on_a, on_b) = both_hold_one();
    let row = b.store.vault_get(&on_b).unwrap().unwrap();
    assert_eq!((row.label.as_str(), row.kind.as_str(), row.vault_id.as_str()), ("Invoice provider key", "api-key", VAULT));
    assert_eq!(b.value(&on_b).as_deref(), Some("sk_test_invented_0001"), "the value is sealed under b's own key");
    assert_eq!(b.link(&on_b), a.link("entry-a"), "the entry has the id the record names, so a link made on a opens it on b");
    assert_ne!(on_b, "entry-a", "the entry's own id is made here");
    // Taking it was not a change made here: a capture finds nothing to write.
    assert_eq!(b.capture(VAULT, NOW + 20), sync_capture::Captured::default());
}

#[test]
fn a_change_that_follows_on_replaces_the_value_and_keeps_the_old_one_for_7_days() {
    let (mut a, mut b, p, _, on_b) = both_hold_one();
    a.edit("entry-a", "sk_test_invented_0002", "[\"prod\",\"billing\"]", NOW + 100);
    a.capture(VAULT, NOW + 100);
    let taken = ship(&a, &mut b, &p, NOW + 110);
    assert_eq!(taken[0].outcome, Outcome::Applied);
    assert_eq!(taken[0].effect, Effect::Wrote { entry: on_b.clone() });
    assert_eq!(b.value(&on_b).as_deref(), Some("sk_test_invented_0002"));
    assert_eq!(b.store.vault_get(&on_b).unwrap().unwrap().tags, "[\"prod\",\"billing\"]");
    assert_eq!(b.versions(&on_b), [(PREVIOUS.to_string(), true)], "the value it replaced is kept");
    assert_eq!(b.capture(VAULT, NOW + 120), sync_capture::Captured::default());
    // A change that left the value as it was keeps no version.
    a.edit("entry-a", "sk_test_invented_0002", "[\"prod\"]", NOW + 200);
    a.capture(VAULT, NOW + 200);
    ship(&a, &mut b, &p, NOW + 210);
    assert_eq!(b.versions(&on_b).len(), 1);
}

#[test]
fn two_changes_made_apart_keep_both_values_and_every_machine_ends_on_the_same_one() {
    let (mut a, mut b, p, _, on_b) = both_hold_one();
    a.edit("entry-a", "sk_test_invented_from_a", "[\"prod\",\"a\"]", NOW + 100);
    b.edit(&on_b, "sk_test_invented_from_b", "[\"prod\",\"b\"]", NOW + 200);
    a.capture(VAULT, NOW + 100);
    b.capture(VAULT, NOW + 200);
    let on_b_taken = ship(&a, &mut b, &p, NOW + 300);
    let on_a_taken = ship(&b, &mut a, &p, NOW + 300);
    assert_eq!(on_b_taken[0].outcome, Outcome::Conflict);
    assert_eq!(on_a_taken[0].outcome, Outcome::Conflict);
    // The later version, b's, is the entry's value on both machines.
    assert_eq!(a.value("entry-a").as_deref(), Some("sk_test_invented_from_b"));
    assert_eq!(b.value(&on_b).as_deref(), Some("sk_test_invented_from_b"));
    // The other is kept on each, not pruned after 7 days, and says where it was made.
    let (kept_a, kept_b) = (a.versions("entry-a"), b.versions(&on_b));
    assert_eq!(kept_a, [(format!("{KEPT_PREFIX}changed on a at the same time"), true)]);
    assert_eq!(kept_b, [(format!("{KEPT_PREFIX}changed on a at the same time"), true)]);
    assert_eq!(on_a_taken[0].kept.as_deref(), Some("changed on a at the same time"));
    // Only the tags are merged, as a union.
    for (m, id) in [(&a, "entry-a"), (&b, on_b.as_str())] {
        let tags: Vec<String> = serde_json::from_str(&m.store.vault_get(id).unwrap().unwrap().tags).unwrap();
        assert!(["prod", "a", "b"].iter().all(|t| tags.iter().any(|x| x == t)), "tags on {}: {tags:?}", m.id);
    }
    // Neither side's taking is written back as a new change.
    assert_eq!(a.capture(VAULT, NOW + 400), sync_capture::Captured::default());
    assert_eq!(b.capture(VAULT, NOW + 400), sync_capture::Captured::default());
    // A month later the value that lost is still there, and the person can put it back.
    b.store.vault_versions_prune(NOW + 30 * DAY, KEPT_PREFIX).unwrap();
    assert_eq!(b.versions(&on_b).len(), 1);
    let seq = b.store.vault_versions(&on_b, 1).unwrap()[0].seq;
    versions::restore(&b.store, &on_b, seq).unwrap();
    assert_eq!(b.value(&on_b).as_deref(), Some("sk_test_invented_from_a"));
}

#[test]
fn a_delete_removes_the_entry_with_its_versions_and_a_second_delete_changes_nothing() {
    let (mut a, mut b, p, _, on_b) = both_hold_one();
    a.edit("entry-a", "sk_test_invented_0002", "[\"prod\"]", NOW + 100);
    a.capture(VAULT, NOW + 100);
    ship(&a, &mut b, &p, NOW + 110);
    assert_eq!(b.versions(&on_b).len(), 1);
    let link = a.link("entry-a");
    a.store.vault_delete("entry-a").unwrap();
    a.capture(VAULT, NOW + 200);
    let taken = ship(&a, &mut b, &p, NOW + 210);
    // The delete waits one round, in case the entry moved to another Vault: the next round's start takes it.
    assert!(matches!(&taken[0].effect, Effect::Deferred(why) if why.contains("moved")), "{taken:?}");
    assert!(b.store.vault_get(&on_b).unwrap().is_some());
    assert!(matches!(&settle(&b, NOW + 220)[..], [(_, Ok(Effect::Deleted { entry }))] if *entry == on_b));
    assert!(b.store.vault_get(&on_b).unwrap().is_none());
    assert!(b.store.vault_versions(&on_b, 1).unwrap().is_empty());
    assert_eq!(b.entry_of(&link), None);
    // Its id is retired here: no other entry is given it.
    assert!(b.store.vault_link_retired(&link).unwrap());
    assert_eq!(b.capture(VAULT, NOW + 300), sync_capture::Captured::default(), "b does not end a record that is already ended");
}

#[test]
fn a_change_made_after_a_delete_brings_the_entry_back_and_keeps_the_delete() {
    let (mut a, mut b, p, _, on_b) = both_hold_one();
    let link = a.link("entry-a");
    // b deletes it, and a, not having heard, changes it later.
    b.store.vault_delete(&on_b).unwrap();
    b.capture(VAULT, NOW + 100);
    a.edit("entry-a", "sk_test_invented_changed_later", "[\"prod\"]", NOW + 200);
    a.capture(VAULT, NOW + 200);
    let taken = ship(&a, &mut b, &p, NOW + 300);
    assert_eq!(taken[0].outcome, Outcome::Conflict);
    let back = b.entry_of(&link).expect("the entry is back under the id the record names");
    assert_eq!(b.value(&back).as_deref(), Some("sk_test_invented_changed_later"));
    assert_eq!(b.versions(&back), [(format!("{KEPT_PREFIX}deleted on b while it was changed elsewhere"), false)], "the delete is kept, and holds no value");
    // A delete that holds no value is not something to put back.
    let seq = b.store.vault_versions(&back, 1).unwrap()[0].seq;
    assert!(versions::restore(&b.store, &back, seq).is_err());
    // And a, which took the delete, ends on the same entry: the change is later, so it is current there as well.
    ship(&b, &mut a, &p, NOW + 310);
    assert_eq!(a.value("entry-a").as_deref(), Some("sk_test_invented_changed_later"));
}

#[test]
fn a_delete_made_after_a_change_ends_the_entry_and_the_change_stays_in_the_log() {
    let (mut a, mut b, p, _, on_b) = both_hold_one();
    // a changes it, and b, not having heard, deletes it later.
    a.edit("entry-a", "sk_test_invented_changed_first", "[\"prod\"]", NOW + 100);
    a.capture(VAULT, NOW + 100);
    b.store.vault_delete(&on_b).unwrap();
    b.capture(VAULT, NOW + 200);
    let link = a.link("entry-a");
    // b takes a's change: the delete is later, so the entry stays deleted, and the change has no entry to be kept on.
    let on_b_taken = ship(&a, &mut b, &p, NOW + 300);
    assert_eq!(on_b_taken[0].effect, Effect::KeptInLog("changed on a while it was deleted elsewhere".to_string()));
    assert_eq!(b.entry_of(&link), None);
    // a takes the delete: the entry ends there too, and its last value is still in a's log, sealed.
    let on_a_taken = ship(&b, &mut a, &p, NOW + 310);
    assert!(matches!(&on_a_taken[0].effect, Effect::Deferred(_)), "{on_a_taken:?}");
    // The retake keeps the conflict the delete won: the change is in the log, and says so.
    assert_eq!(settle(&a, NOW + 320).into_iter().map(|(_, e)| e).collect::<Vec<_>>(), [Ok(Effect::KeptInLog("changed on a while it was deleted elsewhere".to_string()))]);
    assert!(a.store.vault_get("entry-a").unwrap().is_none());
}

#[test]
fn a_delete_does_not_take_an_entry_that_moved_to_another_vault() {
    let (mut a, mut b, p, _, on_b) = both_hold_one();
    assert!(a.store.vault_move_entry("entry-a", "vault-b").unwrap());
    a.capture("vault-b", NOW + 100);
    let taken = ship(&a, &mut b, &p, NOW + 200);
    // The record in the new Vault moves b's entry, and the old Vault's tombstone, whichever came first, leaves it.
    assert!(taken.iter().all(|t| !matches!(t.effect, Effect::Deleted { .. })), "{taken:?}");
    let row = b.store.vault_get(&on_b).unwrap().expect("the entry is still on b");
    assert_eq!(row.vault_id, "vault-b");
    assert_eq!(b.value(&on_b).as_deref(), Some("sk_test_invented_0001"));
}

#[test]
fn a_rotation_started_while_a_delete_waited_is_not_deleted_with_the_entry() {
    let (mut a, mut b, p, _, on_b) = both_hold_one();
    a.store.vault_delete("entry-a").unwrap();
    a.capture(VAULT, NOW + 100);
    let taken = ship(&a, &mut b, &p, NOW + 110);
    assert!(matches!(&taken[0].effect, Effect::Deferred(_)), "{taken:?}");
    // A rotation starts here before the next round: its new value waits beside the entry.
    let row = b.store.vault_get(&on_b).unwrap().unwrap();
    let pending = VaultVersionRow { seq: 0, entry_id: on_b.clone(), entry_created_at: row.created_at, sealed: row.sealed.clone(), replaced_at: NOW + 150, replaced_by: "you".into(), how: "rotation".into(), state: versions::PENDING.into() };
    b.store.vault_version_add(&pending).unwrap();
    let again = settle(&b, NOW + 200);
    assert!(matches!(&again[..], [(_, Ok(Effect::Wrote { .. }))]), "{again:?}");
    assert!(b.store.vault_get(&on_b).unwrap().is_some(), "the entry and its pending value stay");
    assert!(b.versions(&on_b).iter().any(|(s, v)| s == &format!("{KEPT_PREFIX}deleted on a while this was being rotated") && *v));
}

#[test]
fn the_old_vaults_tombstone_taken_before_the_new_vaults_record_does_not_delete_the_entry() {
    let (mut a, mut b, p, _, on_b) = both_hold_one();
    // A kept earlier value the entry has on b: a delete and a re-make would lose it.
    let held = b.store.vault_get(&on_b).unwrap().unwrap();
    let earlier = VaultVersionRow { seq: 0, entry_id: on_b.clone(), entry_created_at: held.created_at, sealed: held.sealed.clone(), replaced_at: NOW + 90, replaced_by: "you".into(), how: "edit".into(), state: "kept".into() };
    b.store.vault_version_add(&earlier).unwrap();
    a.store.vault_move_entry("entry-a", "vault-b").unwrap();
    a.capture(VAULT, NOW + 100);
    a.capture("vault-b", NOW + 100);
    let own = key();
    let taker = Taker { store: &b.store, ring: &b.ring, key: &own, profiles: &Known(&[]) };
    let first = |vault: &str| log::since(&a.store.conn().lock().unwrap(), vault, &Vector::default(), 100).unwrap();
    // The old Vault's tombstone comes first, on its own.
    let old = first(VAULT);
    let tomb = old.iter().find(|r| r.header.tombstone).expect("the old Vault ends the record");
    let taken = taker.receive(tomb, &p, &mut b.clock, NOW + 200).unwrap();
    assert!(matches!(taken.effect, Effect::Deferred(_)), "{taken:?}");
    assert_eq!(b.store.vault_get(&on_b).unwrap().unwrap().vault_id, VAULT, "still there");
    // Then the new Vault's record: it moves the same entry.
    for rec in first("vault-b") {
        taker.receive(&rec, &p, &mut b.clock, NOW + 210).unwrap();
    }
    let row = b.store.vault_get(&on_b).unwrap().expect("the entry is the same one, not made again");
    assert_eq!(row.vault_id, "vault-b");
    assert_eq!(b.versions(&on_b).len(), 1, "its kept value is still there");
    // The next round takes the delete again, and finds the entry in another Vault: nothing is deleted.
    let again = taker.retake(VAULT, NOW + 300).unwrap();
    assert!(matches!(&again[..], [(_, Ok(Effect::None))]), "{again:?}");
    assert!(b.store.vault_get(&on_b).unwrap().is_some());
}

#[test]
fn a_rotation_waiting_here_is_not_overwritten_and_the_change_waits_beside_it() {
    let (mut a, mut b, p, _, on_b) = both_hold_one();
    let row = b.store.vault_get(&on_b).unwrap().unwrap();
    b.store
        .vault_version_add(&VaultVersionRow {
            seq: 0,
            entry_id: on_b.clone(),
            entry_created_at: row.created_at,
            sealed: row.sealed.clone(),
            replaced_at: NOW + 150,
            replaced_by: "you".into(),
            how: "rotation".into(),
            state: versions::PENDING.into(),
        })
        .unwrap();
    a.edit("entry-a", "sk_test_invented_changed_apart", "[\"prod\"]", NOW + 100);
    a.capture(VAULT, NOW + 100);
    let taken = ship(&a, &mut b, &p, NOW + 200);
    assert_eq!(taken[0].kept.as_deref(), Some("changed on a while this was being rotated"));
    assert_eq!(b.value(&on_b).as_deref(), Some("sk_test_invented_0001"), "the entry still holds the value being rotated");
    assert!(b.versions(&on_b).iter().any(|(s, v)| s == &format!("{KEPT_PREFIX}changed on a while this was being rotated") && *v));
}

#[test]
fn a_kind_this_machine_does_not_know_is_not_taken() {
    let mut a = machine("a", 1);
    let mut b = machine("b", 2);
    let p = pins(&a, &b);
    a.store.vault_insert(&entry("odd", "From a newer version", "hologram", "x", "[]")).unwrap();
    a.capture(VAULT, NOW);
    let taken = ship(&a, &mut b, &p, NOW + 5);
    assert!(matches!(&taken[0].effect, Effect::Deferred(why) if why.contains("hologram")), "{taken:?}");
    assert!(b.store.vault_list().unwrap().is_empty());
    // The log holds it, so it is not sent again: it stays noted, and a retake (after an upgrade) tries it again.
    let again = Taker { store: &b.store, ring: &b.ring, key: &key(), profiles: &Known(&[]) }.retake(VAULT, NOW + 10).unwrap();
    assert!(matches!(&again[..], [(_, Ok(Effect::Deferred(_)))]), "{again:?}");
}

#[test]
fn a_record_the_store_could_not_take_is_taken_again_from_the_log_by_retake() {
    let mut a = machine("a", 1);
    let mut b = machine("b", 2);
    let p = pins(&a, &b);
    a.store.vault_insert(&entry("entry-a", "Invoice provider key", "api-key", "sk_test_invented_0001", "[]")).unwrap();
    a.capture(VAULT, NOW);
    // b has no key for the record's epoch yet: the layer holds the record, and the store cannot open it.
    let vector: Vector = log::vector(&b.store.conn().lock().unwrap(), VAULT).unwrap();
    let records = log::since(&a.store.conn().lock().unwrap(), VAULT, &vector, 100).unwrap();
    let (own, none) = (key(), KeyRing::new());
    let blind = Taker { store: &b.store, ring: &none, key: &own, profiles: &Known(&[]) };
    let failed: Vec<_> = records.iter().map(|r| blind.receive(r, &p, &mut b.clock, NOW + 5)).collect();
    assert!(failed.iter().all(|r| r.as_ref().is_err_and(|e| e.contains("no key for epoch") && e.contains("retake"))), "{failed:?}");
    assert!(b.store.vault_list().unwrap().is_empty());
    // A resend finds nothing to send: the log has the record. The key comes, and the retake takes it.
    assert!(ship(&a, &mut b, &p, NOW + 6).is_empty());
    let sighted = Taker { store: &b.store, ring: &b.ring, key: &own, profiles: &Known(&[]) };
    let again = sighted.retake(VAULT, NOW + 10).unwrap();
    assert!(matches!(&again[..], [(_, Ok(Effect::Wrote { .. }))]), "{again:?}");
    let on_b = b.entry_of(&a.link("entry-a")).expect("the entry is taken");
    assert_eq!(b.value(&on_b).as_deref(), Some("sk_test_invented_0001"));
    // Taken once: nothing is left to retake.
    assert!(sighted.retake(VAULT, NOW + 20).unwrap().is_empty());
}

#[test]
fn a_conflict_won_by_a_record_that_could_not_be_taken_is_kept_as_a_conflict_when_retaken() {
    let (mut a, mut b, p, _, on_b) = both_hold_one();
    // Changed apart: a's change is the later one, so it wins on b, and what b held is kept.
    b.edit(&on_b, "sk_test_invented_from_b", "[\"prod\",\"b\"]", NOW + 100);
    a.edit("entry-a", "sk_test_invented_from_a", "[\"prod\",\"a\"]", NOW + 200);
    b.capture(VAULT, NOW + 100);
    a.capture(VAULT, NOW + 200);
    let vector: Vector = log::vector(&b.store.conn().lock().unwrap(), VAULT).unwrap();
    let records = log::since(&a.store.conn().lock().unwrap(), VAULT, &vector, 100).unwrap();
    let (own, none) = (key(), KeyRing::new());
    let blind = Taker { store: &b.store, ring: &none, key: &own, profiles: &Known(&[]) };
    assert!(records.iter().all(|r| blind.receive(r, &p, &mut b.clock, NOW + 300).is_err()));
    // The retake takes it as the conflict it was: the value b held is kept for good, not for 7 days, and the tags are both sides'.
    let again = settle(&b, NOW + 400);
    assert!(matches!(&again[..], [(_, Ok(Effect::Wrote { .. }))]), "{again:?}");
    assert_eq!(b.value(&on_b).as_deref(), Some("sk_test_invented_from_a"));
    assert_eq!(b.versions(&on_b), [(format!("{KEPT_PREFIX}changed on b at the same time"), true)]);
    let tags: Vec<String> = serde_json::from_str(&b.store.vault_get(&on_b).unwrap().unwrap().tags).unwrap();
    assert!(["prod", "a", "b"].iter().all(|x| tags.iter().any(|t| t == x)), "{tags:?}");
}

#[test]
fn a_label_edited_here_between_the_read_and_the_write_is_not_overwritten() {
    let (_a, b, _p, _, on_b) = both_hold_one();
    let read = b.store.vault_get(&on_b).unwrap().unwrap();
    // A change made here after the read that touches neither the value nor the time of change.
    let mut edited = read.clone();
    edited.label = "Renamed here".into();
    b.store.vault_update(&edited).unwrap();
    let mut next = read.clone();
    next.tags = "[\"from-the-record\"]".into();
    let write = TakeWrite { row: &next, was: Some(&read), primary: None, aliases: &[], revive: None, kept: &[] };
    assert!(!b.store.vault_take_write(&write).unwrap(), "the write sees that the entry is no longer as it was read");
    let now = b.store.vault_get(&on_b).unwrap().unwrap();
    assert_eq!((now.label.as_str(), now.tags.as_str()), ("Renamed here", read.tags.as_str()));
}

#[test]
fn what_is_kept_to_the_machine_is_not_replaced_by_a_record() {
    let (mut a, mut b, p, _, on_b) = both_hold_one();
    // The agents b shares the entry with, and when it was last used there, are b's own.
    b.store.vault_set_fields(&on_b, "{\"service\":\"payments\",\"shared\":\"agent-x\"}").unwrap();
    b.run_last_used(&on_b, 4242);
    b.capture(VAULT, NOW + 50);
    a.edit("entry-a", "sk_test_invented_0003", "[\"prod\"]", NOW + 100);
    a.capture(VAULT, NOW + 100);
    ship(&a, &mut b, &p, NOW + 110);
    let after = b.store.vault_get(&on_b).unwrap().unwrap();
    assert!(after.fields.contains("agent-x"), "{}", after.fields);
    assert_eq!(after.last_used_at, 4242);
}

impl Machine {
    fn run_last_used(&self, entry_id: &str, at: i64) {
        self.store.vault_run_sql_for_tests(&format!("UPDATE db_vault_entries SET last_used_at = {at} WHERE id = '{entry_id}'"));
    }
}
