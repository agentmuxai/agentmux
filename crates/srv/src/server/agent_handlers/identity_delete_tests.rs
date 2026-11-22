// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use super::{delete_identity_account, release_sign_in_values};
use crate::backend::mps::Broker;
use crate::backend::storage::store::{IdentityAccount, SecretRef, Store};
use crate::backend::transcript::withhold;
use crate::identity::resolver::probe_oauth_status;

fn account(id: &str, dir: &std::path::Path) -> IdentityAccount {
    IdentityAccount {
        id: id.to_string(),
        name: id.to_string(),
        provider: "claude".to_string(),
        kind: "oauth".to_string(),
        display_name: String::new(),
        secret_ref: SecretRef::OAuthConfigDir { dir: dir.to_string_lossy().into_owned() },
        context: serde_json::json!({}),
        status: "ok".to_string(),
        created_at: 0,
        updated_at: 0,
    }
}

/// A Claude sign-in file and a Codex one in `dir`, each read once. Values: `<tag>-claude-access`, `<tag>-claude-refresh`,
/// `<tag>-codex-refresh`.
fn sign_in(dir: &std::path::Path, tag: &str) {
    let now = agentmux_common::time::now_ms();
    let claude = serde_json::json!({ "claudeAiOauth": { "accessToken": format!("{tag}-claude-access"), "refreshToken": format!("{tag}-claude-refresh"), "expiresAt": now + 3_600_000 } });
    std::fs::write(dir.join(".credentials.json"), claude.to_string()).unwrap();
    std::fs::write(dir.join("auth.json"), serde_json::json!({ "tokens": { "refresh_token": format!("{tag}-codex-refresh") } }).to_string()).unwrap();
    for provider in ["claude", "codex"] {
        probe_oauth_status(provider, dir.to_str().unwrap(), now);
    }
}

fn values(tag: &str) -> [String; 3] {
    ["claude-access", "claude-refresh", "codex-refresh"].map(|what| format!("{tag}-{what}"))
}

fn all_hidden(tag: &str) -> bool {
    values(tag).iter().all(|v| withhold::text(v) == withhold::MARK)
}

fn none_hidden(tag: &str) -> bool {
    values(tag).iter().all(|v| withhold::text(v) == *v)
}

fn revoked() -> serde_json::Value {
    serde_json::json!({ "outcome": "revoked" })
}

fn removed() -> serde_json::Value {
    serde_json::json!({ "outcome": "removed" })
}

#[test]
fn a_grant_revoked_and_a_directory_removed_release_the_sign_in_values_and_not_another_accounts() {
    let (gone, kept) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    sign_in(gone.path(), "canary-revoked-G1");
    sign_in(kept.path(), "canary-revoked-K1");
    assert!(all_hidden("canary-revoked-G1"), "withheld after the read, on both read paths");
    release_sign_in_values(Some(&account("acct-gone", gone.path())), &revoked(), &removed());
    assert!(none_hidden("canary-revoked-G1"), "released with the revoked grant and the removed directory");
    assert!(all_hidden("canary-revoked-K1"), "another account's stays");
}

#[test]
fn a_grant_not_revoked_or_a_directory_kept_leaves_the_sign_in_values_withheld() {
    let dir = tempfile::tempdir().unwrap();
    sign_in(dir.path(), "canary-unrevoked-U1");
    let account = account("acct-unrevoked", dir.path());
    let failed = serde_json::json!({ "outcome": "failed", "detail": "x" });
    let skipped = serde_json::json!({ "outcome": "skipped", "detail": "outside the identities root" });
    let not_revoked = [(serde_json::Value::Null, removed()), (failed.clone(), removed())];
    let not_removed = [(revoked(), skipped), (revoked(), failed), (revoked(), serde_json::json!({ "outcome": "absent" }))];
    for (revoke, cleanup) in not_revoked.into_iter().chain(not_removed) {
        release_sign_in_values(Some(&account), &revoke, &cleanup);
        assert!(all_hidden("canary-unrevoked-U1"), "{revoke} {cleanup}");
    }
}

#[tokio::test]
async fn deleting_an_account_whose_grant_was_not_revoked_leaves_its_sign_in_values_withheld() {
    let (store, identity_store) = (Arc::new(Store::open_in_memory().unwrap()), Arc::new(Store::open_in_memory().unwrap()));
    let broker = Arc::new(Broker::new());
    let dir = tempfile::tempdir().unwrap();
    store.identity_upsert(&account("acct-gone", dir.path())).unwrap();
    sign_in(dir.path(), "canary-delete-D1");
    let done = delete_identity_account(&store, &identity_store, &broker, "acct-gone", false).await.unwrap();
    assert!(done.deleted);
    assert!(all_hidden("canary-delete-D1"), "an account with no revoke at its provider keeps its values withheld");
}
