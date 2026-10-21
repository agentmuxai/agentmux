// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Built from the structure 1Password documents for `.1pux` (accounts, vaults,
//! items, `categoryUuid`, `loginFields`, `sections[].fields[].value` as a
//! one-key object). Not from a real export.

use std::io::Write;

use serde_json::json;

use super::*;

fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut out));
        for (name, bytes) in files {
            zip.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
    }
    out
}

fn export(items: serde_json::Value) -> Vec<u8> {
    let data = json!({ "accounts": [{ "attrs": { "accountName": "Alex" }, "vaults": [{ "attrs": { "name": "Personal" }, "items": items }] }] });
    zip_of(&[("export.attributes", b"{}"), ("export.data", data.to_string().as_bytes())])
}

fn field(id: &str, title: &str, value: serde_json::Value) -> serde_json::Value {
    json!({ "id": id, "title": title, "value": value })
}

fn items() -> serde_json::Value {
    json!([
        {
            "categoryUuid": "001",
            "overview": { "title": "Example", "url": "https://example.com/login", "urls": [{ "url": "https://example.com/login" }] },
            "details": {
                "loginFields": [{ "designation": "username", "value": "alex" }, { "designation": "password", "value": "hunter2-pw" }],
                "notesPlain": "recovery: 4455",
                "sections": [{ "fields": [field("totp", "one-time password", json!({ "totp": "otpauth://totp/Example?secret=JBSWY3DPEHPK3PXP" })), field("pin", "PIN", json!({ "concealed": "9876" }))] }]
            }
        },
        {
            "categoryUuid": "002",
            "overview": { "title": "Visa" },
            "details": { "sections": [{ "fields": [
                field("ccnum", "number", json!({ "creditCardNumber": "4111111111111111" })),
                field("expiry", "expiry date", json!({ "monthYear": 203012 })),
                field("cvv", "verification number", json!({ "concealed": "123" })),
                field("cardholder", "cardholder name", json!({ "string": "Alex Doe" })),
                field("type", "type", json!({ "creditCardType": "visa" }))
            ] }] }
        },
        {
            "categoryUuid": "101",
            "overview": { "title": "Checking" },
            "details": { "sections": [{ "fields": [field("bankName", "bank name", json!({ "string": "First Bank" })), field("routingNo", "routing number", json!({ "string": "021000021" })), field("accountNo", "account number", json!({ "string": "000123456789" }))] }] }
        },
        {
            "categoryUuid": "108",
            "overview": { "title": "My SSN" },
            "details": { "sections": [{ "fields": [field("name", "name", json!({ "string": "Alex Doe" })), field("number", "number", json!({ "concealed": "123-45-6789" }))] }] }
        },
        {
            "categoryUuid": "112",
            "overview": { "title": "Stripe key", "url": "https://stripe.com" },
            "details": { "sections": [{ "fields": [field("type", "type", json!({ "string": "jwt" })), field("credential", "credential", json!({ "concealed": "sk_live_abcdefgh1234" }))] }] }
        },
        { "categoryUuid": "003", "overview": { "title": "Wifi" }, "details": { "notesPlain": "password is correct-horse" } },
        {
            "categoryUuid": "110",
            "overview": { "title": "Build server" },
            "details": { "sections": [{ "fields": [field("url", "URL", json!({ "url": "ssh://build.example.com" })), field("password", "password", json!({ "concealed": "srv-secret-1" }))] }] }
        },
        { "categoryUuid": "001", "trashed": true, "overview": { "title": "Old" }, "details": { "loginFields": [{ "designation": "password", "value": "x1234" }] } },
        { "categoryUuid": "001", "overview": { "title": "No secret" }, "details": {} }
    ])
}

fn imported() -> Imported {
    match read(&export(items())) {
        Ok(i) => i,
        Err(ReadError::Other(e)) => panic!("{e}"),
        Err(_) => panic!("unexpected"),
    }
}

#[test]
fn a_1password_export_comes_in_by_category() {
    let i = imported();
    assert_eq!(i.source, Source::OnePassword);
    assert!(i.plaintext, "the pane offers to delete it");
    let by = |label: &str| i.entries.iter().find(|e| e.label == label).unwrap_or_else(|| panic!("no {label}: {:?}", i.entries.iter().map(|e| &e.label).collect::<Vec<_>>()));
    let login = by("Example");
    assert_eq!((login.kind.as_str(), login.parts["password"].as_str(), login.fields["username"].as_str()), ("login", "hunter2-pw", "alex"));
    assert!(login.parts["totp"].starts_with("otpauth://") && login.fields["site"] == "https://example.com/login");
    assert_eq!(login.tags, vec!["Personal".to_string()]);
    // The notes and the field the login has no place for are kept sealed, not dropped.
    let notes = by("Example (notes)");
    assert!(notes.parts["text"].contains("recovery: 4455") && notes.parts["text"].contains("PIN: 9876"));
    assert!(!notes.parts["text"].contains("otpauth"), "the seed went to the login");

    let card = by("Visa");
    assert_eq!((card.kind.as_str(), card.parts["number"].as_str(), card.parts["expiry"].as_str()), ("card", "4111111111111111", "12/2030"));
    assert_eq!((card.parts["cvc"].as_str(), card.parts["name"].as_str()), ("123", "Alex Doe"));
    let bank = by("Checking");
    assert_eq!((bank.kind.as_str(), bank.parts["account"].as_str(), bank.parts["routing"].as_str(), bank.fields["bank"].as_str()), ("bank-account", "000123456789", "021000021", "First Bank"));
    let ssn = by("My SSN");
    assert_eq!((ssn.kind.as_str(), ssn.parts["value"].as_str(), ssn.fields["owner"].as_str()), ("ssn", "123-45-6789", "Alex Doe"));
    let key = by("Stripe key");
    assert_eq!((key.kind.as_str(), key.parts["value"].as_str()), ("api-key", "sk_live_abcdefgh1234"));
    assert_eq!(by("Wifi").kind, "note");
}

#[test]
fn an_item_with_no_kind_in_the_vault_is_kept_whole_as_a_note() {
    let i = imported();
    let server = i.entries.iter().find(|e| e.label == "Build server").expect("kept");
    assert_eq!(server.kind, "note");
    assert!(server.parts["text"].contains("URL: ssh://build.example.com") && server.parts["text"].contains("password: srv-secret-1"));
}

#[test]
fn the_trash_and_empty_items_are_listed_not_imported() {
    let i = imported();
    assert!(i.entries.iter().all(|e| e.label != "Old" && e.label != "No secret"));
    let why: std::collections::HashMap<_, _> = i.skipped.iter().cloned().collect();
    assert!(why["Old"].contains("trash"), "{why:?}");
    assert!(why["No secret"].contains("no password"), "{why:?}");
}

#[test]
fn a_zip_that_isnt_a_1password_export_is_refused_with_why() {
    let other = zip_of(&[("readme.txt", b"hi")]);
    assert!(!looks_like(&other));
    assert!(matches!(read(&other), Err(ReadError::Other(e)) if e.contains("no export.data")));
    let bad = zip_of(&[("export.data", b"not json")]);
    assert!(matches!(read(&bad), Err(ReadError::Other(e)) if e.contains("JSON")));
    let empty = zip_of(&[("export.data", br#"{"accounts": []}"#)]);
    assert!(matches!(read(&empty), Err(ReadError::Other(e)) if e.contains("no items")));
    assert!(matches!(read(b"PK\x03\x04 not really a zip"), Err(ReadError::Other(_))));
    assert!(looks_like(&export(items())));
}

#[test]
fn a_huge_export_data_is_refused_without_reading_it_all() {
    // A zip bomb in miniature: a highly compressible export.data over the cap.
    let mut big = Vec::new();
    big.extend_from_slice(br#"{"accounts": ["#);
    big.resize((MAX_DATA_BYTES + 10) as usize, b' ');
    let mut out = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut out));
        zip.start_file("export.data", zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated)).unwrap();
        zip.write_all(&big).unwrap();
        zip.finish().unwrap();
    }
    assert!(out.len() < 1024 * 1024, "the bomb is small on disk: {}", out.len());
    assert!(matches!(read(&out), Err(ReadError::Other(e)) if e.contains("larger than")));
}

#[test]
fn a_login_with_no_password_keeps_its_username_and_addresses_in_the_note() {
    let items = json!([
        {
            "categoryUuid": "001",
            "overview": { "title": "Forum", "url": "https://forum.example.com", "urls": [{ "url": "https://forum.example.com" }, { "url": "https://forum.example.com/alt" }] },
            "details": { "loginFields": [{ "designation": "username", "value": "alex" }, { "designation": "", "name": "account no", "value": "A-77" }], "notesPlain": "joined 2019" }
        },
        {
            "categoryUuid": "001",
            "overview": { "title": "Bank site", "url": "https://bank.example.com", "urls": [{ "url": "https://bank.example.com/en" }] },
            "details": { "loginFields": [{ "designation": "username", "value": "alex2" }, { "designation": "password", "value": "pw-1234" }, { "designation": "", "name": "memorable word", "value": "tulip" }] }
        },
        { "categoryUuid": "110", "overview": { "title": "Box", "url": "ssh://box.example.com" }, "details": { "notesPlain": "rack 4" } }
    ]);
    let i = read(&export(items)).ok().expect("reads");
    let text = |label: &str| i.entries.iter().find(|e| e.label == label).unwrap_or_else(|| panic!("no {label}")).parts["text"].clone();
    let forum = text("Forum");
    assert!(forum.contains("joined 2019") && forum.contains("username: alex") && forum.contains("account no: A-77"), "{forum}");
    assert!(forum.contains("url: https://forum.example.com") && forum.contains("url: https://forum.example.com/alt"), "{forum}");
    // A login keeps its first address and username; its other addresses and fields are kept in the note.
    assert_eq!(i.entries.iter().find(|e| e.label == "Bank site").unwrap().fields["site"], "https://bank.example.com");
    let bank = text("Bank site (notes)");
    assert!(bank.contains("url: https://bank.example.com/en") && bank.contains("memorable word: tulip"), "{bank}");
    assert!(!bank.contains("pw-1234") && !bank.contains("alex2"), "{bank}");
    assert_eq!(text("Box"), "rack 4
url: ssh://box.example.com");
}

#[test]
fn a_username_only_login_is_kept_and_an_id_item_without_its_number_is_not_guessed() {
    let items = json!([
        { "categoryUuid": "001", "overview": { "title": "Old forum", "url": "https://old.example.com" }, "details": { "loginFields": [{ "designation": "username", "value": "alex" }] } },
        { "categoryUuid": "106", "overview": { "title": "Half passport" }, "details": { "sections": [{ "fields": [field("fullname", "full name", json!({ "string": "Alex Doe" }))] }] } },
        { "categoryUuid": "112", "overview": { "title": "Typed only" }, "details": { "sections": [{ "fields": [field("type", "type", json!({ "string": "jwt" }))] }] } }
    ]);
    let i = read(&export(items)).ok().expect("reads");
    let get = |label: &str| i.entries.iter().find(|e| e.label == label).unwrap_or_else(|| panic!("no {label}"));
    let forum = get("Old forum");
    assert_eq!(forum.kind, "note");
    assert!(forum.parts["text"].contains("username: alex") && forum.parts["text"].contains("url: https://old.example.com"), "{:?}", forum.parts);
    // Nothing that isn't the number becomes the number.
    for label in ["Half passport", "Typed only"] {
        let e = get(label);
        assert_eq!(e.kind, "note", "{label}");
    }
    assert!(get("Half passport").parts["text"].contains("full name: Alex Doe"));
}
