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
    let Outcome::Stored { ids, scrubber, .. } = cap.finish(&s, AWS, AWS, false, 0) else { panic!("stored") };
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
    let Outcome::Stored { ids, scrubber, .. } = cap.finish(&s, out, out, false, 0) else { panic!("stored") };
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

fn held(lines: &[(&'static str, &str)]) -> Vec<HeldLine> {
    lines.iter().enumerate().map(|(n, (k, l))| (*k, l.to_string(), n as u64)).collect()
}

#[test]
fn a_value_over_several_lines_is_cleaned_out_whatever_its_lines_length() {
    let s = store();
    let cap = plan(&s, &[CaptureRule { into: CaptureInto { kind: Some("note".into()), label: Some("Codes".into()), ..Default::default() }, from: CaptureFrom::Stdout("all".into()) }], "Agent2-6", "codes").unwrap();
    let out = held(&[("stdout", "abcd"), ("stdout", "1234")]);
    let Outcome::Stored { ids, scrubber, short } = cap.finish(&s, "abcd\n1234", "abcd\n1234", false, 0) else { panic!("stored") };
    let clean = clean_held(&out, &scrubber, &short).unwrap();
    assert_eq!(clean, vec![("stdout", format!("[vault:{}]", ids[0]), 1)]);

    // Its lines printed apart (a line of stderr between them): a short one can't be found alone.
    let apart = held(&[("stdout", "abcd"), ("stderr", "warning: slow"), ("stdout", "1234")]);
    let why = clean_held(&apart, &scrubber, &short).unwrap_err();
    assert!(why.contains("shorter than 6"), "{why}");
}

#[test]
fn a_captured_value_containing_a_given_one_is_cleaned_out_whole() {
    let given = crate::backend::vault::scrub::Scrubber::new(&[("api-key-old".to_string(), "abcdefgh".to_string())]);
    let captured = crate::backend::vault::scrub::Scrubber::new(&[("api-key-new".to_string(), "abcdefgh12345678".to_string())]);
    let out = held(&[("stdout", "old abcdefgh new abcdefgh12345678")]);
    let clean = clean_held(&out, &given.merged(&captured), &[]).unwrap();
    assert_eq!(clean[0].1, "old [vault:api-key-old] new [vault:api-key-new]");
}

#[test]
fn a_capture_into_any_part_keeps_the_old_entry_for_seven_days() {
    let s = store();
    let card = create(
        &s,
        NewEntry {
            kind: "card".into(),
            label: "Company card".into(),
            fields: Default::default(),
            tags: vec![],
            parts: Parts([("number".to_string(), "4111111111111111".to_string()), ("cvc".to_string(), "123".to_string())].into()),
        },
    )
    .unwrap();
    let rule = CaptureRule { into: CaptureInto { id: Some(card.clone()), part: Some("cvc".into()), ..Default::default() }, from: CaptureFrom::Stdout("all".into()) };
    let cap = plan(&s, &[rule], "Agent2-6", "issue-cvc").unwrap();
    let Outcome::Stored { .. } = cap.finish(&s, "cvc-98765432", "cvc-98765432", false, 0) else { panic!("stored") };
    assert_eq!(value(&s, &card, "cvc"), "cvc-98765432");
    assert_eq!(versions::list(&s, &card).unwrap().len(), 1, "the person was told the old value is kept");
}

// ── A card from several values of one output (SPEC_VAULT_CARD_ENTRIES... C3) ──────────────────────────────────

const VIRTUAL_CARD: &str = r#"{"pan": "4242 4242 4242 4242", "expiry": "03/30", "cvv": "123", "holder": "Alex Doe", "zip": "94107"}"#;

fn json(path: &str) -> CaptureFrom {
    CaptureFrom::Json(path.into())
}

fn card_from(parts: &[(&str, CaptureFrom)]) -> CaptureFrom {
    CaptureFrom::Parts(parts.iter().map(|(k, f)| (k.to_string(), f.clone())).collect())
}

fn virtual_card() -> CaptureRule {
    CaptureRule {
        into: CaptureInto { kind: Some("card".into()), label: Some("Virtual Visa".into()), ..Default::default() },
        from: card_from(&[("number", json("$.pan")), ("exp", json("$.expiry")), ("cvc", json("$.cvv")), ("name", json("$.holder")), ("zip", json("$.zip"))]),
    }
}

#[test]
fn a_card_is_made_whole_from_one_output_settled_and_the_output_is_cleaned_of_every_part() {
    let s = store();
    let cap = plan(&s, &[virtual_card()], "Agent2-6", "virtual-card show").unwrap();
    assert_eq!(cap.replaces().count(), 0, "a new card asks nobody");
    let Outcome::Stored { ids, scrubber, .. } = cap.finish(&s, VIRTUAL_CARD, VIRTUAL_CARD, false, 0) else { panic!("stored") };
    assert_eq!(ids.len(), 1, "one entry from five values");
    let id = &ids[0];
    // Settled as a save by the person is: the number as digits, the expiry as month and year, no older `expiry`.
    assert_eq!(value(&s, id, "number"), "4242424242424242");
    assert_eq!((value(&s, id, "exp_month").as_str(), value(&s, id, "exp_year").as_str()), ("03", "2030"));
    assert_eq!((value(&s, id, "cvc").as_str(), value(&s, id, "name").as_str(), value(&s, id, "zip").as_str()), ("123", "Alex Doe", "94107"));
    assert!(reveal(&s, id, "expiry", "revealed").is_err(), "the older single expiry is read into the month and year and dropped");
    let fields: BTreeMap<String, String> = serde_json::from_str(&s.vault_get(id).unwrap().unwrap().fields).unwrap();
    assert_eq!((fields.get("brand").map(String::as_str), fields.get("expires").map(String::as_str)), (Some("visa"), Some("2030-03-31")));
    // The output shows the entry in place of each part, and none of them is left.
    let clean = scrubber.scrub(VIRTUAL_CARD);
    for secret in ["4242", "03/30", "123", "Alex Doe", "94107"] {
        assert!(!clean.contains(secret), "{secret} is left in {clean}");
    }
    assert_eq!(clean.matches(&format!("[vault:{id}]")).count(), 5, "{clean}");
    let log = activity(&s, 10).unwrap();
    assert!(log.iter().any(|a| a.entry_id == *id && a.action == "captured" && a.actor == "Agent2-6"), "{log:?}");
}

#[test]
fn a_parts_capture_makes_only_a_new_card_and_needs_its_number() {
    let s = store();
    let refused = |rule: CaptureRule| plan(&s, &[rule], "Agent2-6", "true").unwrap_err();
    let into = |kind: &str| CaptureInto { kind: Some(kind.into()), label: Some("C".into()), ..Default::default() };
    let parts = |list: &[(&str, CaptureFrom)]| card_from(list);
    assert!(refused(CaptureRule { into: into("api-key"), from: parts(&[("number", json("$.a"))]) }).contains("new card"));
    assert!(refused(CaptureRule { into: CaptureInto { id: Some("card-x".into()), ..Default::default() }, from: parts(&[("number", json("$.a"))]) }).contains("new card"));
    assert!(refused(CaptureRule { into: into("card"), from: parts(&[("cvc", json("$.a"))]) }).contains("needs the card's \"number\""));
    assert!(refused(CaptureRule { into: into("card"), from: parts(&[("number", json("$.a")), ("pin", json("$.b"))]) }).contains("no \"pin\""));
    assert!(refused(CaptureRule { into: into("card"), from: parts(&[("number", json("$.a")), ("exp", json("$.b")), ("exp_year", json("$.c"))]) }).contains("not both"));
    assert!(refused(CaptureRule { into: into("card"), from: parts(&[("number", parts(&[("number", json("$.a"))]))]) }).contains("number: "));
    assert!(refused(CaptureRule { into: CaptureInto { part: Some("cvc".into()), ..into("card") }, from: parts(&[("number", json("$.a"))]) }).contains("leave part out"));
    assert!(refused(CaptureRule { into: into("card"), from: parts(&[("number", CaptureFrom::Line(0))]) }).contains("number: lines are counted from 1"));
}

#[test]
fn a_part_that_is_not_in_the_output_withholds_it_and_names_the_part_never_the_value() {
    let s = store();
    let cap = plan(&s, &[virtual_card()], "Agent2-6", "virtual-card show").unwrap();
    let no_cvv = VIRTUAL_CARD.replace("\"cvv\": \"123\", ", "");
    let Outcome::Withheld { why, .. } = cap.finish(&s, &no_cvv, &no_cvv, false, 0) else { panic!("withheld") };
    assert!(why.contains("the card's cvc") && why.contains("matched nothing"), "{why}");
    assert!(!why.contains("4242") && !why.contains("94107"), "{why}");
    assert!(s.vault_list().unwrap().iter().all(|r| r.kind != "card"), "nothing was stored");
    // A one-character part is too short to clean; the number still needs eight.
    let one_char = VIRTUAL_CARD.replace("\"123\"", "\"1\"");
    let Outcome::Withheld { why, .. } = cap.finish(&s, &one_char, &one_char, false, 0) else { panic!("withheld") };
    assert!(why.contains("the card's cvc is too short"), "{why}");
    let short_number = VIRTUAL_CARD.replace("4242 4242 4242 4242", "4242");
    let Outcome::Withheld { why, .. } = cap.finish(&s, &short_number, &short_number, false, 0) else { panic!("withheld") };
    assert!(why.contains("shorter than 8"), "{why}");
}

#[test]
fn a_captured_expiry_replaces_the_month_and_year_of_a_card_that_exists_and_is_settled() {
    let s = store();
    let id = create(
        &s,
        NewEntry {
            kind: "card".into(),
            label: "Everyday".into(),
            fields: Default::default(),
            tags: vec![],
            parts: Parts([("number", "4242424242424242"), ("exp_month", "03"), ("exp_year", "2027")].map(|(k, v)| (k.to_string(), v.to_string())).into()),
        },
    )
    .unwrap();
    let rule = CaptureRule { into: CaptureInto { id: Some(id.clone()), part: Some("expiry".into()), ..Default::default() }, from: CaptureFrom::Stdout("all".into()) };
    let cap = plan(&s, &[rule], "Agent2-6", "virtual-card renew").unwrap();
    let Outcome::Stored { ids, .. } = cap.finish(&s, "08/31\n", "08/31", false, 0) else { panic!("a five-character expiry is a card's to take") };
    assert_eq!(ids, [id.clone()]);
    assert_eq!((value(&s, &id, "exp_month").as_str(), value(&s, &id, "exp_year").as_str()), ("08", "2031"), "the next save does not put 03/2027 back");
    let fields: BTreeMap<String, String> = serde_json::from_str(&s.vault_get(&id).unwrap().unwrap().fields).unwrap();
    assert_eq!(fields.get("expires").map(String::as_str), Some("2031-08-31"), "the reminders follow the new date");
    // A captured number is settled too: digits only, the brand worked out again.
    let rule = CaptureRule { into: CaptureInto { id: Some(id.clone()), part: Some("number".into()), ..Default::default() }, from: CaptureFrom::Stdout("all".into()) };
    let cap = plan(&s, &[rule], "Agent2-6", "virtual-card renew").unwrap();
    let Outcome::Stored { .. } = cap.finish(&s, "5555 5555 5555 4444\n", "5555 5555 5555 4444", false, 0) else { panic!("stored") };
    assert_eq!(value(&s, &id, "number"), "5555555555554444");
    let fields: BTreeMap<String, String> = serde_json::from_str(&s.vault_get(&id).unwrap().unwrap().fields).unwrap();
    assert_eq!(fields.get("brand").map(String::as_str), Some("mastercard"));
}

#[test]
fn a_card_part_other_than_the_number_may_be_short_but_any_other_entry_still_needs_eight() {
    let s = store();
    let id = create(
        &s,
        NewEntry { kind: "card".into(), label: "Everyday".into(), fields: Default::default(), tags: vec![], parts: Parts([("number".to_string(), "4242424242424242".to_string())].into()) },
    )
    .unwrap();
    let cvc = CaptureRule { into: CaptureInto { id: Some(id), part: Some("cvc".into()), ..Default::default() }, from: CaptureFrom::Stdout("all".into()) };
    let cap = plan(&s, &[cvc], "Agent2-6", "true").unwrap();
    assert!(matches!(cap.finish(&s, "123\n", "123", false, 0), Outcome::Stored { .. }));
    let key = new_key("K", CaptureFrom::Stdout("all".into()));
    let cap = plan(&s, &[key], "Agent2-6", "true").unwrap();
    assert!(matches!(cap.finish(&s, "123\n", "123", false, 0), Outcome::Withheld { .. }), "an API key of three characters is still refused");
}

#[test]
fn a_card_part_the_scrubber_could_not_clean_is_refused_naming_the_part_and_never_the_value() {
    let s = store();
    let with = |extra: (&str, &str)| {
        let out = format!(r#"{{"pan": "4242 4242 4242 4242", "exp": "03/30", "cvv": "123", "m": "03", "y": "2030", "n": "Al", "z": "SW1", "c2": "12", "{}": "{}"}}"#, extra.0, extra.1);
        out
    };
    let rule = |list: &[(&str, &str)]| CaptureRule {
        into: CaptureInto { kind: Some("card".into()), label: Some("C".into()), ..Default::default() },
        from: card_from(&[&[("number", "$.pan")][..], list].concat().iter().map(|(k, p)| (*k, json(p))).collect::<Vec<_>>()),
    };
    let out = with(("x", "y"));
    for (key, path, why) in [("exp_month", "$.m", "exp_month"), ("cvc", "$.c2", "cvc"), ("name", "$.n", "name"), ("zip", "$.z", "zip")] {
        let cap = plan(&s, &[rule(&[(key, path)])], "Agent2-6", "show").unwrap();
        let Outcome::Withheld { why: said, .. } = cap.finish(&s, &out, &out, false, 0) else { panic!("{key} should be withheld") };
        assert!(said.contains(&format!("the card's {why} is too short")), "{said}");
        for value in ["03", "Al", "SW1", "12"] {
            assert!(!said.contains(&format!("\"{value}\"")), "the value is in {said}");
        }
    }
    // A three-digit code, a four-digit year and a five-character expiry are taken.
    let cap = plan(&s, &[rule(&[("cvc", "$.cvv"), ("exp", "$.exp")])], "Agent2-6", "show").unwrap();
    assert!(matches!(cap.finish(&s, &out, &out, false, 0), Outcome::Stored { .. }));
    // The same for a part captured into a card that exists.
    let id = create(&s, NewEntry { kind: "card".into(), label: "Everyday".into(), fields: Default::default(), tags: vec![], parts: Parts([("number".to_string(), "4111111111111111".to_string())].into()) }).unwrap();
    let month = CaptureRule { into: CaptureInto { id: Some(id), part: Some("exp_month".into()), ..Default::default() }, from: CaptureFrom::Stdout("all".into()) };
    let cap = plan(&s, &[month], "Agent2-6", "show").unwrap();
    let Outcome::Withheld { why, .. } = cap.finish(&s, "03\n", "03", false, 0) else { panic!("withheld") };
    assert!(why.contains("the card's exp_month is too short") && why.contains("exp (like 03/30)"), "{why}");
}

#[test]
fn a_capture_into_an_entry_that_is_not_a_card_leaves_its_plain_fields_exactly_as_they_are() {
    let s = store();
    let id = create(
        &s,
        NewEntry {
            kind: "api-key".into(),
            label: "Stripe".into(),
            fields: [("service".to_string(), "stripe".to_string()), ("notes".to_string(), "keep me".to_string())].into(),
            tags: vec![],
            parts: Parts([("value".to_string(), "sk_old_value_123".to_string())].into()),
        },
    )
    .unwrap();
    let before = s.vault_get(&id).unwrap().unwrap().fields;
    let rule = CaptureRule { into: CaptureInto { id: Some(id.clone()), ..Default::default() }, from: CaptureFrom::Stdout("all".into()) };
    let cap = plan(&s, &[rule], "Agent2-6", "rotate").unwrap();
    assert!(matches!(cap.finish(&s, "sk_new_value_456\n", "sk_new_value_456", false, 0), Outcome::Stored { .. }));
    assert_eq!(s.vault_get(&id).unwrap().unwrap().fields, before, "the plain fields are byte for byte what they were");
    assert_eq!(value(&s, &id, "value"), "sk_new_value_456");
}
