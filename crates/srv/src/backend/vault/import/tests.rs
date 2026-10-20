// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Each reader against a file in its service's export format.

use super::*;

fn read_ok(text: &str, name: &str) -> Imported {
    match read(text.as_bytes(), name, None) {
        Ok(i) => i,
        Err(ReadError::Other(e)) => panic!("{e}"),
        Err(ReadError::NeedsMapping(_)) => panic!("needs a mapping"),
    }
}

fn err_of(bytes: &[u8], name: &str) -> String {
    match read(bytes, name, None) {
        Err(ReadError::Other(e)) => e,
        Err(ReadError::NeedsMapping(m)) => format!("needs mapping {:?}", m.columns),
        Ok(_) => panic!("expected an error"),
    }
}

fn find<'a>(i: &'a Imported, label: &str) -> &'a Incoming {
    i.entries.iter().find(|e| e.label == label).unwrap_or_else(|| panic!("no entry {label:?} in {:?}", i.entries.iter().map(|e| &e.label).collect::<Vec<_>>()))
}

// ── LastPass ────────────────────────────────────────────────────────────────

const LASTPASS: &str = "url,username,password,totp,extra,name,grouping,fav\n\
https://example.com/login,alex,hunter2,JBSWY3DPEHPK3PXP,\"recovery codes: 1234 5678\",Example,Shopping,0\n\
http://sn,,,,\"NoteType:Credit Card\nLanguage:en-US\nName on Card:Alex Doe\nType:Visa\nNumber:4111111111111111\nSecurity Code:123\nStart Date:January,2020\nExpiration Date:March,2030\nNotes:Company card\nsecond line\",Company Visa,Finance,0\n\
http://sn,,,,\"NoteType:Bank Account\nLanguage:en-US\nBank Name:First Bank\nAccount Type:Checking\nRouting Number:021000021\nAccount Number:1234567890\nNotes:\",Checking,Finance,0\n\
http://sn,,,,\"NoteType:Social Security\nLanguage:en-US\nName:Alex Doe\nNumber:123-45-6789\",My SSN,,0\n\
http://sn,,,,\"NoteType:Driver's License\nLanguage:en-US\nNumber:D1234567\nExpiration Date:June,2028\nName:Alex Doe\",Licence,,0\n\
http://sn,,,,\"NoteType:Passport\nLanguage:en-US\nName:Alex Doe\nNumber:X9876543\nIssuing Authority:US Dept of State\nExpiration Date:May,2031\",Passport,,0\n\
http://sn,,,,\"NoteType:Address\nLanguage:en-US\nTitle:Mr\nFirst Name:Alex\nAddress 1:1 Main St\",Home,,0\n\
http://sn,,,,\"NoteType:SSH Key\nLanguage:en-US\nBit Strength:2048\nPrivate Key:-----BEGIN-----\nabc\n-----END-----\",Server key,,0\n\
http://sn,,,,,Empty note,,0\n\
https://nopass.example,bob,,,,No password,,0\n";

#[test]
fn lastpass_logins_and_every_kind_of_secure_note_come_in_as_their_kind() {
    let i = read_ok(LASTPASS, "lastpass_export.csv");
    assert_eq!(i.source, Source::LastPass);

    let login = find(&i, "Example");
    assert_eq!(login.kind, "login");
    assert_eq!((login.parts["password"].as_str(), login.parts["totp"].as_str()), ("hunter2", "JBSWY3DPEHPK3PXP"));
    assert_eq!((login.fields["site"].as_str(), login.fields["username"].as_str()), ("https://example.com/login", "alex"));
    assert_eq!(login.tags, ["Shopping"], "the group becomes a tag");
    assert_eq!(find(&i, "Example (notes)").parts["text"], "recovery codes: 1234 5678", "notes are sealed, in a note of their own");
    assert!(!login.fields.contains_key("notes"), "no secret sits in a plain field");

    let card = find(&i, "Company Visa");
    assert_eq!(card.kind, "card");
    assert_eq!(card.parts["number"], "4111111111111111");
    assert_eq!((card.parts["cvc"].as_str(), card.parts["name"].as_str(), card.parts["expiry"].as_str()), ("123", "Alex Doe", "03/2030"));
    assert_eq!(find(&i, "Company Visa (notes)").parts["text"], "Company card\nsecond line", "a note runs over several lines");

    let bank = find(&i, "Checking");
    assert_eq!((bank.kind.as_str(), bank.parts["account"].as_str(), bank.parts["routing"].as_str(), bank.fields["bank"].as_str()), ("bank-account", "1234567890", "021000021", "First Bank"));
    let ssn = find(&i, "My SSN");
    assert_eq!((ssn.kind.as_str(), ssn.parts["value"].as_str(), ssn.fields["owner"].as_str()), ("ssn", "123-45-6789", "Alex Doe"));
    let dl = find(&i, "Licence");
    assert_eq!((dl.kind.as_str(), dl.parts["value"].as_str(), dl.fields["expires"].as_str()), ("drivers-licence", "D1234567", "06/2028"));
    let passport = find(&i, "Passport");
    assert_eq!((passport.kind.as_str(), passport.fields["issuer"].as_str()), ("passport", "US Dept of State"));

    // Every other type is kept whole, as a secret note.
    let address = find(&i, "Home");
    assert_eq!(address.kind, "note");
    assert!(address.parts["text"].contains("1 Main St") || address.parts["text"].contains("First Name"), "{}", address.parts["text"]);
    let ssh = find(&i, "Server key");
    assert!(ssh.parts["text"].contains("-----BEGIN-----") && ssh.parts["text"].contains("abc"), "a multi-line value stays whole: {}", ssh.parts["text"]);

    let why: std::collections::BTreeMap<_, _> = i.skipped.iter().map(|(l, w)| (l.as_str(), w.as_str())).collect();
    assert!(why["Empty note"].contains("empty"));
    assert!(why["No password"].contains("no password"));
}

// ── Browsers ────────────────────────────────────────────────────────────────

#[test]
fn chrome_edge_and_brave_exports_are_read() {
    let i = read_ok("name,url,username,password,note\nGitHub,https://github.com/login,alex,pw1,\nExample,https://www.example.com/,bob,pw2,my note\n,https://untitled.example/x,carol,pw3,\n", "Chrome Passwords.csv");
    assert_eq!(i.source, Source::Chrome);
    assert_eq!(find(&i, "GitHub").parts["password"], "pw1");
    assert_eq!(find(&i, "Example (notes)").parts["text"], "my note");
    assert_eq!(find(&i, "untitled.example").fields["username"], "carol", "a row with no name is labelled by its site");
}

#[test]
fn firefox_exports_are_read_and_their_other_columns_ignored() {
    let i = read_ok(
        "url,username,password,httpRealm,formActionOrigin,guid,timeCreated,timeLastUsed,timePasswordChanged\nhttps://example.org,alex,pw1,,https://example.org,{abc},1,2,3\n",
        "logins.csv",
    );
    assert_eq!(i.source, Source::Firefox);
    let e = find(&i, "example.org");
    assert_eq!((e.parts["password"].as_str(), e.fields["site"].as_str()), ("pw1", "https://example.org"));
}

#[test]
fn safari_and_apple_passwords_keep_their_one_time_codes() {
    let i = read_ok("Title,URL,Username,Password,Notes,OTPAuth\nApple,https://appleid.apple.com,alex,pw1,,otpauth://totp/Apple?secret=ABCD\n", "Passwords.csv");
    assert_eq!(i.source, Source::Safari);
    assert_eq!(find(&i, "Apple").parts["totp"], "otpauth://totp/Apple?secret=ABCD");
}

#[test]
fn one_password_csv_is_read_by_its_column_names() {
    let i = read_ok("Title,Website,Username,Password,Notes,Type,Tags\nBank,https://bank.example,alex,pw1,pin is 4321,Login,finance\n", "1password.csv");
    assert_eq!(i.source, Source::OnePassword);
    assert_eq!(find(&i, "Bank").tags, ["finance"]);
    assert_eq!(find(&i, "Bank (notes)").parts["text"], "pin is 4321");
}

// ── Bitwarden ───────────────────────────────────────────────────────────────

const BITWARDEN: &str = r#"{
  "encrypted": false,
  "folders": [{"id": "f1", "name": "Work"}],
  "items": [
    {"type": 1, "name": "GitHub", "folderId": "f1", "notes": "backup codes: 1111",
     "login": {"username": "alex", "password": "pw1", "totp": "JBSWY3DPEHPK3PXP", "uris": [{"uri": "https://github.com"}]},
     "fields": [{"name": "pin", "value": "9876", "type": 1}]},
    {"type": 3, "name": "Amex", "card": {"cardholderName": "Alex Doe", "number": "378282246310005", "expMonth": "4", "expYear": "2029", "code": "1234"}},
    {"type": 2, "name": "Wifi", "notes": "ssid: home\npass: s3cret"},
    {"type": 4, "name": "Me", "identity": {"firstName": "Alex", "lastName": "Doe", "ssn": "123-45-6789", "passportNumber": "X123", "licenseNumber": ""}},
    {"type": 1, "name": "No password", "login": {"username": "x"}},
    {"type": 5, "name": "A key"},
    {"type": 2, "name": "Blank note", "notes": ""}
  ]
}"#;

#[test]
fn a_bitwarden_json_export_comes_in_as_logins_cards_notes_and_identity_numbers() {
    let i = read_ok(BITWARDEN, "bitwarden_export_20261020.json");
    assert_eq!(i.source, Source::Bitwarden);
    let login = find(&i, "GitHub");
    assert_eq!((login.parts["password"].as_str(), login.parts["totp"].as_str(), login.fields["site"].as_str()), ("pw1", "JBSWY3DPEHPK3PXP", "https://github.com"));
    assert_eq!(login.tags, ["Work"], "a folder becomes a tag");
    let extra = &find(&i, "GitHub (notes)").parts["text"];
    assert!(extra.contains("backup codes: 1111") && extra.contains("pin: 9876"), "notes and custom fields are kept sealed: {extra}");

    let card = find(&i, "Amex");
    assert_eq!((card.parts["number"].as_str(), card.parts["expiry"].as_str(), card.parts["cvc"].as_str(), card.parts["name"].as_str()), ("378282246310005", "04/2029", "1234", "Alex Doe"));
    assert_eq!(find(&i, "Wifi").parts["text"], "ssid: home\npass: s3cret");
    assert_eq!(find(&i, "Me (Social Security number)").parts["value"], "123-45-6789");
    assert_eq!(find(&i, "Me (passport)").fields["owner"], "Alex Doe");
    assert!(i.entries.iter().all(|e| e.label != "Me (driver's licence)"), "an empty number isn't an entry");

    let why: std::collections::BTreeMap<_, _> = i.skipped.iter().map(|(l, w)| (l.as_str(), w.as_str())).collect();
    assert!(why["No password"].contains("no password") && why["A key"].contains("can't be imported") && why["Blank note"].contains("empty"));
}

#[test]
fn a_password_protected_bitwarden_export_says_how_to_get_one_that_isnt() {
    let e = err_of(br#"{"encrypted": true, "passwordProtected": true, "data": "2.abc"}"#, "x.json");
    assert!(e.contains("password-protected") && e.contains("unencrypted"), "{e}");
    assert!(err_of(b"{\"hello\": 1}", "x.json").contains("no items"));
    assert!(err_of(b"{not json", "x.json").contains("isn't a Bitwarden export"));
}

// ── .env ────────────────────────────────────────────────────────────────────

#[test]
fn an_env_file_is_a_list_of_api_keys_named_for_their_variables() {
    let i = read_ok(
        "# keys\nSTRIPE_API_KEY=sk_test_123\nexport GITHUB_TOKEN=\"ghp_abc def\"\nSINGLE='raw $value'\nWITH_COMMENT=abc # a comment\nEMPTY=\nQUOTED=\"line1\\nline2\"\nSTRIPE_API_KEY=sk_live_456\nnot a variable line\n",
        ".env",
    );
    assert_eq!(i.source, Source::Env);
    let stripe = find(&i, "STRIPE_API_KEY");
    assert_eq!((stripe.kind.as_str(), stripe.parts["value"].as_str(), stripe.fields["env_name"].as_str()), ("api-key", "sk_live_456", "STRIPE_API_KEY"));
    assert_eq!(i.entries.iter().filter(|e| e.label == "STRIPE_API_KEY").count(), 1, "the later assignment wins");
    assert_eq!(find(&i, "GITHUB_TOKEN").parts["value"], "ghp_abc def");
    assert_eq!(find(&i, "SINGLE").parts["value"], "raw $value", "single quotes are literal");
    assert_eq!(find(&i, "WITH_COMMENT").parts["value"], "abc");
    assert_eq!(find(&i, "QUOTED").parts["value"], "line1\nline2");
    assert_eq!(i.skipped, [("EMPTY".to_string(), "it has no value".to_string())]);
}

#[test]
fn a_file_is_an_env_file_only_if_every_line_is_a_variable() {
    assert!(env::looks_like_env("A=1\n# c\n\nexport B=2\n"));
    assert!(!env::looks_like_env("name,url,username,password\na,b,c,d\n"));
    assert!(!env::looks_like_env("A=1\nsomething else\n"));
    assert!(!env::looks_like_env("# only a comment\n"));
    assert!(!env::looks_like_env("1A=2\n"), "a name doesn't start with a digit");
}

// ── Any other CSV, and what isn't a CSV ─────────────────────────────────────

#[test]
fn a_csv_no_reader_knows_asks_the_person_which_column_is_which() {
    let text = "Service,Secret,Owner\nStripe,sk_test_1,alex\nTwilio,tw_2,bob\n";
    match read(text.as_bytes(), "keys.csv", None) {
        Err(ReadError::NeedsMapping(m)) => assert_eq!((m.columns, m.rows), (vec!["Service".to_string(), "Secret".into(), "Owner".into()], 2)),
        _ => panic!("expected needs mapping"),
    }
    let m = Mapping { kind: "api-key".into(), label: 0, value: 1, site: None, username: None };
    let i = match read(text.as_bytes(), "keys.csv", Some(&m)) {
        Ok(i) => i,
        Err(_) => panic!("mapped"),
    };
    assert_eq!(i.source, Source::Csv);
    assert_eq!((find(&i, "Stripe").kind.as_str(), find(&i, "Stripe").parts["value"].as_str(), find(&i, "Twilio").parts["value"].as_str()), ("api-key", "sk_test_1", "tw_2"));

    let login = Mapping { kind: "login".into(), label: 0, value: 1, site: None, username: Some(2) };
    let i = match read(text.as_bytes(), "keys.csv", Some(&login)) {
        Ok(i) => i,
        Err(_) => panic!("mapped"),
    };
    assert_eq!((find(&i, "Stripe").parts["password"].as_str(), find(&i, "Stripe").fields["username"].as_str()), ("sk_test_1", "alex"));
}

#[test]
fn a_bad_mapping_is_refused_with_why() {
    let text = "a,b\n1,2\n";
    let bad = |m: Mapping| match read(text.as_bytes(), "x.csv", Some(&m)) {
        Err(ReadError::Other(e)) => e,
        _ => panic!("expected an error"),
    };
    assert!(bad(Mapping { kind: "rocket".into(), label: 0, value: 1, site: None, username: None }).contains("unknown kind"));
    assert!(bad(Mapping { kind: "api-key".into(), label: 0, value: 9, site: None, username: None }).contains("isn't in the file"));
    assert!(bad(Mapping { kind: "login".into(), label: 0, value: 1, site: None, username: None }).contains("site or a username"));
}

#[test]
fn files_that_arent_a_known_text_format_are_refused_with_what_is_missing() {
    assert!(err_of(&[0x03, 0xD9, 0xA2, 0x9A, 0x67, 0xFB, 0x4B, 0xB5, 0, 0], "x.kdbx").contains("KeePass"));
    assert!(err_of(b"PK\x03\x04rest", "x.1pux").contains("zip"));
    assert!(err_of(&[0xff, 0xfe, 0x00, 0x80], "x.bin").contains("isn't a text file"));
    assert!(err_of(b"only one column\nrow\n", "x.csv").contains("one column"));
    assert!(err_of(b"", "empty.csv").contains("CSV") || err_of(b"", "empty.csv").contains("column"));
}

#[test]
fn a_byte_order_mark_is_ignored() {
    let text = "\u{feff}name,url,username,password\nA,https://a.example,u,p\n";
    assert_eq!(read_ok(text, "x.csv").entries.len(), 1);
}

#[test]
fn more_rows_than_the_cap_are_refused() {
    let mut text = String::from("name,url,username,password\n");
    for i in 0..(MAX_ITEMS + 1) {
        text.push_str(&format!("a{i},https://a.example,u,p\n"));
    }
    assert!(err_of(text.as_bytes(), "big.csv").contains("more than"));
}

#[test]
fn a_bitwarden_item_with_null_lists_still_comes_in() {
    let i = read_ok(r#"{"folders": null, "items": [{"type": 1, "name": "Site", "fields": null, "login": {"username": "me", "password": "pw", "uris": null}}]}"#, "x.json");
    assert_eq!(i.entries.len(), 1);
    assert_eq!(i.entries[0].parts["password"], "pw");
}
