// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A backed entry: what one may hold, that each use reads AWS Secrets Manager and keeps no copy, and that a failed read
//! says why without any of the value. The AWS CLI is a stand-in; every secret named here is invented.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use super::*;
use crate::backend::storage::store::Store;
use crate::backend::vault::linked::{BACKED, LINK, ONLY};
use crate::backend::vault::source_link::{AWS_SECRET, GITHUB_SECRET, KEY, KIND, LOCATION, REGION};
use crate::backend::vault::{command_target, create, fill_target, seal, value_for_command, value_for_fill, NewEntry, Parts};
use crate::identity::aws_broker::mint::{RunError, RunOutput, RunSpec};

const TOKEN_V1: &str = "backed-sentinel-4417-aa";
const TOKEN_V2: &str = "backed-sentinel-9082-zq";
const DENIED: &str = "An error occurred (AccessDeniedException) when calling the GetSecretValue operation: User is not authorized to perform that action";

/// A stand-in AWS CLI that holds secrets (the id and the text a read answers with) and counts the reads.
pub(crate) struct Aws {
    secrets: Mutex<Vec<(String, Result<String, String>)>>,
    reads: Mutex<Vec<String>>,
}

impl Aws {
    pub(crate) fn with(secrets: &[(&str, Result<&str, &str>)]) -> Self {
        let owned = secrets.iter().map(|(id, said)| (id.to_string(), said.map(str::to_string).map_err(str::to_string))).collect();
        Self { secrets: Mutex::new(owned), reads: Mutex::new(Vec::new()) }
    }

    /// The secret as it is now in AWS: what the next read gets.
    fn change(&self, id: &str, text: &str) {
        for (name, said) in self.secrets.lock().unwrap().iter_mut() {
            if name == id {
                *said = Ok(text.to_string());
            }
        }
    }

    fn reads(&self) -> usize {
        self.reads.lock().unwrap().len()
    }
}

impl Runner for Aws {
    fn which(&self, _program: &str) -> Option<PathBuf> {
        Some(PathBuf::from("/bin/aws"))
    }

    async fn run(&self, spec: RunSpec) -> Result<RunOutput, RunError> {
        let arg = |i: usize| spec.args.get(i).cloned().unwrap_or_default();
        assert_eq!(arg(1), "get-secret-value", "a use reads the value and nothing else");
        let id = arg(3);
        self.reads.lock().unwrap().push(id.clone());
        let said = self.secrets.lock().unwrap().iter().find(|(name, _)| *name == id).map(|(_, said)| said.clone());
        Ok(match said {
            // The CLI prints `SecretString` as a JSON string.
            Some(Ok(text)) => RunOutput { success: true, stdout: serde_json::to_vec(&text).unwrap(), stderr: String::new() },
            Some(Err(said)) => RunOutput { success: false, stdout: Vec::new(), stderr: said },
            None => RunOutput { success: false, stdout: Vec::new(), stderr: "An error occurred (ResourceNotFoundException) when calling the GetSecretValue operation: Secrets Manager can't find the specified secret.".into() },
        })
    }
}

fn store() -> Arc<Store> {
    seal::use_test_key();
    let s = Store::open_in_memory().unwrap();
    s.apply_identity_schema_for_tests().unwrap();
    Arc::new(s)
}

fn fields(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

fn place(mark: &'static str) -> Vec<(&'static str, &'static str)> {
    vec![(LINK, mark), (KIND, AWS_SECRET), (LOCATION, "prod/invoice-sync"), (KEY, "api.token"), (REGION, "eu-west-1")]
}

fn made_as(s: &Store, label: &str, f: &[(&str, &str)]) -> Result<String, String> {
    create(s, NewEntry { kind: "api-key".into(), label: label.into(), fields: fields(f), tags: vec![], parts: Parts::default() })
}

fn made(s: &Store, f: &[(&str, &str)]) -> Result<String, String> {
    made_as(s, "Invoice provider key", f)
}

fn backed_entry(s: &Store) -> (String, VaultRow) {
    let id = made(s, &place(BACKED)).unwrap();
    let row = s.vault_get(&id).unwrap().unwrap();
    (id, row)
}

fn json(token: &str) -> String {
    format!(r#"{{"api": {{"token": "{token}", "user": "svc"}}, "other": 1}}"#)
}

#[test]
fn a_backed_entry_names_its_place_and_holds_no_value() {
    let s = store();
    let (_, row) = backed_entry(&s);
    assert!(linked::is_backed_row(&row) && linked::is_linked_row(&row));
    assert!(seal::open(&seal::key(false).unwrap(), &row.id, &row.sealed).unwrap().0.is_empty());
    assert!(!linked::is_backed_row(&s.vault_get(&made_as(&s, "Recorded only", &place(ONLY)).unwrap()).unwrap().unwrap()));

    let valued = NewEntry { kind: "api-key".into(), label: "Held".into(), fields: fields(&place(BACKED)), tags: vec![], parts: Parts([("value".to_string(), TOKEN_V1.to_string())].into()) };
    assert!(create(&s, valued).unwrap_err().contains("holds no value"));
    // Only an AWS secret can be read from here.
    let ci = made(&s, &[(LINK, BACKED), (KIND, GITHUB_SECRET), (LOCATION, "example-org/example-app"), (KEY, "EXAMPLE_TOKEN")]).unwrap_err();
    assert!(ci.contains("read from an AWS secret"), "{ci}");
    assert!(made(&s, &[(LINK, "mirror"), (KIND, AWS_SECRET), (LOCATION, "prod/x")]).unwrap_err().contains("\"backed\""));
}

#[test]
fn each_use_reads_aws_and_keeps_no_copy() {
    let s = store();
    let (id, row) = backed_entry(&s);
    let v1 = json(TOKEN_V1);
    let aws = Aws::with(&[("prod/invoice-sync", Ok(v1.as_str()))]);

    assert_eq!(&**value_with(&aws, &row).unwrap(), TOKEN_V1);
    // The secret is rotated in AWS: the next use has the new value, with nothing changed in the Vault.
    aws.change("prod/invoice-sync", &json(TOKEN_V2));
    assert_eq!(&**value_with(&aws, &row).unwrap(), TOKEN_V2);
    assert_eq!(aws.reads(), 2);

    let after = s.vault_get(&id).unwrap().unwrap();
    assert_eq!((after.sealed.clone(), after.fields.clone(), after.updated_at), (row.sealed.clone(), row.fields.clone(), row.updated_at), "a use writes nothing to the entry");
    let sealed = seal::open(&seal::key(false).unwrap(), &after.id, &after.sealed).unwrap();
    assert!(sealed.0.is_empty(), "no copy of the value is in the Vault");
}

#[test]
fn the_whole_secret_is_the_value_when_no_key_is_named() {
    let s = store();
    let id = made(&s, &[(LINK, BACKED), (KIND, AWS_SECRET), (LOCATION, "prod/plain-token")]).unwrap();
    let row = s.vault_get(&id).unwrap().unwrap();
    let aws = Aws::with(&[("prod/plain-token", Ok(TOKEN_V1))]);
    assert_eq!(&**value_with(&aws, &row).unwrap(), TOKEN_V1);
}

#[test]
fn a_failed_read_says_why_and_never_shows_a_value() {
    let s = store();
    let (_, row) = backed_entry(&s);

    let denied = value_with(&Aws::with(&[("prod/invoice-sync", Err(DENIED))]), &row).unwrap_err();
    assert!(denied.contains("AWS refused") && denied.contains("Invoice provider key"), "{denied}");
    let gone = value_with(&Aws::with(&[]), &row).unwrap_err();
    assert!(gone.contains("AWS has no secret by that id"), "{gone}");
    // A secret that isn't JSON has no key to pick, and one without the key says which.
    let plain = value_with(&Aws::with(&[("prod/invoice-sync", Ok(TOKEN_V1))]), &row).unwrap_err();
    assert!(plain.contains("isn't JSON") && !plain.contains(TOKEN_V1), "{plain}");
    let other = value_with(&Aws::with(&[("prod/invoice-sync", Ok(r#"{"api": {"user": "svc"}}"#))]), &row).unwrap_err();
    assert!(other.contains("has no key api.token"), "{other}");
}

#[test]
fn a_use_of_a_backed_entry_is_allowed_where_a_link_only_entry_is_refused() {
    let s = store();
    let (id, row) = backed_entry(&s);
    let only = made_as(&s, "Recorded only", &place(ONLY)).unwrap();

    let target = command_target(&s, &id, None).unwrap();
    assert_eq!((target.label.as_str(), target.part), ("Invoice provider key", "value"));
    assert!(fill_target(&s, &id, None).is_ok());
    assert!(command_target(&s, &only, None).err().unwrap().contains("holds no value"));

    // The Vault keeps no earlier value of a backed entry, and an entry has no part but its main one.
    let previous = command_target(&s, &format!("{id}@previous"), None).unwrap();
    assert!(value_for_command(&s, &previous).unwrap_err().contains("keeps no earlier value"));
    assert!(value_for_fill(&s, &id, row.created_at, "extra").unwrap_err().contains("has no extra saved"));
}

#[test]
fn a_change_that_needs_a_value_is_sent_to_aws() {
    let s = store();
    let (_, row) = backed_entry(&s);
    let said = linked::takes_no_value(&row, "rotate it").unwrap();
    assert!(said.contains("backed by AWS Secrets Manager") && said.contains("change the secret there"), "{said}");
}

#[test]
fn the_fields_of_a_backed_batch_item_mark_it_and_name_the_place() {
    let source = Source { aws_secret_id: "prod/invoice-sync".into(), key: Some("api.token".into()), user_key: None, region: Some("eu-west-1".into()), profile: None };
    let f = linked::fields_for_backed(&source, &fields(&[("readers", "invoice-sync (lambda): at run time")]));
    assert_eq!(f.get(LINK).map(String::as_str), Some(BACKED));
    assert_eq!(f.get(LOCATION).map(String::as_str), Some("prod/invoice-sync"));
    assert!(linked::validate(&f, false).is_ok());
    assert_eq!(linked::fields_for(&source, &BTreeMap::new()).get(LINK).map(String::as_str), Some(ONLY));
}
