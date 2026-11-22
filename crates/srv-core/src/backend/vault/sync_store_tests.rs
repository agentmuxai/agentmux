// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The Vault's sync rules against the store: link ids that open an entry from another machine, none reassigned or reused,
//! and activity lines other machines made. Two stores stand for two machines. Every value is invented.

use super::*;
use crate::backend::storage::vault::{VaultActivityRow, VaultRow};

const A: &str = "e_aaaaaaaaaaaaaaaa";
const B: &str = "e_bbbbbbbbbbbbbbbb";
const C: &str = "e_cccccccccccccccc";

fn machine() -> Store {
    let s = Store::open_in_memory().unwrap();
    s.apply_identity_schema_for_tests().unwrap();
    s
}

fn entry(s: &Store, id: &str) {
    let row = VaultRow {
        id: id.into(),
        kind: "api-key".into(),
        label: "Invoice provider key".into(),
        fields: "{}".into(),
        tags: "[]".into(),
        sealed: vec![1],
        created_at: 1,
        updated_at: 1,
        last_used_at: 0,
        vault_id: "vault-local".into(),
    };
    s.vault_insert(&row).unwrap();
}

fn ids(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn a_link_made_on_either_machine_opens_the_entry_on_both() {
    let (one, two) = (machine(), machine());
    for s in [&one, &two] {
        entry(s, "entry-1");
    }
    // Each machine gave the entry an id before they met.
    one.vault_link_id_assign("entry-1", A).unwrap();
    two.vault_link_id_assign("entry-1", B).unwrap();
    let from_one = links_of(&one, "entry-1").unwrap().ids();
    let from_two = links_of(&two, "entry-1").unwrap().ids();
    apply_links(&one, "entry-1", &from_two).unwrap();
    apply_links(&two, "entry-1", &from_one).unwrap();
    for (s, own) in [(&one, A), (&two, B)] {
        assert_eq!(s.vault_link_primary("entry-1").unwrap().as_deref(), Some(own), "the id it showed stays its own");
        assert_eq!(s.vault_entry_of_link(A).unwrap().as_deref(), Some("entry-1"));
        assert_eq!(s.vault_entry_of_link(B).unwrap().as_deref(), Some("entry-1"));
    }
    // Hearing it again changes nothing.
    let again = apply_links(&one, "entry-1", &from_two).unwrap();
    assert_eq!(again, links_of(&one, "entry-1").unwrap());
}

#[test]
fn an_id_held_by_another_entry_is_never_reassigned() {
    let s = machine();
    entry(&s, "entry-1");
    entry(&s, "entry-2");
    s.vault_link_id_assign("entry-2", A).unwrap();
    s.vault_link_id_assign("entry-1", B).unwrap();
    let got = apply_links(&s, "entry-1", &ids(&[A, C])).unwrap();
    assert_eq!((got.primary.as_deref(), got.aliases), (Some(B), ids(&[C])));
    assert_eq!(s.vault_entry_of_link(A).unwrap().as_deref(), Some("entry-2"), "entry-2 keeps it");
    // Even a write that skipped the rules cannot give it away: the table holds an id once.
    s.vault_links_set("entry-1", None, &ids(&[A])).unwrap();
    assert!(s.vault_link_aliases("entry-1").unwrap().is_empty());
    assert_eq!(s.vault_link_holder(A).unwrap().as_deref(), Some("entry-2"));
}

#[test]
fn a_removed_entrys_ids_are_not_reused_and_no_longer_open_anything() {
    let s = machine();
    entry(&s, "entry-1");
    s.vault_link_id_assign("entry-1", A).unwrap();
    apply_links(&s, "entry-1", &ids(&[B])).unwrap();
    assert!(s.vault_delete("entry-1").unwrap());
    assert_eq!(s.vault_entry_of_link(A).unwrap(), None);
    assert_eq!(s.vault_entry_of_link(B).unwrap(), None);
    assert!(s.vault_link_retired(A).unwrap() && s.vault_link_retired(B).unwrap());
    // Another entry is not given them, by a record or by a write that skipped the rules.
    entry(&s, "entry-2");
    let got = apply_links(&s, "entry-2", &ids(&[A, B, C])).unwrap();
    assert_eq!((got.primary.as_deref(), got.aliases), (Some(C), vec![]));
    s.vault_links_set("entry-2", Some(A), &ids(&[B])).unwrap();
    assert_eq!(s.vault_entry_of_link(A).unwrap(), None);
    assert!(s.vault_link_aliases("entry-2").unwrap().is_empty());
}

fn line() -> ActivityBody {
    ActivityBody {
        at: 5_000,
        entry_id: "entry-1".into(),
        entry_label: "Invoice provider key".into(),
        actor: "you".into(),
        action: "revealed".into(),
        target: String::new(),
        outcome: "ok".into(),
        version: 2,
        actor_kind: "person".into(),
        host: "machine-b".into(),
        reason: String::new(),
        refs: String::new(),
    }
}

#[test]
fn another_machines_activity_line_is_added_once_and_outside_this_machines_chain() {
    let s = machine();
    let own = s.vault_activity_add(&VaultActivityRow { at: 4_000, entry_id: "entry-1".into(), action: "created".into(), host: "machine-a".into(), ..Default::default() }).unwrap();
    assert_eq!(apply_activity(&s, "machine-b", 7, &line()).unwrap(), Decision::Applied);
    assert_eq!(apply_activity(&s, "machine-b", 7, &line()).unwrap(), Decision::KeptLocal, "the same line again");
    // The same sequence number on another machine is another line.
    assert_eq!(apply_activity(&s, "machine-c", 7, &line()).unwrap(), Decision::Applied);

    let theirs = s.vault_activity_remote_list(10).unwrap();
    assert_eq!(theirs.len(), 2);
    let (who, row) = &theirs[0];
    assert_eq!((who.as_str(), row.seq, row.action.as_str(), row.host.as_str()), ("machine-b", 7, "revealed", "machine-b"));
    assert!(row.hash.is_empty() && row.prev_hash.is_empty());

    // This machine's own lines and their chain are as they were.
    let mine = s.vault_activity_list(10).unwrap();
    assert_eq!(mine.len(), 1);
    assert_eq!((mine[0].seq, mine[0].hash.as_str()), (own.seq, own.hash.as_str()));
}
