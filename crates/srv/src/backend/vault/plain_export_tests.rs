// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The plain CSV and JSON export: what the files hold, that AgentMux and other managers' readers
//! take them back, and the care around the write.

use std::collections::BTreeMap;

use super::*;
use crate::backend::vault::import::{self, Source};
use crate::backend::vault::seal::{self, Parts};
use crate::backend::vault::{create, NewEntry};

fn store() -> Store {
    seal::use_test_key();
    let s = Store::open_in_memory().unwrap();
    s.apply_identity_schema_for_tests().unwrap();
    s
}

fn add(s: &Store, kind: &str, label: &str, f: &[(&str, &str)], tags: &[&str], p: &[(&str, &str)]) -> String {
    let map = |pairs: &[(&str, &str)]| -> BTreeMap<String, String> { pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() };
    create(s, NewEntry { kind: kind.into(), label: label.into(), fields: map(f), tags: tags.iter().map(|t| t.to_string()).collect(), parts: Parts(map(p)) }).unwrap()
}

fn seed(s: &Store) {
    add(s, "login", "Example", &[("site", "https://example.com/login"), ("username", "alex"), ("notes", "recovery 4455")], &["Websites/Work"], &[("password", "  hunter2, \"quoted\"\nline  "), ("totp", "JBSWY3DPEHPK3PXP")]);
    add(s, "ssn", "Alex SSN", &[("owner", "Alex Doe"), ("notes", "from the card")], &["Personal"], &[("value", "123-45-6789")]);
    add(s, "card", "Company card", &[], &[], &[("number", "4111111111111111"), ("expiry", "12/30"), ("cvc", "123"), ("name", "A DOE")]);
    add(s, "note", "Router", &[], &[], &[("text", "admin / correct-horse\nsecond line")]);
}

#[test]
fn a_login_is_a_row_with_its_columns_and_everything_else_is_a_secure_note() {
    let s = store();
    seed(&s);
    let bytes = csv_bytes(&gather(&s, None).unwrap()).unwrap();
    let mut reader = csv::Reader::from_reader(&bytes[..]);
    let headers: Vec<String> = reader.headers().unwrap().iter().map(str::to_string).collect();
    assert_eq!(headers, HEADER);
    let rows: Vec<csv::StringRecord> = reader.records().map(Result::unwrap).collect();
    assert_eq!(rows.len(), 4);
    let by = |name: &str| rows.iter().find(|r| &r[3] == name).unwrap_or_else(|| panic!("no row {name}"));
    let login = by("Example");
    assert_eq!((&login[0], &login[2], &login[4], &login[7], &login[8], &login[10]), ("Websites/Work", "login", "recovery 4455", "https://example.com/login", "alex", "JBSWY3DPEHPK3PXP"));
    assert_eq!(&login[9], "  hunter2, \"quoted\"\nline  ", "a password keeps its spaces, quote and line break");
    let ssn = by("Alex SSN");
    assert_eq!((&ssn[2], &ssn[9]), ("note", ""), "nothing in the login columns");
    assert_eq!(&ssn[4], "SSN: 123-45-6789\nOwner: Alex Doe\n\nfrom the card");
    assert_eq!(&by("Company card")[4], "Card number: 4111111111111111\nExpiry: 12/30\nCVC: 123\nCardholder: A DOE");
    assert_eq!(&by("Router")[4], "admin / correct-horse\nsecond line");
}

#[test]
fn the_csv_comes_back_through_the_importer_with_the_logins_exact() {
    let s = store();
    seed(&s);
    let bytes = csv_bytes(&gather(&s, None).unwrap()).unwrap();
    let back = import::read_keyed(&bytes, "export.csv", None, None, None).ok().expect("imports");
    assert!(back.plaintext);
    let login = back.entries.iter().find(|e| e.kind == "login").unwrap();
    assert_eq!(login.parts["password"], "  hunter2, \"quoted\"\nline  ");
    assert_eq!(login.fields["username"], "alex");
    assert_eq!(login.tags, vec!["Websites/Work".to_string()]);
    // Its notes, and the secure notes, arrive as notes.
    let notes: Vec<&str> = back.entries.iter().filter(|e| e.kind == "note").map(|e| e.parts["text"].as_str()).collect();
    assert!(notes.contains(&"recovery 4455") && notes.iter().any(|n| n.starts_with("SSN: 123-45-6789")), "{notes:?}");
}

#[test]
fn the_json_is_the_vault_file_unencrypted_and_comes_back_as_the_same_entries() {
    let s = store();
    seed(&s);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.json");
    let r = export(&s, None, "json", &path).unwrap();
    assert_eq!((r.count, r.file_name.as_str()), (4, "out.json"));
    let bytes = std::fs::read(&path).unwrap();
    let back = import::read_keyed(&bytes, "out.json", None, None, None).ok().expect("imports");
    assert_eq!(back.source, Source::Vault);
    assert!(back.plaintext, "so the pane offers to delete the file");
    assert_eq!(back.entries.len(), 4);
    let card = back.entries.iter().find(|e| e.kind == "card").unwrap();
    assert_eq!((card.parts["number"].as_str(), card.parts["cvc"].as_str()), ("4111111111111111", "123"));
    let ssn = back.entries.iter().find(|e| e.kind == "ssn").unwrap();
    assert_eq!((ssn.fields["owner"].as_str(), ssn.fields["notes"].as_str()), ("Alex Doe", "from the card"));
}

#[test]
fn another_managers_json_is_still_read_as_theirs() {
    let bitwarden = br#"{"encrypted":false,"folders":[],"items":[{"type":1,"name":"x","login":{"username":"u","password":"p"}}]}"#;
    let back = import::read_keyed(bitwarden, "bw.json", None, None, None).ok().expect("reads");
    assert_eq!(back.source, Source::Bitwarden);
    let newer = br#"{"format":"agentmux-vault","version":99,"entries":[]}"#;
    match import::read_keyed(newer, "x.json", None, None, None) {
        Err(import::ReadError::Other(e)) => assert!(e.contains("newer AgentMux"), "{e}"),
        _ => panic!("a newer file is refused"),
    }
}

#[test]
fn an_export_is_checked_before_it_is_written_and_logged_without_a_value() {
    let s = store();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.csv");
    assert!(export(&s, None, "csv", &path).unwrap_err().contains("nothing"));
    seed(&s);
    assert!(export(&s, None, "xml", &path).unwrap_err().contains("unknown"));
    assert!(!path.exists(), "nothing written for a refused export");
    let r = export(&s, None, "csv", &path).unwrap();
    assert_eq!(r.count, 4);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("123-45-6789") && text.contains("4111111111111111"), "it is the plain file the person asked for");
    let log = s.vault_activity_list(10).unwrap().iter().map(|a| format!("{} {} {}", a.action, a.target, a.outcome)).collect::<Vec<_>>().join("\n");
    assert!(log.contains("exported out.csv") && log.contains("unencrypted CSV") && !log.contains("123-45-6789") && !log.contains("hunter2"), "{log}");
    // No temporary file is left beside it.
    let left: Vec<_> = std::fs::read_dir(dir.path()).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(left, vec!["out.csv".to_string()]);
}

#[cfg(unix)]
#[test]
fn the_plain_file_is_readable_by_its_owner_alone() {
    use std::os::unix::fs::PermissionsExt;
    let s = store();
    seed(&s);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.json");
    export(&s, None, "json", &path).unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
}
