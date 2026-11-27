// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The key of an API key account (Connectors > Accounts), kept as an `api-key` entry of the Vault. The account row holds
//! only the entry's id (`SecretRef::Vault`); the key is sealed in the entry like any other, so the person sees it in the
//! Vault, can copy or replace it there, and deleting the account deletes the entry.

use super::{kinds, seal, EntryChange, NewEntry, Store};
use crate::backend::transcript::withhold_sources;
use std::collections::BTreeMap;

/// The tag every account key entry carries.
pub const TAG: &str = "account-key";

/// The label of the entry for an account: it names the account, and no model route looks for it.
fn label(provider: &str, name: &str) -> String {
    format!("{provider} account key: {name}")
}

fn parts(key: &str) -> seal::Parts {
    seal::Parts(BTreeMap::from([("value".to_string(), key.to_string())]))
}

/// Saves `key` as a new entry for the account `account_id`; returns the entry's id.
pub fn create(store: &Store, account_id: &str, provider: &str, name: &str, key: &str) -> Result<String, String> {
    let fields = BTreeMap::from([
        ("service".to_string(), provider.to_string()),
        ("notes".to_string(), format!("Added in Connectors > Accounts for the account {account_id}. The account reads its key from here.")),
    ]);
    let entry = NewEntry { kind: "api-key".into(), label: label(provider, name), fields, tags: vec![TAG.to_string()], parts: parts(key) };
    let id = super::create(store, entry)?;
    withhold_sources::account_key(&id, key);
    Ok(id)
}

/// Puts a new key in the entry, leaving its label, fields and tags as they are.
pub fn replace(store: &Store, entry: &str, key: &str) -> Result<(), String> {
    let row = store.vault_get(entry).map_err(|e| e.to_string())?.ok_or_else(|| format!("no entry {entry}"))?;
    let fields = serde_json::from_str(&row.fields).map_err(|e| e.to_string())?;
    let tags = serde_json::from_str(&row.tags).map_err(|e| e.to_string())?;
    let id = row.id.clone();
    super::update(store, EntryChange { id: row.id, label: row.label, fields, tags, parts: parts(key), clear: Vec::new() })?;
    withhold_sources::account_key(&id, key);
    Ok(())
}

/// The key in the entry. The model gateway asks on every request, so a read is not written to the activity log. A key
/// written or read here is withheld from transcripts, as one in the OS keychain is.
pub fn read(store: &Store, entry: &str) -> Result<zeroize::Zeroizing<String>, String> {
    let row = store.vault_get(entry).map_err(|e| e.to_string())?.ok_or_else(|| "the Vault entry that holds this account's key is gone".to_string())?;
    let kind = kinds::kind(&row.kind).filter(|k| k.name == "api-key").ok_or_else(|| "the Vault entry that holds this account's key is not an api-key entry".to_string())?;
    let opened = seal::open(&seal::key(false)?, &row.id, &row.sealed)?;
    let value = opened.get(kind.parts[0]).ok_or_else(|| "the Vault entry that holds this account's key has no value".to_string())?;
    withhold_sources::account_key(&row.id, value);
    Ok(zeroize::Zeroizing::new(value.to_string()))
}

/// Deletes the entry; one that is already gone is not an error.
pub fn remove(store: &Store, entry: &str) -> Result<(), String> {
    match store.vault_get(entry).map_err(|e| e.to_string())? {
        Some(_) => super::delete(store, entry),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        seal::use_test_key();
        let store = Store::open_in_memory().unwrap();
        store.apply_identity_schema_for_tests().unwrap();
        store
    }

    #[test]
    fn an_account_key_is_an_api_key_entry_that_reads_back_and_is_replaced_in_place() {
        let store = store();
        let id = create(&store, "acct-1", "Anthropic", "work", "sk-ant-one").unwrap();
        let row = store.vault_get(&id).unwrap().unwrap();
        assert_eq!((row.kind.as_str(), row.label.as_str()), ("api-key", "Anthropic account key: work"));
        assert!(row.tags.contains(TAG));
        assert_eq!(&**read(&store, &id).unwrap(), "sk-ant-one");
        replace(&store, &id, "sk-ant-two").unwrap();
        assert_eq!(&**read(&store, &id).unwrap(), "sk-ant-two");
        assert_eq!(store.vault_get(&id).unwrap().unwrap().label, row.label, "the label stays");
    }

    #[test]
    fn a_key_made_replaced_or_read_is_withheld_from_transcripts() {
        let store = store();
        let made = create(&store, "acct-1", "Anthropic", "work", "sk-ant-made-aaaaaaaaaaaaaaaa").unwrap();
        assert!(crate::backend::transcript::withhold::holds("the key sk-ant-made-aaaaaaaaaaaaaaaa was printed"));
        replace(&store, &made, "sk-ant-replaced-bbbbbbbbbbbbbbbb").unwrap();
        assert!(crate::backend::transcript::withhold::holds("sk-ant-replaced-bbbbbbbbbbbbbbbb"));
        let other = store.vault_get(&made).unwrap().unwrap();
        let sealed = seal::seal(&seal::key(false).unwrap(), &other.id, &seal::Parts(BTreeMap::from([("value".to_string(), "sk-ant-read-cccccccccccccccc".to_string())]))).unwrap();
        store.vault_update(&crate::backend::storage::vault::VaultRow { sealed, ..other }).unwrap();
        assert!(!crate::backend::transcript::withhold::holds("sk-ant-read-cccccccccccccccc"));
        read(&store, &made).unwrap();
        assert!(crate::backend::transcript::withhold::holds("sk-ant-read-cccccccccccccccc"), "a read registers it too");
    }

    #[test]
    fn a_removed_entry_reads_as_gone_and_is_removed_again_without_error() {
        let store = store();
        let id = create(&store, "acct-1", "Anthropic", "work", "sk-ant-one").unwrap();
        remove(&store, &id).unwrap();
        assert!(read(&store, &id).unwrap_err().contains("is gone"));
        remove(&store, &id).unwrap();
    }

    #[test]
    fn no_model_route_takes_an_account_key_for_its_own() {
        let store = store();
        create(&store, "acct-1", "OpenRouter", "work", "sk-or-one").unwrap();
        assert!(!super::super::route_key::present(&store, "OpenRouter API key"));
    }
}
