// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The `.vault` file: export, inspect, commit.

use super::transfer::*;
use super::*;

const PASS: &str = "correct horse battery staple";

fn store() -> Store {
    seal::use_test_key();
    let s = Store::open_in_memory().unwrap();
    s.apply_identity_schema_for_tests().unwrap();
    s
}

fn parts(pairs: &[(&str, &str)]) -> Parts {
    Parts(pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect())
}

fn fields(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

/// An SSN, a card with every part and a login with a one-time-code seed.
fn seed(s: &Store) -> Vec<String> {
    vec![
        create(s, NewEntry { kind: "ssn".into(), label: "Alex".into(), fields: fields(&[("owner", "Alex Doe")]), tags: vec!["tax".into()], parts: parts(&[("value", "123-45-6789")]) }).unwrap(),
        create(
            s,
            NewEntry {
                kind: "card".into(),
                label: "Company card".into(),
                fields: Default::default(),
                tags: vec![],
                parts: parts(&[("number", "4111111111111111"), ("expiry", "12/30"), ("cvc", "123"), ("name", "A DOE")]),
            },
        )
        .unwrap(),
        create(
            s,
            NewEntry {
                kind: "login".into(),
                label: "Example".into(),
                fields: fields(&[("site", "https://example.com"), ("username", "alex")]),
                tags: vec![],
                parts: parts(&[("password", "hunter2hunter2"), ("totp", "JBSWY3DPEHPK3PXP")]),
            },
        )
        .unwrap(),
    ]
}

fn wipe(s: &Store) {
    for r in s.vault_list().unwrap() {
        s.vault_delete(&r.id).unwrap();
    }
}

fn all_parts(s: &Store, id: &str) -> BTreeMap<String, String> {
    let row = s.vault_get(id).unwrap().unwrap();
    seal::open(&seal::test_key(), id, &row.sealed).unwrap().0.clone()
}

#[test]
fn a_round_trip_keeps_every_entry_kind_part_field_tag_and_id() {
    let s = store();
    let ids = seed(&s);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("backup.vault");
    let out = export(&s, None, PASS, &path).unwrap();
    assert_eq!((out.count, out.file_name.as_str()), (3, "backup.vault"));

    let bytes = std::fs::read(&path).unwrap();
    assert!(bytes.starts_with(b"age-encryption.org/v1"), "an age file, so `age -d` opens it");
    for secret in ["123-45-6789", "4111111111111111", "hunter2hunter2", "JBSWY3DPEHPK3PXP", "Alex Doe"] {
        assert!(!bytes.windows(secret.len()).any(|w| w == secret.as_bytes()), "{secret} is in the file in plain text");
    }

    wipe(&s);
    let preview = inspect(&s, &path, Some(PASS)).unwrap();
    assert_eq!((preview.total, preview.duplicates, preview.skipped.len()), (3, 0, 0));
    assert_eq!(preview.sections.iter().map(|p| (p.section, p.count)).collect::<Vec<_>>().len(), 3);
    let shown = serde_json::to_string(&preview).unwrap();
    for secret in ["123-45-6789", "4111", "hunter2", "JBSWY3"] {
        assert!(!shown.contains(secret), "the preview shows a value: {shown}");
    }
    assert_eq!(s.vault_count().unwrap(), 0, "inspect writes nothing");

    let done = commit(&s, &preview.token, Duplicates::Skip).unwrap();
    assert_eq!((done.added, done.replaced, done.skipped), (3, 0, 0));
    for id in &ids {
        assert!(s.vault_get(id).unwrap().is_some(), "{id} keeps its id");
    }
    assert_eq!(all_parts(&s, "card-company-card").len(), 4);
    assert_eq!(all_parts(&s, "login-example")["totp"], "JBSWY3DPEHPK3PXP");
    assert_eq!(all_parts(&s, "ssn-alex")["value"], "123-45-6789");
    let row = s.vault_get("ssn-alex").unwrap().unwrap();
    assert!(row.fields.contains("Alex Doe") && row.tags.contains("tax"));
    let log = activity(&s, 10).unwrap();
    assert_eq!(log.iter().map(|a| a.action.as_str()).take(2).collect::<Vec<_>>(), ["imported", "exported"]);
    assert_eq!(log[0].target, "backup.vault");
    assert!(!serde_json::to_string(&log).unwrap().contains("6789"));
}

#[test]
fn a_chosen_selection_is_all_that_is_exported() {
    let s = store();
    let ids = seed(&s);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("some.vault");
    assert_eq!(export(&s, Some(&ids[..1]), PASS, &path).unwrap().count, 1);
    wipe(&s);
    assert_eq!(inspect(&s, &path, Some(PASS)).unwrap().total, 1);
    assert!(export(&s, Some(&["nope".to_string()]), PASS, &path).unwrap_err().contains("nothing to export"));
}

#[test]
fn a_short_passphrase_is_refused_and_no_file_is_written() {
    let s = store();
    seed(&s);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("x.vault");
    assert!(export(&s, None, "short", &path).unwrap_err().contains("at least 8"));
    assert!(!path.exists());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0, "no temp file is left");
}

#[test]
fn a_wrong_passphrase_a_missing_one_and_a_tampered_file_write_nothing() {
    let s = store();
    seed(&s);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("b.vault");
    export(&s, None, PASS, &path).unwrap();
    wipe(&s);

    assert_eq!(inspect(&s, &path, None).unwrap_err(), InspectError::NeedsPassphrase);
    assert_eq!(inspect(&s, &path, Some("")).unwrap_err(), InspectError::NeedsPassphrase);
    match inspect(&s, &path, Some("not the passphrase")).unwrap_err() {
        InspectError::Other(m) => assert!(m.contains("didn't open the file"), "{m}"),
        e => panic!("{e:?}"),
    }
    let mut bytes = std::fs::read(&path).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    let tampered = dir.path().join("t.vault");
    std::fs::write(&tampered, &bytes).unwrap();
    match inspect(&s, &tampered, Some(PASS)).unwrap_err() {
        InspectError::Other(m) => assert!(m.contains("damaged") || m.contains("changed") || m.contains("decrypt"), "{m}"),
        e => panic!("{e:?}"),
    }
    assert_eq!(s.vault_count().unwrap(), 0);
}

#[test]
fn files_that_are_not_vault_files_are_refused_with_a_reason() {
    let s = store();
    let dir = tempfile::tempdir().unwrap();
    let text = dir.path().join("notes.txt");
    std::fs::write(&text, "hello").unwrap();
    assert!(matches!(inspect(&s, &text, Some(PASS)).unwrap_err(), InspectError::Other(m) if m.contains("age-encrypted")));
    assert!(matches!(inspect(&s, &dir.path().join("missing.vault"), Some(PASS)).unwrap_err(), InspectError::Other(m) if m.contains("couldn't read")));
    assert!(matches!(inspect(&s, dir.path(), Some(PASS)).unwrap_err(), InspectError::Other(m) if m.contains("isn't a file")));

    // An age file, but not a Vault's JSON; and one from a newer format.
    let encrypt = |json: &str| -> Vec<u8> {
        let mut out = Vec::new();
        let mut w = age::Encryptor::with_user_passphrase(age::secrecy::SecretString::from(PASS.to_string())).wrap_output(&mut out).unwrap();
        std::io::Write::write_all(&mut w, json.as_bytes()).unwrap();
        w.finish().unwrap();
        out
    };
    let other = dir.path().join("other.vault");
    std::fs::write(&other, encrypt("{\"hello\":1}")).unwrap();
    assert!(matches!(inspect(&s, &other, Some(PASS)).unwrap_err(), InspectError::Other(m) if m.contains("isn't a Vault file")));
    let newer = dir.path().join("newer.vault");
    std::fs::write(&newer, encrypt(r#"{"format":"agentmux-vault","version":2,"exported_at":0,"entries":[]}"#)).unwrap();
    assert!(matches!(inspect(&s, &newer, Some(PASS)).unwrap_err(), InspectError::Other(m) if m.contains("newer AgentMux")));
}

#[test]
fn a_file_over_the_size_cap_is_refused_before_it_is_read() {
    let s = store();
    let dir = tempfile::tempdir().unwrap();
    let big = dir.path().join("big.vault");
    std::fs::File::create(&big).unwrap().set_len(65 * 1024 * 1024).unwrap();
    assert!(matches!(inspect(&s, &big, Some(PASS)).unwrap_err(), InspectError::Other(m) if m.contains("over 64 MiB")));
}

#[test]
fn entries_that_cant_come_in_are_listed_with_why_and_the_rest_still_import() {
    let s = store();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mixed.vault");
    let json = r#"{"format":"agentmux-vault","version":1,"exported_at":0,"entries":[
        {"kind":"ssn","label":"Good","parts":{"value":"111-11-1111"}},
        {"kind":"rocket","label":"Odd kind","parts":{"value":"x"}},
        {"kind":"ssn","label":"No value","parts":{}},
        {"kind":"ssn","label":"Wrong part","parts":{"value":"1","cvc":"2"}}
    ]}"#;
    let mut out = Vec::new();
    let mut w = age::Encryptor::with_user_passphrase(age::secrecy::SecretString::from(PASS.to_string())).wrap_output(&mut out).unwrap();
    std::io::Write::write_all(&mut w, json.as_bytes()).unwrap();
    w.finish().unwrap();
    std::fs::write(&path, out).unwrap();
    let preview = inspect(&s, &path, Some(PASS)).unwrap();
    assert_eq!(preview.total, 1);
    let why: BTreeMap<_, _> = preview.skipped.iter().map(|k| (k.label.as_str(), k.reason.as_str())).collect();
    assert!(why["Odd kind"].contains("unknown kind"));
    assert!(why["No value"].contains("no value"));
    assert!(why["Wrong part"].contains("no part"));
    assert_eq!(commit(&s, &preview.token, Duplicates::Skip).unwrap().added, 1);
}

#[test]
fn duplicates_are_marked_and_skipped_replaced_or_kept_as_the_person_chooses() {
    let s = store();
    seed(&s);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("d.vault");
    export(&s, None, PASS, &path).unwrap();
    // Change what's in the Vault so replacing is visible: same SSN value under a new label.
    update(&s, EntryChange { id: "ssn-alex".into(), label: "Alex (renamed)".into(), fields: Default::default(), tags: vec![], parts: Parts::default(), clear: vec![] }).unwrap();

    let skip = inspect(&s, &path, Some(PASS)).unwrap();
    assert_eq!((skip.total, skip.duplicates), (3, 3), "the same SSN, card number and login are duplicates");
    assert!(skip.sections.iter().flat_map(|p| &p.first).all(|e| e.duplicate));
    let r = commit(&s, &skip.token, Duplicates::Skip).unwrap();
    assert_eq!((r.added, r.replaced, r.skipped), (0, 0, 3));
    assert_eq!(s.vault_get("ssn-alex").unwrap().unwrap().label, "Alex (renamed)");

    let replace = inspect(&s, &path, Some(PASS)).unwrap();
    let r = commit(&s, &replace.token, Duplicates::Replace).unwrap();
    assert_eq!((r.added, r.replaced, r.skipped), (0, 3, 0));
    assert_eq!(s.vault_get("ssn-alex").unwrap().unwrap().label, "Alex", "replaced by the file's version, same id");
    assert_eq!(s.vault_count().unwrap(), 3);

    let both = inspect(&s, &path, Some(PASS)).unwrap();
    let r = commit(&s, &both.token, Duplicates::KeepBoth).unwrap();
    assert_eq!((r.added, r.replaced, r.skipped), (3, 0, 0));
    assert_eq!(s.vault_count().unwrap(), 6);
    assert!(s.vault_get("ssn-alex-2").unwrap().is_some(), "the file's id was taken, so a new one");
}

#[test]
fn a_login_is_a_duplicate_by_site_and_username_not_password() {
    let s = store();
    seed(&s);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("l.vault");
    export(&s, Some(&["login-example".to_string()]), PASS, &path).unwrap();
    update(&s, EntryChange { id: "login-example".into(), label: "Example".into(), fields: fields(&[("site", "https://EXAMPLE.com/"), ("username", "ALEX")]), tags: vec![], parts: parts(&[("password", "changed")]), clear: vec![] }).unwrap();
    assert_eq!(inspect(&s, &path, Some(PASS)).unwrap().duplicates, 1);
}

#[test]
fn an_import_is_all_or_nothing() {
    let s = store();
    seed(&s);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.vault");
    export(&s, None, PASS, &path).unwrap();
    wipe(&s);
    let preview = inspect(&s, &path, Some(PASS)).unwrap();
    // One of the incoming ids is taken by a row written after the preview.
    let row = VaultRow { id: "card-company-card".into(), kind: "note".into(), label: "x".into(), fields: "{}".into(), tags: "[]".into(), sealed: vec![1], created_at: 1, updated_at: 1, last_used_at: 0 };
    // The writer takes the file's id only when it's free, so it picks another: nothing fails…
    s.vault_insert(&row).unwrap();
    assert_eq!(commit(&s, &preview.token, Duplicates::Skip).unwrap().added, 3);
    assert!(s.vault_get("card-company-card-2").unwrap().is_some());
    // …and a write that does fail leaves nothing behind.
    let bad = [VaultRow { id: "dupe".into(), ..row.clone() }, VaultRow { id: "dupe".into(), ..row.clone() }];
    let before = s.vault_count().unwrap();
    assert!(s.vault_write_many(&bad, &[]).is_err());
    assert_eq!(s.vault_count().unwrap(), before, "the first of the two was rolled back");
}

#[test]
fn a_staged_import_is_used_once_and_can_be_cancelled() {
    let s = store();
    seed(&s);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.vault");
    export(&s, None, PASS, &path).unwrap();
    wipe(&s);
    let p = inspect(&s, &path, Some(PASS)).unwrap();
    commit(&s, &p.token, Duplicates::Skip).unwrap();
    assert!(commit(&s, &p.token, Duplicates::Skip).unwrap_err().contains("expired"));
    let p = inspect(&s, &path, Some(PASS)).unwrap();
    cancel(&p.token);
    assert!(commit(&s, &p.token, Duplicates::Skip).unwrap_err().contains("expired"));
    assert!(commit(&s, "no-such-token", Duplicates::Skip).is_err());
}

#[test]
fn a_suggested_passphrase_is_eight_words_and_different_each_time() {
    let a = suggest_passphrase().unwrap();
    assert_eq!(a.split('-').count(), 8);
    assert!(a.len() >= MIN_PASSPHRASE);
    assert_ne!(a, suggest_passphrase().unwrap());
}
