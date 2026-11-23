// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A Vault's entries written into the sync layer's log: one record per entry under its link id, sealed, only when the entry
//! changed, with a tombstone when it is gone. Two machines stand for two stores. Every value is invented.

use std::collections::BTreeMap;

use agentmux_common::jekt_sign::generate_lan_keypair;
use zeroize::Zeroizing;

use super::*;
use crate::backend::storage::vault::{VaultInfoRow, VaultRow};
use crate::backend::user_sync::hlc::Clock;
use crate::backend::user_sync::log::{Outcome, Pins};
use crate::backend::user_sync::record::Record;
use crate::backend::vault::seal::Parts;
use crate::backend::vault::signins::PROFILE;
use crate::backend::vault::sync_adapter::{Rotations, VaultAdapter};
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
    assert_eq!(a.capture(VAULT, NOW), Captured { written: 1, tombstoned: 0 });
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
    assert_eq!(a.capture(VAULT, NOW + 20), Captured { written: 1, tombstoned: 0 });
    let second = a.record(VAULT, "entry-1").unwrap();
    assert_eq!(second.header.base.as_ref(), Some(&first.header.version));
    assert_eq!(opened(&a.ring, &second).label, "Invoice provider key (live)");
}

#[test]
fn an_entry_that_is_deleted_ends_its_record_once() {
    let mut a = machine("a", 1);
    a.store.vault_insert(&row("entry-1", "Invoice provider key", VAULT, "{}")).unwrap();
    a.capture(VAULT, NOW);
    a.store.vault_delete("entry-1").unwrap();
    assert_eq!(a.capture(VAULT, NOW + 10), Captured { written: 0, tombstoned: 1 });
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
    assert_eq!(a.capture("vault-b", NOW + 20), Captured { written: 1, tombstoned: 1 });
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
