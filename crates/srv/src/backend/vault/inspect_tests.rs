// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Vault` `show`, `update`, `verify` and `health`: what each answers, who may ask, and that none holds a value
//! (docs/specs/SPEC_VAULT_PARITY_ENTRY_HISTORY_REASONS_AND_IMPORT_2026_11_06.md §8, §11).

use chrono::NaiveDate;
use zeroize::Zeroizing;

use super::*;
use crate::backend::vault::{signins, signups};
use crate::backend::vault::history::Asker;
use crate::backend::vault::vaults::VaultAgents;
use crate::backend::vault::{create, create_in, reveal, update, EntryChange, NewEntry};

const VALUE: &str = "inspect-sentinel-5521-zz";
const NOTE: &str = "inspect-note-sentinel-77";
const REASON: &str = "keep the staging key's details tidy for task 28072";

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, 7).unwrap()
}

fn store() -> Store {
    seal::use_test_key();
    let s = Store::open_in_memory().unwrap();
    s.apply_identity_schema_for_tests().unwrap();
    s
}

fn key() -> KeyResult {
    Ok(seal::test_key())
}

fn entry(label: &str, fields: &[(&str, &str)]) -> NewEntry {
    NewEntry {
        kind: "api-key".into(),
        label: label.into(),
        fields: fields.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        tags: vec![],
        parts: seal::Parts([("value".to_string(), VALUE.to_string())].into()),
    }
}

fn asked(id: &str) -> UpdateRequest {
    UpdateRequest { id: id.into(), label: None, fields: BTreeMap::new(), tags: None, reason: Some(REASON.into()) }
}

fn no_value(texts: &[String]) {
    for t in texts {
        assert!(!t.contains(VALUE), "the value is in {t}");
    }
}

#[test]
fn show_gives_names_states_and_counts_and_no_value() {
    let s = store();
    let id = create(&s, entry("Stripe", &[("service", "stripe"), ("expires", "2026-11-10")])).unwrap();
    let shown = show(&s, &id, Some("builder"), &key(), today()).unwrap();
    assert_eq!((shown.kind.as_str(), shown.label.as_str(), shown.vault.as_str()), ("api-key", "Stripe", "Personal"));
    assert_eq!((shown.parts_set.as_slice(), shown.readable, shown.value_version, shown.kept_versions), (["value".to_string()].as_slice(), true, 1, 0));
    assert_eq!((shown.expires.as_deref(), shown.expiry, shown.days_left), (Some("2026-11-10"), "soon", Some(3)));
    assert_eq!(shown.fields.get("service").map(String::as_str), Some("stripe"));
    assert_eq!(shown.last_used_at, 0);

    // A new value is the next version and the old one is kept.
    update(&s, EntryChange { id: id.clone(), label: "Stripe".into(), fields: BTreeMap::new(), tags: vec![], parts: seal::Parts([("value".to_string(), "second".to_string())].into()), clear: vec![] }).unwrap();
    let again = show(&s, &id, Some("builder"), &key(), today()).unwrap();
    assert_eq!((again.value_version, again.kept_versions), (2, 1));
    no_value(&[serde_json::to_string(&shown).unwrap(), serde_json::to_string(&again).unwrap()]);
}

#[test]
fn show_says_so_when_the_key_cant_open_the_entry_and_hides_vaults_the_agent_may_not_use() {
    let s = store();
    let id = create(&s, entry("Stripe", &[])).unwrap();
    let blind = show(&s, &id, Some("builder"), &Err("the key store is locked".into()), today()).unwrap();
    assert!(!blind.readable && blind.parts_set.is_empty());

    let closed = vaults::create(&s, "Closed", VaultAgents::None).unwrap();
    let theirs = create_in(&s, entry("Theirs", &[]), &closed.id).unwrap();
    assert_eq!(show(&s, &theirs, Some("builder"), &key(), today()).unwrap_err(), format!("no entry {theirs}"));
    assert_eq!(show(&s, "nothing-here", Some("builder"), &key(), today()).unwrap_err(), "no entry nothing-here");
    assert!(show(&s, &theirs, None, &key(), today()).is_err(), "an unproven caller sees only the Vaults open to everyone");
}

#[test]
fn update_changes_details_records_one_line_with_the_reason_and_leaves_the_value() {
    let s = store();
    let id = create(&s, entry("Stripe", &[("service", "stripe")])).unwrap();
    let req = UpdateRequest {
        label: Some("Stripe live".into()),
        fields: [("service".to_string(), "stripe-live".to_string()), ("notes".to_string(), NOTE.to_string()), ("expires".to_string(), "2027-01-31".to_string())].into(),
        tags: Some(vec!["payments".into()]),
        ..asked(&id)
    };
    let done = update_details(&s, &req, "builder").unwrap();
    assert_eq!(done.label, "Stripe live");
    assert_eq!(done.changed, ["label", "expires 2027-01-31", "field notes", "field service", "tags"]);

    assert_eq!(reveal(&s, &id, "value", "revealed").unwrap().as_str(), VALUE, "the value is as it was");
    let rows = history::history_for(&s, &id, Asker::Person, 20).unwrap();
    let line = rows.iter().find(|r| r.action == CHANGED_DETAILS).expect("a changed details line");
    assert_eq!((line.actor.as_str(), line.outcome.as_str(), line.reason.as_str(), line.version), ("builder", "done", REASON, 0));
    assert_eq!(line.target, "label, expires 2027-01-31, field notes, field service, tags");
    assert!(!line.target.contains(NOTE) && !line.target.contains("stripe-live"), "a field's value is not in the line");
    let shown = show(&s, &id, Some("builder"), &key(), today()).unwrap();
    assert_eq!((shown.label.as_str(), shown.tags.as_slice(), shown.value_version), ("Stripe live", ["payments".to_string()].as_slice(), 1));
}

#[test]
fn an_empty_field_value_removes_it_and_a_change_that_changes_nothing_is_refused() {
    let s = store();
    let id = create(&s, entry("Stripe", &[("service", "stripe"), ("expires", "2027-01-31")])).unwrap();
    let removed = update_details(&s, &UpdateRequest { fields: [("expires".to_string(), "  ".to_string())].into(), ..asked(&id) }, "builder").unwrap();
    assert_eq!(removed.changed, ["no expiry date"]);
    assert_eq!(show(&s, &id, Some("builder"), &key(), today()).unwrap().expiry, "none");
    let err = update_details(&s, &asked(&id), "builder").unwrap_err();
    assert!(err.starts_with("nothing to change"), "{err}");
}

#[test]
fn update_refuses_without_a_reason_a_field_the_kind_lacks_a_bad_date_and_the_persons_own_settings() {
    let s = store();
    let id = create(&s, entry("Stripe", &[])).unwrap();
    let with = |fields: &[(&str, &str)]| UpdateRequest { fields: fields.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(), ..asked(&id) };
    for reason in [None, Some("".to_string()), Some("tidy".to_string())] {
        assert!(update_details(&s, &UpdateRequest { reason, label: Some("New".into()), ..asked(&id) }, "builder").unwrap_err().contains("reason"));
    }
    assert!(update_details(&s, &with(&[("owner", "x")]), "builder").unwrap_err().contains("no field"));
    assert!(update_details(&s, &with(&[("expires", "next week")]), "builder").is_err());

    let login = create(&s, NewEntry { kind: "login".into(), label: "Intranet".into(), fields: Default::default(), tags: vec![], parts: seal::Parts([("password".to_string(), "pw-1".to_string())].into()) }).unwrap();
    for field in ["site", "realm", "profile", "proxy"] {
        let err = update_details(&s, &UpdateRequest { fields: [(field.to_string(), "yes".to_string())].into(), ..asked(&login) }, "builder").unwrap_err();
        assert!(err.contains("the person sets it"), "{field}: {err}");
    }
    assert_eq!(history::history_for(&s, &id, Asker::Person, 20).unwrap().iter().filter(|r| r.action == CHANGED_DETAILS).count(), 0, "a refused change leaves no line");
}

#[test]
fn an_agent_cannot_add_or_clear_the_tags_agentmux_sets_to_keep_a_sign_in_from_being_offered() {
    let s = store();
    let refused = signins::REFUSED.to_string();
    let id = create(&s, NewEntry { tags: vec![refused.clone(), "work".into()], ..entry("Stripe", &[]) }).unwrap();

    // Replacing the tags keeps the reserved one the entry has.
    let done = update_details(&s, &UpdateRequest { tags: Some(vec!["payments".into()]), ..asked(&id) }, "builder").unwrap();
    assert_eq!(done.changed, ["tags"]);
    let shown = show(&s, &id, Some("builder"), &key(), today()).unwrap();
    assert_eq!(shown.tags, ["payments".to_string(), refused.clone()]);
    // Clearing every tag leaves it too, so that is not a change.
    let cleared = update_details(&s, &UpdateRequest { tags: Some(vec![]), ..asked(&id) }, "builder").unwrap();
    assert_eq!(cleared.changed, ["tags"], "only the agent's own tag went");
    assert_eq!(show(&s, &id, Some("builder"), &key(), today()).unwrap().tags, [refused]);

    // Asking for a reserved tag the entry lacks is refused.
    let plain = create(&s, entry("Plain", &[])).unwrap();
    let err = update_details(&s, &UpdateRequest { tags: Some(vec![signups::UNCONFIRMED.to_string()]), ..asked(&plain) }, "builder").unwrap_err();
    assert!(err.contains("set by AgentMux"), "{err}");
}

#[test]
fn update_works_only_on_an_entry_in_a_vault_the_agent_may_use() {
    let s = store();
    let closed = vaults::create(&s, "Closed", VaultAgents::None).unwrap();
    let theirs = create_in(&s, entry("Theirs", &[]), &closed.id).unwrap();
    let err = update_details(&s, &UpdateRequest { label: Some("Mine now".into()), ..asked(&theirs) }, "builder").unwrap_err();
    assert_eq!(err, format!("no entry {theirs}"));
}

#[test]
fn verify_passes_a_sound_entry_and_names_what_is_wrong_with_the_rest_without_a_value() {
    let s = store();
    let good = create(&s, entry("Good", &[("expires", "2027-01-31")])).unwrap();
    let all = verify(&s, None, Some("builder"), &key(), today()).unwrap();
    assert_eq!((all.checked, all.ok, all.problems.len()), (1, true, 0), "{all:?}");
    assert_eq!(verify(&s, Some(&good), Some("builder"), &key(), today()).unwrap().checked, 1);

    // An unset required part, an expired entry, a field the kind doesn't have and a date that isn't one.
    let bare = create(&s, entry("Bare", &[])).unwrap();
    let k = seal::test_key();
    let row = s.vault_get(&bare).unwrap().unwrap();
    let empty = VaultRow { sealed: seal::seal(&k, &bare, &seal::Parts::default()).unwrap(), ..row.clone() };
    assert!(s.vault_update_kept(&empty, &row.sealed, None).unwrap());
    let old = create(&s, entry("Old", &[("expires", "2026-11-01")])).unwrap();
    let odd = create(&s, entry("Odd", &[])).unwrap();
    s.vault_run_sql_for_tests(&format!("UPDATE db_vault_entries SET fields = '{{\"nope\":\"{NOTE}\",\"expires\":\"{NOTE}\"}}' WHERE id = '{odd}'"));

    let got = verify(&s, None, Some("builder"), &key(), today()).unwrap();
    assert!(!got.ok);
    let said = |id: &str| got.problems.iter().filter(|p| p.id == id).map(|p| p.problem.clone()).collect::<Vec<_>>();
    assert_eq!(said(&bare), ["its value isn't set"], "{:?}", got.problems);
    assert!(said(&old)[0].contains("expired on 2026-11-01"), "{:?}", said(&old));
    let odd_said = said(&odd).join("; ");
    assert!(odd_said.contains("field a api-key entry doesn't have: nope") && odd_said.contains("expiry date isn't a date"), "{odd_said}");
    assert!(said(&good).is_empty());
    let text = serde_json::to_string(&got).unwrap();
    assert!(!text.contains(NOTE) && !text.contains(VALUE), "no value, not even a field's, in a problem line: {text}");
}

#[test]
fn verify_says_when_the_key_is_unreadable_or_the_wrong_one() {
    let s = store();
    let id = create(&s, entry("Stripe", &[])).unwrap();
    let blind = verify(&s, Some(&id), Some("builder"), &Err("the key store is locked".into()), today()).unwrap();
    assert!(blind.problems[0].problem.contains("Vault key can't be read"), "{blind:?}");
    let wrong: KeyResult = Ok(Zeroizing::new([9u8; 32]));
    let got = verify(&s, Some(&id), Some("builder"), &wrong, today()).unwrap();
    assert!(got.problems[0].problem.contains("doesn't open"), "{got:?}");
    assert!(!got.problems[0].problem.contains(VALUE));
}

#[test]
fn verify_finds_a_broken_chain() {
    let s = store();
    create(&s, entry("Stripe", &[])).unwrap();
    create(&s, entry("Other", &[])).unwrap();
    assert!(verify(&s, None, Some("builder"), &key(), today()).unwrap().ok);

    s.vault_run_sql_for_tests("UPDATE db_vault_activity SET target = 'edited by hand' WHERE seq = 1");
    let got = verify(&s, None, Some("builder"), &key(), today()).unwrap();
    let chain = got.problems.iter().find(|p| p.id.is_empty()).expect("a problem for the history as a whole");
    assert!(chain.problem.contains("chain doesn't hold at line"), "{}", chain.problem);
}

#[test]
fn health_counts_what_the_agent_may_use_and_says_when_the_key_store_is_unreadable() {
    let s = store();
    create(&s, entry("Soon", &[("expires", "2026-11-10")])).unwrap();
    create(&s, entry("Gone", &[("expires", "2026-11-01")])).unwrap();
    let stale = create(&s, entry("Stale", &[])).unwrap();
    s.vault_run_sql_for_tests(&format!("UPDATE db_vault_entries SET created_at = 1000, last_used_at = 0 WHERE id = '{stale}'"));
    let closed = vaults::create(&s, "Closed", VaultAgents::None).unwrap();
    create_in(&s, entry("Theirs", &[("expires", "2026-11-01")]), &closed.id).unwrap();

    let h = health(&s, Some("builder"), &key(), "keychain", today()).unwrap();
    assert_eq!((h.entries, h.vaults, h.expiring_within_14_days, h.expired, h.unused_for_90_days), (3, 1, 1, 1, 1), "{h:?}");
    assert!(h.key_store.readable && h.key_store.backend == "keychain" && h.key_store.problem.is_empty());
    assert!(h.history.intact && h.history.checked > 0);
    assert!(h.schema_revision >= 8);
    assert_eq!((h.pending_requests, h.pending_uses), (0, 0));

    let blind = health(&s, Some("builder"), &Err("the key store is locked".into()), "keychain", today()).unwrap();
    assert!(!blind.key_store.readable);
    assert_eq!(blind.key_store.problem, "the key store is locked");

    s.vault_run_sql_for_tests("UPDATE db_vault_activity SET target = 'edited by hand' WHERE seq = 1");
    let broken = health(&s, Some("builder"), &key(), "keychain", today()).unwrap();
    assert!(!broken.history.intact && !broken.history.problem.is_empty());
    no_value(&[serde_json::to_string(&h).unwrap(), serde_json::to_string(&broken).unwrap()]);
}
