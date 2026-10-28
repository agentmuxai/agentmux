// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A Claude sign-in that is lost while agents run on it is noticed when it happens, not at the next
//! spawn.
//!
//! The Claude CLI keeps its token in memory, so an account whose credentials file is emptied (two
//! CLIs raced a refresh, a sign-out in another window) keeps working until its agents restart.
//! The one probe of the file runs at spawn (`oauth_probe`), so for hours nothing said the sign-in
//! was gone, and the first sign of it was every agent failing at the next upgrade.
//!
//! [`check`] runs on the account tick. It probes each Claude account's config dir, and when an
//! account that had a sign-in has none now it logs it once, marks the account `needs_reauth`
//! (Connectors shows Reconnect) and tells the account list to reload. It never reads a token
//! value, never writes the credentials file, and does not start a sign-in.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::backend::mps::Broker;
use crate::backend::providers::resolve_provider_alias;
use crate::backend::storage::store::{IdentityAccount, SecretRef, Store};
use crate::identity::resolver::{oauth_status, probe_oauth_status, OAuthProbeStatus};

/// Whether an account had a sign-in the last time it was looked at, by account id.
fn last_seen() -> &'static Mutex<HashMap<String, bool>> {
    static SEEN: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();
    SEEN.get_or_init(Default::default)
}

/// `Some(true)` signed in (a valid or renewable token), `Some(false)` signed out, `None` when the
/// file cannot tell (a platform that keeps it in a keychain, or not a Claude account).
fn signed_in(account: &IdentityAccount, now_ms: i64) -> Option<bool> {
    if resolve_provider_alias(&account.provider) != "claude" {
        return None;
    }
    let SecretRef::OAuthConfigDir { dir } = &account.secret_ref else { return None };
    match probe_oauth_status("claude", dir, now_ms)? {
        OAuthProbeStatus::Valid | OAuthProbeStatus::Expired => Some(true),
        OAuthProbeStatus::NeedsReauth => Some(false),
    }
}

/// Whether `now` is a sign-in that was lost: it had one, and has none. A first look, or an account
/// that never had one, is not news.
fn lost(before: Option<bool>, now: bool) -> bool {
    before == Some(true) && !now
}

/// Look at every Claude account once and act on the ones whose sign-in was lost. BLOCKING (file
/// and store reads). Returns the ids it marked, for tests.
pub fn check(id_store: &Store, identity_store: &Store, broker: Option<&Broker>) -> Vec<String> {
    let now_ms = agentmux_common::time::now_ms();
    let accounts = id_store.identity_list(None).unwrap_or_default();
    let mut marked = Vec::new();
    let mut seen = last_seen().lock().unwrap_or_else(|e| e.into_inner());
    seen.retain(|id, _| accounts.iter().any(|a| &a.id == id));
    for account in &accounts {
        let Some(now) = signed_in(account, now_ms) else { continue };
        let before = seen.insert(account.id.clone(), now);
        if !lost(before, now) {
            continue;
        }
        tracing::warn!(
            target: "identity",
            account_id = %account.id,
            old_status = %account.status,
            "credentials emptied: this Claude account had a sign-in and its credentials file now has no token; agents on it work until they restart, then need a sign-in"
        );
        if account.status != oauth_status::NEEDS_REAUTH {
            for store in [id_store, identity_store] {
                if let Err(e) = store.identity_set_status(&account.id, oauth_status::NEEDS_REAUTH) {
                    tracing::warn!(target: "identity", account_id = %account.id, error = %e, "credentials emptied: status write failed");
                }
            }
            if let Some(b) = broker {
                b.publish_event::<crate::backend::mps_events::IdentityAccountsChanged>(&(), vec![], 0);
            }
        }
        marked.push(account.id.clone());
    }
    marked
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(id: &str, dir: &str) -> IdentityAccount {
        IdentityAccount {
            id: id.into(),
            name: id.into(),
            provider: "claude".into(),
            kind: "oauth".into(),
            display_name: String::new(),
            secret_ref: SecretRef::OAuthConfigDir { dir: dir.into() },
            context: serde_json::json!({}),
            status: "ok".into(),
            created_at: 0,
            updated_at: 0,
        }
    }

    fn write(dir: &std::path::Path, body: &str) {
        std::fs::write(dir.join(".credentials.json"), body).unwrap();
    }

    #[test]
    fn only_a_sign_in_that_was_there_counts_as_lost() {
        assert!(lost(Some(true), false));
        assert!(!lost(None, false), "a first look at a signed-out account is not news");
        assert!(!lost(Some(false), false));
        assert!(!lost(Some(true), true));
    }

    #[test]
    fn an_emptied_credentials_file_marks_the_account_once() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_str().unwrap().to_string();
        write(tmp.path(), r#"{"claudeAiOauth":{"accessToken":"a","refreshToken":"r","expiresAt":4102444800000}}"#);
        let store = Store::open_in_memory().unwrap();
        let other = Store::open_in_memory().unwrap();
        let acct = account("watch-acct-1", &dir);
        store.identity_upsert(&acct).unwrap();
        other.identity_upsert(&acct).unwrap();

        assert!(check(&store, &other, None).is_empty(), "signed in: nothing to say");

        write(tmp.path(), r#"{"claudeAiOauth":{}}"#);
        assert_eq!(check(&store, &other, None), vec!["watch-acct-1".to_string()]);
        assert_eq!(store.identity_list(None).unwrap()[0].status, oauth_status::NEEDS_REAUTH);
        assert_eq!(other.identity_list(None).unwrap()[0].status, oauth_status::NEEDS_REAUTH);

        assert!(check(&store, &other, None).is_empty(), "said once, not at every tick");
    }

    #[test]
    fn an_account_that_was_never_signed_in_is_not_reported() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), r#"{}"#);
        let store = Store::open_in_memory().unwrap();
        let acct = account("watch-acct-2", tmp.path().to_str().unwrap());
        store.identity_upsert(&acct).unwrap();
        assert!(check(&store, &store, None).is_empty());
        assert!(check(&store, &store, None).is_empty());
    }
}
