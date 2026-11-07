// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Vault` `import`: what is checked before the person is asked, how the secret is read and picked, what is kept, and
//! that no value is in any answer, error or history row
//! (docs/specs/SPEC_VAULT_PARITY_ENTRY_HISTORY_REASONS_AND_IMPORT_2026_11_06.md §7, §11). Secret ids are invented.

use std::sync::Arc;

use zeroize::Zeroizing;

use super::*;
use crate::backend::storage::vault::VaultActivityRow;
use crate::backend::vault::history::Asker;
use crate::backend::vault::vaults::VaultAgents;
use crate::backend::vault::{create_in, reveal};
use crate::identity::aws_broker::mint::tests::Fake;
use crate::identity::aws_broker::mint::{RunOutput, RunError};

const VALUE: &str = "imported-sentinel-8841-pq";
const REASON: &str = "copy the staging database password for task 28071";

fn store() -> Arc<Store> {
    seal::use_test_key();
    let s = Store::open_in_memory().unwrap();
    s.apply_identity_schema_for_tests().unwrap();
    Arc::new(s)
}

fn source(id: &str, key: Option<&str>) -> Source {
    Source { aws_secret_id: id.into(), key: key.map(String::from), region: Some("eu-west-1".into()), profile: None }
}

fn request(src: Source) -> ImportRequest {
    ImportRequest { source: src, kind: "api-key".into(), label: "Staging database".into(), fields: Default::default(), vault: None, id: None, reason: Some(REASON.into()), refs: vec!["task 28071".into()] }
}

fn plan(s: &Store, req: &ImportRequest) -> Plan {
    check(s, req, "builder").unwrap()
}

fn json_secret() -> String {
    // What the CLI prints for `--query SecretString --output json`: a JSON string holding a JSON object.
    serde_json::to_string(&format!(r#"{{"db":{{"password":"{VALUE}","port":5432,"tags":["a"]}},"user":"app"}}"#)).unwrap()
}

fn plain_secret() -> String {
    serde_json::to_string(VALUE).unwrap()
}

fn rows(s: &Store) -> Vec<VaultActivityRow> {
    s.vault_activity_all().unwrap()
}

/// No value, in any of the places an import writes.
fn nothing_leaked(s: &Store, texts: &[&str]) {
    for row in rows(s) {
        assert!(!format!("{row:?}").contains(VALUE), "a history row holds the value: {row:?}");
    }
    for t in texts {
        assert!(!t.contains(VALUE), "the value is in {t}");
    }
}

#[test]
fn the_source_is_held_to_a_narrow_character_set() {
    let ok = check_source(&Source { aws_secret_id: " staging/db-creds ".into(), key: Some(" db.password ".into()), region: Some("us-gov-west-1".into()), profile: Some("work-read".into()) }).unwrap();
    assert_eq!((ok.aws_secret_id.as_str(), ok.key.as_deref(), ok.region.as_deref()), ("staging/db-creds", Some("db.password"), Some("us-gov-west-1")));
    let full = "staging/db-AbCdEf:with:colons";
    assert!(check_source(&source(full, None)).is_ok(), "a secret's full name (an ARN has colons) is a secret id");

    for bad in ["", "-leading", "has space", "semi;colon", "new\nline", "back`tick", "dollar$x", &"x".repeat(513)] {
        assert!(check_source(&source(bad, None)).is_err(), "{bad:?} must be refused");
    }
    for bad in ["", ".a", "a.", "a..b", "a b", "a;b", "-a", "a/b"] {
        let key = bad.to_string();
        let got = check_source(&Source { key: Some(key), ..source("ok", None) });
        // An empty key means "the whole secret"; the rest are refused.
        assert_eq!(got.is_err(), !bad.is_empty(), "key {bad:?}");
    }
    for bad in ["us east", "useast1", "us-east", "US-EAST-1", "us-east-1; rm", "x-1"] {
        assert!(check_source(&Source { region: Some(bad.into()), ..source("ok", None) }).is_err(), "region {bad:?}");
    }
    for bad in ["-p", "p q", "p;q", "p\nq"] {
        assert!(check_source(&Source { profile: Some(bad.into()), ..source("ok", None) }).is_err(), "profile {bad:?}");
    }
}

#[test]
fn a_missing_or_short_reason_is_refused_before_anything_else() {
    let s = store();
    for reason in [None, Some("".to_string()), Some("because".to_string())] {
        let req = ImportRequest { reason, ..request(source("staging/db", None)) };
        let err = check(&s, &req, "builder").unwrap_err();
        assert!(err.contains("reason"), "{err}");
    }
}

#[test]
fn the_kind_the_vault_and_the_entry_are_checked_before_the_card() {
    let s = store();
    let err = check(&s, &ImportRequest { kind: "passport-photo".into(), ..request(source("staging/db", None)) }, "builder").unwrap_err();
    assert!(err.contains("api-key"), "the answer lists the kinds: {err}");
    let err = check(&s, &ImportRequest { label: "  ".into(), ..request(source("staging/db", None)) }, "builder").unwrap_err();
    assert!(err.contains("label"), "{err}");
    let err = check(&s, &ImportRequest { vault: Some("nowhere".into()), ..request(source("staging/db", None)) }, "builder").unwrap_err();
    assert!(!err.is_empty());
    let err = check(&s, &ImportRequest { id: Some("api-key-nothing".into()), ..request(source("staging/db", None)) }, "builder").unwrap_err();
    assert_eq!(err, "no entry api-key-nothing");
}

#[test]
fn an_agent_can_import_only_into_a_vault_it_may_use_and_replace_only_an_entry_there() {
    let s = store();
    let closed = vaults::create(&s, "Closed", VaultAgents::None).unwrap();
    let open = vaults::create(&s, "Staging", VaultAgents::Named(vec!["builder".into()])).unwrap();
    let theirs = create_in(&s, NewEntry { kind: "api-key".into(), label: "Theirs".into(), fields: Default::default(), tags: vec![], parts: seal::Parts([("value".to_string(), "x".to_string())].into()) }, &closed.id).unwrap();

    let into_closed = ImportRequest { vault: Some(closed.id.clone()), ..request(source("staging/db", None)) };
    assert!(check(&s, &into_closed, "builder").is_err(), "a Vault closed to the agent is not a place to import into");
    let p = check(&s, &ImportRequest { vault: Some("Staging".into()), ..request(source("staging/db", None)) }, "builder").unwrap();
    assert_eq!((p.vault_name.as_str(), p.label()), ("Staging", "Staging database"));
    assert!(matches!(&p.target, Target::New { vault_id, .. } if *vault_id == open.id));

    let replace = ImportRequest { id: Some(theirs.clone()), kind: String::new(), ..request(source("staging/db", None)) };
    assert_eq!(check(&s, &replace, "builder").unwrap_err(), format!("no entry {theirs}"), "an entry in a closed Vault does not exist for the agent");
}

#[test]
fn replacing_names_the_entry_and_refuses_another_kind() {
    let s = store();
    let id = create_in(&s, NewEntry { kind: "api-key".into(), label: "Stripe".into(), fields: Default::default(), tags: vec![], parts: seal::Parts([("value".to_string(), "old".to_string())].into()) }, "personal").unwrap();
    let p = check(&s, &ImportRequest { id: Some(id.clone()), kind: String::new(), label: String::new(), ..request(source("staging/db", None)) }, "builder").unwrap();
    assert_eq!((p.entry_id(), p.label(), p.kind()), (id.as_str(), "Stripe", "api-key"));
    let err = check(&s, &ImportRequest { id: Some(id), kind: "login".into(), ..request(source("staging/db", None)) }, "builder").unwrap_err();
    assert!(err.contains("api-key"), "{err}");
}

#[tokio::test]
async fn the_cli_is_asked_for_the_one_secret_and_a_native_agent_uses_the_hosts_profile() {
    let fake = Fake::ok(plain_secret());
    let src = Source { profile: Some("work-read".into()), ..source("staging/db", None) };
    let got = read_secret(&fake, &src, Via::Host).await.unwrap();
    assert_eq!(got.as_str(), VALUE);
    let seen = fake.seen.lock().unwrap();
    let args = &seen[0].1;
    assert!(args.iter().any(|a| a == "staging/db"), "the one secret is named as an argument of its own: {args:?}");
    assert_eq!(args.iter().filter(|a| a.as_str() == "staging/db").count(), 1);
    assert!(args.windows(2).any(|w| w == ["--region", "eu-west-1"]) && args.windows(2).any(|w| w == ["--profile", "work-read"]));
    assert!(!seen[0].2.iter().any(|(k, _)| k == "AWS_SECRET_ACCESS_KEY"), "the host's own setup is used, no keys are added");
}

#[tokio::test]
async fn a_container_agent_reads_with_the_session_it_was_minted_and_never_a_profile() {
    let fake = Fake::ok(plain_secret());
    let minted = crate::identity::aws_broker::mint::parse_process_json(crate::identity::aws_broker::mint::tests::creds_json(60).as_bytes(), crate::identity::aws_broker::mint::tests::now()).unwrap();
    let src = Source { profile: Some("read".into()), ..source("staging/db", None) };
    read_secret(&fake, &src, Via::Session(&minted)).await.unwrap();
    let seen = fake.seen.lock().unwrap();
    assert!(!seen[0].1.iter().any(|a| a == "--profile"), "the grant is the session, not a profile on this computer");
    let env: Vec<&str> = seen[0].2.iter().map(|(k, _)| k.as_str()).collect();
    assert!(env.contains(&"AWS_ACCESS_KEY_ID") && env.contains(&"AWS_SECRET_ACCESS_KEY") && env.contains(&"AWS_SESSION_TOKEN"));
}

#[test]
fn a_key_picks_a_string_or_a_number_and_nothing_else() {
    let whole = Zeroizing::new(format!(r#"{{"db":{{"password":"{VALUE}","port":5432,"tags":["a"]}}}}"#));
    assert_eq!(pick(whole.clone(), Some("db.password")).unwrap().as_str(), VALUE);
    assert_eq!(pick(whole.clone(), Some("db.port")).unwrap().as_str(), "5432");
    assert!(pick(whole.clone(), Some("db.tags")).unwrap_err().contains("not a string or a number"));
    assert!(pick(whole.clone(), Some("db")).unwrap_err().contains("not a string or a number"));
    assert!(pick(whole.clone(), Some("db.nope")).unwrap_err().contains("no key db.nope"));
    assert!(pick(Zeroizing::new("plain text".into()), Some("a")).unwrap_err().contains("isn't JSON"));
    assert_eq!(pick(Zeroizing::new("plain text".into()), None).unwrap().as_str(), "plain text");
    assert!(pick(Zeroizing::new("  ".into()), None).unwrap_err().contains("empty"));
    for err in [pick(whole.clone(), Some("db.tags")).unwrap_err(), pick(whole, Some("db.nope")).unwrap_err()] {
        assert!(!err.contains(VALUE), "{err}");
    }
}

#[tokio::test]
async fn an_import_makes_an_entry_and_answers_with_ids_never_the_value() {
    let s = store();
    let plan = plan(&s, &request(source("staging/db-creds", Some("db.password"))));
    let fake = Fake::ok(json_secret());
    let done = perform(&s, &fake, &plan, Via::Host, "builder").await.unwrap();
    assert_eq!((done.label.as_str(), done.vault.as_str()), ("Staging database", "Personal"));
    let answer = serde_json::to_string(&done).unwrap();

    assert_eq!(reveal(&s, &done.id, "value", "revealed").unwrap().as_str(), VALUE, "the secret's field is the entry's main value");
    let history = history::history_for(&s, &done.id, Asker::Person, 10).unwrap();
    let imported = history.iter().find(|r| r.action == IMPORTED).expect("an imported line");
    assert_eq!((imported.actor.as_str(), imported.target.as_str(), imported.outcome.as_str()), ("builder", "aws-sm:staging/db-creds#db.password", "done"));
    assert_eq!((imported.reason.as_str(), imported.version, imported.refs.as_slice()), (REASON, 1, ["task 28071".to_string()].as_slice()));
    nothing_leaked(&s, &[&answer, &format!("{imported:?}")]);
}

#[tokio::test]
async fn an_import_with_an_id_replaces_the_value_and_keeps_the_old_one_as_a_version() {
    let s = store();
    let id = create_in(&s, NewEntry { kind: "api-key".into(), label: "Stripe".into(), fields: Default::default(), tags: vec![], parts: seal::Parts([("value".to_string(), "the-old-value-77".to_string())].into()) }, "personal").unwrap();
    let req = ImportRequest { id: Some(id.clone()), kind: String::new(), label: String::new(), ..request(source("staging/db", None)) };
    let plan = plan(&s, &req);
    let done = perform(&s, &Fake::ok(plain_secret()), &plan, Via::Host, "builder").await.unwrap();
    assert_eq!((done.id.as_str(), done.label.as_str()), (id.as_str(), "Stripe"));

    assert_eq!(reveal(&s, &id, "value", "revealed").unwrap().as_str(), VALUE);
    let kept = versions::list(&s, &id).unwrap();
    assert_eq!(kept.len(), 1, "the value it replaced is kept");
    assert_eq!((kept[0].replaced_by.as_str(), kept[0].how.as_str(), kept[0].state.as_str()), ("builder", IMPORTED, "previous"));
    let history = history::history_for(&s, &id, Asker::Person, 10).unwrap();
    let line = history.iter().find(|r| r.action == IMPORTED).unwrap();
    assert_eq!(line.version, 2, "an import is the next version of the value");
    nothing_leaked(&s, &[&serde_json::to_string(&done).unwrap(), &format!("{kept:?}")]);
}

#[tokio::test]
async fn a_refusal_from_aws_says_why_stores_nothing_and_holds_no_value() {
    let s = store();
    let plan = plan(&s, &request(source("staging/db", None)));
    let account = "7".repeat(12);
    let denied = format!("An error occurred (AccessDeniedException) when calling the GetSecretValue operation: User: {account}/x/y is not authorized to perform that action");
    let err = perform(&s, &Fake::failing(&denied), &plan, Via::Host, "builder").await.unwrap_err();
    assert_eq!(err, "AWS refused: this identity can't read that secret");
    assert!(!err.contains(&account));
    let err = perform(&s, &Fake::failing("An error occurred (ResourceNotFoundException) when calling the GetSecretValue operation: Secrets Manager can't find the specified secret."), &plan, Via::Host, "builder").await.unwrap_err();
    assert!(err.contains("no secret by that id"), "{err}");
    let err = perform(&s, &Fake::failing("Error when retrieving token from sso: Token has expired and refresh failed"), &plan, Via::Host, "builder").await.unwrap_err();
    assert!(err.contains("aws sso login"), "{err}");
    assert!(s.vault_count().unwrap() == 0, "nothing was stored");
}

#[tokio::test]
async fn an_unknown_failure_shows_one_cleaned_line_and_never_the_cli_output() {
    let s = store();
    let plan = plan(&s, &request(source("staging/db", None)));
    let account = "7".repeat(12);
    let stderr = format!("\nsomething odd for account {account} happened\nsecond line");
    let fake = Fake::answering(Ok(RunOutput { success: false, stdout: VALUE.as_bytes().to_vec(), stderr }));
    let err = perform(&s, &fake, &plan, Via::Host, "builder").await.unwrap_err();
    assert_eq!(err, "AWS said: something odd for account … happened");
    assert!(!err.contains(VALUE));
    // A success that is not a secret's text is not echoed either.
    let junk = Fake::ok(format!("{VALUE} is not json"));
    let err = perform(&s, &junk, &plan, Via::Host, "builder").await.unwrap_err();
    assert!(!err.contains(VALUE), "{err}");
    nothing_leaked(&s, &[&err]);
}

#[tokio::test]
async fn a_binary_secret_a_missing_cli_and_a_timeout_each_say_so() {
    let s = store();
    let plan = plan(&s, &request(source("staging/db", None)));
    let err = perform(&s, &Fake::ok("null".into()), &plan, Via::Host, "builder").await.unwrap_err();
    assert!(err.contains("binary"), "{err}");
    let no_cli = Fake { which: None, ..Fake::ok(plain_secret()) };
    assert!(perform(&s, &no_cli, &plan, Via::Host, "builder").await.unwrap_err().contains("no AWS CLI"));
    let slow = Fake::answering(Err(RunError::TimedOut));
    assert!(perform(&s, &slow, &plan, Via::Host, "builder").await.unwrap_err().contains("too long"));
    assert_eq!(s.vault_count().unwrap(), 0);
}

#[test]
fn a_denied_import_is_a_line_with_the_reason_and_no_value() {
    let s = store();
    let plan = plan(&s, &request(source("staging/db", None)));
    note(&s, &plan, "", plan.label(), "builder", "denied");
    let all = rows(&s);
    let line = all.last().unwrap();
    assert_eq!((line.action.as_str(), line.outcome.as_str(), line.reason.as_str(), line.target.as_str()), (IMPORTED, "denied", REASON, "aws-sm:staging/db"));
    assert_eq!(line.version, 0, "a denied import set no value");
}

fn grants(text: &str) -> Result<Option<Arc<Policy>>, String> {
    Policy::parse(text).map(|p| Some(Arc::new(p)))
}

const POLICY: &str = r#"{ "version": 1, "grants": {
    "read": { "tier": "read", "mint": { "kind": "host_profile", "profile": "work" }, "agents": ["builder"] },
    "off": { "tier": "read", "mint": { "kind": "host_profile", "profile": "work" }, "agents": ["builder"], "disabled": true },
    "other": { "tier": "read", "mint": { "kind": "host_profile", "profile": "work" }, "agents": ["someone-else"] }
} }"#;

#[test]
fn a_container_agent_needs_a_grant_that_is_on_and_open_to_it() {
    let p = grants(POLICY);
    assert!(check_grant(&p, Some("read"), "builder").is_ok());
    assert!(check_grant(&p, None, "builder").unwrap_err().contains("source.profile"));
    assert!(check_grant(&p, Some("nope"), "builder").unwrap_err().contains("No AWS grant named 'nope'"));
    assert!(check_grant(&p, Some("off"), "builder").unwrap_err().contains("switched off"));
    assert!(check_grant(&p, Some("other"), "builder").unwrap_err().contains("isn't open to you"));
    assert!(check_grant(&Ok(None), Some("read"), "builder").unwrap_err().contains("isn't shared"));
    assert!(check_grant(&Err("bad".into()), Some("read"), "builder").unwrap_err().contains("doesn't parse"));
}
