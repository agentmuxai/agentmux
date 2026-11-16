// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Vault` `import_batch`: what is checked before the person is asked, that only the rows they approved are read, what
//! the history says of a batch, and that no value is in an answer or a row
//! (docs/specs/SPEC_VAULT_LINKED_ENTRIES_FLEET_VAULT_AND_BATCH_IMPORT_2026_11_15.md §4). Secret ids are invented.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use super::*;
use crate::backend::vault::vaults::VaultAgents;
use crate::backend::vault::{create_in, reveal, seal, NewEntry};
use crate::identity::aws_broker::mint::{RunError, RunOutput, RunSpec};

const VALUE: &str = "batch-sentinel-5512-zk";
const REASON: &str = "copy the build hosts' passwords for task 31007";
const NOT_FOUND: &str = "An error occurred (ResourceNotFoundException) when calling the GetSecretValue operation: Secrets Manager can't find the specified secret.";

fn store() -> Arc<Store> {
    seal::use_test_key();
    let s = Store::open_in_memory().unwrap();
    s.apply_identity_schema_for_tests().unwrap();
    Arc::new(s)
}

/// A stand-in AWS CLI: each secret it knows answers with its own text, and any other is not found.
struct Cli {
    secrets: Vec<(&'static str, String)>,
    seen: Mutex<Vec<String>>,
}

impl Cli {
    fn holding(ids: &[&'static str]) -> Self {
        Self { secrets: ids.iter().map(|id| (*id, format!("{VALUE}-{id}"))).collect(), seen: Mutex::new(Vec::new()) }
    }
}

impl Runner for Cli {
    fn which(&self, _program: &str) -> Option<PathBuf> {
        Some(PathBuf::from("/bin/aws"))
    }

    async fn run(&self, spec: RunSpec) -> Result<RunOutput, RunError> {
        // The secret's id is the fourth argument of the read `import` runs (`aws_import::argv`).
        let id = spec.args.get(3).cloned().unwrap_or_default();
        self.seen.lock().unwrap().push(id.clone());
        Ok(match self.secrets.iter().find(|(name, _)| *name == id) {
            Some((_, text)) => RunOutput { success: true, stdout: serde_json::to_string(text).unwrap().into_bytes(), stderr: String::new() },
            None => RunOutput { success: false, stdout: Vec::new(), stderr: NOT_FOUND.into() },
        })
    }
}

fn item(secret: &str, label: &str) -> BatchItem {
    BatchItem { aws_secret_id: secret.into(), kind: "login".into(), label: label.into(), ..Default::default() }
}

fn batch(items: Vec<BatchItem>) -> BatchRequest {
    BatchRequest {
        batch: "host passwords".into(),
        vault: None,
        reason: Some(REASON.into()),
        source: BatchSource { region: Some("eu-west-1".into()), profile: None },
        items,
        refs: vec!["task 31007".into()],
    }
}

fn three() -> BatchRequest {
    batch(vec![item("hosts/alpha", "Alpha"), item("hosts/beta", "Beta"), item("hosts/gamma", "Gamma")])
}

fn refused(s: &Store, req: &BatchRequest) -> String {
    check(s, req, "builder").unwrap_err()
}

fn entry(s: &Store, label: &str, vault: &str) -> String {
    create_in(s, NewEntry { kind: "api-key".into(), label: label.into(), fields: Default::default(), tags: vec![], parts: seal::Parts([("value".to_string(), "the-old-value-31".to_string())].into()) }, vault).unwrap()
}

#[test]
fn a_batch_needs_a_name_a_reason_and_one_to_forty_items() {
    let s = store();
    let plan = check(&s, &three(), "builder").unwrap();
    assert_eq!((plan.name.as_str(), plan.vault_name.as_str(), plan.items.len(), plan.region.as_deref()), ("host passwords", "Personal", 3, Some("eu-west-1")));
    assert_eq!(plan.refs, vec![format!("{BATCH_REF}{}", plan.id), "task 31007".to_string()], "the batch's reference is first, so a row's three references keep it");

    assert!(refused(&s, &BatchRequest { reason: None, ..three() }).contains("a reason is required"));
    assert!(refused(&s, &BatchRequest { batch: "  ".into(), ..three() }).contains("needs batch"));
    assert!(refused(&s, &BatchRequest { batch: "x".repeat(61), ..three() }).contains("at most 60 characters"));
    assert!(refused(&s, &batch(vec![])).contains("needs items"));
    let many = (0..41).map(|i| item(&format!("hosts/h{i}"), &format!("Host {i}"))).collect();
    assert!(refused(&s, &batch(many)).contains("at most 40 items"));
    let forty = (0..40).map(|i| item(&format!("hosts/h{i}"), &format!("Host {i}"))).collect();
    assert_eq!(check(&s, &batch(forty), "builder").unwrap().items.len(), 40);
}

#[test]
fn each_item_is_checked_as_an_import_is_and_the_refusal_names_it() {
    let s = store();
    let mut req = three();
    req.items[1].kind = "nonsense".into();
    let said = refused(&s, &req);
    assert!(said.starts_with("item 2 (Beta): ") && said.contains("import makes a new entry of a kind"), "{said}");

    let mut req = three();
    req.items[2].aws_secret_id = "has space".into();
    assert!(refused(&s, &req).starts_with("item 3 (Gamma): aws_secret_id is the secret's name or ARN"));

    let mut req = three();
    req.items[0].link = true;
    let said = refused(&s, &req);
    assert!(said.starts_with("item 1 (Alpha): link: true") && said.contains("leave link out"), "{said}");

    let mut req = three();
    req.items[0].id = Some("api-key-nowhere".into());
    assert_eq!(refused(&s, &req), "item 1 (Alpha): no entry api-key-nowhere");
}

#[test]
fn a_batch_is_one_account_and_region() {
    let s = store();
    let mut req = three();
    req.items[1].region = Some("us-east-1".into());
    assert!(refused(&s, &req).contains("one batch, one account and region"));
    let mut req = three();
    req.items[1].profile = Some("other-read".into());
    assert!(refused(&s, &req).contains("one batch, one account and region"));
    let mut req = three();
    req.items[1].region = Some("eu-west-1".into());
    assert!(check(&s, &req, "builder").is_ok(), "an item may repeat the batch's own region");

    // Secrets named by ARN say where they are: two accounts, two regions, or a region that isn't the batch's.
    let arn = |region: &str, account: &str, name: &str| format!("arn:aws:secretsmanager:{region}:{account}:secret:{name}");
    let here = batch(vec![item(&arn("eu-west-1", "account-one", "hosts/alpha"), "Alpha"), item(&arn("eu-west-1", "account-one", "hosts/beta"), "Beta"), item("hosts/gamma", "Gamma")]);
    assert!(check(&s, &here, "builder").is_ok());
    let two_accounts = batch(vec![item(&arn("eu-west-1", "account-one", "hosts/alpha"), "Alpha"), item(&arn("eu-west-1", "account-two", "hosts/beta"), "Beta")]);
    assert!(refused(&s, &two_accounts).contains("one batch, one account and region"));
    let elsewhere = batch(vec![item(&arn("us-east-1", "account-one", "hosts/alpha"), "Alpha")]);
    assert!(refused(&s, &elsewhere).contains("one batch, one account and region"));
    assert!(refused(&s, &BatchRequest { source: BatchSource { region: Some("nowhere".into()), profile: None }, ..three() }).contains("region is an AWS region"));
}

#[test]
fn a_batch_is_one_vault_and_only_one_the_agent_may_use() {
    let s = store();
    let fleet = vaults::create(&s, "Fleet", VaultAgents::Named(vec!["builder".into()])).unwrap();
    vaults::create(&s, "Closed", VaultAgents::None).unwrap();
    let personal = entry(&s, "Stripe", "personal");
    let in_fleet = entry(&s, "Deploy key", &fleet.id);

    let plan = check(&s, &BatchRequest { vault: Some("Fleet".into()), ..three() }, "builder").unwrap();
    assert_eq!((plan.vault_id.as_str(), plan.vault_name.as_str()), (fleet.id.as_str(), "Fleet"));
    assert!(plan.items.iter().all(|p| matches!(&p.target, Target::New { vault_id, .. } if *vault_id == fleet.id)), "every new entry goes into the batch's Vault");

    assert!(refused(&s, &BatchRequest { vault: Some("Closed".into()), ..three() }).contains("no Vault"), "a Vault the agent may not use is as one that isn't there");
    assert!(refused(&s, &BatchRequest { vault: Some("Nowhere".into()), ..three() }).contains("no Vault"));

    // An item can't name another Vault, and can't replace an entry that is in another one.
    let mut req = BatchRequest { vault: Some("Fleet".into()), ..three() };
    req.items[2].vault = Some("Personal".into());
    assert_eq!(refused(&s, &req), "item 3 (Gamma): one batch, one Vault: this batch goes into Fleet, and this item is for another");
    let mut req = BatchRequest { vault: Some("Fleet".into()), ..three() };
    req.items[0] = BatchItem { aws_secret_id: "pay/stripe".into(), id: Some(personal.clone()), ..Default::default() };
    assert!(refused(&s, &req).contains("one batch, one Vault"));
    req.items[0].id = Some(in_fleet.clone());
    let plan = check(&s, &req, "builder").unwrap();
    assert_eq!((plan.items[0].entry_id(), plan.items[0].label()), (in_fleet.as_str(), "Deploy key"));
}

#[test]
fn no_entry_and_no_secret_is_in_a_batch_twice() {
    let s = store();
    let mut req = three();
    req.items[2].label = " alpha ".into();
    assert!(refused(&s, &req).contains("two items are for"), "labels differing only in case and spaces are one entry to a reader");
    let mut req = three();
    req.items[2].aws_secret_id = "hosts/alpha".into();
    assert!(refused(&s, &req).contains("two items copy aws-sm:hosts/alpha"));

    // Two fields of one JSON secret are two rows.
    let user = BatchItem { key: Some("username".into()), ..item("hosts/alpha", "Alpha user") };
    let password = BatchItem { key: Some("password".into()), ..item("hosts/alpha", "Alpha password") };
    assert!(check(&s, &batch(vec![user.clone(), password]), "builder").is_ok());
    assert!(refused(&s, &batch(vec![user.clone(), BatchItem { label: "Again".into(), ..user }])).contains("two items copy aws-sm:hosts/alpha#username"));

    let stripe = entry(&s, "Stripe", "personal");
    let replace = |secret: &str| BatchItem { aws_secret_id: secret.into(), id: Some(stripe.clone()), ..Default::default() };
    assert!(refused(&s, &batch(vec![replace("pay/one"), replace("pay/two")])).contains("two items are for Stripe"), "an entry's value is replaced once");
}

#[test]
fn the_approved_rows_are_the_ticked_ones_in_the_lists_order() {
    let s = store();
    let plan = check(&s, &three(), "builder").unwrap();
    let approved = plan.approved(&[2, 0, 7]);
    assert_eq!(approved.items.iter().map(Plan::label).collect::<Vec<_>>(), ["Alpha", "Gamma"]);
    assert_eq!(approved.left_out, ["Beta"]);
    assert_eq!(approved.list.len(), 16);
    assert_eq!(approved.list, plan.approved(&[0, 2]).list, "the same rows are the same list");
    assert_ne!(approved.list, plan.approved(&[0, 1, 2]).list);
    assert_ne!(approved.list, plan.approved(&[0]).list);
    assert_ne!(plan.approved(&[0, 1, 2]).list, check(&s, &three(), "builder").unwrap().approved(&[0, 1, 2]).list, "another asking of the same list is another batch");
    let none = plan.approved(&[]);
    assert!(none.items.is_empty() && none.left_out.len() == 3);
}

#[tokio::test]
async fn only_the_approved_rows_are_read_and_a_failed_one_does_not_stop_the_rest() {
    let s = store();
    let plan = check(&s, &three(), "builder").unwrap();
    let approved = plan.approved(&[0, 2]);
    // AWS holds the third secret and not the first; the second is never asked for.
    let cli = Cli::holding(&["hosts/beta", "hosts/gamma"]);
    let results = perform(&s, &cli, &approved.items, Via::Host, "builder", BUDGET).await;
    assert_eq!(*cli.seen.lock().unwrap(), ["hosts/alpha", "hosts/gamma"], "a row the person left out is never read");

    assert_eq!((results[0].label.as_str(), results[0].id.as_deref(), results[0].error.as_deref()), ("Alpha", None, Some("AWS has no secret by that id, in that region")));
    let kept = results[1].id.clone().expect("the third row was stored");
    assert_eq!((results[1].label.as_str(), results[1].error.as_deref()), ("Gamma", None));
    assert_eq!(reveal(&s, &kept, "password", "revealed").unwrap().as_str(), format!("{VALUE}-hosts/gamma"), "a login's main part is its password");

    let (outcome, what) = plan.ran_text(&approved, &results);
    assert_eq!((outcome, what.as_str()), ("partly done", format!("2 of 3 approved, 1 stored, 1 failed; list {}; not approved: Beta", approved.list).as_str()));
    note(&s, &plan, "builder", outcome, &what);

    let rows = s.vault_activity_all().unwrap();
    let batch_ref = format!("{BATCH_REF}{}", plan.id);
    let of = |action: &str, outcome: &str| rows.iter().filter(|r| r.action == action && r.outcome == outcome).collect::<Vec<_>>();
    let (stored, failed, whole) = (of(aws_import::IMPORTED, "done"), of(aws_import::IMPORTED, "failed"), of(IMPORTED_BATCH, "partly done"));
    assert_eq!((stored.len(), failed.len(), whole.len()), (1, 1, 1));
    assert_eq!((stored[0].entry_id.as_str(), stored[0].target.as_str(), stored[0].reason.as_str()), (kept.as_str(), "aws-sm:hosts/gamma", REASON));
    assert_eq!((failed[0].entry_label.as_str(), failed[0].target.as_str()), ("Alpha", "aws-sm:hosts/alpha"));
    assert_eq!((whole[0].entry_label.as_str(), whole[0].entry_id.as_str(), whole[0].actor.as_str()), ("host passwords", "", "builder"));
    for row in [stored[0], failed[0], whole[0]] {
        assert_eq!(history::refs_of(&row.refs), [batch_ref.clone(), "task 31007".to_string()], "every line of a batch names it: {row:?}");
    }
    assert!(!rows.iter().any(|r| r.entry_label == "Beta"), "a row left out has no line of its own");
    let answer = serde_json::to_string(&results).unwrap();
    assert!(!answer.contains(VALUE) && !rows.iter().any(|r| format!("{r:?}").contains(VALUE)), "no value in the answer or in a history row");
    assert_eq!(answer, format!(r#"[{{"label":"Alpha","error":"AWS has no secret by that id, in that region"}},{{"label":"Gamma","id":"{kept}"}}]"#));
}

#[tokio::test]
async fn a_batch_replaces_an_entrys_value_and_keeps_the_old_one() {
    let s = store();
    let stripe = entry(&s, "Stripe", "personal");
    let req = batch(vec![BatchItem { aws_secret_id: "pay/stripe".into(), id: Some(stripe.clone()), ..Default::default() }, item("hosts/alpha", "Alpha")]);
    let plan = check(&s, &req, "builder").unwrap();
    let approved = plan.approved(&[0, 1]);
    let results = perform(&s, &Cli::holding(&["pay/stripe", "hosts/alpha"]), &approved.items, Via::Host, "builder", BUDGET).await;
    assert_eq!(results[0].id.as_deref(), Some(stripe.as_str()));
    assert_eq!(reveal(&s, &stripe, "value", "revealed").unwrap().as_str(), format!("{VALUE}-pay/stripe"));
    assert_eq!(crate::backend::vault::versions::list(&s, &stripe).unwrap().len(), 1, "the value it replaced is kept as a version");
    assert_eq!(plan.ran_text(&approved, &results), ("done", format!("2 of 2 approved, 2 stored, 0 failed; list {}", approved.list)));
}

#[tokio::test]
async fn a_batch_out_of_time_reads_no_more_and_says_so_for_each_row_left() {
    let s = store();
    let plan = check(&s, &three(), "builder").unwrap();
    let approved = plan.approved(&[0, 1, 2]);
    let cli = Cli::holding(&["hosts/alpha", "hosts/beta", "hosts/gamma"]);
    let results = perform(&s, &cli, &approved.items, Via::Host, "builder", Duration::ZERO).await;
    assert!(cli.seen.lock().unwrap().is_empty());
    assert!(results.iter().all(|r| r.id.is_none() && r.error.as_deref().is_some_and(|e| e.starts_with("not reached"))));
    assert_eq!(plan.ran_text(&approved, &results).0, "failed");
    assert_eq!(s.vault_activity_all().unwrap().iter().filter(|r| r.outcome == "failed").count(), 3);
}

#[test]
fn a_batch_nobody_approved_is_one_line_and_many_rows_left_out_are_counted() {
    let s = store();
    let plan = check(&s, &three(), "builder").unwrap();
    note(&s, &plan, "builder", "denied", &plan.asked_text());
    let rows = s.vault_activity_all().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!((rows[0].action.as_str(), rows[0].target.as_str(), rows[0].outcome.as_str(), rows[0].reason.as_str()), (IMPORTED_BATCH, "3 secrets from AWS Secrets Manager", "denied", REASON));
    assert_eq!(secrets(1), "1 secret");

    let labels: Vec<String> = (0..39).map(|i| format!("A label of some length, number {i}")).collect();
    let said = listed(&labels);
    assert!(said.chars().count() < 340 && said.ends_with(" more") && said.starts_with("A label of some length, number 0, "), "{said}");
    assert_eq!(listed(&["One".to_string(), "Two".to_string()]), "One, Two");
}
