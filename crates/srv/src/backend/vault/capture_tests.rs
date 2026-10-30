// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use agentmux_common::api_types::CaptureInto;

use super::*;
use crate::backend::storage::vault_versions::CaptureWrite;
use crate::backend::vault::{activity, create, reveal, versions};

fn store() -> Store {
    seal::use_test_key();
    let s = Store::open_in_memory().unwrap();
    s.apply_identity_schema_for_tests().unwrap();
    s
}

fn new_key(label: &str, from: CaptureFrom) -> CaptureRule {
    CaptureRule { into: CaptureInto { kind: Some("api-key".into()), label: Some(label.into()), ..Default::default() }, from }
}

fn into_id(id: &str, from: CaptureFrom) -> CaptureRule {
    CaptureRule { into: CaptureInto { id: Some(id.into()), ..Default::default() }, from }
}

fn value(s: &Store, id: &str, part: &str) -> String {
    reveal(s, id, part, "revealed").unwrap().to_string()
}

fn one(from: CaptureFrom, stdout: &str) -> Result<String, String> {
    extract(&rule_from(&from)?, stdout).map(|f| f.value.to_string())
}

const AWS: &str = r#"{
    "AccessKey": {
        "UserName": "deploy",
        "AccessKeyId": "AKIAEXAMPLEKEY0001",
        "Status": "Active",
        "SecretAccessKey": "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
    },
    "items": [{"token": "tok_first_000001"}, {"token": "tok_second_00002"}]
}"#;

#[test]
fn each_rule_kind_finds_its_value() {
    assert_eq!(one(CaptureFrom::Stdout("all".into()), "\n  ghp_abcdefghijklmnop  \n").unwrap(), "ghp_abcdefghijklmnop");
    assert_eq!(one(CaptureFrom::Json("$.AccessKey.SecretAccessKey".into()), AWS).unwrap(), "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY");
    assert_eq!(one(CaptureFrom::Json("$.items[1].token".into()), AWS).unwrap(), "tok_second_00002");
    assert_eq!(one(CaptureFrom::Json("$['AccessKey'][\"AccessKeyId\"]".into()), AWS).unwrap(), "AKIAEXAMPLEKEY0001");
    assert_eq!(one(CaptureFrom::Regex(r"token:\s*(\S+)".into()), "created\ntoken: abcd1234efgh\ndone").unwrap(), "abcd1234efgh");
    assert_eq!(one(CaptureFrom::Line(2), "Your new key:\n  key_0123456789  \n").unwrap(), "key_0123456789");
}

#[test]
fn no_match_says_why_and_never_what() {
    let why = |from, out| one(from, out).unwrap_err();
    assert_eq!(why(CaptureFrom::Json("$.AccessKey.Missing".into()), AWS), "the JSON path matched nothing");
    assert_eq!(why(CaptureFrom::Json("$.AccessKey".into()), AWS), "the JSON path matched an object, a list or true/false, not a value");
    assert_eq!(why(CaptureFrom::Json("$.a".into()), "Error: not json at all"), "the output isn't JSON");
    assert_eq!(why(CaptureFrom::Regex(r"key=(\w+)".into()), "nothing here"), "the regex matched nothing");
    assert_eq!(why(CaptureFrom::Line(5), "one\ntwo"), "the output has fewer than 5 lines");
    assert_eq!(why(CaptureFrom::Stdout("all".into()), "   \n"), "the value it matched is empty");
    let short = why(CaptureFrom::Stdout("all".into()), "abc1234");
    assert!(short.contains("shorter than 8"), "{short}");
    assert!(!short.contains("abc1234"));
}

#[test]
fn a_rule_that_cant_work_is_refused_before_the_command_runs() {
    let s = store();
    let refused = |rules: &[CaptureRule]| plan(&s, rules, "Agent2-6", "true").unwrap_err();
    assert!(refused(&[new_key("K", CaptureFrom::Regex(r"\w+".into()))]).contains("exactly one capture group"));
    assert!(refused(&[new_key("K", CaptureFrom::Regex("(".into()))]).contains("doesn't compile"));
    assert!(refused(&[new_key("K", CaptureFrom::Json("AccessKey.Secret".into()))]).contains("doesn't start with $"));
    assert!(refused(&[new_key("K", CaptureFrom::Json("$.a[x]".into()))]).contains("isn't a number"));
    assert!(refused(&[new_key("K", CaptureFrom::Stdout("first".into()))]).contains("takes \"all\""));
    assert!(refused(&[new_key("K", CaptureFrom::Line(0))]).contains("from 1"));
    assert!(refused(&[new_key("", CaptureFrom::Line(1))]).contains("needs a label"));
    assert!(refused(&[into_id("api-key-nothing", CaptureFrom::Line(1))]).contains("no entry"));
    let card = CaptureRule { into: CaptureInto { kind: Some("card".into()), label: Some("C".into()), part: Some("cvc".into()), ..Default::default() }, from: CaptureFrom::Line(1) };
    assert!(refused(&[card]).contains("made from its number"));
    let both = CaptureRule { into: CaptureInto { id: Some("x".into()), kind: Some("api-key".into()), ..Default::default() }, from: CaptureFrom::Line(1) };
    assert!(refused(&[both]).contains("not both"));
    let many: Vec<CaptureRule> = (0..=MOST).map(|n| new_key(&format!("K{n}"), CaptureFrom::Line(1))).collect();
    assert!(refused(&many).contains("at most"));
    assert!(refused(&[new_key("K", CaptureFrom::Line(1)), new_key("L", CaptureFrom::Line(0))]).starts_with("capture rule 2:"));
}

#[test]
fn matches_become_new_entries_and_the_output_is_cleaned_of_them() {
    let s = store();
    let rules = [
        new_key("AWS deploy secret", CaptureFrom::Json("$.AccessKey.SecretAccessKey".into())),
        new_key("AWS deploy key id", CaptureFrom::Json("$.AccessKey.AccessKeyId".into())),
    ];
    let cap = plan(&s, &rules, "Agent2-6", "aws iam create-access-key --user-name deploy").unwrap();
    assert_eq!(cap.replaces().count(), 0, "a new entry asks nobody");
    let Outcome::Stored { ids, scrubber } = cap.finish(&s, AWS, AWS, false, 0) else { panic!("stored") };
    assert_eq!(ids, ["api-key-aws-deploy-secret", "api-key-aws-deploy-key-id"]);
    assert_eq!(value(&s, &ids[0], "value"), "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY");
    let clean = scrubber.scrub(AWS);
    assert!(clean.contains("\"SecretAccessKey\": \"[vault:api-key-aws-deploy-secret]\""), "{clean}");
    assert!(!clean.contains("wJalr") && !clean.contains("AKIAEXAMPLE"));
    let log = activity(&s, 10).unwrap();
    let made = log.iter().find(|a| a.entry_id == ids[0] && a.action == "captured").unwrap();
    assert_eq!((made.actor.as_str(), made.action.as_str(), made.target.as_str()), ("Agent2-6", "captured", "aws iam create-access-key --user-name deploy"));
}

#[test]
fn a_match_into_an_existing_entry_is_its_new_value_and_the_old_one_is_kept() {
    let s = store();
    let id = create(
        &s,
        NewEntry {
            kind: "api-key".into(),
            label: "GitHub".into(),
            fields: Default::default(),
            tags: vec![],
            parts: Parts([("value".to_string(), "ghp_old_value_000".to_string())].into()),
        },
    )
    .unwrap();
    let cap = plan(&s, &[into_id(&id, CaptureFrom::Stdout("all".into()))], "Agent2-6", "gh auth token").unwrap();
    assert_eq!(cap.replaces().count(), 1, "the person is asked first");
    let Outcome::Stored { ids, .. } = cap.finish(&s, "ghp_new_value_111\n", "ghp_new_value_111", false, 0) else { panic!("stored") };
    assert_eq!(ids, [id.clone()]);
    assert_eq!(value(&s, &id, "value"), "ghp_new_value_111");
    let kept = versions::list(&s, &id).unwrap();
    assert_eq!(kept.len(), 1);
    assert_eq!((kept[0].replaced_by.as_str(), kept[0].how.as_str()), ("Agent2-6", "captured"));
}

#[test]
fn an_entry_deleted_and_made_again_while_the_command_ran_is_not_written() {
    let s = store();
    let entry = |v: &str| NewEntry { kind: "api-key".into(), label: "GitHub".into(), fields: Default::default(), tags: vec![], parts: Parts([("value".to_string(), v.to_string())].into()) };
    let id = create(&s, entry("ghp_old_value_000")).unwrap();
    let cap = plan(&s, &[into_id(&id, CaptureFrom::Stdout("all".into()))], "Agent2-6", "gh auth token").unwrap();
    crate::backend::vault::delete(&s, &id).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));
    assert_eq!(create(&s, entry("ghp_persons_new_0")).unwrap(), id);
    let Outcome::Withheld { why, .. } = cap.finish(&s, "ghp_new_value_111", "ghp_new_value_111", false, 0) else { panic!("withheld") };
    assert!(why.contains("deleted while the command ran"), "{why}");
    assert_eq!(value(&s, &id, "value"), "ghp_persons_new_0");
}

#[test]
fn no_match_withholds_the_output_and_keeps_it_for_the_person() {
    let s = store();
    let cap = plan(&s, &[new_key("Token", CaptureFrom::Regex(r"token=(\S+)".into()))], "Agent2-6", "make-token --verbose").unwrap();
    let output = "starting\nsecret_value_printed_differently\ndone";
    let Outcome::Withheld { why, kept_as } = cap.finish(&s, output, output, false, 0) else { panic!("withheld") };
    assert_eq!(why, "the regex matched nothing");
    let note = kept_as.expect("kept as a note");
    assert_eq!(value(&s, &note, "text"), output);
    let row = s.vault_get(&note).unwrap().unwrap();
    assert_eq!(row.label, "Output withheld from Agent2-6: make-token --verbose");
    assert!(s.vault_get("api-key-token").unwrap().is_none(), "nothing captured");
}

#[test]
fn a_failed_command_or_too_much_output_is_withheld() {
    let s = store();
    let cap = plan(&s, &[new_key("Token", CaptureFrom::Stdout("all".into()))], "Agent2-6", "make-token").unwrap();
    let Outcome::Withheld { why, .. } = cap.finish(&s, "Error: denied for user", "Error: denied for user", false, 1) else { panic!("withheld") };
    assert!(why.contains("exit code 1"), "{why}");
    let Outcome::Withheld { why, .. } = cap.finish(&s, "x", "x", true, 0) else { panic!("withheld") };
    assert!(why.contains("over 1 MB"), "{why}");
}

#[test]
fn a_json_value_with_escapes_is_cleaned_out_as_the_output_printed_it() {
    let s = store();
    // A password with a quote, a backslash and a slash, and a key over two lines.
    let out = r#"{"password": "pa\"ss\\wo/rd-1234", "key": "-----BEGIN KEY-----\nMIIEabcdefgh\n-----END KEY-----"}"#;
    let rules = [
        new_key("Password", CaptureFrom::Json("$.password".into())),
        CaptureRule { into: CaptureInto { kind: Some("note".into()), label: Some("Key".into()), ..Default::default() }, from: CaptureFrom::Json("$.key".into()) },
    ];
    let cap = plan(&s, &rules, "Agent2-6", "make-creds --json").unwrap();
    let Outcome::Stored { ids, scrubber } = cap.finish(&s, out, out, false, 0) else { panic!("stored") };
    assert_eq!(value(&s, &ids[0], "value"), "pa\"ss\\wo/rd-1234", "the value is stored decoded");
    assert_eq!(value(&s, &ids[1], "text"), "-----BEGIN KEY-----\nMIIEabcdefgh\n-----END KEY-----");
    let clean = scrubber.scrub(out);
    assert_eq!(clean, format!(r#"{{"password": "[vault:{}]", "key": "[vault:{}]"}}"#, ids[0], ids[1]));
}

#[test]
fn the_scrubber_knows_a_values_json_forms_whoever_prints_it() {
    let s = crate::backend::vault::scrub::Scrubber::new(&[("login-x".to_string(), "pä\"ss/wörd\\1".to_string())]);
    for printed in [r#"pä\"ss/wörd\\1"#, r#"pä\"ss\/wörd\\1"#, r#"p\u00e4\"ss/w\u00f6rd\\1"#] {
        assert_eq!(s.scrub(&format!("{{\"p\": \"{printed}\"}}")), "{\"p\": \"[vault:login-x]\"}", "{printed}");
    }
}

#[test]
fn two_rules_writing_the_same_entry_are_refused() {
    let s = store();
    let twice = [new_key("Token", CaptureFrom::Line(1)), new_key("token", CaptureFrom::Line(2))];
    assert!(plan(&s, &twice, "Agent2-6", "x").unwrap_err().contains("same entry"));
    let id = create(&s, NewEntry { kind: "api-key".into(), label: "GitHub".into(), fields: Default::default(), tags: vec![], parts: Parts([("value".to_string(), "ghp_old_value_000".to_string())].into()) }).unwrap();
    assert!(plan(&s, &[into_id(&id, CaptureFrom::Line(1)), into_id(&id, CaptureFrom::Line(2))], "Agent2-6", "x").unwrap_err().contains("same entry"));
}

#[test]
fn a_capture_stores_all_its_values_or_none() {
    let s = store();
    let entry = |v: &str| NewEntry { kind: "api-key".into(), label: "GitHub".into(), fields: Default::default(), tags: vec![], parts: Parts([("value".to_string(), v.to_string())].into()) };
    let id = create(&s, entry("ghp_old_value_000")).unwrap();
    let rules = [new_key("Deploy key", CaptureFrom::Line(1)), into_id(&id, CaptureFrom::Line(2))];
    let cap = plan(&s, &rules, "Agent2-6", "make-both").unwrap();
    // The second rule's entry is deleted while the command runs: the first rule's new entry isn't made either.
    crate::backend::vault::delete(&s, &id).unwrap();
    let out = "deploy_value_0001\nghp_new_value_111";
    let Outcome::Withheld { why, .. } = cap.finish(&s, out, out, false, 0) else { panic!("withheld") };
    assert!(why.contains("deleted while the command ran"), "{why}");
    assert!(s.vault_get("api-key-deploy-key").unwrap().is_none(), "nothing was stored");
}

#[test]
fn the_commit_writes_nothing_when_an_entry_changed_since_it_was_read() {
    let s = store();
    let id = create(&s, NewEntry { kind: "api-key".into(), label: "GitHub".into(), fields: Default::default(), tags: vec![], parts: Parts([("value".to_string(), "ghp_old_value_000".to_string())].into()) }).unwrap();
    let read = s.vault_get(&id).unwrap().unwrap();
    // Someone writes a new value after the capture read the entry.
    crate::backend::vault::update(
        &s,
        crate::backend::vault::EntryChange { id: id.clone(), label: "GitHub".into(), fields: Default::default(), tags: vec![], parts: Parts([("value".to_string(), "ghp_persons_value".to_string())].into()), clear: vec![] },
    )
    .unwrap();
    let mut fresh = read.clone();
    fresh.id = "api-key-other".into();
    let writes = [
        CaptureWrite::Insert(fresh),
        CaptureWrite::Replace { row: VaultRow { updated_at: read.updated_at + 1, ..read.clone() }, was: read.sealed.clone(), kept: Some(versions::of(&read, "Agent2-6", CAPTURED)) },
    ];
    assert!(!s.vault_capture_commit(&writes).unwrap());
    assert!(s.vault_get("api-key-other").unwrap().is_none(), "the insert was rolled back");
    assert_eq!(value(&s, &id, "value"), "ghp_persons_value");
    assert_eq!(versions::list(&s, &id).unwrap().len(), 1, "only the person's own edit kept a version");
}

#[test]
fn two_labels_that_make_the_same_id_get_ids_of_their_own() {
    let s = store();
    let rules = [new_key("AWS key", CaptureFrom::Line(1)), new_key("AWS-key", CaptureFrom::Line(2))];
    let cap = plan(&s, &rules, "Agent2-6", "make-two").unwrap();
    let out = "first_value_00001\nsecond_value_0002";
    let Outcome::Stored { ids, .. } = cap.finish(&s, out, out, false, 0) else { panic!("stored") };
    assert_eq!(ids, ["api-key-aws-key", "api-key-aws-key-2"]);
    assert_eq!(value(&s, &ids[1], "value"), "second_value_0002");
}
