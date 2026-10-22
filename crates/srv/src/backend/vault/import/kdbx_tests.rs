// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use keepass::config::{DatabaseConfig, KdfConfig};
use keepass::db::Value;

/// Most tests open a database by its password alone.
fn read(bytes: &[u8], passphrase: &str) -> Result<Imported, ReadError> {
    super::read(bytes, passphrase, None)
}

/// A database as KeePassXC would save it, with a cheap key derivation.
fn database(kdf: KdfConfig) -> Database {
    let mut config = DatabaseConfig::default();
    config.kdf_config = kdf;
    let mut db = Database::with_config(config);
    {
        let mut root = db.root_mut();
        root.edit(|g| g.name = "Passwords".into());
        let mut e = root.add_entry();
        e.edit(|e| {
            e.set_unprotected("Title", "Example");
            e.set_unprotected("UserName", "alex");
            e.set_protected("Password", "hunter2-pw");
            e.set_unprotected("URL", "https://example.com/login");
            e.set_unprotected("Notes", "recovery: 4455");
            e.set("otp", Value::protected("otpauth://totp/Example?secret=JBSWY3DPEHPK3PXP"));
            e.set_protected("SSN", "123-45-6789");
            e.set_protected("PIN", "9876");
        });
        let mut work = root.add_group();
        work.edit(|g| g.name = "Work".into());
        work.add_entry().edit(|e| {
            e.set_unprotected("Title", "Server note");
            e.set_unprotected("Notes", "only a note");
        });
        work.add_entry().edit(|e| e.set_unprotected("Title", "Empty"));
        work.add_entry().edit(|e| {
            e.set_unprotected("Title", "Authenticator only");
            e.set("otp", Value::protected("otpauth://totp/Only?secret=KRSXG5CTMVRXEZLU"));
        });
    }
    db
}

fn saved(db: &Database, password: &str) -> Vec<u8> {
    let mut out = Vec::new();
    db.save(&mut out, DatabaseKey::new().with_password(password)).unwrap();
    out
}

fn cheap_aes() -> KdfConfig {
    KdfConfig::Aes { rounds: 100 }
}

#[test]
fn a_keepass_database_comes_in_as_logins_notes_and_kind_named_fields() {
    let bytes = saved(&database(cheap_aes()), "master-pw");
    let i = read(&bytes, "master-pw").ok().expect("opens");
    assert_eq!(i.source, Source::KeePass);
    assert!(!i.plaintext, "a KeePass database is encrypted, so the pane doesn't offer to delete it as a plaintext export");
    let by = |label: &str| i.entries.iter().find(|e| e.label == label).unwrap_or_else(|| panic!("no {label}: {:?}", i.entries.iter().map(|e| &e.label).collect::<Vec<_>>()));
    let login = by("Example");
    assert_eq!(login.kind, "login");
    assert_eq!(login.parts["password"], "hunter2-pw");
    assert!(login.parts["totp"].starts_with("otpauth://"));
    assert_eq!((login.fields["site"].as_str(), login.fields["username"].as_str()), ("https://example.com/login", "alex"));
    assert!(login.tags.is_empty(), "the root group's own name says nothing: {:?}", login.tags);
    let ssn = by("Example (SSN)");
    assert_eq!((ssn.kind.as_str(), ssn.parts["value"].as_str()), ("ssn", "123-45-6789"));
    // The notes and the field no kind is named for are kept together, sealed.
    let notes = by("Example (notes)");
    assert_eq!(notes.kind, "note");
    assert!(notes.parts["text"].contains("recovery: 4455") && notes.parts["text"].contains("PIN: 9876"));
    // A group below the root is a tag; an entry that is only a note is a note.
    let only = by("Server note");
    assert_eq!((only.kind.as_str(), only.tags.clone()), ("note", vec!["Work".to_string()]));
    // A seed with no password isn't dropped: it is kept in a sealed note.
    let seed = by("Authenticator only");
    assert_eq!(seed.kind, "note");
    assert!(seed.parts["text"].contains("secret=KRSXG5CTMVRXEZLU"), "{:?}", seed.parts);
    assert!(i.skipped.iter().any(|(l, why)| l == "Empty" && why.contains("no password")), "{:?}", i.skipped);
}

#[test]
fn an_argon2_database_opens_too() {
    let db = database(KdfConfig::Argon2id { iterations: 2, memory: 1 << 20, parallelism: 1, version: argon2::Version::Version13 });
    let i = read(&saved(&db, "pw"), "pw").ok().expect("opens");
    assert!(i.entries.iter().any(|e| e.label == "Example"));
}

#[test]
fn a_wrong_or_missing_password_opens_nothing() {
    let bytes = saved(&database(cheap_aes()), "master-pw");
    match read(&bytes, "") {
        Err(ReadError::NeedsPassphrase) => {}
        _ => panic!("a missing password is asked for"),
    }
    match read(&bytes, "wrong") {
        Err(ReadError::Other(e)) => assert!(e.contains("didn't open"), "{e}"),
        _ => panic!("a wrong password is refused"),
    }
}

#[test]
fn entries_in_the_recycle_bin_are_not_imported() {
    let mut db = database(cheap_aes());
    let bin = db.root_mut().add_group().edit(|g| g.name = "Recycle Bin".into()).id();
    db.group_mut(bin).unwrap().add_entry().edit(|e| {
        e.set_unprotected("Title", "Deleted");
        e.set_protected("Password", "gone");
    });
    db.meta.recyclebin_uuid = Some(bin.uuid());
    db.meta.recyclebin_enabled = Some(true);
    let i = read(&saved(&db, "pw"), "pw").ok().unwrap();
    assert!(i.entries.iter().all(|e| e.label != "Deleted"));
}

// ── The header's caps (§8.7) ────────────────────────────────────────────────

fn header4(kdf: &[u8]) -> Vec<u8> {
    let mut h = Vec::new();
    h.extend(MAGIC);
    h.extend(SIGNATURE_2);
    h.extend(0u16.to_le_bytes());
    h.extend(4u16.to_le_bytes());
    h.push(11);
    h.extend((kdf.len() as u32).to_le_bytes());
    h.extend(kdf);
    h.push(0);
    h.extend(4u32.to_le_bytes());
    h.extend([13, 10, 13, 10]);
    h
}

fn variant(entries: &[(u8, &str, Vec<u8>)]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend(0x0100u16.to_le_bytes());
    for (kind, key, value) in entries {
        v.push(*kind);
        v.extend((key.len() as u32).to_le_bytes());
        v.extend(key.as_bytes());
        v.extend((value.len() as u32).to_le_bytes());
        v.extend(value);
    }
    v.push(0);
    v
}

fn argon(memory: u64, iterations: u64, parallelism: u32) -> Vec<u8> {
    variant(&[
        (0x42, "$UUID", KDF_ARGON2ID.to_vec()),
        (0x05, "M", memory.to_le_bytes().to_vec()),
        (0x05, "I", iterations.to_le_bytes().to_vec()),
        (0x04, "P", parallelism.to_le_bytes().to_vec()),
    ])
}

#[test]
fn a_database_asking_for_too_much_key_stretching_is_refused_before_a_password_is_tried() {
    assert!(check_header(&header4(&argon(64 << 20, 10, 2))).is_ok());
    assert!(check_header(&header4(&argon(MAX_ARGON_MEMORY, MAX_ARGON_ITERATIONS, 8))).is_ok());
    let heavy = check_header(&header4(&argon(MAX_ARGON_MEMORY + 1, 10, 2))).unwrap_err();
    assert!(heavy.contains("memory"), "{heavy}");
    assert!(check_header(&header4(&argon(64 << 20, MAX_ARGON_ITERATIONS + 1, 2))).is_err());
    assert!(check_header(&header4(&argon(64 << 20, 10, 65))).is_err());
    let aes = |rounds: u64| variant(&[(0x42, "$UUID", vec![0xC9; 16]), (0x05, "R", rounds.to_le_bytes().to_vec())]);
    assert!(check_header(&header4(&aes(6_000_000))).is_ok());
    assert!(check_header(&header4(&aes(MAX_AES_ROUNDS + 1))).is_err());
    // read() applies it too, and needs no password to refuse.
    assert!(matches!(read(&header4(&argon(u64::MAX, 1, 1)), "pw"), Err(ReadError::Other(e)) if e.contains("memory")));

    // KDBX 3: TransformRounds is field 6.
    let mut h3 = Vec::new();
    h3.extend(MAGIC);
    h3.extend(SIGNATURE_2);
    h3.extend(1u16.to_le_bytes());
    h3.extend(3u16.to_le_bytes());
    h3.push(6);
    h3.extend(8u16.to_le_bytes());
    h3.extend((MAX_AES_ROUNDS + 1).to_le_bytes());
    h3.push(0);
    h3.extend(4u16.to_le_bytes());
    h3.extend([13, 10, 13, 10]);
    assert!(check_header(&h3).is_err());
}

#[test]
fn a_cut_off_or_old_or_foreign_header_is_refused_without_panicking() {
    let good = header4(&argon(1 << 20, 2, 1));
    for cut in 0..good.len() {
        // Every prefix is either a complete header or an error, never a panic.
        let _ = check_header(&good[..cut]);
    }
    assert!(check_header(&good[..10]).is_err());
    let mut kdb = good.clone();
    kdb[4..8].copy_from_slice(&SIGNATURE_2_KDB);
    assert!(check_header(&kdb).unwrap_err().contains("1.x"));
    let mut future = good.clone();
    future[10..12].copy_from_slice(&9u16.to_le_bytes());
    assert!(check_header(&future).unwrap_err().contains("version"));
    // A table of hostile bytes after the signatures.
    for n in 0..64u8 {
        let mut junk = MAGIC.to_vec();
        junk.extend(SIGNATURE_2);
        junk.extend((0..200u32).map(|i| (i as u8).wrapping_mul(n).wrapping_add(n)));
        let _ = read(&junk, "pw");
    }
}

// Databases written by another implementation (pykeepass 4.2, a Python library with its own KDBX
// writer, so not the keepass crate that reads them): Argon2d with AES-256, nested groups, an attachment,
// a password with edge spaces, and an entry in the recycle bin. Password: "correct horse battery".
const FROM_PYKEEPASS: &[u8] = include_bytes!("fixtures/pykeepass_plain.kdbx");
// The same with a protected custom field and an `otp` seed, which pykeepass writes after the entry's
// AutoType rather than among its other strings as KeePass does.
const FROM_PYKEEPASS_CUSTOM: &[u8] = include_bytes!("fixtures/pykeepass_custom_fields.kdbx");

#[test]
fn a_database_another_implementation_wrote_opens_with_everything_it_holds() {
    let i = match read(FROM_PYKEEPASS, "correct horse battery") {
        Ok(i) => i,
        Err(ReadError::Other(e)) => panic!("{e}"),
        Err(_) => panic!("unexpected"),
    };
    let by = |label: &str| i.entries.iter().find(|e| e.label == label).unwrap_or_else(|| panic!("no {label}: {:?}", i.entries.iter().map(|e| &e.label).collect::<Vec<_>>()));
    let login = by("Example");
    assert_eq!((login.kind.as_str(), login.parts["password"].as_str(), login.fields["username"].as_str()), ("login", "hunter2-pw", "alex"));
    assert_eq!(login.fields["site"], "https://example.com/login");
    assert_eq!(login.tags, vec!["Websites".to_string()]);
    assert_eq!(by("Example (notes)").parts["text"], "recovery: 4455");
    assert_eq!(by("Spaced").parts["password"], "  edge spaces  ", "a password keeps its spaces");
    assert_eq!(by("Server note").tags, vec!["Websites/Work".to_string()]);
    assert!(i.entries.iter().all(|e| e.label != "Deleted one"), "the recycle bin is skipped");
}

#[test]
fn a_wrong_password_is_called_wrong_and_a_layout_the_reader_cannot_parse_is_said_plainly() {
    match read(FROM_PYKEEPASS, "wrong") {
        Err(ReadError::Other(e)) => assert!(e.contains("didn't open"), "{e}"),
        _ => panic!("refused"),
    }
    // Custom fields written out of KeePass's order fail in the keepass crate's XML reader. Said so,
    // and without the crate's detail, which can quote the file.
    match read(FROM_PYKEEPASS_CUSTOM, "correct horse battery") {
        Err(ReadError::Other(e)) => assert!(e.contains("lays its entries out differently") && !e.contains("duplicate field"), "{e}"),
        Ok(_) => {} // a newer keepass crate may read it: then it must simply work
        Err(_) => panic!("unexpected"),
    }
}

// ── Key files ───────────────────────────────────────────────────────────────
// Written by pykeepass with a KeePass XML key file: one by its password and the key file, one by the
// key file alone. Password "correct horse battery".
const WITH_KEYFILE: &[u8] = include_bytes!("fixtures/pykeepass_password_and_keyfile.kdbx");
const KEYFILE_ONLY: &[u8] = include_bytes!("fixtures/pykeepass_keyfile_only.kdbx");
const KEYFILE: &[u8] = include_bytes!("fixtures/livekey.keyx");

fn err_of(r: Result<Imported, ReadError>) -> String {
    match r {
        Err(ReadError::Other(e)) => e,
        Err(ReadError::NeedsPassphrase) => "needs a password".to_string(),
        Err(_) => "other".to_string(),
        Ok(_) => "opened".to_string(),
    }
}

#[test]
fn a_database_that_needs_its_key_file_opens_with_it_and_says_so_without_it() {
    let i = match super::read(WITH_KEYFILE, "correct horse battery", Some(KEYFILE)) {
        Ok(i) => i,
        Err(ReadError::Other(e)) => panic!("{e}"),
        Err(_) => panic!("unexpected"),
    };
    let login = i.entries.iter().find(|e| e.label == "Keyed example").expect("read");
    assert_eq!((login.kind.as_str(), login.parts["password"].as_str()), ("login", "hunter2-keyed"));
    // Without it, the password alone is wrong, and the message points at the key file.
    let without = err_of(super::read(WITH_KEYFILE, "correct horse battery", None));
    assert!(without.contains("didn't open") && without.contains("key file"), "{without}");
    // A different key file, or a wrong password with the right one, is refused without a hint to look elsewhere.
    let other_key = KEYFILE.iter().map(|b| if *b == b'a' { b'b' } else { *b }).collect::<Vec<u8>>();
    let wrong_key = err_of(super::read(WITH_KEYFILE, "correct horse battery", Some(&other_key)));
    assert!(wrong_key.contains("password and key file didn't open"), "{wrong_key}");
    let wrong_pw = err_of(super::read(WITH_KEYFILE, "wrong", Some(KEYFILE)));
    assert!(wrong_pw.contains("password and key file didn't open"), "{wrong_pw}");
}

#[test]
fn a_database_with_only_a_key_file_opens_with_no_password() {
    assert!(super::read(KEYFILE_ONLY, "", Some(KEYFILE)).is_ok(), "key file alone");
    // Nothing at all asks for a password first.
    assert_eq!(err_of(super::read(KEYFILE_ONLY, "", None)), "needs a password");
    // A password that isn't part of it fails.
    assert!(err_of(super::read(KEYFILE_ONLY, "extra", Some(KEYFILE))).contains("didn't open"));
}

#[test]
fn a_file_that_is_not_a_key_file_or_is_huge_is_refused() {
    let big = vec![b'x'; MAX_KEY_FILE + 1];
    assert!(err_of(super::read(WITH_KEYFILE, "correct horse battery", Some(&big))).contains("too large"));
    // KeePass reads any other file as a raw key; an empty one is the only thing it can't use.
    let empty = err_of(super::read(WITH_KEYFILE, "correct horse battery", Some(b"")));
    assert!(empty.contains("key file"), "{empty}");
}
