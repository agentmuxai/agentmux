// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The KeePass export: every kind, read back by the importer, and the file itself.

use std::collections::BTreeMap;

use super::*;
use crate::backend::vault::import;
use crate::backend::vault::seal::{self, Parts};
use crate::backend::vault::{create, NewEntry};

const PASS: &str = "correct horse battery staple";

/// Cheap, so a test isn't spent stretching a password.
fn cheap() -> KdfConfig {
    KdfConfig::Aes { rounds: 100 }
}

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

fn add(s: &Store, kind: &str, label: &str, f: &[(&str, &str)], tags: &[&str], p: &[(&str, &str)]) -> String {
    create(s, NewEntry { kind: kind.into(), label: label.into(), fields: fields(f), tags: tags.iter().map(|t| t.to_string()).collect(), parts: parts(p) }).unwrap()
}

/// One entry of each shape: every kind, with its details and tags.
fn seed(s: &Store) {
    add(s, "ssn", "Alex SSN", &[("owner", "Alex Doe"), ("notes", "from the card")], &["Personal/Tax"], &[("value", "123-45-6789")]);
    add(s, "card", "Company card", &[("owner", "A Doe")], &[], &[("number", "4111111111111111"), ("expiry", "12/30"), ("cvc", "123"), ("name", "A DOE")]);
    add(s, "bank-account", "Checking", &[("bank", "First Bank"), ("owner", "A Doe")], &["Money"], &[("account", "000123456789"), ("routing", "021000021")]);
    add(s, "api-key", "Stripe key", &[("service", "Stripe"), ("env_name", "STRIPE_API_KEY"), ("expires", "2027-01-01")], &["Work", "Dev"], &[("value", "sk_live_abcdefgh1234")]);
    add(s, "passport", "Passport", &[("owner", "Alex Doe"), ("issuer", "USA"), ("expires", "2031-05-01")], &[], &[("value", "X1234567")]);
    add(s, "drivers-licence", "Licence", &[("owner", "Alex Doe")], &[], &[("value", "D123-4567")]);
    add(s, "login", "Example", &[("site", "https://example.com/login"), ("username", "alex"), ("notes", "recovery 4455")], &["Websites/Work"], &[("password", "  hunter2 with spaces  "), ("totp", "JBSWY3DPEHPK3PXP")]);
    add(s, "note", "Router", &[], &["Websites"], &[("text", "admin / correct-horse\nsecond line")]);
}

fn exported(s: &Store) -> Vec<u8> {
    let entries = gather(s, None).unwrap();
    build(&entries, PASS, cheap()).unwrap()
}

#[test]
fn every_kind_leaves_as_a_keepass_entry_and_comes_back_as_the_same_entry() {
    let s = store();
    seed(&s);
    let bytes = exported(&s);
    let back = match import::read_with(&bytes, "x.kdbx", None, Some(PASS)) {
        Ok(i) => i,
        Err(import::ReadError::Other(e)) => panic!("{e}"),
        Err(_) => panic!("unexpected"),
    };
    assert!(back.skipped.is_empty(), "{:?}", back.skipped);
    let by = |label: &str| back.entries.iter().find(|e| e.label == label).unwrap_or_else(|| panic!("no {label}: {:?}", back.entries.iter().map(|e| (&e.label, &e.kind)).collect::<Vec<_>>()));
    let ssn = by("Alex SSN");
    assert_eq!((ssn.kind.as_str(), ssn.parts["value"].as_str(), ssn.fields["owner"].as_str()), ("ssn", "123-45-6789", "Alex Doe"));
    assert_eq!(ssn.tags, vec!["Personal/Tax".to_string()]);
    // Plain notes of a typed entry stay with it, sealed, in a note beside it (KeePass keeps them as notes).
    assert_eq!(by("Alex SSN (notes)").parts["text"], "from the card");

    let card = by("Company card");
    assert_eq!(card.kind, "card");
    for (part, want) in [("number", "4111111111111111"), ("expiry", "12/30"), ("cvc", "123"), ("name", "A DOE")] {
        assert_eq!(card.parts[part], want, "card {part}");
    }
    let bank = by("Checking");
    assert_eq!((bank.kind.as_str(), bank.parts["account"].as_str(), bank.parts["routing"].as_str(), bank.fields["bank"].as_str()), ("bank-account", "000123456789", "021000021", "First Bank"));
    let key = by("Stripe key");
    assert_eq!((key.kind.as_str(), key.parts["value"].as_str(), key.fields["env_name"].as_str(), key.fields["service"].as_str()), ("api-key", "sk_live_abcdefgh1234", "STRIPE_API_KEY", "Stripe"));
    assert_eq!(key.fields["expires"], "2027-01-01");
    assert_eq!(key.tags, vec!["Work".to_string()]);
    let passport = by("Passport");
    assert_eq!((passport.kind.as_str(), passport.parts["value"].as_str(), passport.fields["issuer"].as_str(), passport.fields["expires"].as_str()), ("passport", "X1234567", "USA", "2031-05-01"));
    assert_eq!(by("Licence").parts["value"], "D123-4567");

    let login = by("Example");
    assert_eq!((login.kind.as_str(), login.fields["username"].as_str(), login.fields["site"].as_str()), ("login", "alex", "https://example.com/login"));
    assert_eq!(login.parts["password"], "  hunter2 with spaces  ", "a password keeps its spaces");
    assert!(login.parts["totp"].contains("secret=JBSWY3DPEHPK3PXP"), "{:?}", login.parts);
    assert_eq!(login.tags, vec!["Websites/Work".to_string()]);
    assert_eq!(by("Example (notes)").parts["text"], "recovery 4455");
    let note = by("Router");
    assert_eq!((note.kind.as_str(), note.parts["text"].as_str()), ("note", "admin / correct-horse\nsecond line"));
    assert_eq!(note.tags, vec!["Websites".to_string()]);
}

#[test]
fn the_file_is_a_password_protected_keepass_database_with_no_plaintext() {
    let s = store();
    seed(&s);
    let bytes = exported(&s);
    assert_eq!(&bytes[..4], &[0x03, 0xD9, 0xA2, 0x9A], "KeePass's signature");
    for secret in ["123-45-6789", "4111111111111111", "sk_live_abcdefgh1234", "hunter2", "JBSWY3DPEHPK3PXP", "Alex SSN"] {
        assert!(!bytes.windows(secret.len()).any(|w| w == secret.as_bytes()), "{secret} is in the file");
    }
    // Opened by the crate itself, not only by our reader, and not with a wrong password.
    assert!(Database::parse(&bytes, DatabaseKey::new().with_password(PASS)).is_ok());
    assert!(Database::parse(&bytes, DatabaseKey::new().with_password("wrong")).is_err());
}

#[test]
fn an_export_is_checked_before_it_is_written() {
    let s = store();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.kdbx");
    // Nothing to export, a short passphrase.
    assert!(export_with(&s, None, PASS, &path, cheap()).unwrap_err().contains("nothing"));
    seed(&s);
    assert!(export_with(&s, None, "short", &path, cheap()).unwrap_err().contains("at least"));
    assert!(!path.exists());
    let r = export_with(&s, None, PASS, &path, cheap()).unwrap();
    assert_eq!((r.count, r.file_name.as_str()), (8, "out.kdbx"));
    assert!(path.exists());
    // Only the chosen entries.
    let some = dir.path().join("some.kdbx");
    let ids = vec!["ssn-alex-ssn".to_string()];
    assert_eq!(export_with(&s, Some(&ids), PASS, &some, cheap()).unwrap().count, 1);
    // The activity log records it without a value.
    let log = s.vault_activity_list(10).unwrap().iter().map(|a| format!("{} {} {}", a.action, a.target, a.outcome)).collect::<Vec<_>>().join("\n");
    assert!(log.contains("exported out.kdbx") && !log.contains("123-45-6789"), "{log}");
}

#[test]
fn a_seed_that_is_not_an_address_becomes_one_keepassxc_reads() {
    assert_eq!(otp_uri("Example", "jbsw y3dp ehpk 3pxp"), "otpauth://totp/Example?secret=JBSWY3DPEHPK3PXP");
    assert_eq!(otp_uri("My Site", "ABC"), "otpauth://totp/My%20Site?secret=ABC");
    assert_eq!(otp_uri("x", "otpauth://totp/x?secret=ZZ&period=60"), "otpauth://totp/x?secret=ZZ&period=60");
}

#[test]
fn the_default_key_stretching_is_inside_what_the_importer_accepts() {
    let KdfConfig::Argon2 { iterations, memory, parallelism, .. } = default_kdf() else { panic!("Argon2") };
    assert!(memory <= import::MAX_ARGON_MEMORY && iterations <= import::MAX_ARGON_ITERATIONS && u64::from(parallelism) <= import::MAX_ARGON_PARALLELISM);
}
