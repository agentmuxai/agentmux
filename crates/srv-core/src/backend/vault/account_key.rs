// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The key of an API key account (Connectors > Accounts), kept as an `api-key` entry of the Vault. The account row holds
//! the entry's id and non-reusable link id (`SecretRef::Vault`); the key is sealed in the entry like any other, so the person sees it in the
//! Vault, can copy or replace it there, and deleting the account deletes the entry.

use super::{kinds, seal, NewEntry, Store};
use crate::backend::storage::{store::SecretRef, vault::VaultRow};
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

fn note(account_id: &str) -> String {
    format!("Added in Connectors > Accounts for the account {account_id}. The account reads its key from here.")
}

/// Saves `key` as a new entry for the account `account_id`; returns its bound pointer.
pub fn create(store: &Store, account_id: &str, provider: &str, name: &str, key: &str) -> Result<SecretRef, String> {
    let fields = BTreeMap::from([
        ("service".to_string(), provider.to_string()),
        ("notes".to_string(), note(account_id)),
    ]);
    let entry = NewEntry { kind: "api-key".into(), label: label(provider, name), fields, tags: vec![TAG.to_string()], parts: parts(key) };
    let link_id = super::new_link_id();
    let (id, label) = super::insert_unrecorded_bound(store, entry, super::PERSONAL_VAULT, Some(&link_id))
        .map_err(|_| "could not create the account key entry")?;
    super::record(store, &id, &label, super::PERSON, "created", "", "done");
    withhold_sources::account_key(&id, key);
    Ok(SecretRef::Vault { entry: id, link_id: Some(link_id) })
}

fn checked(store: &Store, account_id: &str, reference: &SecretRef) -> Result<(VaultRow, String), String> {
    store.vault_account_key_row(account_id, reference, &super::new_link_id(), &note(account_id))
        .map_err(|_| "could not check the account key entry identity".to_string())?
        .ok_or_else(|| {
            tracing::warn!(target: "identity", "account.key: refused a missing or unowned Vault entry");
            "the Vault entry that holds this account's key is gone or no longer belongs to it".to_string()
        })
}

/// Puts a new key in the entry, leaving its label, fields and tags as they are.
pub fn replace(store: &Store, account_id: &str, reference: &SecretRef, key: &str) -> Result<SecretRef, String> {
    let (row, link_id) = checked(store, account_id, reference)?;
    if row.kind != "api-key" { return Err("the account key entry is not an api-key entry".into()); }
    let kept = super::versions::of(&row, super::PERSON, "replaced");
    let (was, was_fields) = (row.sealed.clone(), row.fields.clone());
    let id = row.id.clone();
    let changed = VaultRow { sealed: seal::seal(&seal::key(false)?, &id, &parts(key))?, updated_at: super::now_ms(), ..row };
    if !store.vault_update_kept_bound(&changed, &was, &was_fields, Some(&kept), Some(&link_id)).map_err(|_| "could not replace the account key")? {
        return Err("the account key entry changed while replacing it".into());
    }
    super::versions::prune(store);
    super::record(store, &id, &changed.label, super::PERSON, "replaced", "", "done");
    withhold_sources::account_key(&id, key);
    Ok(SecretRef::Vault { entry: id, link_id: Some(link_id) })
}

/// The key in the entry. The model gateway asks on every request, so a read is not written to the activity log. A key
/// written or read here is withheld from transcripts, as one in the OS keychain is.
pub fn read(store: &Store, account_id: &str, reference: &SecretRef) -> Result<zeroize::Zeroizing<String>, String> {
    let (row, _) = checked(store, account_id, reference)?;
    let kind = kinds::kind(&row.kind).filter(|k| k.name == "api-key").ok_or_else(|| "the Vault entry that holds this account's key is not an api-key entry".to_string())?;
    let opened = seal::open(&seal::key(false)?, &row.id, &row.sealed).map_err(|_| "the account key entry could not be opened")?;
    let value = opened.get(kind.parts[0]).ok_or_else(|| "the Vault entry that holds this account's key has no value".to_string())?;
    withhold_sources::account_key(&row.id, value);
    Ok(zeroize::Zeroizing::new(value.to_string()))
}

/// Deletes the entry; one that is already gone is not an error.
pub fn remove(store: &Store, account_id: &str, reference: &SecretRef) -> Result<(), String> {
    let Some((row, link_id)) = store.vault_account_key_row(account_id, reference, &super::new_link_id(), &note(account_id))
        .map_err(|_| "could not check the account key entry identity")? else {
        tracing::warn!(target: "identity", "account.key: left a missing or unowned Vault entry alone");
        return Ok(());
    };
    if store.vault_delete_bound(&row.id, Some(&link_id)).map_err(|_| "could not remove the account key entry")? {
        super::requests::requests().forget_entry(&row.id);
        super::record(store, &row.id, &row.label, super::PERSON, "deleted", "", "done");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(reference: &SecretRef) -> &str {
        let SecretRef::Vault { entry, .. } = reference else { panic!("expected a Vault pointer") };
        entry
    }

    fn read(store: &Store, reference: &SecretRef) -> Result<zeroize::Zeroizing<String>, String> {
        super::read(store, "acct-1", reference)
    }

    fn replace(store: &Store, reference: &SecretRef, key: &str) -> Result<SecretRef, String> {
        super::replace(store, "acct-1", reference, key)
    }

    fn remove(store: &Store, reference: &SecretRef) -> Result<(), String> {
        super::remove(store, "acct-1", reference)
    }

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
        let row = store.vault_get(entry(&id)).unwrap().unwrap();
        assert_eq!((row.kind.as_str(), row.label.as_str()), ("api-key", "Anthropic account key: work"));
        assert!(row.tags.contains(TAG));
        assert_eq!(&**read(&store, &id).unwrap(), "sk-ant-one");
        replace(&store, &id, "sk-ant-two").unwrap();
        assert_eq!(&**read(&store, &id).unwrap(), "sk-ant-two");
        assert_eq!(store.vault_get(entry(&id)).unwrap().unwrap().label, row.label, "the label stays");
    }

    #[test]
    fn a_key_made_replaced_or_read_is_withheld_from_transcripts() {
        let store = store();
        let made = create(&store, "acct-1", "Anthropic", "work", "sk-ant-made-aaaaaaaaaaaaaaaa").unwrap();
        assert!(crate::backend::transcript::withhold::holds("the key sk-ant-made-aaaaaaaaaaaaaaaa was printed"));
        replace(&store, &made, "sk-ant-replaced-bbbbbbbbbbbbbbbb").unwrap();
        assert!(crate::backend::transcript::withhold::holds("sk-ant-replaced-bbbbbbbbbbbbbbbb"));
        let other = store.vault_get(entry(&made)).unwrap().unwrap();
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
    fn a_reused_entry_does_not_let_the_previous_account_read_replace_or_remove_it() {
        let store = store();
        let previous = create(&store, "account-a", "provider", "same-name", "first-test-value").unwrap();
        super::super::delete(&store, entry(&previous)).unwrap();
        let current = create(&store, "account-b", "provider", "same-name", "second-test-value").unwrap();
        assert!(entry(&previous) == entry(&current), "the label-derived entry id is reused");
        assert!(previous != current, "the non-reusable link id is different");
        assert!(read(&store, &previous).is_err(), "the previous account must not read the replacement entry");
        assert!(replace(&store, &previous, "third-test-value").is_err(), "the previous account must not replace another account's key");
        remove(&store, &previous).unwrap();
        assert!(read(&store, &current).is_ok_and(|value| &**value == "second-test-value"), "the current account keeps its own key");
    }

    fn legacy(store: &Store, reference: &SecretRef) -> crate::backend::storage::store::IdentityAccount {
        let account = serde_json::from_value(serde_json::json!({
            "id": "acct-1", "name": "work", "provider": "provider", "kind": "api_key",
            "secret_ref": { "backend": "vault", "entry": entry(reference) },
        })).unwrap();
        store.identity_upsert(&account).unwrap();
        account
    }

    #[test]
    fn a_legacy_pointer_with_a_matching_note_is_bound_once_and_keeps_working() {
        let store = store();
        let current = create(&store, "acct-1", "provider", "same-name", "first-test-value").unwrap();
        let account = legacy(&store, &current);
        assert!(read(&store, &account.secret_ref).is_ok());
        assert!(store.identity_get("acct-1").unwrap().unwrap().secret_ref == current);
        store.vault_set_fields(entry(&current), "{}").unwrap();
        store.identity_upsert(&account).unwrap();
        assert!(store.identity_get("acct-1").unwrap().unwrap().secret_ref == current, "a stale account save preserves the binding");
        assert!(read(&store, &account.secret_ref).is_ok(), "a stale legacy pointer uses the persisted binding, not the note again");
        assert!(replace(&store, &account.secret_ref, "second-test-value").unwrap() == current);
        super::super::delete(&store, entry(&current)).unwrap();
        let reused = create(&store, "acct-1", "provider", "same-name", "third-test-value").unwrap();
        assert!(read(&store, &account.secret_ref).is_err(), "even another matching note cannot rebind a migrated pointer");
        assert!(replace(&store, &account.secret_ref, "fourth-test-value").is_err());
        remove(&store, &account.secret_ref).unwrap();
        assert!(read(&store, &reused).is_ok_and(|value| &**value == "third-test-value"));
    }

    fn refuses_legacy(store: &Store, current: &SecretRef) {
        let account = legacy(store, current);
        assert!(read(store, &account.secret_ref).is_err());
        assert!(replace(store, &account.secret_ref, "replacement-test-value").is_err());
        remove(store, &account.secret_ref).unwrap();
        assert!(store.identity_get("acct-1").unwrap().unwrap().secret_ref == account.secret_ref);
        assert!(read(store, current).is_ok_and(|value| &**value == "first-test-value"));
    }

    #[test]
    fn a_legacy_pointer_without_an_account_note_is_refused() {
        let store = store();
        let current = create(&store, "acct-1", "provider", "same-name", "first-test-value").unwrap();
        store.vault_set_fields(entry(&current), "{}").unwrap();
        refuses_legacy(&store, &current);
    }

    #[test]
    fn a_legacy_pointer_with_another_accounts_note_is_refused() {
        let store = store();
        let current = create(&store, "account-b", "provider", "same-name", "first-test-value").unwrap();
        refuses_legacy(&store, &current);
    }

    #[test]
    fn a_legacy_pointer_can_be_bound_by_replace_or_remove_before_its_first_read() {
        for removing in [false, true] {
            let store = store();
            let current = create(&store, "acct-1", "provider", "same-name", "first-test-value").unwrap();
            let account = legacy(&store, &current);
            if removing {
                remove(&store, &account.secret_ref).unwrap();
                assert!(store.vault_get(entry(&current)).unwrap().is_none());
            } else {
                assert!(replace(&store, &account.secret_ref, "second-test-value").unwrap() == current);
                assert!(read(&store, &current).is_ok_and(|value| &**value == "second-test-value"));
            }
            assert!(store.identity_get("acct-1").unwrap().unwrap().secret_ref == current);
        }
    }

    #[test]
    fn a_legacy_entry_without_a_link_gets_one_before_its_first_read() {
        let store = store();
        let id = super::super::create(&store, NewEntry {
            kind: "api-key".into(), label: "legacy fixture".into(),
            fields: BTreeMap::from([("notes".into(), note("acct-1"))]),
            tags: Vec::new(), parts: parts("first-test-value"),
        }).unwrap();
        let account = legacy(&store, &SecretRef::Vault { entry: id.clone(), link_id: None });
        assert!(store.vault_link_primary(&id).unwrap().is_none());
        assert!(read(&store, &account.secret_ref).is_ok());
        let saved = store.identity_get("acct-1").unwrap().unwrap().secret_ref;
        assert!(matches!(saved, SecretRef::Vault { link_id: Some(_), .. }));
        assert!(read(&store, &saved).is_ok());
    }

    #[test]
    fn a_checked_entry_reused_before_a_write_cannot_be_replaced_or_removed() {
        let store = store();
        let previous = create(&store, "acct-1", "provider", "same-name", "first-test-value").unwrap();
        let (row, link_id) = checked(&store, "acct-1", &previous).unwrap();
        super::super::delete(&store, &row.id).unwrap();
        let current = create(&store, "account-b", "provider", "same-name", "second-test-value").unwrap();
        let current_row = store.vault_get(entry(&current)).unwrap().unwrap();
        assert!(!store.vault_update_kept_bound(&current_row, &current_row.sealed, &current_row.fields, None, Some(&link_id)).unwrap());
        assert!(!store.vault_delete_bound(&row.id, Some(&link_id)).unwrap());
        assert!(read(&store, &current).is_ok_and(|value| &**value == "second-test-value"));
    }

    #[test]
    fn no_model_route_takes_an_account_key_for_its_own() {
        let store = store();
        create(&store, "acct-1", "OpenRouter", "work", "sk-or-one").unwrap();
        assert!(!super::super::route_key::present(&store, "OpenRouter API key"));
    }
}
