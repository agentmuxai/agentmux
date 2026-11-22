// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use super::delete_identity_account;
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

fn sign_in(dir: &std::path::Path, access: &str, refresh: &str) {
    let expires = agentmux_common::time::now_ms() + 3_600_000;
    let body = serde_json::json!({ "claudeAiOauth": { "accessToken": access, "refreshToken": refresh, "expiresAt": expires } });
    std::fs::write(dir.join(".credentials.json"), body.to_string()).unwrap();
    probe_oauth_status("claude", dir.to_str().unwrap(), agentmux_common::time::now_ms());
}

#[tokio::test]
async fn deleting_an_account_releases_its_sign_in_values_and_not_another_accounts() {
    let (store, identity_store) = (Arc::new(Store::open_in_memory().unwrap()), Arc::new(Store::open_in_memory().unwrap()));
    let broker = Arc::new(Broker::new());
    let (gone, kept) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    store.identity_upsert(&account("acct-gone", gone.path())).unwrap();
    store.identity_upsert(&account("acct-kept", kept.path())).unwrap();
    sign_in(gone.path(), "canary-delete-access-G1", "canary-delete-refresh-G1");
    sign_in(kept.path(), "canary-delete-access-K1", "canary-delete-refresh-K1");
    assert_eq!(withhold::text("canary-delete-refresh-G1"), withhold::MARK, "withheld after the read");

    let done = delete_identity_account(&store, &identity_store, &broker, "acct-gone", false).await.unwrap();
    assert!(done.deleted);
    for value in ["canary-delete-access-G1", "canary-delete-refresh-G1"] {
        assert_eq!(withhold::text(value), value, "released with its account");
    }
    for value in ["canary-delete-access-K1", "canary-delete-refresh-K1"] {
        assert_eq!(withhold::text(value), withhold::MARK, "another account's stays");
    }
}
