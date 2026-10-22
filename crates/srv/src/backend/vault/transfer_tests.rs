// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The `.vault` file: export, inspect, commit.

use super::import::{Mapping, Source};
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

    assert!(matches!(inspect(&s, &path, None).unwrap_err(), InspectError::NeedsPassphrase));
    assert!(matches!(inspect(&s, &path, Some("")).unwrap_err(), InspectError::NeedsPassphrase));
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
    assert!(matches!(inspect(&s, &text, Some(PASS)).unwrap_err(), InspectError::Other(m) if m.contains("one column")));
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

// A failed write leaves the import staged, so pressing Import again doesn't need the file opened again.
#[test]
fn a_failed_write_leaves_the_import_staged_to_try_again() {
    let s = store();
    seed(&s);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("r.vault");
    export(&s, None, PASS, &path).unwrap();
    wipe(&s);
    let p = inspect(&s, &path, Some(PASS)).unwrap();
    // The write fails: the table isn't there.
    s.conn().lock().unwrap().execute_batch("ALTER TABLE db_vault_entries RENAME TO db_vault_entries_away").unwrap();
    assert!(commit(&s, &p.token, Duplicates::Skip).is_err());
    s.conn().lock().unwrap().execute_batch("ALTER TABLE db_vault_entries_away RENAME TO db_vault_entries").unwrap();
    let done = commit(&s, &p.token, Duplicates::Skip).unwrap();
    assert_eq!(done.added, 3, "the same staged import, written on the second try");
}

// A write that stops part way doesn't leave ciphertext beside the destination.
#[test]
fn a_failed_file_write_leaves_no_temp_file() {
    let s = store();
    seed(&s);
    let dir = tempfile::tempdir().unwrap();
    // The destination's folder doesn't exist: the temp file can't be written.
    let path = dir.path().join("no-such-folder").join("x.vault");
    assert!(export(&s, None, PASS, &path).is_err());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    // The destination is a folder: the write succeeds but the rename can't, and the temp file goes.
    let taken = dir.path().join("taken.vault");
    std::fs::create_dir(&taken).unwrap();
    assert!(export(&s, None, PASS, &taken).is_err());
    let leftovers: Vec<_> = std::fs::read_dir(dir.path()).unwrap().flatten().map(|e| e.file_name()).collect();
    assert_eq!(leftovers.len(), 1, "only the folder: {leftovers:?}");
}

// ── Other services' exports, through the same preview and commit ────────────

fn write(dir: &tempfile::TempDir, name: &str, text: &str) -> std::path::PathBuf {
    let p = dir.path().join(name);
    std::fs::write(&p, text).unwrap();
    p
}

#[test]
fn a_lastpass_export_is_previewed_by_kind_then_imported_as_those_kinds() {
    let s = store();
    let dir = tempfile::tempdir().unwrap();
    let csv = "url,username,password,totp,extra,name,grouping,fav\n\
https://example.com,alex,hunter2,JBSWY3DPEHPK3PXP,,Example,Shopping,0\n\
http://sn,,,,\"NoteType:Social Security\nName:Alex Doe\nNumber:123-45-6789\",My SSN,,0\n\
http://sn,,,,\"NoteType:Credit Card\nNumber:4111111111111111\nSecurity Code:123\",Visa,,0\n";
    let path = write(&dir, "lastpass_vault_export.csv", csv);
    let preview = inspect(&s, &path, None).unwrap();
    assert_eq!((preview.source, preview.total, preview.skipped.len()), (Source::LastPass, 3, 0));
    assert!(preview.plaintext_file, "the person is offered to delete it");
    assert!(preview.source_label.contains("LastPass"));
    assert_eq!(preview.sections.len(), 3, "a login, a card and an SSN");
    let shown = serde_json::to_string(&preview).unwrap();
    for secret in ["hunter2", "123-45", "4111", "JBSWY3"] {
        assert!(!shown.contains(secret), "the preview shows {secret}");
    }
    let done = commit(&s, &preview.token, Duplicates::Skip).unwrap();
    assert_eq!(done.added, 3);
    assert_eq!(all_parts(&s, "login-example")["totp"], "JBSWY3DPEHPK3PXP");
    assert_eq!(all_parts(&s, "ssn-my-ssn")["value"], "123-45-6789");
    assert_eq!(all_parts(&s, "card-visa")["cvc"], "123");
    assert!(s.vault_get("login-example").unwrap().unwrap().tags.contains("Shopping"));
    let log = activity(&s, 3).unwrap();
    assert_eq!(log[0].action, "imported");
    assert!(!serde_json::to_string(&log).unwrap().contains("hunter2"));
}

#[test]
fn a_second_import_of_the_same_export_marks_every_entry_a_duplicate() {
    let s = store();
    let dir = tempfile::tempdir().unwrap();
    let path = write(&dir, "chrome.csv", "name,url,username,password\nGitHub,https://github.com,alex,pw1\nExample,https://example.com,bob,pw2\n");
    let first = inspect(&s, &path, None).unwrap();
    commit(&s, &first.token, Duplicates::Skip).unwrap();
    let again = inspect(&s, &path, None).unwrap();
    assert_eq!((again.total, again.duplicates), (2, 2));
    let done = commit(&s, &again.token, Duplicates::Skip).unwrap();
    assert_eq!((done.added, done.skipped), (0, 2));
    assert_eq!(s.vault_count().unwrap(), 2);
}

#[test]
fn a_csv_no_reader_knows_is_imported_with_the_persons_column_choices() {
    let s = store();
    let dir = tempfile::tempdir().unwrap();
    let path = write(&dir, "keys.csv", "Service,Secret,Owner\nStripe,sk_test_1,alex\nTwilio,tw_2,bob\n");
    match inspect(&s, &path, None) {
        Err(InspectError::NeedsMapping(m)) => assert_eq!((m.columns.len(), m.rows), (3, 2)),
        other => panic!("{other:?}"),
    }
    assert_eq!(s.vault_count().unwrap(), 0, "nothing is read in until the columns are chosen");
    let mapping = Mapping { kind: "api-key".into(), label: 0, value: 1, site: None, username: None };
    let preview = inspect_with(&s, &path, None, Some(&mapping)).unwrap();
    assert_eq!((preview.source, preview.total), (Source::Csv, 2));
    commit(&s, &preview.token, Duplicates::Skip).unwrap();
    assert_eq!(all_parts(&s, "api-key-stripe")["value"], "sk_test_1");
}

#[test]
fn an_env_file_and_a_bitwarden_export_are_found_by_their_contents_not_their_names() {
    let s = store();
    let dir = tempfile::tempdir().unwrap();
    let env = write(&dir, "secrets.txt", "STRIPE_API_KEY=sk_test_123\nGITHUB_TOKEN=ghp_abc\n");
    let preview = inspect(&s, &env, None).unwrap();
    assert_eq!((preview.source, preview.total), (Source::Env, 2));
    let bw = write(&dir, "backup", r#"{"encrypted":false,"folders":[],"items":[{"type":1,"name":"A","login":{"username":"u","password":"p","uris":[{"uri":"https://a.example"}]}}]}"#);
    let preview = inspect(&s, &bw, None).unwrap();
    assert_eq!((preview.source, preview.total), (Source::Bitwarden, 1));
}

#[test]
fn rows_that_break_the_vaults_limits_are_listed_not_imported_and_the_rest_still_come_in() {
    let s = store();
    let dir = tempfile::tempdir().unwrap();
    let huge = "x".repeat(1024 * 1024 + 1);
    let path = write(&dir, "chrome.csv", &format!("name,url,username,password\nGood,https://g.example,u,p\nHuge,https://h.example,u,{huge}\n"));
    let preview = inspect(&s, &path, None).unwrap();
    assert_eq!(preview.total, 1);
    assert_eq!(preview.skipped.len(), 1);
    assert!(preview.skipped[0].reason.contains("1 MiB"), "{}", preview.skipped[0].reason);
}

#[test]
fn a_vault_file_is_not_called_plaintext() {
    let s = store();
    seed(&s);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("b.vault");
    export(&s, None, PASS, &path).unwrap();
    let preview = inspect(&s, &path, Some(PASS)).unwrap();
    assert_eq!((preview.source, preview.plaintext_file), (Source::Vault, false));
}

// A parser must never panic on a file made to break it (spec §8.7).
#[test]
fn hostile_files_are_refused_and_never_panic() {
    let s = store();
    let dir = tempfile::tempdir().unwrap();
    let samples: &[&[u8]] = &[
        b"",
        b"\0\0\0\0",
        b"{",
        b"{\"items\": 5}",
        b"{\"items\": [null, 1, \"x\", {\"type\": 99}]}",
        b"a,b\n\"unterminated",
        b"a,b,c\n1,2\n1,2,3,4,5\n",
        b"=\n=\n=",
        b"NAME=\"unterminated",
        b"\xEF\xBB\xBF",
        b"url,username,password\n\x00,\x00,\x00\n",
        &[0xff; 4096],
        b"age-encryption.org/v1\n-> scrypt \n",
    ];
    for (i, bytes) in samples.iter().enumerate() {
        let path = dir.path().join(format!("hostile{i}"));
        std::fs::write(&path, bytes).unwrap();
        let _ = inspect(&s, &path, Some(PASS));
    }
    assert_eq!(s.vault_count().unwrap(), 0);
}

#[test]
fn a_keepass_database_asks_for_its_password_then_previews_and_is_not_called_a_plaintext_file() {
    use keepass::config::{DatabaseConfig, KdfConfig};
    let mut config = DatabaseConfig::default();
    config.kdf_config = KdfConfig::Aes { rounds: 100 };
    let mut db = keepass::Database::with_config(config);
    db.root_mut().add_entry().edit(|e| {
        e.set_unprotected("Title", "Example");
        e.set_unprotected("UserName", "alex");
        e.set_protected("Password", "hunter2");
    });
    let mut bytes = Vec::new();
    db.save(&mut bytes, keepass::DatabaseKey::new().with_password("master-pw")).unwrap();
    let s = store();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Passwords.kdbx");
    std::fs::write(&path, &bytes).unwrap();

    assert!(matches!(inspect(&s, &path, None).unwrap_err(), InspectError::NeedsPassphrase));
    let wrong = inspect(&s, &path, Some("nope")).unwrap_err();
    assert!(matches!(&wrong, InspectError::Other(m) if m.contains("didn't open")));
    let preview = inspect(&s, &path, Some("master-pw")).unwrap();
    assert_eq!((preview.source, preview.total, preview.plaintext_file), (Source::KeePass, 1, false));
    assert!(preview.source_label.contains("KeePass"));
    let result = commit(&s, &preview.token, Duplicates::Skip).unwrap();
    assert_eq!(result.added, 1);
}

#[test]
fn a_keepass_database_that_has_only_a_key_file_is_previewed_with_it() {
    let s = store();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Keyed.kdbx");
    std::fs::write(&path, include_bytes!("import/fixtures/pykeepass_keyfile_only.kdbx")).unwrap();
    let key: &[u8] = include_bytes!("import/fixtures/livekey.keyx");
    assert!(matches!(inspect(&s, &path, None).unwrap_err(), InspectError::NeedsPassphrase));
    let preview = inspect_keyed(&s, &path, None, None, Some(key)).unwrap();
    assert_eq!((preview.source, preview.total, preview.plaintext_file), (Source::KeePass, 2, false), "the login and its note");
    let wrong = inspect_keyed(&s, &path, None, None, Some(b"not the key")).unwrap_err();
    assert!(matches!(&wrong, InspectError::Other(m) if m.contains("didn't open")));
}
