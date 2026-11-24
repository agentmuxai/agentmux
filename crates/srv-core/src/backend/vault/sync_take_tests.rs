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
use crate::backend::user_sync::seal::KeyRing;
use crate::backend::vault::sync_capture::{self, capture};
use crate::backend::vault::sync_record::Known;
use crate::backend::vault::{sync_activity, sync_grant};

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
    let taker = Taker { machine: &to.id, store: &to.store, ring: &to.ring, key: &own, profiles: &Known(&[]) };
    records.iter().map(|r| taker.receive(r, pins, &mut to.clock, at).unwrap()).collect()
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
    assert_eq!(taken[0].effect, Effect::Deleted { entry: on_b.clone() });
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
    assert_eq!(on_a_taken[0].effect, Effect::KeptInLog("changed on a while it was deleted elsewhere".to_string()));
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
    assert!(matches!(&taken[0].effect, Effect::Skipped(why) if why.contains("hologram")), "{taken:?}");
    assert!(b.store.vault_list().unwrap().is_empty());
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

/// Writes `body`, sealed for the place the record will have, as the machine's own record.
fn write_sealed<T: serde::Serialize>(m: &mut Machine, record: &str, body: &T, at: i64) {
    let bytes = serde_json::to_vec(body).unwrap();
    let mut signer = Signer { machine: &m.id, private: &m.private, clock: &mut m.clock };
    let conn = m.store.conn().lock().unwrap();
    log::append_local_with(&conn, &mut signer, VAULT, record, false, at, sync_capture::sealer(&m.ring, &bytes)).unwrap();
}

fn line(label: &str) -> ActivityBody {
    ActivityBody {
        at: NOW,
        entry_id: "entry-a".into(),
        entry_label: label.into(),
        actor: "you".into(),
        action: "used".into(),
        target: "example.com".into(),
        outcome: "done".into(),
        version: 1,
        actor_kind: "person".into(),
        host: "a".into(),
        reason: String::new(),
        refs: String::new(),
    }
}

#[test]
fn an_activity_line_from_another_machine_is_added_once_beside_this_machine_s_own() {
    let mut a = machine("a", 1);
    let mut b = machine("b", 2);
    let p = pins(&a, &b);
    write_sealed(&mut a, &sync_activity::record_id("a", 1), &line("Invoice provider key"), NOW);
    let taken = ship(&a, &mut b, &p, NOW + 5);
    assert_eq!(taken[0].effect, Effect::Line { machine: "a".into(), seq: 1 });
    let shown = b.store.vault_activity_remote_list(10).unwrap();
    assert_eq!(shown.len(), 1);
    assert_eq!((shown[0].0.as_str(), shown[0].1.seq, shown[0].1.entry_label.as_str()), ("a", 1, "Invoice provider key"));
    // Sent again, it is the same line: the layer holds it, and nothing is added.
    let again = ship(&a, &mut b, &p, NOW + 10);
    assert!(again.is_empty());
    assert_eq!(b.store.vault_activity_remote_list(10).unwrap().len(), 1);
}

#[test]
fn an_activity_line_named_for_another_machine_or_ended_is_not_taken() {
    let mut a = machine("a", 1);
    let mut b = machine("b", 2);
    let p = pins(&a, &b);
    // a makes a line that says machine b made it.
    write_sealed(&mut a, &sync_activity::record_id("b", 1), &line("Forged"), NOW);
    let taken = ship(&a, &mut b, &p, NOW + 5);
    assert!(matches!(&taken[0].effect, Effect::Skipped(why) if why.contains("another machine")), "{taken:?}");
    assert!(b.store.vault_activity_remote_list(10).unwrap().is_empty());
}

#[test]
fn a_grant_that_is_not_its_entry_s_and_site_s_or_a_record_that_is_not_an_entry_s_is_kept_in_the_log_and_not_taken_into_the_store() {
    let mut a = machine("a", 1);
    let mut b = machine("b", 2);
    let p = pins(&a, &b);
    // A grant record whose id is not the one its entry and site make.
    let grant = format!("g-{}", "0123456789abcdef0123456789abcdef");
    write_sealed(&mut a, &grant, &sync_grant::GrantBody::new("entry-a", "https://example.com", NOW, 1), NOW);
    write_sealed(&mut a, "plain-name", &serde_json::json!({ "label": "Not an entry" }), NOW + 1);
    let taken = ship(&a, &mut b, &p, NOW + 5);
    assert_eq!(taken.len(), 2);
    assert!(taken.iter().all(|t| matches!(t.effect, Effect::Skipped(_))), "{taken:?}");
    assert!(b.store.vault_list().unwrap().is_empty());
    // The layer still holds both, so the other members get them from here.
    let conn = b.store.conn().lock().unwrap();
    assert!(log::current(&conn, VAULT, &grant).unwrap().is_some());
}

const P_A: &str = "e_0000000000000001";
const P_B: &str = "e_0000000000000002";

/// Two machines that hold the same entry under different link ids, and b knows a's as its alias (as when the person has said
/// the two are one entry). The ids are picked so that a's is the smaller. Each has written its own record.
fn diverged(value_on_b: &str) -> (Machine, Machine, Pins) {
    let mut a = machine("a", 1);
    let mut b = machine("b", 2);
    let p = pins(&a, &b);
    a.store.vault_insert(&entry("entry-a", "Invoice provider key", "api-key", "sk_test_invented_0001", "[\"prod\"]")).unwrap();
    b.store.vault_insert(&entry("entry-b", "Invoice provider key", "api-key", value_on_b, "[\"prod\"]")).unwrap();
    a.store.vault_link_id_assign("entry-a", P_A).unwrap();
    b.store.vault_link_id_assign("entry-b", P_B).unwrap();
    b.store.vault_links_set("entry-b", Some(P_B), &[P_A.to_string()]).unwrap();
    a.capture(VAULT, NOW);
    b.capture(VAULT, NOW + 1);
    (a, b, p)
}

#[test]
fn an_entry_with_two_ids_is_followed_under_the_smaller_one_and_the_larger_record_is_ended() {
    let (mut a, mut b, p) = diverged("sk_test_invented_0001");
    // b's record is the larger: a does not take it, but learns its id, so a link made on b opens the entry.
    let on_a = ship(&b, &mut a, &p, NOW + 10);
    assert!(matches!(&on_a[0].effect, Effect::Skipped(why) if why.contains("smaller id")), "{on_a:?}");
    assert_eq!(a.entry_of(P_B).as_deref(), Some("entry-a"));
    assert_eq!(a.store.vault_list().unwrap().len(), 1);
    // b takes a's record, which it holds as an alias: it follows that record from now on.
    let on_b = ship(&a, &mut b, &p, NOW + 20);
    assert_eq!(on_b[0].effect, Effect::Wrote { entry: "entry-b".into() });
    assert_eq!(b.store.vault_list().unwrap().len(), 1, "one entry, not two");
    assert_eq!(b.capture(VAULT, NOW + 30), sync_capture::Captured { tombstoned: 1, ..Default::default() }, "its own record is ended");
    // The end of the larger record does not delete the entry on a, which follows the smaller.
    let ended = ship(&b, &mut a, &p, NOW + 40);
    assert_eq!(ended[0].effect, Effect::None);
    assert!(a.store.vault_get("entry-a").unwrap().is_some());
    // An edit on either end is taken by the other, as one record.
    a.edit("entry-a", "sk_test_invented_0002", "[\"prod\"]", NOW + 100);
    a.capture(VAULT, NOW + 100);
    ship(&a, &mut b, &p, NOW + 110);
    assert_eq!(b.value("entry-b").as_deref(), Some("sk_test_invented_0002"));
    b.edit("entry-b", "sk_test_invented_0003", "[\"prod\"]", NOW + 200);
    b.capture(VAULT, NOW + 200);
    ship(&b, &mut a, &p, NOW + 210);
    assert_eq!(a.value("entry-a").as_deref(), Some("sk_test_invented_0003"));
}

#[test]
fn a_delete_made_under_the_followed_record_ends_the_entry_on_both_machines() {
    let (mut a, mut b, p) = diverged("sk_test_invented_0001");
    ship(&b, &mut a, &p, NOW + 10);
    ship(&a, &mut b, &p, NOW + 20);
    b.capture(VAULT, NOW + 30);
    ship(&b, &mut a, &p, NOW + 40);
    a.store.vault_delete("entry-a").unwrap();
    a.capture(VAULT, NOW + 100);
    let on_b = ship(&a, &mut b, &p, NOW + 110);
    assert_eq!(on_b.last().unwrap().effect, Effect::Deleted { entry: "entry-b".into() });
    assert!(b.store.vault_list().unwrap().is_empty());
}

#[test]
fn what_this_machine_held_under_its_larger_record_is_kept_beside_the_smaller_one() {
    let (mut a, mut b, p) = diverged("sk_test_invented_only_on_b");
    ship(&b, &mut a, &p, NOW + 10);
    let on_b = ship(&a, &mut b, &p, NOW + 20);
    assert_eq!(b.value("entry-b").as_deref(), Some("sk_test_invented_0001"), "the smaller record's value is current");
    let kept = b.versions("entry-b");
    assert_eq!(kept, [(format!("{KEPT_PREFIX}changed on b at the same time"), true)], "b's own value is still there");
    assert_eq!(b.capture(VAULT, NOW + 30).written, 0, "and taking it is not a change to write back");
}

const SITE: &str = "https://billing.example.com";

/// Sends what `from` holds and `to` lacks, the records `first` made before all the others (each machine's own in order, as
/// the layer requires).
fn ship_first_from(from: &Machine, to: &mut Machine, pins: &Pins, first: &str, at: i64) -> Vec<Taken> {
    let vector: Vector = log::vector(&to.store.conn().lock().unwrap(), VAULT).unwrap();
    let mut records = log::since(&from.store.conn().lock().unwrap(), VAULT, &vector, 100).unwrap();
    records.sort_by(|x, y| (x.header.origin != first, &x.header.version).cmp(&(y.header.origin != first, &y.header.version)));
    let own = key();
    let taker = Taker { machine: &to.id, store: &to.store, ring: &to.ring, key: &own, profiles: &Known(&[]) };
    records.iter().map(|r| taker.receive(r, pins, &mut to.clock, at).unwrap()).collect()
}

#[test]
fn a_grant_that_comes_before_its_entry_waits_and_is_made_when_the_entry_is_taken() {
    // a makes the entry; c takes it and grants a site; b gets c's records, the grant among them, before a's entry.
    let mut a = machine("a", 1);
    let mut b = machine("b", 2);
    let mut c = machine("c", 3);
    let p: Pins = [(a.id.clone(), a.public), (b.id.clone(), b.public), (c.id.clone(), c.public)].into_iter().collect();
    a.store.vault_insert(&entry("entry-a", "Invoice provider key", "api-key", "sk_test_invented_0001", "[]")).unwrap();
    a.capture(VAULT, NOW);
    ship(&a, &mut c, &p, NOW + 5);
    let on_c = c.entry_of(&a.link("entry-a")).unwrap();
    c.store.vault_grant_add(&on_c, SITE, NOW + 6, 1).unwrap();
    c.capture(VAULT, NOW + 7);
    let taken = ship_first_from(&c, &mut b, &p, "c", NOW + 10);
    assert!(taken.iter().any(|t| matches!(&t.effect, Effect::Skipped(why) if why.contains("waits"))), "{taken:?}");
    let on_b = b.entry_of(&a.link("entry-a")).expect("the entry came after the grant");
    assert!(b.store.vault_grant_check(&on_b, SITE).unwrap(), "the grant is made once the entry is here");
}

#[test]
fn a_grant_made_for_another_entry_of_the_same_name_is_not_made_here() {
    let mut a = machine("a", 1);
    let mut b = machine("b", 2);
    let p = pins(&a, &b);
    a.store.vault_insert(&entry("entry-a", "Invoice provider key", "api-key", "sk_test_invented_0001", "[]")).unwrap();
    a.capture(VAULT, NOW);
    ship(&a, &mut b, &p, NOW + 10);
    let on_b = b.entry_of(&a.link("entry-a")).unwrap();
    // A grant for the entry as it was made at another time.
    let body = sync_grant::GrantBody::new(&a.link("entry-a"), SITE, NOW, 2);
    write_sealed(&mut a, &body.record_id(), &body, NOW + 20);
    let taken = ship(&a, &mut b, &p, NOW + 30);
    assert!(matches!(&taken[0].effect, Effect::Skipped(why) if why.contains("another entry")), "{taken:?}");
    assert!(!b.store.vault_grant_check(&on_b, SITE).unwrap());
}

#[test]
fn an_activity_line_names_its_entry_by_the_id_it_has_on_the_machine_that_takes_it() {
    let mut a = machine("a", 1);
    let mut b = machine("b", 2);
    let p = pins(&a, &b);
    a.store.vault_insert(&entry("entry-a", "Invoice provider key", "api-key", "sk_test_invented_0001", "[]")).unwrap();
    a.store.vault_activity_add(&crate::backend::storage::vault::VaultActivityRow { at: NOW, entry_id: "entry-a".into(), entry_label: "Invoice provider key".into(), action: "revealed".into(), actor: "you".into(), ..Default::default() }).unwrap();
    a.capture(VAULT, NOW);
    ship(&a, &mut b, &p, NOW + 10);
    let on_b = b.entry_of(&a.link("entry-a")).unwrap();
    let lines = b.store.vault_activity_remote_list(10).unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].1.entry_id, on_b, "b's own id for the entry, not a's");
}
