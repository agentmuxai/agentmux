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

/// Connectors → Accounts: validate (optional) and securely store an API key. The plaintext goes to the OS
/// keychain; the DB row keeps only the `SecretRef::Keychain` pointer, the masked tail and non-secret metadata.
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
    store_secret(&account_id, &req.api_key).await?;

    // The existing row, once (replacement path): preserves created_at and any previously stored context.
    let existing = mstore.identity_get(&account_id).ok().flatten();
    let account = key_account_row(&req, &account_id, existing.as_ref(), probe.status, probe.metadata, &probe.masked_tail);
    if let Err(e) = mstore.identity_upsert_with_mirror(&identity_store, &account) {
        // The DB write failed after the keychain write. A new account has nothing referencing the secret yet,
        // so it is rolled back rather than orphaned. A replacement keeps it: the existing row still points at
        // this keychain entry, and deleting it would destroy the previously working credential (the secret is
        // now the new key, resolution still works, and the user can retry to fix metadata).
        if is_new {
            drop_orphaned_secret(&account_id).await;
        }
        return Err(format!("account.key.verify: {e}"));
    }
    broker.publish_event::<crate::backend::mps_events::IdentityAccountsChanged>(&(), vec![], 0);
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

/// The plaintext goes to the OS keychain; the DB never sees it. `keyring` is blocking (sync D-Bus on Linux),
/// so it runs off the async runtime worker.
async fn store_secret(account_id: &str, api_key: &str) -> Result<(), String> {
    let (aid, key) = (account_id.to_string(), api_key.to_string());
    tokio::task::spawn_blocking(move || crate::identity::secret_store::put(&aid, &key))
        .await
        .map_err(|e| format!("account.key.verify: keychain task: {e}"))?
        .map_err(|e| format!("account.key.verify: {e}"))
}

async fn drop_orphaned_secret(account_id: &str) {
    let aid = account_id.to_string();
    if let Err(de) = tokio::task::spawn_blocking(move || crate::identity::secret_store::delete(&aid))
        .await
        .unwrap_or_else(|je| Err(format!("join: {je}")))
    {
        tracing::warn!(target: "identity", "rollback of orphaned keychain secret for {account_id} failed: {de}");
    }
}

/// The row a stored key is saved as. Its non-secret context is merged so nothing the user set is lost:
/// existing context, then user-entered context, then validation metadata, then the masked tail.
fn key_account_row(
    req: &VerifyKeyReq,
    account_id: &str,
    existing: Option<&IdentityAccount>,
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
        secret_ref: SecretRef::Keychain {
            service: crate::identity::secret_store::SERVICE.to_string(),
            account: crate::identity::secret_store::account_key(account_id),
        },
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
        let row = key_account_row(&req, "acct-1", Some(&existing), "valid".to_string(), json!({ "scopes": "from the probe" }), "••••1234");
        assert_eq!(row.context, json!({ "note": "from the user", "extra": 1, "scopes": "from the probe", "masked_tail": "••••1234" }));
        assert_eq!((row.created_at, row.status.as_str(), row.kind.as_str()), (42, "valid", "api_key"));
        assert!(matches!(row.secret_ref, SecretRef::Keychain { .. }), "the DB keeps a pointer, never the key");
    }

    #[test]
    fn a_new_keys_row_has_its_own_time_and_the_kind_the_caller_gave() {
        let before = agentmux_common::time::now_ms();
        let row = key_account_row(&key_req(json!({ "kind": "pat", "displayName": "Work" })), "acct-2", None, "unknown".to_string(), json!({}), "tail");
        assert!(row.created_at >= before && row.created_at == row.updated_at);
        assert_eq!((row.kind.as_str(), row.display_name.as_str(), row.id.as_str()), ("pat", "Work", "acct-2"));
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
