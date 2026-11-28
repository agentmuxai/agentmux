// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The account RPCs (`listidentityaccounts`, `getidentityaccount`, `upsertidentityaccount`,
//! `account.key.verify`), moved out of `identity::register` unchanged as named functions.

use std::sync::Arc;

use super::{merge_json_object, reject_ambient_home_dir_binding, AccountKeyVerifyResult, VerifyKeyReq};
use crate::backend::mps::Broker;
use crate::backend::storage::store::{IdentityAccount, SecretRef, Store};

/// On the blocking pool: the email backfill reads each OAuth account's `.claude.json` and canonicalizes
/// its dir (#3617), like this module's other fs/keyring calls.
pub(super) async fn list_identity_accounts(
    mstore: Arc<Store>,
    identity_store: Arc<Store>,
    provider: Option<String>,
) -> Result<Vec<IdentityAccount>, String> {
    tokio::task::spawn_blocking(move || list_with_numbers(&mstore, &identity_store, provider.as_deref()))
        .await
        .map_err(|e| format!("listidentityaccounts: task: {e}"))?
}

fn list_with_numbers(mstore: &Store, identity_store: &Store, provider: Option<&str>) -> Result<Vec<IdentityAccount>, String> {
    let mut accounts = mstore.identity_list(provider).map_err(|e| format!("listidentityaccounts: {e}"))?;
    backfill_emails(&mut accounts, mstore, identity_store);
    allocate_numbers(mstore, identity_store);
    // Answer with the numbers as stored now, including any a list that ran just before this one allocated.
    let numbered = mstore.identity_list(None).unwrap_or_default();
    copy_stored_numbers(&mut accounts, &numbered);
    Ok(accounts)
}

/// SPEC_ACCOUNT_EMAIL_IN_ARMORY_2026_09_23.md §4: backfill each OAuth account's login email from its own
/// config dir, so accounts signed in before #3541, and every Claude login (whose transcript prints no email),
/// show it. Written only when it changed, and only that one field, in this store and the global mirror (a full
/// upsert here could re-create an account deleted since the list, undo a status written meanwhile, or repoint
/// the mirror's `secret_ref`). A failed write still returns the email.
fn backfill_emails(accounts: &mut [IdentityAccount], mstore: &Store, identity_store: &Store) {
    for account in accounts {
        let before = account.context.get("email").and_then(|e| e.as_str()).unwrap_or("").to_string();
        if let Some(email) = crate::identity::account_email::refresh_account_email(account) {
            // Another address is another group (the store drops the number the same way).
            if !crate::identity::account_numbering::same_email(&before, &email) {
                if let Some(ctx) = account.context.as_object_mut() {
                    ctx.remove("seq");
                    ctx.remove("seqmax");
                }
            }
            for store in [mstore, identity_store] {
                if let Err(e) = store.identity_set_context_email(&account.id, &email) {
                    tracing::warn!(target: "identity", account_id = %account.id, error = %e, "listidentityaccounts: email backfill write failed");
                }
            }
        }
    }
}

/// #4936 §3.3: give each account that shares a provider and email with another a number that stays its own.
/// Computed over the unfiltered list, so a provider filter can't renumber a group. One allocation at a time,
/// from a fresh read: two overlapping lists must not both plan the same number, or the one written last
/// renumbers what the first returned.
fn allocate_numbers(mstore: &Store, identity_store: &Store) {
    static NUMBERING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _numbering = NUMBERING.lock().unwrap_or_else(|e| e.into_inner());
    let all = mstore.identity_list(None).unwrap_or_default();
    for u in crate::identity::account_numbering::numbering_updates(&all) {
        for store in [mstore, identity_store] {
            if let Err(e) = store.identity_set_context_numbering(&u.id, u.seq, u.high) {
                tracing::warn!(target: "identity", account_id = %u.id, error = %e, "listidentityaccounts: number write failed");
            }
        }
    }
}

/// Each listed account carries the `seq` and `seqmax` the store holds now, or neither.
fn copy_stored_numbers(accounts: &mut [IdentityAccount], numbered: &[IdentityAccount]) {
    for a in accounts {
        let Some(stored) = numbered.iter().find(|n| n.id == a.id) else { continue };
        for key in ["seq", "seqmax"] {
            match stored.context.get(key) {
                Some(v) => {
                    if !a.context.is_object() {
                        a.context = serde_json::json!({});
                    }
                    a.context[key] = v.clone();
                }
                None => {
                    if let Some(ctx) = a.context.as_object_mut() {
                        ctx.remove(key);
                    }
                }
            }
        }
    }
}

pub(super) fn get_identity_account(mstore: &Store, id: &str) -> Result<IdentityAccount, String> {
    match mstore.identity_get(id).map_err(|e| format!("getidentityaccount: {e}"))? {
        Some(a) => Ok(a),
        None => Err(format!("getidentityaccount: not found id={id}")),
    }
}

/// Accepts the full `IdentityAccount` payload. A missing `id` mints a fresh UUID; `created_at` and
/// `updated_at` are server-set so callers don't have to know the current time.
pub(super) fn upsert_identity_account(
    mstore: &Store,
    identity_store: &Store,
    broker: &Broker,
    data: serde_json::Value,
) -> Result<IdentityAccount, String> {
    let mut account: IdentityAccount = serde_json::from_value(data).map_err(|e| format!("upsertidentityaccount: {e}"))?;
    reject_ambient_home_dir_binding(&account)?;
    if account.id.is_empty() {
        account.id = uuid::Uuid::new_v4().to_string();
    }
    let now = agentmux_common::time::now_ms();
    if account.created_at == 0 {
        account.created_at = now;
    }
    account.updated_at = now;
    mstore
        .identity_upsert_with_mirror(identity_store, &account)
        .map_err(|e| format!("upsertidentityaccount: {e}"))?;
    broker.publish_event::<crate::backend::mps_events::IdentityAccountsChanged>(&(), vec![], 0);
    Ok(account)
}

/// What the optional live probe learned about a key.
struct KeyProbe {
    status: String,
    metadata: serde_json::Value,
    masked_tail: String,
    valid: bool,
}

/// Connectors → Accounts: validate (optional) and securely store an API key. The plaintext goes to the Vault (an account
/// that already keeps its key in the OS keychain keeps it there); the DB row keeps only the `SecretRef` pointer, the
/// masked tail and non-secret metadata.
/// See docs/specs/archive/SPEC_TRUST_CENTER_2026_06_15.md §5/§6.
pub(super) async fn verify_account_key(
    mstore: Arc<Store>,
    identity_store: Arc<Store>,
    broker: Arc<Broker>,
    data: serde_json::Value,
) -> Result<AccountKeyVerifyResult, String> {
    // NB: `req.api_key` is a secret, so never log `req`.
    let req: VerifyKeyReq = serde_json::from_value(data).map_err(|e| format!("account.key.verify: {e}"))?;

    // New account (mint id) vs. key replacement on an existing account. Rollback semantics differ: see below.
    let is_new = req.account_id.is_empty();
    let account_id = if is_new { uuid::Uuid::new_v4().to_string() } else { req.account_id.clone() };

    let probe = match probe_key(&req).await {
        Ok(probe) => probe,
        // Nothing stored: a structured error keeps the UI in the entry state.
        Err(rejected) => return Ok(rejected),
    };
    // The existing row, once (replacement path): preserves created_at and any previously stored context.
    let existing = mstore.identity_get(&account_id).ok().flatten();
    let StoredSecret { secret_ref, created: made_here } = store_secret(&identity_store, &account_id, &req, existing.as_ref()).await?;
    let account = key_account_row(&req, &account_id, existing.as_ref(), secret_ref, probe.status, probe.metadata, &probe.masked_tail);
    if let Err(e) = mstore.identity_upsert_with_mirror(&identity_store, &account) {
        // The DB write failed after the key was stored. A key stored for a new account (or in a new Vault entry)
        // has nothing referencing it yet, so it is rolled back rather than orphaned. A replacement keeps it: the
        // existing row still points at that keychain or Vault entry, and deleting it would destroy the previously
        // working credential (it now holds the new key, resolution still works, and the user can retry).
        if made_here {
            drop_orphaned_secret(&identity_store, &account.secret_ref, &account_id).await;
        }
        return Err(format!("account.key.verify: {e}"));
    }
    broker.publish_event::<crate::backend::mps_events::IdentityAccountsChanged>(&(), vec![], 0);
    if matches!(account.secret_ref, SecretRef::Vault { .. }) {
        crate::server::vault_handlers::publish_change(&broker);
    }
    // A key the provider just accepted proves the account works (#2237).
    if probe.valid {
        crate::server::account_episodes::after_sign_in(&account_id);
    }
    Ok(AccountKeyVerifyResult {
        valid: probe.valid,
        error: None,
        account_id: Some(account_id),
        masked_tail: Some(account.context.get("masked_tail").cloned().unwrap_or_default()),
        status: Some(account.status),
        metadata: Some(account.context),
    })
}

/// The single outbound probe, fired only when the user clicked "Validate" (`validate = true`). An invalid key
/// comes back as the result to answer with.
async fn probe_key(req: &VerifyKeyReq) -> Result<KeyProbe, AccountKeyVerifyResult> {
    if !req.validate {
        return Ok(KeyProbe {
            status: "unknown".to_string(),
            metadata: serde_json::json!({}),
            masked_tail: crate::identity::key_validator::masked_tail(&req.api_key),
            valid: false,
        });
    }
    let outcome = crate::identity::key_validator::validate(&req.provider, &req.api_key).await;
    if !outcome.valid {
        return Err(AccountKeyVerifyResult {
            valid: false,
            error: Some(outcome.error.unwrap_or_else(|| "validation failed".to_string())),
            account_id: None,
            masked_tail: None,
            status: None,
            metadata: None,
        });
    }
    Ok(KeyProbe { status: "valid".to_string(), metadata: outcome.metadata, masked_tail: outcome.masked_tail, valid: true })
}

struct StoredSecret {
    secret_ref: SecretRef,
    created: bool,
}

/// Where the key is kept: a key added to a new account goes into the Vault, as an `api-key` entry the account row points
/// at. An account that already keeps its key in the OS keychain keeps it there (a replaced key goes to the same place),
/// and one whose Vault entry is gone gets a new entry. Both are blocking (the keychain's sync D-Bus on Linux, the
/// Vault's database), so they run off the async runtime worker. Returns the pointer and whether a new entry was created.
async fn store_secret(identity_store: &Arc<Store>, account_id: &str, req: &VerifyKeyReq, existing: Option<&IdentityAccount>) -> Result<StoredSecret, String> {
    let (vault, aid, key) = (identity_store.clone(), account_id.to_string(), req.api_key.clone());
    let (provider, name) = (req.provider.clone(), req.name.clone());
    let current = existing.map(|a| a.secret_ref.clone());
    tokio::task::spawn_blocking(move || match current {
        Some(SecretRef::Keychain { service, account }) => {
            crate::identity::secret_store::put(&aid, &key)?;
            Ok(StoredSecret { secret_ref: SecretRef::Keychain { service, account }, created: false })
        }
        Some(reference @ SecretRef::Vault { .. }) => crate::backend::vault::account_key::replace(&vault, &aid, &reference, &key)
            .map(|secret_ref| StoredSecret { secret_ref, created: false })
            .or_else(|_| crate::backend::vault::account_key::create(&vault, &aid, &provider, &name, &key)
                .map(|secret_ref| StoredSecret { secret_ref, created: true })),
        _ => crate::backend::vault::account_key::create(&vault, &aid, &provider, &name, &key)
            .map(|secret_ref| StoredSecret { secret_ref, created: true }),
    })
    .await
    .map_err(|e| format!("account.key.verify: key task: {e}"))?
    .map_err(|e| format!("account.key.verify: {e}"))
}

/// A new account whose row could not be saved leaves nothing behind that points at no account.
async fn drop_orphaned_secret(identity_store: &Arc<Store>, secret_ref: &SecretRef, account_id: &str) {
    let (vault, secret_ref, aid) = (identity_store.clone(), secret_ref.clone(), account_id.to_string());
    let gone = tokio::task::spawn_blocking(move || match &secret_ref {
        SecretRef::Vault { .. } => crate::backend::vault::account_key::remove(&vault, &aid, &secret_ref),
        _ => crate::identity::secret_store::delete(&aid),
    })
    .await
    .unwrap_or_else(|je| Err(format!("join: {je}")));
    if let Err(de) = gone {
        tracing::warn!(target: "identity", "rollback of orphaned key for {account_id} failed: {de}");
    }
}

/// The row a stored key is saved as. Its non-secret context is merged so nothing the user set is lost:
/// existing context, then user-entered context, then validation metadata, then the masked tail.
fn key_account_row(
    req: &VerifyKeyReq,
    account_id: &str,
    existing: Option<&IdentityAccount>,
    secret_ref: SecretRef,
    status: String,
    metadata: serde_json::Value,
    masked_tail: &str,
) -> IdentityAccount {
    let mut context = existing.map(|a| a.context.clone()).unwrap_or_else(|| serde_json::json!({}));
    if !context.is_object() {
        context = serde_json::json!({});
    }
    merge_json_object(&mut context, &req.context);
    merge_json_object(&mut context, &metadata);
    if let serde_json::Value::Object(ref mut m) = context {
        m.insert("masked_tail".to_string(), serde_json::json!(masked_tail));
    }
    let now = agentmux_common::time::now_ms();
    let created_at = existing.map(|a| a.created_at).filter(|&c| c != 0).unwrap_or(now);
    IdentityAccount {
        id: account_id.to_string(),
        name: req.name.clone(),
        provider: req.provider.clone(),
        kind: if req.kind.is_empty() { "api_key".to_string() } else { req.kind.clone() },
        display_name: req.display_name.clone(),
        secret_ref,
        context,
        status,
        created_at,
        updated_at: now,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn account(id: &str, context: serde_json::Value) -> IdentityAccount {
        serde_json::from_value(json!({
            "id": id, "name": id, "provider": "anthropic", "kind": "api_key",
            "secret_ref": { "backend": "env", "env_var": "K" }, "context": context,
        }))
        .unwrap()
    }

    fn key_req(extra: serde_json::Value) -> VerifyKeyReq {
        let mut base = json!({ "provider": "openai", "name": "work", "apiKey": "sk-secret-1234" });
        base.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        serde_json::from_value(base).unwrap()
    }

    #[test]
    fn a_stored_keys_context_is_the_old_then_the_users_then_the_probes_then_the_tail() {
        let mut existing = account("acct-1", json!({ "note": "kept", "scopes": "old" }));
        existing.created_at = 42;
        let req = key_req(json!({ "context": { "note": "from the user", "extra": 1 } }));
        let row = key_account_row(&req, "acct-1", Some(&existing), SecretRef::Vault { entry: "e1".into(), link_id: Some("link-1".into()) }, "valid".to_string(), json!({ "scopes": "from the probe" }), "••••1234");
        assert_eq!(row.context, json!({ "note": "from the user", "extra": 1, "scopes": "from the probe", "masked_tail": "••••1234" }));
        assert_eq!((row.created_at, row.status.as_str(), row.kind.as_str()), (42, "valid", "api_key"));
        assert_eq!(row.secret_ref, SecretRef::Vault { entry: "e1".into(), link_id: Some("link-1".into()) }, "the DB keeps a pointer, never the key");
    }

    #[test]
    fn a_new_keys_row_has_its_own_time_and_the_kind_the_caller_gave() {
        let before = agentmux_common::time::now_ms();
        let row = key_account_row(&key_req(json!({ "kind": "pat", "displayName": "Work" })), "acct-2", None, SecretRef::Vault { entry: "e2".into(), link_id: None }, "unknown".to_string(), json!({}), "tail");
        assert!(row.created_at >= before && row.created_at == row.updated_at);
        assert_eq!((row.kind.as_str(), row.display_name.as_str(), row.id.as_str()), ("pat", "Work", "acct-2"));
    }

    fn vault() -> Arc<Store> {
        crate::backend::vault::seal::use_test_key();
        let store = Store::open_in_memory().unwrap();
        store.apply_identity_schema_for_tests().unwrap();
        Arc::new(store)
    }

    #[tokio::test]
    async fn a_key_added_to_a_new_account_is_a_vault_entry_the_account_reads() {
        let vault = vault();
        let req = key_req(json!({ "provider": "anthropic", "name": "work" }));
        let StoredSecret { secret_ref, created } = store_secret(&vault, "acct-1", &req, None).await.unwrap();
        assert!(created);
        let SecretRef::Vault { entry, .. } = &secret_ref else { panic!("expected a Vault pointer") };
        let row = vault.vault_get(entry).unwrap().expect("the entry exists");
        assert_eq!((row.kind.as_str(), row.label.as_str()), ("api-key", "anthropic account key: work"));
        let mut account = key_account_row(&req, "acct-1", None, secret_ref.clone(), "valid".into(), json!({}), "tail");
        account.provider = "anthropic".into();
        let key = crate::identity::resolver::secret::resolve_account_secret(&account, &vault, &vault).unwrap();
        assert_eq!(key, "sk-secret-1234");
        assert!(!serde_json::to_string(&account).unwrap().contains("sk-secret"), "the row never holds the key");
    }

    #[tokio::test]
    async fn a_replaced_key_goes_into_the_same_entry_and_a_missing_entry_is_made_again() {
        let vault = vault();
        let first = store_secret(&vault, "acct-1", &key_req(json!({})), None).await.unwrap().secret_ref;
        let mut existing = account("acct-1", json!({}));
        existing.secret_ref = first.clone();
        let StoredSecret { secret_ref: again, created } = store_secret(&vault, "acct-1", &key_req(json!({ "apiKey": "sk-new-5678" })), Some(&existing)).await.unwrap();
        assert!(!created);
        assert_eq!(again, first, "the same entry");
        assert_eq!(&**crate::backend::vault::account_key::read(&vault, "acct-1", &first).unwrap(), "sk-new-5678");
        crate::backend::vault::account_key::remove(&vault, "acct-1", &first).unwrap();
        let StoredSecret { secret_ref: remade, created } = store_secret(&vault, "acct-1", &key_req(json!({ "apiKey": "sk-newer-9012" })), Some(&existing)).await.unwrap();
        assert!(created);
        assert_eq!(&**crate::backend::vault::account_key::read(&vault, "acct-1", &remade).unwrap(), "sk-newer-9012");
    }

    #[tokio::test]
    async fn a_new_accounts_entry_is_removed_when_the_row_could_not_be_saved() {
        let vault = vault();
        let secret_ref = store_secret(&vault, "acct-1", &key_req(json!({})), None).await.unwrap().secret_ref;
        drop_orphaned_secret(&vault, &secret_ref, "acct-1").await;
        let SecretRef::Vault { entry, .. } = &secret_ref else { panic!("expected a Vault pointer") };
        assert!(vault.vault_get(entry).unwrap().is_none());
    }

    #[tokio::test]
    async fn a_reused_or_unowned_entry_gets_a_new_pointer_when_the_account_key_is_saved() {
        for legacy in [false, true] {
            let vault = vault();
            let req = key_req(json!({}));
            let first = store_secret(&vault, "acct-1", &req, None).await.unwrap().secret_ref;
            let SecretRef::Vault { entry, .. } = &first else { panic!("expected a Vault pointer") };
            crate::backend::vault::delete(&vault, entry).unwrap();
            let second = store_secret(&vault, "acct-2", &req, None).await.unwrap().secret_ref;
            let mut previous = account("acct-1", json!({}));
            previous.secret_ref = if legacy { SecretRef::Vault { entry: entry.clone(), link_id: None } } else { first };
            vault.identity_upsert(&previous).unwrap();
            let StoredSecret { secret_ref: remade, created } = store_secret(&vault, "acct-1", &key_req(json!({ "apiKey": "new-test-value" })), Some(&previous)).await.unwrap();
            assert!(created);
            assert!(remade != second, "saving the previous account creates its own entry");
            assert!(crate::backend::vault::account_key::read(&vault, "acct-2", &second).is_ok_and(|value| &**value == "sk-secret-1234"));
            assert!(crate::backend::vault::account_key::read(&vault, "acct-1", &remade).is_ok_and(|value| &**value == "new-test-value"));
        }
    }

    fn refuse_account_saves(store: &Store) {
        store.hold_for_tests().execute_batch(
            "CREATE TRIGGER refuse_account_save BEFORE INSERT ON db_accounts BEGIN SELECT RAISE(ABORT, 'forced account save failure'); END;",
        ).unwrap();
    }

    async fn failed_legacy_save_keeps_entry(already_bound: bool) {
        let primary = vault();
        let vault = vault();
        let current = store_secret(&vault, "acct-1", &key_req(json!({})), None).await.unwrap().secret_ref;
        let SecretRef::Vault { entry, .. } = &current else { panic!("expected a Vault pointer") };
        let mut previous = account("acct-1", json!({}));
        previous.secret_ref = SecretRef::Vault { entry: entry.clone(), link_id: None };
        primary.identity_upsert_with_mirror(&vault, &previous).unwrap();
        if already_bound {
            assert!(crate::backend::vault::account_key::read(&vault, "acct-1", &previous.secret_ref).is_ok());
        }
        refuse_account_saves(&primary);
        let result = verify_account_key(primary.clone(), vault.clone(), Arc::new(Broker::new()), json!({
            "accountId": "acct-1", "provider": "openai", "name": "work", "apiKey": "replacement-test-value", "validate": false,
        })).await;
        assert!(matches!(result, Err(ref error) if error.contains("forced account save failure")));
        assert!(primary.identity_get("acct-1").unwrap().unwrap().secret_ref == previous.secret_ref);
        assert!(vault.identity_get("acct-1").unwrap().unwrap().secret_ref == current);
        assert!(crate::backend::vault::account_key::read(&vault, "acct-1", &previous.secret_ref)
            .is_ok_and(|value| &**value == "replacement-test-value"), "a failed row save must not delete the existing credential");
    }

    #[tokio::test]
    async fn a_legacy_pointer_bound_during_replace_survives_a_failed_account_save() {
        failed_legacy_save_keeps_entry(false).await;
    }

    #[tokio::test]
    async fn a_legacy_pointer_bound_only_in_the_mirror_survives_a_failed_account_save() {
        failed_legacy_save_keeps_entry(true).await;
    }

    #[tokio::test]
    async fn a_new_or_recreated_entry_is_rolled_back_after_a_failed_account_save() {
        for existing_account in [false, true] {
            let primary = vault();
            let vault = vault();
            if existing_account {
                let current = store_secret(&vault, "acct-1", &key_req(json!({})), None).await.unwrap().secret_ref;
                let mut previous = account("acct-1", json!({}));
                previous.secret_ref = current;
                primary.identity_upsert_with_mirror(&vault, &previous).unwrap();
                crate::backend::vault::account_key::remove(&vault, "acct-1", &previous.secret_ref).unwrap();
            }
            refuse_account_saves(&primary);
            let result = verify_account_key(primary, vault.clone(), Arc::new(Broker::new()), json!({
                "accountId": "acct-1", "provider": "openai", "name": "work", "apiKey": "replacement-test-value", "validate": false,
            })).await;
            assert!(matches!(result, Err(ref error) if error.contains("forced account save failure")));
            assert!(vault.vault_count().unwrap() == 0, "only an entry created by this operation is rolled back");
        }
    }

    #[test]
    fn a_listed_account_takes_the_numbers_as_stored_or_loses_them() {
        let mut listed = vec![account("a", json!({ "seq": 9 })), account("b", json!({})), account("c", json!({ "seq": 1 }))];
        let stored = vec![account("a", json!({})), account("b", json!({ "seq": 2, "seqmax": 3 }))];
        copy_stored_numbers(&mut listed, &stored);
        assert_eq!(listed[0].context, json!({}), "a number the store no longer holds is dropped");
        assert_eq!(listed[1].context, json!({ "seq": 2, "seqmax": 3 }));
        assert_eq!(listed[2].context, json!({ "seq": 1 }), "an account the store doesn't know is left as listed");
    }

    #[test]
    fn a_get_of_a_missing_account_names_the_id() {
        let store = Store::open_in_memory().unwrap();
        let err = get_identity_account(&store, "ghost").unwrap_err();
        assert_eq!(err, "getidentityaccount: not found id=ghost");
    }
}
