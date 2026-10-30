// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::vault::{activity, create, delete, list_for_pane, reveal, update, EntryChange, NewEntry};

fn store() -> Store {
    seal::use_test_key();
    let s = Store::open_in_memory().unwrap();
    s.apply_identity_schema_for_tests().unwrap();
    s
}

fn key(value: &str) -> NewEntry {
    NewEntry {
        kind: "api-key".into(),
        label: "Stripe".into(),
        fields: Default::default(),
        tags: vec![],
        parts: seal::Parts([("value".to_string(), value.to_string())].into()),
    }
}

fn set(s: &Store, id: &str, value: Option<&str>, notes: &str) {
    let change = EntryChange {
        id: id.into(),
        label: "Stripe".into(),
        fields: [("notes".to_string(), notes.to_string())].into(),
        tags: vec![],
        parts: seal::Parts(value.map(|v| ("value".to_string(), v.to_string())).into_iter().collect()),
        clear: vec![],
    };
    update(s, change).unwrap();
}

fn current(s: &Store, id: &str) -> String {
    reveal(s, id, "value", "revealed").unwrap().to_string()
}

#[test]
fn a_new_value_keeps_the_old_one_and_a_label_change_doesnt() {
    let s = store();
    let id = create(&s, key("sk_live_first_value")).unwrap();
    set(&s, &id, None, "no new value");
    assert!(list(&s, &id).unwrap().is_empty(), "nothing was replaced");
    set(&s, &id, Some("sk_live_second_value"), "");
    let versions = list(&s, &id).unwrap();
    assert_eq!(versions.len(), 1);
    assert_eq!((versions[0].tail.as_str(), versions[0].replaced_by.as_str(), versions[0].how.as_str()), ("alue", "you", "replaced"));
    assert_eq!(current(&s, &id), "sk_live_second_value", "only the new one is current");
    assert_eq!(list_for_pane(&s).unwrap().0[0].versions, 1);
    let shown = serde_json::to_string(&versions).unwrap();
    assert!(!shown.contains("first_value"), "no value in the list: {shown}");
}

#[test]
fn restoring_puts_the_old_value_back_and_keeps_the_one_it_replaces() {
    let s = store();
    let id = create(&s, key("sk_live_first_value")).unwrap();
    set(&s, &id, Some("sk_live_second_value"), "");
    let seq = list(&s, &id).unwrap()[0].seq;
    restore(&s, &id, seq).unwrap();
    assert_eq!(current(&s, &id), "sk_live_first_value");
    let versions = list(&s, &id).unwrap();
    assert_eq!(versions.len(), 1, "the restored one left the list, the replaced one joined it");
    assert_eq!(versions[0].how, "restored");
    restore(&s, &id, versions[0].seq).unwrap();
    assert_eq!(current(&s, &id), "sk_live_second_value", "and back again");
    assert!(restore(&s, &id, seq).is_err(), "a version is restored once");
    let line = activity(&s, 20).unwrap().into_iter().find(|l| l.action == "restored").unwrap();
    assert!(line.target.starts_with("the value replaced "), "{}", line.target);
    assert!(!serde_json::to_string(&activity(&s, 20).unwrap()).unwrap().contains("first_value"));
}

#[test]
fn versions_go_after_seven_days_and_with_their_entry() {
    let s = store();
    let id = create(&s, key("sk_live_first_value")).unwrap();
    set(&s, &id, Some("sk_live_second_value"), "");
    let row = s.vault_get(&id).unwrap().unwrap();
    let old = VaultVersionRow {
        seq: 0,
        entry_id: id.clone(),
        entry_created_at: row.created_at,
        sealed: row.sealed.clone(),
        replaced_at: now_ms() - KEEP_MS - 1,
        replaced_by: "you".into(),
        how: "replaced".into(),
        state: PREVIOUS.into(),
    };
    s.vault_version_add(&old).unwrap();
    assert_eq!(list(&s, &id).unwrap().len(), 1, "the one past 7 days is gone");
    delete(&s, &id).unwrap();
    assert!(s.vault_versions(&id, row.created_at).unwrap().is_empty());
}

#[test]
fn an_entry_made_again_under_a_reused_id_has_no_earlier_values() {
    let s = store();
    let id = create(&s, key("sk_live_first_value")).unwrap();
    set(&s, &id, Some("sk_live_second_value"), "");
    let first = list(&s, &id).unwrap()[0].seq;
    delete(&s, &id).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));
    let again = create(&s, key("sk_live_third_value")).unwrap();
    assert_eq!(again, id);
    assert!(list(&s, &id).unwrap().is_empty());
    assert!(restore(&s, &id, first).is_err());
}

#[test]
fn a_restore_from_a_stale_read_never_overwrites_a_value_written_since() {
    let s = store();
    let id = create(&s, key("sk_live_first_value")).unwrap();
    set(&s, &id, Some("sk_live_second_value"), "");
    let seq = list(&s, &id).unwrap()[0].seq;
    // What restore read, before a capture or an edit wrote a third value.
    let stale = s.vault_get(&id).unwrap().unwrap();
    set(&s, &id, Some("sk_live_third_value"), "");
    let previous = VaultVersionRow {
        seq: 0,
        entry_id: id.clone(),
        entry_created_at: stale.created_at,
        sealed: stale.sealed,
        replaced_at: 1,
        replaced_by: "you".into(),
        how: "restored".into(),
        state: PREVIOUS.into(),
    };
    assert!(!s.vault_version_swap(&id, seq, &previous, 2, 0).unwrap(), "the entry changed since it was read");
    assert_eq!(current(&s, &id), "sk_live_third_value");
    assert_eq!(list(&s, &id).unwrap().len(), 2, "both earlier values are still kept, and nothing stale was added");
}

#[test]
fn an_edit_from_a_stale_read_saves_nothing_and_keeps_no_stray_version() {
    let s = store();
    let id = create(&s, key("sk_live_first_value")).unwrap();
    let read = s.vault_get(&id).unwrap().unwrap();
    // A capture or restore writes a new value after the edit form read the entry.
    set(&s, &id, Some("sk_live_captured_value"), "");
    let edited = crate::backend::storage::vault::VaultRow { label: "Renamed".into(), updated_at: read.updated_at + 1, ..read.clone() };
    let kept = of(&read, "you", "replaced");
    assert!(!s.vault_update_kept(&edited, &read.sealed, Some(&kept)).unwrap());
    assert_eq!(current(&s, &id), "sk_live_captured_value");
    assert_eq!(s.vault_get(&id).unwrap().unwrap().label, "Stripe");
    assert_eq!(list(&s, &id).unwrap().len(), 1, "only the write that went through kept a version");
}

#[test]
fn a_version_past_its_seven_days_is_never_put_back_even_before_it_is_pruned() {
    let s = store();
    let id = create(&s, key("sk_live_first_value")).unwrap();
    let row = s.vault_get(&id).unwrap().unwrap();
    // The pane listed it a moment before it expired; nothing has pruned since.
    let expired = VaultVersionRow {
        seq: 0,
        entry_id: id.clone(),
        entry_created_at: row.created_at,
        sealed: row.sealed.clone(),
        replaced_at: now_ms() - KEEP_MS - 1,
        replaced_by: "you".into(),
        how: "replaced".into(),
        state: PREVIOUS.into(),
    };
    let seq = s.vault_version_add(&expired).unwrap();
    let err = restore(&s, &id, seq).unwrap_err();
    assert!(err.contains("past its 7 days"), "{err}");
}
