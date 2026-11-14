// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Proton Pass, Enpass and Sticky Password (spec §12 P6) against files in each service's export
//! format. The files are written from the format as the services' own code (Proton Pass) or
//! Bitwarden's open-source importer (Enpass, Sticky Password) reads it, with made-up values, not
//! from exports the services made: a mismatch with a real export is likely to be a member name,
//! not a crash. Each test says what it checks.

use std::io::Write;

use serde_json::{json, Value};

use super::second_wave_tests::{find, read_ok, text_of};
use super::*;

// The lines that frame a private key, written in two pieces so no scanner takes the test for a leak.
const KEY_BEGIN: &str = concat!("-----BEGIN OPENSSH ", "PRIVATE KEY-----");
const KEY_END: &str = concat!("-----END OPENSSH ", "PRIVATE KEY-----");

pub(super) fn proton_json() -> String {
    let login = json!({
        "itemId": "i1", "shareId": "s1", "state": 1, "aliasEmail": null, "createTime": 1, "modifyTime": 2, "pinned": false, "folderId": "f2",
        "data": {
            "metadata": { "name": "Example", "note": "recovery 1234", "itemUuid": "u1" },
            "extraFields": [
                { "fieldName": "PIN", "type": "hidden", "data": { "content": "4321" } },
                { "fieldName": "Backup code", "type": "totp", "data": { "totpUri": "otpauth://totp/B?secret=ZZZ" } },
                { "fieldName": "Since", "type": "timestamp", "data": { "timestamp": "2024-01-02T00:00:00Z" } }
            ],
            "type": "login",
            "content": {
                "itemEmail": "bob@example.test", "itemUsername": "bob", "password": "  pw 1 ", "totpUri": "otpauth://totp/E?secret=JBSWY3DP",
                "urls": ["https://example.test/login"],
                "autofillUrls": [{ "url": "https://example.test/login", "mode": 0 }, { "url": "https://b.test", "mode": 1 }],
                "passkeys": [{ "keyId": "k" }]
            }
        }
    });
    let email_only = json!({ "state": 1, "data": { "metadata": { "name": "", "note": "", "itemUuid": "u2" }, "extraFields": [], "type": "login",
        "content": { "itemEmail": "amy@example.test", "itemUsername": "", "password": "s3cret", "urls": ["https://amy.test"], "passkeys": [] } } });
    let alias = json!({ "state": 1, "aliasEmail": "x.y@alias.test", "data": { "metadata": { "name": "Shop alias", "note": "for shops", "itemUuid": "u3" }, "extraFields": [], "type": "alias", "content": {} } });
    let note = json!({ "state": 1, "data": { "metadata": { "name": "Memo", "note": "remember the milk", "itemUuid": "u4" }, "extraFields": [], "type": "note", "content": {} } });
    let card = json!({ "state": 1, "data": { "metadata": { "name": "Visa", "note": "limit 500", "itemUuid": "u5" }, "extraFields": [], "type": "creditCard",
        "content": { "cardholderName": "Jane Doe", "cardType": 1, "number": "4111111111111111", "verificationNumber": "123", "expirationDate": "2030-03", "pin": "9999" } } });
    let identity = json!({ "state": 1, "data": { "metadata": { "name": "Me", "note": "", "itemUuid": "u6" }, "extraFields": [{ "fieldName": "Extra", "type": "text", "data": { "content": "top" } }], "type": "identity",
        "content": { "fullName": "Jane Doe", "email": "jane@example.test", "socialSecurityNumber": "123-45-6789", "passportNumber": "X9876543", "licenseNumber": "",
            "city": "Springfield", "streetAddress": "1 Main St",
            "extraAddressDetails": [{ "fieldName": "Landmark", "type": "text", "data": { "content": "the old mill" } }],
            "extraSections": [{ "sectionName": "Pets", "sectionFields": [{ "fieldName": "Dog", "type": "text", "data": { "content": "Rex" } }] }] } } });
    let ssh = json!({ "state": 1, "data": { "metadata": { "name": "Server key", "note": "", "itemUuid": "u7" }, "extraFields": [], "type": "sshKey",
        "content": { "privateKey": format!("{KEY_BEGIN}\nabc\n{KEY_END}"), "publicKey": "ssh-ed25519 AAAA", "sections": [] } } });
    let wifi = json!({ "state": 1, "data": { "metadata": { "name": "Home Wi-Fi", "note": "", "itemUuid": "u8" }, "extraFields": [], "type": "wifi",
        "content": { "ssid": "home", "password": "wifi-pass", "security": 2, "sections": [] } } });
    let custom = json!({ "state": 1, "data": { "metadata": { "name": "Router", "note": "", "itemUuid": "u9" }, "extraFields": [], "type": "custom",
        "content": { "sections": [{ "sectionName": "Admin", "sectionFields": [{ "fieldName": "Admin password", "type": "hidden", "data": { "content": "router-pw" } }] }] } } });
    let trashed = json!({ "state": 2, "data": { "metadata": { "name": "Old login", "note": "", "itemUuid": "u10" }, "extraFields": [], "type": "login",
        "content": { "itemEmail": "", "itemUsername": "old", "password": "old", "urls": [], "passkeys": [] } } });
    let items: Vec<Value> = vec![login, email_only, alias, note, card, identity, ssh, wifi, custom, trashed];
    json!({
        "version": "1.20.0", "userId": "user", "encrypted": false,
        "vaults": { "s1": { "name": "Personal", "description": "", "display": {}, "items": items,
            "folders": [{ "folderId": "f1", "parentFolderId": null, "name": "Work" }, { "folderId": "f2", "parentFolderId": "f1", "name": "Servers" }] } }
    })
    .to_string()
}

fn csv(rows: &[&[&str]]) -> String {
    let lines: Vec<String> = rows.iter().map(|r| r.iter().map(|c| format!("\"{}\"", c.replace('"', "\"\""))).collect::<Vec<_>>().join(",")).collect();
    lines.join("\n") + "\n"
}

pub(super) fn proton_csv() -> String {
    let card_note = json!({ "cardholderName": "Jane Doe", "cardType": 1, "number": "4111111111111111", "verificationNumber": "123", "expirationDate": "2030-03", "pin": "9999", "note": "limit 500" }).to_string();
    let identity_note = json!({ "fullName": "Jane Doe", "passportNumber": "X9876543", "city": "Springfield", "note": "" }).to_string();
    let urls = json!([{ "url": "https://example.test", "mode": 0 }, { "url": "https://b.test", "mode": 1 }]).to_string();
    csv(&[
        &["type", "name", "url", "autofillUrls", "email", "username", "password", "note", "totp", "createTime", "modifyTime", "vault"],
        &["login", "Example", "https://example.test", &urls, "bob@example.test", "bob", " pw, 1 ", "a note", "otpauth://totp/E?secret=ABC", "1", "2", "Personal"],
        &["alias", "Shop alias", "", "", "x.y@alias.test", "", "", "for shops", "", "1", "2", "Personal"],
        &["creditCard", "Visa", "", "", "", "", "", &card_note, "", "1", "2", "Personal"],
        &["identity", "Me", "", "", "", "", "", &identity_note, "", "1", "2", "Personal"],
        &["note", "Memo", "", "", "", "", "", "remember the milk", "", "1", "2", "Work"],
    ])
}

fn zip_with(files: &[(&str, &[u8])]) -> Vec<u8> {
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

pub(super) fn proton_zip() -> Vec<u8> {
    zip_with(&[("Proton Pass/data.json", proton_json().as_bytes()), ("Proton Pass/files/photo.png", b"png")])
}

pub(super) fn enpass_json() -> String {
    json!({
        "folders": [{ "uuid": "fa", "title": "Work", "parent_uuid": "" }, { "uuid": "fb", "title": "Servers", "parent_uuid": "fa" }],
        "items": [
            { "title": "Example", "template_type": "login.default", "category": "login", "note": "keep safe", "trashed": 0, "folders": ["fb"], "fields": [
                { "label": "Username", "type": "username", "value": "bob", "deleted": 0 },
                { "label": "E-mail", "type": "email", "value": "bob@example.test", "deleted": 0 },
                { "label": "Password", "type": "password", "value": " pw 1 ", "deleted": 0, "sensitive": 1 },
                { "label": "Website", "type": "url", "value": "https://example.test/login", "deleted": 0 },
                { "label": "Sign-in page", "type": "url", "value": "https://b.test", "deleted": 0 },
                { "label": "TOTP", "type": "totp", "value": "otpauth://totp/E?secret=JBSWY3DP", "deleted": 0 },
                { "label": "Security answer", "type": "text", "value": "blue", "deleted": 0 },
                { "label": "Old", "type": "text", "value": "gone", "deleted": 1 },
                { "label": "Details", "type": "section", "value": "", "deleted": 0 }
            ] },
            { "title": "Phone app", "template_type": "login.default", "trashed": 0, "fields": [
                { "label": "Username", "type": "username", "value": "amy", "deleted": 0 },
                { "label": "Password", "type": "password", "value": "app-pw", "deleted": 0 },
                { "label": "Android app", "type": ".Android#", "value": "androidapp://com.example.app", "deleted": 0 }
            ] },
            { "title": "Visa", "template_type": "creditcard.default", "trashed": 0, "note": "limit 500", "fields": [
                { "label": "Cardholder", "type": "ccName", "value": "Jane Doe", "deleted": 0 },
                { "label": "Number", "type": "ccNumber", "value": "4111111111111111", "deleted": 0 },
                { "label": "CVC", "type": "ccCvc", "value": "123", "deleted": 0 },
                { "label": "Expiry date", "type": "ccExpiry", "value": "03/30", "deleted": 0 },
                { "label": "Type", "type": "ccType", "value": "visa", "deleted": 0 },
                { "label": "PIN", "type": "ccPin", "value": "9999", "deleted": 0 }
            ] },
            { "title": "Memo", "template_type": "note.default", "trashed": 0, "note": "remember the milk", "fields": [] },
            { "title": "Me", "template_type": "identity.default", "trashed": 0, "fields": [
                { "label": "Name", "type": "text", "value": "Jane Doe", "deleted": 0 },
                { "label": "Password hint", "type": "password", "value": "not a login", "deleted": 0 }
            ] },
            { "title": "Old", "template_type": "login.default", "trashed": 1, "fields": [{ "label": "Password", "type": "password", "value": "x", "deleted": 0 }] }
        ]
    })
    .to_string()
}

pub(super) fn enpass_csv() -> String {
    csv(&[
        &["Title", "Field", "Value", "Field", "Value", "Field", "Value", "Field", "Value", "Note"],
        &["Example", "Username", "bob", "*Password", " pw, 1 ", "Website", "https://example.test", "Security answer", "blue", "keep safe"],
        &["Visa", "Cardholder", "Jane Doe", "Number", "4111111111111111", "Expiry date", "03/2030", "CVC", "123"],
        &["Memo", "Text", "remember the milk"],
    ])
}

pub(super) const STICKY: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<root><Database>
<Groups><Group ID="1" Name="Work" ParentID="0"/><Group ID="2" Name="Servers" ParentID="1"/></Groups>
<Logins>
<Login ID="10" Name="bob" Password=" pw &amp; 1 "/>
<Login ID="11" Name="amy" Password="s3cret"/>
<Login ID="" Name="nobody" Password="x"/>
<Login ID="12" Name="empty" Password=""/>
</Logins>
<Accounts>
<Account ID="100" Name="Example" Link="https://example.test/login" ParentID="2" Comments="first/nsecond"><LoginLinks><Login SourceLoginID="10"/></LoginLinks></Account>
<Account ID="101" Name="" Link="https://amy.test" ParentID="" Comments=""><LoginLinks><Login SourceLoginID="11"/></LoginLinks></Account>
</Accounts>
</Database></root>"#;

#[test]
fn proton_json_items_come_in_by_their_type() {
    let i = read_ok(&proton_json(), "data.json");
    assert_eq!(i.source, Source::ProtonPass);
    assert!(i.plaintext);

    let login = find(&i, "Example");
    assert_eq!(login.kind, "login");
    assert_eq!((login.parts["password"].as_str(), login.parts["totp"].as_str()), ("  pw 1 ", "otpauth://totp/E?secret=JBSWY3DP"), "a password is kept exactly as exported");
    assert_eq!((login.fields["username"].as_str(), login.fields["site"].as_str()), ("bob", "https://example.test/login"));
    assert_eq!(login.tags, ["Personal/Work/Servers"], "the vault and the folders it sits in become a tag");
    let notes = text_of(&i, "Example (notes)");
    for want in ["recovery 1234", "email: bob@example.test", "url: https://b.test", "PIN: 4321", "Backup code: otpauth://totp/B?secret=ZZZ", "Since: 2024-01-02T00:00:00Z"] {
        assert!(notes.contains(want), "{want:?} not in {notes}");
    }
    assert!(!login.fields.contains_key("notes"), "no secret sits in a plain field");
    assert!(i.skipped.iter().any(|(l, why)| l == "Example (passkey)" && why.contains("passkey")), "{:?}", i.skipped);

    let amy = find(&i, "amy.test");
    assert_eq!((amy.fields["username"].as_str(), amy.parts["password"].as_str()), ("amy@example.test", "s3cret"), "a login with no username is signed into with its email");

    assert!(text_of(&i, "Shop alias").contains("alias: x.y@alias.test"));
    assert_eq!(text_of(&i, "Memo"), "remember the milk");
    assert!(!i.entries.iter().any(|e| e.label == "Old login"));
    assert!(i.skipped.iter().any(|(l, why)| l == "Old login" && why.contains("trash")), "{:?}", i.skipped);
}

#[test]
fn proton_cards_identities_keys_and_the_rest_keep_what_they_hold() {
    let i = read_ok(&proton_json(), "data.json");
    let card = find(&i, "Visa");
    assert_eq!(card.kind, "card");
    assert_eq!((card.parts["number"].as_str(), card.parts["cvc"].as_str(), card.parts["name"].as_str(), card.parts["expiry"].as_str()), ("4111111111111111", "123", "Jane Doe", "03/2030"));
    let notes = text_of(&i, "Visa (notes)");
    assert!(notes.contains("limit 500") && notes.contains("PIN: 9999"), "{notes}");

    assert_eq!(find(&i, "Me (Social Security number)").kind, "ssn");
    let passport = find(&i, "Me (passport)");
    assert_eq!((passport.kind.as_str(), passport.parts["value"].as_str(), passport.fields["owner"].as_str()), ("passport", "X9876543", "Jane Doe"));
    assert!(!i.entries.iter().any(|e| e.label == "Me (driver's licence)"), "an empty number makes no entry");
    let me = text_of(&i, "Me");
    for want in ["city: Springfield", "streetAddress: 1 Main St", "Landmark: the old mill", "Pets", "Dog: Rex", "Extra: top"] {
        assert!(me.contains(want), "{want:?} not in {me}");
    }
    assert!(!me.contains("123-45-6789") && !me.contains("X9876543"), "an id number is its own entry, not text in the note");

    assert!(text_of(&i, "Server key").contains(&format!("private key: {KEY_BEGIN}")));
    let wifi = text_of(&i, "Home Wi-Fi");
    assert!(wifi.contains("SSID: home") && wifi.contains("password: wifi-pass") && wifi.contains("security: WPA2"), "{wifi}");
    assert!(text_of(&i, "Router").contains("Admin password: router-pw"));
}

#[test]
fn a_proton_zip_is_read_and_its_attachments_are_listed_as_skipped() {
    let i = read(&proton_zip(), "export.zip", None).map_err(|_| "unreadable").unwrap();
    assert_eq!(i.source, Source::ProtonPass);
    assert!(i.entries.iter().any(|e| e.label == "Example"));
    assert!(i.skipped.iter().any(|(l, why)| l == "attachments" && why.contains("1 attached file")), "{:?}", i.skipped);
}

#[test]
fn a_passphrase_protected_proton_export_is_refused_with_what_to_do() {
    for bytes in [zip_with(&[("Proton Pass/data.pgp", b"-----BEGIN PGP MESSAGE-----\n\nabc\n-----END PGP MESSAGE-----")]), b"-----BEGIN PGP MESSAGE-----\n\nabc\n-----END PGP MESSAGE-----\n".to_vec()] {
        match read(&bytes, "export", None) {
            Err(ReadError::Other(e)) => assert!(e.contains("passphrase") && e.contains("export it again"), "{e}"),
            _ => panic!("expected a refusal"),
        }
    }
}

#[test]
fn a_zip_that_is_neither_1password_nor_proton_pass_is_still_refused() {
    match read(&zip_with(&[("something.txt", b"hi")]), "x.zip", None) {
        Err(ReadError::Other(e)) => assert!(e.contains("isn't a 1Password or Proton Pass export"), "{e}"),
        _ => panic!("expected a refusal"),
    }
}

#[test]
fn proton_csv_rows_come_in_by_their_type() {
    let i = read_ok(&proton_csv(), "Proton Pass export.csv");
    assert_eq!(i.source, Source::ProtonPass);
    let login = find(&i, "Example");
    assert_eq!((login.parts["password"].as_str(), login.parts["totp"].as_str()), (" pw, 1 ", "otpauth://totp/E?secret=ABC"));
    assert_eq!((login.fields["username"].as_str(), login.fields["site"].as_str()), ("bob", "https://example.test"));
    assert_eq!(login.tags, ["Personal"]);
    let notes = text_of(&i, "Example (notes)");
    assert!(notes.contains("a note") && notes.contains("email: bob@example.test") && notes.contains("url: https://b.test"), "{notes}");

    assert!(text_of(&i, "Shop alias").contains("alias: x.y@alias.test"));
    let card = find(&i, "Visa");
    assert_eq!((card.kind.as_str(), card.parts["number"].as_str(), card.parts["expiry"].as_str()), ("card", "4111111111111111", "03/2030"));
    assert!(text_of(&i, "Visa (notes)").contains("limit 500"));
    assert_eq!(find(&i, "Me (passport)").parts["value"], "X9876543");
    assert!(text_of(&i, "Me").contains("city: Springfield"));
    let memo = find(&i, "Memo");
    assert_eq!((memo.kind.as_str(), memo.tags.as_slice()), ("note", &["Work".to_string()][..]));
}

#[test]
fn an_older_proton_csv_has_the_email_in_its_username_column() {
    let old = csv(&[&["name", "url", "username", "password", "note", "totp", "createTime", "modifyTime", "type", "vault"], &["Old", "https://old.test", "me@old.test", "pw", "", "", "1", "2", "login", "Personal"]]);
    let i = read_ok(&old, "old.csv");
    assert_eq!(find(&i, "Old").fields["username"], "me@old.test");
}

#[test]
fn enpass_json_items_come_in_by_their_template() {
    let i = read_ok(&enpass_json(), "enpass.json");
    assert_eq!(i.source, Source::Enpass);
    assert!(i.plaintext);
    let login = find(&i, "Example");
    assert_eq!((login.parts["password"].as_str(), login.parts["totp"].as_str()), (" pw 1 ", "otpauth://totp/E?secret=JBSWY3DP"), "a password is kept exactly as exported");
    assert_eq!((login.fields["username"].as_str(), login.fields["site"].as_str()), ("bob", "https://example.test/login"));
    assert_eq!(login.tags, ["Work/Servers"]);
    let notes = text_of(&i, "Example (notes)");
    for want in ["keep safe", "E-mail: bob@example.test", "url: https://b.test", "Security answer: blue"] {
        assert!(notes.contains(want), "{want:?} not in {notes}");
    }
    assert!(!notes.contains("gone"), "a deleted field's old value is not imported");

    let app = find(&i, "Phone app");
    assert_eq!((app.parts["password"].as_str(), app.fields.get("site")), ("app-pw", None), "an app sign-in has no address");
    assert!(text_of(&i, "Phone app (notes)").contains("app: androidapp://com.example.app"));

    let card = find(&i, "Visa");
    assert_eq!((card.kind.as_str(), card.parts["number"].as_str(), card.parts["cvc"].as_str(), card.parts["name"].as_str(), card.parts["expiry"].as_str()), ("card", "4111111111111111", "123", "Jane Doe", "03/2030"));
    let card_notes = text_of(&i, "Visa (notes)");
    assert!(card_notes.contains("limit 500") && card_notes.contains("PIN: 9999") && !card_notes.contains("visa"), "{card_notes}");

    assert_eq!(text_of(&i, "Memo"), "remember the milk");
    let me = find(&i, "Me");
    assert_eq!(me.kind, "note", "an identity is kept as a note even when a field is a password");
    assert!(me.parts["text"].contains("Name: Jane Doe") && me.parts["text"].contains("Password hint: not a login"));
    assert!(!i.entries.iter().any(|e| e.label == "Old") && i.skipped.iter().any(|(l, why)| l == "Old" && why.contains("trash")), "{:?}", i.skipped);
}

#[test]
fn enpass_csv_rows_come_in_by_their_fields() {
    let i = read_ok(&enpass_csv(), "enpass.csv");
    assert_eq!(i.source, Source::Enpass);
    let login = find(&i, "Example");
    assert_eq!((login.parts["password"].as_str(), login.fields["username"].as_str(), login.fields["site"].as_str()), (" pw, 1 ", "bob", "https://example.test"));
    let notes = text_of(&i, "Example (notes)");
    assert!(notes.contains("keep safe") && notes.contains("Security answer: blue"), "{notes}");
    let card = find(&i, "Visa");
    assert_eq!((card.kind.as_str(), card.parts["expiry"].as_str(), card.parts["name"].as_str()), ("card", "03/2030", "Jane Doe"));
    assert_eq!(text_of(&i, "Memo"), "Text: remember the milk");
}

#[test]
fn sticky_password_logins_are_joined_to_their_accounts_and_groups() {
    let i = read_ok(STICKY, "sticky.xml");
    assert_eq!(i.source, Source::StickyPassword);
    assert!(i.plaintext);
    let login = find(&i, "Example");
    assert_eq!((login.parts["password"].as_str(), login.fields["username"].as_str(), login.fields["site"].as_str()), (" pw & 1 ", "bob", "https://example.test/login"));
    assert_eq!(login.tags, ["Work/Servers"]);
    assert_eq!(text_of(&i, "Example (notes)"), "first\nsecond");
    assert_eq!(find(&i, "amy.test").parts["password"], "s3cret", "a login whose account has no title is named for its address");
    assert!(!i.entries.iter().any(|e| e.fields.get("username").is_some_and(|u| u == "nobody")), "a login with no id is left out");
    assert!(i.skipped.iter().any(|(l, why)| l == "empty" && why.contains("no password")), "{:?}", i.skipped);
}

#[test]
fn an_xml_file_that_is_not_a_sticky_password_export_is_refused() {
    match read(b"<?xml version=\"1.0\"?><root><Database><Other/></Database></root>", "x.xml", None) {
        Err(ReadError::Other(e)) => assert!(e.contains("Sticky Password"), "{e}"),
        _ => panic!("expected a refusal"),
    }
    assert!(matches!(read(b"<root><Database><Logins><Login ID=\"1\"", "x.xml", None), Err(ReadError::Other(_))));
}

#[test]
fn bitwarden_json_is_not_taken_for_enpass_or_proton() {
    let i = read_ok(super::tests::BITWARDEN, "bitwarden.json");
    assert_eq!(i.source, Source::Bitwarden);
}

#[test]
fn deep_folders_and_loops_end() {
    // A folder that is its own parent, and a chain of folders, must not run forever.
    let mut v: Value = serde_json::from_str(&proton_json()).unwrap();
    v["vaults"]["s1"]["folders"] = json!([{ "folderId": "f1", "parentFolderId": "f2", "name": "A" }, { "folderId": "f2", "parentFolderId": "f1", "name": "B" }]);
    let i = read_ok(&v.to_string(), "data.json");
    assert!(i.entries.iter().any(|e| e.label == "Example"));
}
