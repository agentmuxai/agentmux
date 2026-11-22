// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! An entry as a sync record's body: what it carries, what stays on the machine, what a receiver makes of it, and that
//! it never prints a value. Every value is invented.

use super::*;
use crate::backend::vault::seal;
use crate::backend::vault::sync_seal::{open_record, seal_record, Place, Version};

const SECRET: &str = "record-sentinel-3388-kd";
const NONE: &Known = &Known(&[]);

fn profile(id: &str, name: &str, owner: Option<&str>) -> BrowserProfile {
    BrowserProfile { id: id.into(), name: name.into(), color: "#112233".into(), created_at: 1, agents_allowed: false, owner_agent: owner.map(String::from) }
}

fn parts(pairs: &[(&str, &str)]) -> Parts {
    Parts(pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect())
}

fn row(fields: &str, sealed_parts: &Parts) -> VaultRow {
    VaultRow {
        id: "entry-1".into(),
        kind: "api-key".into(),
        label: "Invoice provider key".into(),
        fields: fields.into(),
        tags: r#"["billing","prod"]"#.into(),
        sealed: seal::seal(&seal::test_key(), "entry-1", sealed_parts).unwrap(),
        created_at: 1_000,
        updated_at: 2_000,
        last_used_at: 1_500,
        vault_id: "vault-local".into(),
    }
}

#[test]
fn a_body_carries_the_entry_and_leaves_out_what_is_the_machines_own() {
    let value = parts(&[("value", SECRET)]);
    let r = row(r#"{"expires":"2027-03-01","shared":"[\"agent-uid-1\"]","readers":"invoice-sync (lambda)"}"#, &value);
    let body = EntryBody::of_row(&r, &value, NONE);
    assert_eq!((body.kind.as_str(), body.label.as_str()), ("api-key", "Invoice provider key"));
    assert_eq!(body.tags, ["billing", "prod"]);
    assert_eq!(body.parts.get("value").map(String::as_str), Some(SECRET));
    assert_eq!(body.fields.get("expires").map(String::as_str), Some("2027-03-01"));
    assert!(!body.fields.contains_key("shared"), "who a login is shared with is this machine's agents");
    assert_eq!((body.created_at, body.updated_at), (1_000, 2_000));
    // No time of last use: it is per machine.
    assert!(!serde_json::to_string(&body).unwrap().contains("1500") && !serde_json::to_string(&body).unwrap().contains("last_used"));
}

#[test]
fn a_backed_entry_is_sent_as_its_reference_with_no_value() {
    let fields = r#"{"link":"backed","source_kind":"aws-secret","source_location":"prod/invoice-sync","source_key":"api.token"}"#;
    let none = parts(&[]);
    let body = EntryBody::of_row(&row(fields, &none), &none, NONE);
    assert!(body.parts.is_empty());
    assert_eq!(body.fields.get("link").map(String::as_str), Some("backed"));
    assert_eq!(body.fields.get("source_location").map(String::as_str), Some("prod/invoice-sync"));
}

#[test]
fn a_received_body_keeps_this_machines_own_fields_and_last_use() {
    let value = parts(&[("value", SECRET)]);
    let local = row(r#"{"shared":"[\"agent-uid-9\"]","expires":"2026-12-01"}"#, &value);
    let remote = row(r#"{"expires":"2027-03-01","shared":"[\"agent-uid-1\"]"}"#, &value);
    let mut body = EntryBody::of_row(&remote, &value, NONE);
    body.updated_at = 3_000;
    body.label = "Invoice key (renamed)".into();
    let made = body.into_row("entry-1", "vault-local", vec![1, 2, 3], Some(&local), NONE);
    let fields: BTreeMap<String, String> = serde_json::from_str(&made.fields).unwrap();
    assert_eq!(fields.get("expires").map(String::as_str), Some("2027-03-01"), "what the entry says follows the record");
    assert_eq!(fields.get("shared").map(String::as_str), Some(r#"["agent-uid-9"]"#), "who it is shared with here stays");
    assert_eq!((made.last_used_at, made.updated_at, made.label.as_str()), (1_500, 3_000, "Invoice key (renamed)"));
    assert_eq!((made.sealed.as_slice(), made.vault_id.as_str()), (&[1u8, 2, 3][..], "vault-local"));

    // An entry this machine didn't have: nothing is shared here, and it was never used here.
    let fresh = body.into_row("entry-1", "vault-local", vec![], None, NONE);
    let fresh_fields: BTreeMap<String, String> = serde_json::from_str(&fresh.fields).unwrap();
    assert!(!fresh_fields.contains_key("shared") && fresh.last_used_at == 0);
}

#[test]
fn a_body_goes_through_the_seal_and_never_prints_a_value() {
    let value = parts(&[("value", SECRET)]);
    let r = row(r#"{"expires":"2027-03-01"}"#, &value);
    let body = EntryBody::of_row(&r, &value, NONE);
    assert!(!format!("{body:?}").contains(SECRET), "the debug form names the parts, not their values");
    let (key, v) = (seal::test_key(), Version::new(5_000, 1, "machine-a"));
    let place = Place { vault: "vault-1", record: "entry-1", version: &v, base: None, epoch: 1 };
    let sealed = seal_record(&key, &place, &body).unwrap();
    let opened: EntryBody = open_record(&key, &place, &sealed).unwrap();
    assert_eq!(opened, body);
}

fn login(profile_field: &str) -> VaultRow {
    let fields = format!(r#"{{"site":"https://billing.example.com","username":"alex","profile":"{profile_field}"}}"#);
    row(&fields, &parts(&[("value", SECRET)]))
}

fn fields_of(row: &VaultRow) -> BTreeMap<String, String> {
    serde_json::from_str(&row.fields).unwrap()
}

#[test]
fn a_sign_in_goes_by_its_profiles_name_and_finds_the_profile_of_that_name_on_the_other_machine() {
    let here = [profile("p-0123456789ab", "Work", None)];
    let there = [profile("p-ffffffffffff", "work", None)];
    let r = login("profile:p-0123456789ab");
    let body = EntryBody::of_row(&r, &parts(&[("value", SECRET)]), &Known(&here));
    assert_eq!(body.fields.get(PROFILE_NAME).map(String::as_str), Some("Work"));
    assert!(!body.fields.contains_key(PROFILE), "this machine's profile id doesn't leave it");
    assert!(!serde_json::to_string(&body).unwrap().contains("p-0123456789ab"));

    let made = body.into_row("entry-1", "vault-local", vec![], None, &Known(&there));
    let f = fields_of(&made);
    assert_eq!(f.get(PROFILE).map(String::as_str), Some("profile:p-ffffffffffff"), "the profile of that name here, whatever its id");
    assert!(!f.contains_key(PROFILE_NAME) && !f.contains_key(PROFILE_FROM));
}

#[test]
fn without_a_profile_of_that_name_a_sign_in_shows_under_personal_marked_with_the_name() {
    let here = [profile("p-0123456789ab", "Work", None)];
    let body = EntryBody::of_row(&login("profile:p-0123456789ab"), &parts(&[("value", SECRET)]), &Known(&here));
    let made = body.into_row("entry-1", "vault-local", vec![], None, NONE);
    let f = fields_of(&made);
    assert_eq!((f.get(PROFILE).map(String::as_str), f.get(PROFILE_FROM).map(String::as_str)), (Some("personal"), Some("Work")));
    // Personal goes as it is, with no name.
    let personal = EntryBody::of_row(&login("personal"), &parts(&[("value", SECRET)]), &Known(&here));
    assert!(!personal.fields.contains_key(PROFILE_NAME) && personal.fields.get(PROFILE).map(String::as_str) == Some("personal"));
}

#[test]
fn a_sign_in_in_an_agents_own_profile_or_an_unnamed_one_stays_on_the_machine() {
    let here = [profile("p-0123456789ab", "Work", None), profile("agent-pulp-0123abcd", "Pulp", Some("pulp-uid"))];
    assert!(!stays_here(&login("personal"), &Known(&here)));
    assert!(!stays_here(&login("profile:p-0123456789ab"), &Known(&here)));
    assert!(stays_here(&login("profile:agent-pulp-0123abcd"), &Known(&here)), "an agent's own profile is that agent on this machine");
    assert!(stays_here(&login("profile:p-deleted"), &Known(&here)), "a profile that no longer exists has no name to carry");
    assert_eq!(Known(&here).id_of("PULP"), None, "an agent's profile is not found by name");
}

#[test]
fn a_profile_id_in_a_received_body_is_another_machines_and_is_dropped() {
    let mut body = EntryBody::of_row(&login("personal"), &parts(&[("value", SECRET)]), NONE);
    body.fields.insert(PROFILE.to_string(), "profile:p-0123456789ab".to_string());
    let made = body.into_row("entry-1", "vault-local", vec![], None, NONE);
    assert!(!fields_of(&made).contains_key(PROFILE) || fields_of(&made).get(PROFILE).map(String::as_str) == Some("personal"));
    assert!(!made.fields.contains("p-0123456789ab"));
}

#[test]
fn a_sign_in_edited_on_a_machine_without_its_profile_goes_back_to_that_profile() {
    let work = [profile("p-0123456789ab", "Work", None)];
    let value = parts(&[("value", SECRET)]);
    // A (has Work) sends it; B (no Work) receives it under Personal, marked with the name.
    let sent = EntryBody::of_row(&login("profile:p-0123456789ab"), &value, &Known(&work));
    let on_b = sent.into_row("entry-1", "vault-b", vec![], None, NONE);
    assert_eq!(fields_of(&on_b).get(PROFILE_FROM).map(String::as_str), Some("Work"));
    // B edits it and sends it back: it still names Work.
    let back = EntryBody::of_row(&on_b, &value, NONE);
    assert_eq!(back.fields.get(PROFILE_NAME).map(String::as_str), Some("Work"));
    assert!(!back.fields.contains_key(PROFILE) && !back.fields.contains_key(PROFILE_FROM));
    // On A it is in Work, not in Personal.
    let on_a = back.into_row("entry-1", "vault-a", vec![], None, &Known(&work));
    assert_eq!(fields_of(&on_a).get(PROFILE).map(String::as_str), Some("profile:p-0123456789ab"));
}
