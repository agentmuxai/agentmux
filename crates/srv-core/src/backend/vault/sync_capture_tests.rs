// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A Vault's entries written into the sync layer's log: one record per entry under its link id, sealed, only when the entry
//! changed, with a tombstone when it is gone. Two machines stand for two stores. Every value is invented.

use std::collections::BTreeMap;

use agentmux_common::jekt_sign::generate_lan_keypair;
use zeroize::Zeroizing;

use super::*;
use crate::backend::storage::vault::{VaultActivityRow, VaultInfoRow, VaultRow};
use crate::backend::user_sync::hlc::Clock;
use crate::backend::user_sync::log::{Outcome, Pins};
use crate::backend::user_sync::record::Record;
use crate::backend::vault::seal::Parts;
use crate::backend::vault::signins::PROFILE;
use crate::backend::vault::sync_activity;
use crate::backend::vault::sync_adapter::{Rotations, VaultAdapter};
use crate::backend::vault::sync_grant;
use crate::backend::vault::sync_record::Known;

const NOW: i64 = 1_800_000_000_000;
const VAULT: &str = "vault-a";
const VALUE: &str = "sk_test_invented_value_0001";

struct Never;
impl Rotations for Never {
    fn rotating(&self, _: &str) -> bool {
        false
    }
}

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

fn machine(name: &str, seed: u8) -> Machine {
    let store = Store::open_in_memory().unwrap();
    store.apply_identity_schema_for_tests().unwrap();
    init(&store.conn().lock().unwrap()).unwrap();
    for id in [VAULT, "vault-b"] {
        store.vault_add(&VaultInfoRow { id: id.into(), name: id.into(), agents: "all".into(), agent_ids: "[]".into(), position: 1, created_at: 1 }).ok();
    }
    let (public, private) = generate_lan_keypair([seed; 32]);
    Machine { id: name.to_string(), store, public, private, clock: Clock::new(name), ring: ring() }
}

fn key() -> Key {
    seal::test_key()
}

fn row(id: &str, label: &str, vault: &str, fields: &str) -> VaultRow {
    let parts = Parts(BTreeMap::from([("secret".to_string(), VALUE.to_string())]));
    VaultRow {
        id: id.into(),
        kind: "api-key".into(),
        label: label.into(),
        fields: fields.into(),
        tags: "[\"prod\"]".into(),
        sealed: seal::seal(&key(), id, &parts).unwrap(),
        created_at: 1,
        updated_at: 1,
        last_used_at: 0,
        vault_id: vault.into(),
    }
}

impl Machine {
    fn capture(&mut self, vault: &str, at: i64) -> Captured {
        let mut signer = Signer { machine: &self.id, private: &self.private, clock: &mut self.clock };
        capture(&self.store, &mut signer, vault, &self.ring, &key(), &Known(&[]), at).unwrap()
    }

    fn record(&self, vault: &str, entry: &str) -> Option<Record> {
        let link = self.store.vault_link_primary(entry).unwrap()?;
        log::current(&self.store.conn().lock().unwrap(), vault, &link).unwrap()
    }
}

/// The body of a record, opened with the Vault's key at the place its header names.
fn opened(ring: &KeyRing, r: &Record) -> EntryBody {
    let h = &r.header;
    let place = Place { vault: &h.collection, record: &h.record, version: &h.version, base: h.base.as_ref(), epoch: h.epoch };
    let plain = sync_seal::open_raw(ring.get(h.epoch).unwrap(), &place, &r.body).unwrap();
    serde_json::from_slice(&plain).unwrap()
}

#[test]
fn a_new_entry_is_one_sealed_record_under_its_link_id_and_an_unchanged_one_is_not_written_again() {
    let mut a = machine("a", 1);
    a.store.vault_insert(&row("entry-1", "Invoice provider key", VAULT, "{}")).unwrap();
    assert_eq!(a.capture(VAULT, NOW), Captured { written: 1, ..Captured::default() });
    let first = a.record(VAULT, "entry-1").expect("the entry has a record");
    assert!(sync_links::is_link_id(&first.header.record), "the record is named by the link id, not the entry's own id");
    // The log holds the label and the value only sealed.
    let text = String::from_utf8_lossy(&first.body).to_string();
    assert!(!text.contains("Invoice provider key") && !text.contains(VALUE));
    let body = opened(&a.ring, &first);
    assert_eq!((body.label.as_str(), body.parts.get("secret").map(String::as_str)), ("Invoice provider key", Some(VALUE)));
    assert_eq!(body.link_ids, vec![first.header.record.clone()]);
    assert_eq!(a.capture(VAULT, NOW + 10), Captured::default(), "nothing changed");
    // An edit is a new version on top of the first.
    let mut edited = a.store.vault_get("entry-1").unwrap().unwrap();
    edited.label = "Invoice provider key (live)".into();
    a.store.vault_update(&edited).unwrap();
    assert_eq!(a.capture(VAULT, NOW + 20), Captured { written: 1, ..Captured::default() });
    let second = a.record(VAULT, "entry-1").unwrap();
    assert_eq!(second.header.base.as_ref(), Some(&first.header.version));
    assert_eq!(opened(&a.ring, &second).label, "Invoice provider key (live)");
}

#[test]
fn what_is_kept_to_tell_a_change_is_keyed_not_a_bare_hash_of_the_body() {
    let (one, two) = (Zeroizing::new([1u8; 32]), Zeroizing::new([2u8; 32]));
    let body = b"{\"secret\":\"guessable\"}";
    assert_eq!(digest(&one, body), digest(&one, body));
    assert_ne!(digest(&one, body), digest(&two, body), "another machine's key gives another digest");
    let bare: String = ring::digest::digest(&ring::digest::SHA256, body).as_ref().iter().map(|b| format!("{b:02x}")).collect();
    assert_ne!(digest(&one, body), bare, "and it is not the hash of the body");
}

#[test]
fn an_entry_that_is_deleted_ends_its_record_once() {
    let mut a = machine("a", 1);
    a.store.vault_insert(&row("entry-1", "Invoice provider key", VAULT, "{}")).unwrap();
    a.capture(VAULT, NOW);
    a.store.vault_delete("entry-1").unwrap();
    assert_eq!(a.capture(VAULT, NOW + 10), Captured { tombstoned: 1, ..Captured::default() });
    assert_eq!(a.capture(VAULT, NOW + 20), Captured::default());
}

#[test]
fn what_is_in_another_vault_or_stays_on_this_machine_is_not_written() {
    let mut a = machine("a", 1);
    a.store.vault_insert(&row("elsewhere", "Other Vault key", "vault-b", "{}")).unwrap();
    // A sign-in for a profile this machine cannot name stays here.
    a.store.vault_insert(&row("signin", "A sign-in", VAULT, &format!("{{\"{PROFILE}\":\"profile:agent-own\"}}"))).unwrap();
    assert_eq!(a.capture(VAULT, NOW), Captured::default());
    assert!(a.record(VAULT, "signin").is_none() && a.record("vault-b", "elsewhere").is_none());
}

#[test]
fn an_entry_moved_to_another_vault_is_written_there_and_ended_in_the_first() {
    let mut a = machine("a", 1);
    a.store.vault_insert(&row("entry-1", "Invoice provider key", VAULT, "{}")).unwrap();
    a.capture(VAULT, NOW);
    let link = a.store.vault_link_primary("entry-1").unwrap().unwrap();
    assert!(a.store.vault_move_entry("entry-1", "vault-b").unwrap());
    // The first Vault's capture leaves it alone: it still exists, in another Vault.
    assert_eq!(a.capture(VAULT, NOW + 10), Captured::default());
    // The second writes it and ends the record in the first.
    assert_eq!(a.capture("vault-b", NOW + 20), Captured { written: 1, tombstoned: 1, ..Captured::default() });
    let old = log::current(&a.store.conn().lock().unwrap(), VAULT, &link).unwrap().unwrap();
    assert!(old.header.tombstone);
    assert_eq!(a.capture("vault-b", NOW + 30), Captured::default());
    assert_eq!(a.capture(VAULT, NOW + 30), Captured::default(), "and the first does not end it a second time");
}

#[test]
fn another_machine_takes_the_record_through_the_layer_and_opens_it() {
    let mut a = machine("a", 1);
    let b = machine("b", 2);
    a.store.vault_insert(&row("entry-1", "Invoice provider key", VAULT, "{\"site\":\"example.com\"}")).unwrap();
    a.capture(VAULT, NOW);
    let all: Pins = [(a.id.clone(), a.public), (b.id.clone(), b.public)].into_iter().collect();
    let conn = b.store.conn().lock().unwrap();
    log::ensure_collection(&conn, VAULT, KIND, NOW).unwrap();
    let mut clock = Clock::new("b");
    let adapter = VaultAdapter { conn: &conn, rotations: &Never };
    let sent = log::since(&a.store.conn().lock().unwrap(), VAULT, &log::vector(&conn, VAULT).unwrap(), 100).unwrap();
    let got: Vec<_> = sent.iter().map(|r| log::apply_remote(&conn, r, &all, &mut clock, NOW + 5, &adapter).unwrap()).collect();
    assert_eq!(got, [Outcome::Applied]);
    let held = log::current(&conn, VAULT, &sent[0].header.record).unwrap().unwrap();
    let body = opened(&b.ring, &held);
    assert_eq!((body.label.as_str(), body.fields.get("site").map(String::as_str)), ("Invoice provider key", Some("example.com")));
}

fn made(entry: &str, label: &str, action: &str) -> VaultActivityRow {
    VaultActivityRow { at: NOW, entry_id: entry.into(), entry_label: label.into(), actor: "you".into(), action: action.into(), target: "example.com".into(), actor_kind: "person".into(), host: "a".into(), ..Default::default() }
}

impl Machine {
    fn line_record(&self, vault: &str, seq: i64) -> Option<Record> {
        log::current(&self.store.conn().lock().unwrap(), vault, &sync_activity::record_id(&self.id, seq)).unwrap()
    }
}

#[test]
fn an_activity_line_of_an_entry_is_written_once_sealed_and_a_line_with_no_entry_stays_here() {
    let mut a = machine("a", 1);
    a.store.vault_insert(&row("entry-1", "Invoice provider key", VAULT, "{}")).unwrap();
    let first = a.store.vault_activity_add(&made("entry-1", "Invoice provider key", "revealed")).unwrap();
    a.store.vault_activity_add(&made("", "", "locked")).unwrap();
    assert_eq!(a.capture(VAULT, NOW), Captured { written: 1, lines: 1, ..Captured::default() });
    let record = a.line_record(VAULT, first.seq).expect("the line is a record named for the machine and its number");
    assert!(!String::from_utf8_lossy(&record.body).contains("Invoice provider key"), "the label is sealed");
    let link = a.store.vault_link_primary("entry-1").unwrap().unwrap();
    assert_eq!(opened_body::<sync_activity::ActivityBody>(&a.ring, &record).entry_id, link, "the line names its entry by its link id");
    assert!(a.line_record(VAULT, first.seq + 1).is_none(), "a line of no entry is not written");
    // Nothing new, nothing written; a later line is.
    assert_eq!(a.capture(VAULT, NOW + 10), Captured::default());
    let later = a.store.vault_activity_add(&made("entry-1", "Invoice provider key", "copied")).unwrap();
    assert_eq!(a.capture(VAULT, NOW + 20), Captured { lines: 1, ..Captured::default() });
    assert!(a.line_record(VAULT, later.seq).is_some());
}

#[test]
fn the_last_lines_of_an_entry_that_was_deleted_are_written_and_a_line_of_another_vault_is_not() {
    let mut a = machine("a", 1);
    a.store.vault_insert(&row("entry-1", "Invoice provider key", VAULT, "{}")).unwrap();
    a.store.vault_insert(&row("elsewhere", "Other Vault key", "vault-b", "{}")).unwrap();
    a.capture(VAULT, NOW);
    let gone = a.store.vault_activity_add(&made("entry-1", "Invoice provider key", "deleted")).unwrap();
    let other = a.store.vault_activity_add(&made("elsewhere", "Other Vault key", "revealed")).unwrap();
    a.store.vault_delete("entry-1").unwrap();
    assert_eq!(a.capture(VAULT, NOW + 10), Captured { tombstoned: 1, lines: 1, ..Captured::default() });
    assert!(a.line_record(VAULT, gone.seq).is_some(), "the delete's own line goes with the tombstone");
    assert!(a.line_record(VAULT, other.seq).is_none(), "a line of an entry in another Vault does not go here");
    assert_eq!(a.capture("vault-b", NOW + 20), Captured { written: 1, lines: 1, ..Captured::default() });
    assert!(a.line_record("vault-b", other.seq).is_some());
}

/// The grant's record: named for the entry's link id and the site, not for the id the entry has here.
fn grant_record(m: &Machine, vault: &str, entry: &str, origin: &str) -> Option<Record> {
    let link = m.store.vault_link_primary(entry).unwrap().expect("the entry has a link id");
    log::current(&m.store.conn().lock().unwrap(), vault, &sync_grant::record_id(&link, origin)).unwrap()
}

fn opened_body<T: serde::de::DeserializeOwned>(ring: &KeyRing, r: &Record) -> T {
    let h = &r.header;
    let place = Place { vault: &h.collection, record: &h.record, version: &h.version, base: h.base.as_ref(), epoch: h.epoch };
    sync_seal::open_record(ring.get(h.epoch).unwrap(), &place, &r.body).unwrap()
}

#[test]
fn a_site_grant_is_written_once_sealed_and_ended_when_it_is_revoked() {
    let mut a = machine("a", 1);
    a.store.vault_insert(&row("entry-1", "Invoice provider key", VAULT, "{}")).unwrap();
    a.store.vault_grant_add("entry-1", "https://billing.example.com", NOW, 1).unwrap();
    assert_eq!(a.capture(VAULT, NOW), Captured { written: 1, grants: 1, ..Captured::default() });
    let record = grant_record(&a, VAULT, "entry-1", "https://billing.example.com").expect("a record named for the entry and the site");
    assert!(!String::from_utf8_lossy(&record.body).contains("billing.example.com"), "the site is sealed, the id says neither");
    let link = a.store.vault_link_primary("entry-1").unwrap().unwrap();
    assert_eq!(opened_body::<sync_grant::GrantBody>(&a.ring, &record).entry_id, link, "the grant names its entry by its link id");
    assert_eq!(a.capture(VAULT, NOW + 10), Captured::default(), "an unchanged grant is not written again");
    a.store.vault_grant_revoke("entry-1", "https://billing.example.com").unwrap();
    assert_eq!(a.capture(VAULT, NOW + 20), Captured { tombstoned: 1, ..Captured::default() });
    assert!(grant_record(&a, VAULT, "entry-1", "https://billing.example.com").unwrap().header.tombstone);
    assert_eq!(a.capture(VAULT, NOW + 30), Captured::default(), "the revoke is written once");
    // Granted again: a newer version of the same record.
    a.store.vault_grant_add("entry-1", "https://billing.example.com", NOW + 40, 1).unwrap();
    assert_eq!(a.capture(VAULT, NOW + 40), Captured { grants: 1, ..Captured::default() });
    assert!(!grant_record(&a, VAULT, "entry-1", "https://billing.example.com").unwrap().header.tombstone);
}

#[test]
fn a_grant_follows_its_entry_out_of_a_vault_and_one_of_an_entry_that_stays_here_is_kept() {
    let mut a = machine("a", 1);
    a.store.vault_insert(&row("entry-1", "Invoice provider key", VAULT, "{}")).unwrap();
    let own = format!("{{\"{PROFILE}\":\"profile:agent-own\"}}");
    a.store.vault_insert(&row("signin", "A sign-in", VAULT, &own)).unwrap();
    a.store.vault_grant_add("entry-1", "https://billing.example.com", NOW, 1).unwrap();
    a.store.vault_grant_add("signin", "https://billing.example.com", NOW, 1).unwrap();
    assert_eq!(a.capture(VAULT, NOW).grants, 1, "the grant of an entry that stays here does not go");
    assert!(a.store.vault_link_primary("signin").unwrap().is_none());
    // The entry moves to another Vault: its grant ends here and is written there.
    a.store.vault_move_entry("entry-1", "vault-b").unwrap();
    assert_eq!(a.capture(VAULT, NOW + 10).tombstoned, 1);
    assert!(grant_record(&a, VAULT, "entry-1", "https://billing.example.com").unwrap().header.tombstone);
    assert_eq!(a.capture("vault-b", NOW + 20).grants, 1);
    assert!(!grant_record(&a, "vault-b", "entry-1", "https://billing.example.com").unwrap().header.tombstone);
}
