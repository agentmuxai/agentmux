// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The sign-in of each account as its credentials file says now, not as the store last recorded it.
//!
//! The store's status is written at a spawn, and by the credential watch only when a file is
//! emptied. An access token that lapsed since, or a file another process rewrote, left it at
//! `valid` for hours: the account list showed a sign-in that a restart would find dead, and a switch
//! moved an agent onto it. Every read that decides where an agent may go reads the file again.

use crate::backend::providers::resolve_provider_alias;
use crate::backend::storage::identities::{IdentityAccount, SecretRef};
use crate::identity::resolver::probe_oauth_status;
use crate::server::AppState;

/// `account`'s status read from its credentials file. Only a signed-in (`valid`, `ok`) or `expired`
/// status is re-read: `invalid` is a refusal the provider gave, `needs_reauth` is the credential
/// watch's, and `unknown` is not checked yet. An account the file cannot speak for (no config dir, a
/// Keychain on macOS) keeps the stored status.
fn status_now(account: &IdentityAccount, now_ms: i64) -> String {
    let SecretRef::OAuthConfigDir { dir } = &account.secret_ref else { return account.status.clone() };
    if !matches!(account.status.as_str(), "valid" | "ok" | "expired") {
        return account.status.clone();
    }
    match probe_oauth_status(resolve_provider_alias(&account.provider), dir, now_ms) {
        // `ok` and `valid` both mean signed in.
        Some(p) if p.as_str() == "valid" && account.status == "ok" => account.status.clone(),
        Some(p) => p.as_str().to_string(),
        None => account.status.clone(),
    }
}

/// `accounts` with each status read from its file now.
pub(super) fn refreshed(mut accounts: Vec<IdentityAccount>, now_ms: i64) -> Vec<IdentityAccount> {
    for a in &mut accounts {
        a.status = status_now(a, now_ms);
    }
    accounts
}

/// Every account on this machine, statuses read now. BLOCKING (store and file reads).
pub(super) fn identity_list(state: &AppState) -> Result<Vec<IdentityAccount>, String> {
    let all = state.id_store.identity_list(None).map_err(|e| e.to_string())?;
    Ok(refreshed(all, agentmux_common::time::now_ms()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 5_000_000;

    fn account(dir: &std::path::Path, status: &str) -> IdentityAccount {
        IdentityAccount {
            id: "fresh-1".into(),
            name: "fresh-1".into(),
            provider: "claude".into(),
            kind: "oauth".into(),
            display_name: String::new(),
            secret_ref: SecretRef::OAuthConfigDir { dir: dir.to_str().unwrap().into() },
            context: serde_json::json!({}),
            status: status.into(),
            created_at: 0,
            updated_at: 0,
        }
    }

    fn read(file: &str, status: &str) -> String {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(".credentials.json"), file).unwrap();
        status_now(&account(tmp.path(), status), NOW)
    }

    #[test]
    fn a_lapsed_access_token_is_not_reported_valid() {
        let lapsed = format!(r#"{{"claudeAiOauth":{{"accessToken":"a","refreshToken":"r","expiresAt":{}}}}}"#, NOW - 1);
        assert_eq!(read(&lapsed, "valid"), "expired");
    }

    #[test]
    fn an_emptied_file_is_signed_out_whatever_the_store_says() {
        assert_eq!(read(r#"{"claudeAiOauth":{}}"#, "valid"), "needs_reauth");
    }

    #[test]
    fn a_live_token_keeps_ok_and_turns_a_stale_expired_back_to_valid() {
        let live = format!(r#"{{"claudeAiOauth":{{"accessToken":"a","refreshToken":"r","expiresAt":{}}}}}"#, NOW + 3_600_000);
        assert_eq!(read(&live, "ok"), "ok");
        assert_eq!(read(&live, "expired"), "valid");
    }

    #[test]
    fn a_refusal_and_an_unchecked_account_stay_as_stored() {
        let live = format!(r#"{{"claudeAiOauth":{{"accessToken":"a","refreshToken":"r","expiresAt":{}}}}}"#, NOW + 3_600_000);
        for stored in ["invalid", "needs_reauth", "unknown"] {
            assert_eq!(read(&live, stored), stored);
        }
    }

    #[test]
    fn an_account_with_no_config_dir_keeps_its_status() {
        let mut a = account(std::path::Path::new("x"), "valid");
        a.secret_ref = SecretRef::Keychain { service: "s".into(), account: "a".into() };
        assert_eq!(status_now(&a, NOW), "valid");
    }
}
