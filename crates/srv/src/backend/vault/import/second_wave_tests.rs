// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The second-wave readers (spec §12 P6) against files in each service's export format. The files
//! are written from the format as Bitwarden's open-source importer reads it (made-up values), not
//! exports the services made: a mismatch with a real export is likely to be a column name, not a
//! crash. Each test says what it checks.

use super::*;

pub(super) const DASHLANE_CREDENTIALS: &str = "username,username2,username3,title,password,note,url,category,otpSecret\n\
bob,bobby,,Example,hunter2,recovery 1234,https://example.test/login,Shopping,JBSWY3DPEHPK3PXP\n\
,,,NoPass,,,https://nopass.test,,\n";

pub(super) const DASHLANE_PAYMENTS: &str = "type,account_name,account_holder,cc_number,code,expiration_month,expiration_year,routing_number,account_number,country,issuing_bank\n\
credit_card,Visa,Alex Doe,4111111111111111,123,3,2030,,,US,First Bank\n\
bank,Checking,Alex Doe,,,,,021000021,1234567890,US,First Bank\n";

pub(super) const DASHLANE_IDS: &str = "type,number,name,issue_date,expiration_date,place_of_issue,state\n\
passport,X9876543,Alex Doe,2020-1-2,2030-1-2,US Dept of State,\n\
license,D1234567,Alex Doe,2019-5-5,2028-6-30,,CA\n\
social_security,123-45-6789,Alex Doe,,,,\n";

pub(super) const DASHLANE_PERSONAL: &str = "type,title,first_name,middle_name,last_name,login,date_of_birth,place_of_birth,email,email_type,item_name,phone_number,address,country,state,city,zip,address_recipient,address_building,address_apartment,address_floor,address_door_code,job_title,url\n\
name,Mr,Alex,,Doe,,1990-04-05,,,,,,,,,,,,,,,,,\n\
email,,,,,,,,alex@example.test,personal,Main,,,,,,,,,,,,,\n";

pub(super) const DASHLANE_NOTES: &str = "title,note\nPlan,\"Do not forget\nthe second line\"\n";

pub(super) const NORDPASS: &str = "name,url,additional_urls,username,password,note,cardholdername,cardnumber,cvc,expirydate,zipcode,folder,full_name,phone_number,email,address1,address2,city,country,state,type,custom_fields\n\
Example,https://example.test,\"[\"\"https://b.test\"\"]\",bob,pw,a note,,,,,,Work,,,,,,,,,password,\"[{\"\"label\"\":\"\"PIN\"\",\"\"type\"\":\"\"hidden\"\",\"\"value\"\":\"\"1234\"\"}]\"\n\
Visa,,,,,,Jane Doe,4111111111111111,123,01 / 22,,Money,,,,,,,,,credit_card,\n\
Memo,,,,,remember the milk,,,,,,,,,,,,,,,note,\n\
Me,,,,,,,,,,90210,,Jane Doe,555-1234,jane@example.test,1 Main St,,Springfield,US,IL,identity,\n\
Odd,,,,,,,,,,,,,,,,,,,,mystery,\n";

pub(super) const ROBOFORM: &str = "Name,Url,MatchUrl,Login,Pwd,Note,Folder,RfFieldsV2\n\
Acme,https://acme.example,https://acme.example,bob@x.test,s3cret,,Work\\Dev,\"User ID$,,,txt,bob@x.test\",\"Password$,,,pwd,s3cret\",\"TOTP KEY$,,,txt,JBSWY3DP\",\"Pet,,,txt,Rex, the dog\"\n\
note - 2023-03-31,,,,,This is a safe note,,\n\
--,,,,,,,\n";

pub(super) const ZOHO: &str = "\"Password Name\",\"Description\",\"Password URL\",\"SecretData\",\"Notes\",\"CustomData\",\"Tags\",\"Classification\",\"Favorite\",\"TOTP\",\"Folder Name\"\n\
Acme,,https://acme.example,\"SecretType:Web Account\nUser Name:bob\nPassword:pw\nRecovery code:9999\n\",remember,\"\",,E,1,otpauth://totp?secret=JBSWY3DP&digits=6,Work\n\
Only notes,,,,just a note,,,,,,\n";

pub(super) const KEEPER: &str = "\"Foo\\Baz\",\"Example\",\"bob@example.test\",\"pw\",\"https://example.test/\",\"a note\",\"\",\"Account ID\",\"12345\",\"TFC:Keeper\",\"otpauth://totp/x?secret=ABC\"\n\
\"\",\"Note only\",\"\",\"\",\"\",\"just text\",\"\",\n";

fn read_ok(text: &str, name: &str) -> Imported {
    match read(text.as_bytes(), name, None) {
        Ok(i) => i,
        Err(ReadError::Other(e)) => panic!("{e}"),
        Err(ReadError::NeedsMapping(m)) => panic!("needs a mapping {:?}", m.columns),
        Err(ReadError::NeedsPassphrase) => panic!("needs a password"),
    }
}

fn find<'a>(i: &'a Imported, label: &str) -> &'a Incoming {
    i.entries.iter().find(|e| e.label == label).unwrap_or_else(|| panic!("no entry {label:?} in {:?}", i.entries.iter().map(|e| &e.label).collect::<Vec<_>>()))
}

fn text_of<'a>(i: &'a Imported, label: &str) -> &'a str {
    &find(i, label).parts["text"]
}

#[test]
fn dashlane_credentials_come_in_as_logins() {
    let i = read_ok(DASHLANE_CREDENTIALS, "credentials.csv");
    assert_eq!(i.source, Source::Dashlane);
    let login = find(&i, "Example");
    assert_eq!((login.parts["password"].as_str(), login.parts["totp"].as_str()), ("hunter2", "JBSWY3DPEHPK3PXP"));
    assert_eq!((login.fields["username"].as_str(), login.fields["site"].as_str()), ("bob", "https://example.test/login"));
    assert_eq!(login.tags, ["Shopping"]);
    let notes = text_of(&i, "Example (notes)");
    assert!(notes.contains("recovery 1234") && notes.contains("username2: bobby"), "{notes}");
    assert!(i.skipped.iter().any(|(l, why)| l == "NoPass" && why.contains("no password")), "{:?}", i.skipped);
    assert!(i.plaintext);
}

#[test]
fn dashlane_payments_ids_and_notes_come_in_as_their_kinds() {
    let i = read_ok(DASHLANE_PAYMENTS, "payments.csv");
    let card = find(&i, "Visa");
    assert_eq!(card.kind, "card");
    assert_eq!((card.parts["number"].as_str(), card.parts["cvc"].as_str(), card.parts["name"].as_str(), card.parts["expiry"].as_str()), ("4111111111111111", "123", "Alex Doe", "03/2030"));
    let bank = find(&i, "Checking");
    assert_eq!(bank.kind, "bank-account");
    assert_eq!(text_of(&i, "Checking (notes)"), "country: US", "what a bank account has no field for is sealed, as a card's is");
    assert_eq!((bank.parts["account"].as_str(), bank.parts["routing"].as_str(), bank.fields["bank"].as_str()), ("1234567890", "021000021", "First Bank"));

    let i = read_ok(DASHLANE_IDS, "ids.csv");
    let passport = find(&i, "Alex Doe (passport)");
    assert_eq!((passport.kind.as_str(), passport.parts["value"].as_str(), passport.fields["issuer"].as_str()), ("passport", "X9876543", "US Dept of State"));
    assert_eq!(find(&i, "Alex Doe (driver's licence)").fields["issuer"], "CA");
    assert_eq!(find(&i, "Alex Doe (Social Security number)").kind, "ssn");

    let i = read_ok(DASHLANE_NOTES, "securenotes.csv");
    assert_eq!(text_of(&i, "Plan"), "Do not forget\nthe second line");
}

#[test]
fn dashlane_personal_details_are_kept_sealed_and_a_birth_date_is_its_kind() {
    let i = read_ok(DASHLANE_PERSONAL, "personalInfo.csv");
    let dob = find(&i, "Alex Doe (date of birth)");
    assert_eq!((dob.kind.as_str(), dob.parts["value"].as_str()), ("date-of-birth", "1990-04-05"));
    let name = text_of(&i, "Alex Doe (name)");
    assert!(name.contains("first_name: Alex") && name.contains("last_name: Doe"), "{name}");
    assert!(text_of(&i, "Main (email)").contains("email: alex@example.test"));
}

#[test]
fn nordpass_rows_come_in_by_their_type() {
    let i = read_ok(NORDPASS, "nordpass.csv");
    assert_eq!(i.source, Source::NordPass);
    let login = find(&i, "Example");
    assert_eq!((login.parts["password"].as_str(), login.fields["username"].as_str(), login.fields["site"].as_str()), ("pw", "bob", "https://example.test"));
    assert_eq!(login.tags, ["Work"]);
    let notes = text_of(&i, "Example (notes)");
    assert!(notes.contains("a note") && notes.contains("url: https://b.test") && notes.contains("PIN: 1234"), "{notes}");
    assert!(!login.fields.contains_key("notes"), "no secret sits in a plain field");

    let card = find(&i, "Visa");
    assert_eq!((card.kind.as_str(), card.parts["expiry"].as_str(), card.parts["name"].as_str(), card.parts["cvc"].as_str()), ("card", "01/2022", "Jane Doe", "123"));
    assert_eq!(card.tags, ["Money"]);
    assert_eq!(text_of(&i, "Memo"), "remember the milk");
    let me = text_of(&i, "Me");
    assert!(me.contains("full name: Jane Doe") && me.contains("address: 1 Main St") && me.contains("zip code: 90210"), "{me}");
    assert!(i.skipped.iter().any(|(l, why)| l == "Odd" && why.contains("mystery")), "{:?}", i.skipped);
}

#[test]
fn roboform_logins_keep_their_one_time_code_and_seal_their_extra_fields() {
    let i = read_ok(ROBOFORM, "roboform.csv");
    assert_eq!(i.source, Source::RoboForm);
    let login = find(&i, "Acme");
    assert_eq!((login.parts["password"].as_str(), login.parts["totp"].as_str()), ("s3cret", "JBSWY3DP"));
    assert_eq!((login.fields["username"].as_str(), login.fields["site"].as_str()), ("bob@x.test", "https://acme.example"));
    assert_eq!(login.tags, ["Work/Dev"], "a folder path becomes a tag");
    let notes = text_of(&i, "Acme (notes)");
    assert_eq!(notes, "Pet: Rex, the dog", "a value with a comma is kept whole, and the repeats of the login are dropped");
    assert_eq!(text_of(&i, "note - 2023-03-31"), "This is a safe note");
    assert!(i.skipped.is_empty(), "the trailing `--` row is not an item: {:?}", i.skipped);
}

#[test]
fn zoho_vault_unpacks_the_secret_data_column() {
    let i = read_ok(ZOHO, "zoho.csv");
    assert_eq!(i.source, Source::ZohoVault);
    let login = find(&i, "Acme");
    assert_eq!((login.parts["password"].as_str(), login.fields["username"].as_str(), login.fields["site"].as_str()), ("pw", "bob", "https://acme.example"));
    assert_eq!(login.parts["totp"], "otpauth://totp?secret=JBSWY3DP&digits=6");
    assert_eq!(login.tags, ["Work"]);
    let notes = text_of(&i, "Acme (notes)");
    assert!(notes.contains("remember") && notes.contains("Recovery code: 9999") && !notes.contains("SecretType"), "{notes}");
    assert_eq!(text_of(&i, "Only notes"), "just a note");
}

#[test]
fn keeper_csv_has_no_header_and_is_found_by_its_shape() {
    let i = read_ok(KEEPER, "keeper.csv");
    assert_eq!(i.source, Source::Keeper);
    let login = find(&i, "Example");
    assert_eq!((login.parts["password"].as_str(), login.parts["totp"].as_str()), ("pw", "otpauth://totp/x?secret=ABC"));
    assert_eq!((login.fields["username"].as_str(), login.fields["site"].as_str()), ("bob@example.test", "https://example.test/"));
    assert_eq!(login.tags, ["Foo/Baz"]);
    let notes = text_of(&i, "Example (notes)");
    assert!(notes.contains("a note") && notes.contains("Account ID: 12345") && !notes.contains("TFC"), "{notes}");
    assert_eq!(text_of(&i, "Note only"), "just text");
}

#[test]
fn files_the_new_readers_do_not_claim_are_read_as_before() {
    // A browser export has name, url, username and password too, and is still a browser's.
    let chrome = read_ok("name,url,username,password,note\nexample.test,https://example.test,bob,pw,\n", "Chrome Passwords.csv");
    assert_eq!(chrome.source, Source::Chrome);
    // A table of unknown columns still asks which is which.
    assert!(matches!(read(b"alpha,beta\n1,2\n", "x.csv", None), Err(ReadError::NeedsMapping(_))));
    // Three columns with no header is not Keeper's seven.
    assert!(matches!(read(b"a,b,c\nd,e,f\n", "x.csv", None), Err(ReadError::NeedsMapping(_))));
    // Neither is a table whose header row names seven unknown columns: its header is no Keeper row.
    assert!(matches!(read(b"id,title,x,y,url,notes,z\n1,Home,,,,hello,\n", "x.csv", None), Err(ReadError::NeedsMapping(_))));
    // The person's own mapping always wins over a guess about the service.
    let mapping = Mapping { kind: "api-key".into(), label: 0, value: 3, site: None, username: None };
    let mapped = read(NORDPASS.as_bytes(), "nordpass.csv", Some(&mapping)).ok().unwrap();
    assert_eq!(mapped.source, Source::Csv);
}
