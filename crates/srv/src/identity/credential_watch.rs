// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A Claude sign-in that is lost while agents run on it is noticed when it happens, not at the
//! next spawn.
//!
//! The Claude CLI keeps its token in memory, so an account whose credentials file is emptied (two
//! CLIs raced a refresh, a sign-out in another window) keeps working until its agents restart.
//! The one probe of the file runs at spawn (`oauth_probe`), so for hours nothing said the sign-in
//! was gone, and the first sign of it was every agent failing at the next upgrade.
//!
//! [`check`] runs on the account tick. It probes each Claude account's config dir. A CLI that is
//! rewriting the file can leave it unreadable for a moment, so a signed-out reading counts only
//! when the next tick says the same; then it logs once, marks the account `needs_reauth`
//! (Connectors shows Reconnect) and tells the account list to reload. An account it marked that
//! reads signed in again is put back. It never reads a token value, never writes the credentials
//! file, and does not start a sign-in.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::backend::mps::Broker;
use crate::backend::providers::resolve_provider_alias;
use crate::backend::storage::store::{IdentityAccount, SecretRef, Store};
use crate::identity::resolver::{oauth_status, probe_oauth_status, OAuthProbeStatus};

/// What the ticks have made of an account's sign-in so far.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Seen {
    /// Signed in on the last look.
    In,
    /// Signed out once after being signed in: one more tick decides.
    Wobble,
    /// Signed out twice running, after being signed in: it was reported.
    Out,
}

/// What a tick does with an account.
#[derive(PartialEq, Eq, Debug)]
enum Step {
    Nothing,
    /// The sign-in is lost: say so and mark the account.
    Lost,
    /// A reported account is signed in again: put its status back.
    Back,
}

/// The next [`Seen`] and what to do, from the last one and this reading. A first look, or an
/// account that was never signed in, is not news.
fn step(before: Option<Seen>, signed_in: bool) -> (Option<Seen>, Step) {
    match (before, signed_in) {
        (Some(Seen::Out), true) => (Some(Seen::In), Step::Back),
        (_, true) => (Some(Seen::In), Step::Nothing),
        (Some(Seen::In), false) => (Some(Seen::Wobble), Step::Nothing),
        (Some(Seen::Wobble), false) => (Some(Seen::Out), Step::Lost),
        (Some(Seen::Out), false) => (Some(Seen::Out), Step::Nothing),
        (None, false) => (None, Step::Nothing),
    }
}

/// What each account's sign-in has been on the last looks, by account id.
fn last_seen() -> &'static Mutex<HashMap<String, Seen>> {
    static SEEN: OnceLock<Mutex<HashMap<String, Seen>>> = OnceLock::new();
    SEEN.get_or_init(Default::default)
}

/// The account's probe: `Valid` or `Expired` (a renewable token) is signed in, `NeedsReauth` is
/// not. `None` when the file cannot tell (a platform that keeps it in a keychain, or not a Claude
/// account).
fn probe(account: &IdentityAccount, now_ms: i64) -> Option<OAuthProbeStatus> {
    if resolve_provider_alias(&account.provider) != "claude" {
        return None;
    }
    let SecretRef::OAuthConfigDir { dir } = &account.secret_ref else { return None };
    probe_oauth_status("claude", dir, now_ms)
}

fn set_status(id_store: &Store, identity_store: &Store, broker: Option<&Broker>, account_id: &str, status: &str) {
    for store in [id_store, identity_store] {
        if let Err(e) = store.identity_set_status(account_id, status) {
            tracing::warn!(target: "identity", account_id = %account_id, error = %e, "credential watch: status write failed");
        }
    }
    if let Some(b) = broker {
        b.publish_event::<crate::backend::mps_events::IdentityAccountsChanged>(&(), vec![], 0);
    }
}

/// Look at every Claude account once and act on the ones whose sign-in was lost or came back.
/// BLOCKING (file and store reads). Returns the ids it marked, for tests.
pub fn check(id_store: &Store, identity_store: &Store, broker: Option<&Broker>) -> Vec<String> {
    let now_ms = agentmux_common::time::now_ms();
    let accounts = id_store.identity_list(None).unwrap_or_default();
    let mut marked = Vec::new();
    let mut seen = last_seen().lock().unwrap_or_else(|e| e.into_inner());
    seen.retain(|id, _| accounts.iter().any(|a| &a.id == id));
    for account in &accounts {
        let Some(status) = probe(account, now_ms) else { continue };
        let signed_in = status != OAuthProbeStatus::NeedsReauth;
        let (next, what) = step(seen.get(&account.id).copied(), signed_in);
        match next {
            Some(n) => seen.insert(account.id.clone(), n),
            None => seen.remove(&account.id),
        };
        match what {
            Step::Nothing => {}
            Step::Lost => {
                tracing::warn!(
                    target: "identity",
                    account_id = %account.id,
                    old_status = %account.status,
                    "credentials emptied: this Claude account had a sign-in and its credentials file has had no token on two looks in a row; agents on it work until they restart, then need a sign-in"
                );
                if account.status != oauth_status::NEEDS_REAUTH {
                    set_status(id_store, identity_store, broker, &account.id, oauth_status::NEEDS_REAUTH);
                }
                marked.push(account.id.clone());
            }
            Step::Back => {
                if account.status == oauth_status::NEEDS_REAUTH {
                    tracing::info!(target: "identity", account_id = %account.id, "credentials are back: this Claude account reads signed in again");
                    set_status(id_store, identity_store, broker, &account.id, status.as_str());
                }
            }
        }
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
    fn a_sign_in_is_lost_only_after_two_signed_out_looks_in_a_row() {
        assert_eq!(step(None, false), (None, Step::Nothing), "a first look at a signed-out account is not news");
        assert_eq!(step(Some(Seen::In), false), (Some(Seen::Wobble), Step::Nothing), "one bad read can be a CLI mid-write");
        assert_eq!(step(Some(Seen::Wobble), false), (Some(Seen::Out), Step::Lost));
        assert_eq!(step(Some(Seen::Wobble), true), (Some(Seen::In), Step::Nothing), "it came back: nothing to say");
        assert_eq!(step(Some(Seen::Out), false), (Some(Seen::Out), Step::Nothing), "said once, not at every tick");
        assert_eq!(step(Some(Seen::Out), true), (Some(Seen::In), Step::Back));
        assert_eq!(step(Some(Seen::In), true), (Some(Seen::In), Step::Nothing));
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
        assert!(check(&store, &other, None).is_empty(), "the first signed-out look waits for a second");
        assert_eq!(store.identity_list(None).unwrap()[0].status, "ok");
        assert_eq!(check(&store, &other, None), vec!["watch-acct-1".to_string()]);
        assert_eq!(store.identity_list(None).unwrap()[0].status, oauth_status::NEEDS_REAUTH);
        assert_eq!(other.identity_list(None).unwrap()[0].status, oauth_status::NEEDS_REAUTH);

        assert!(check(&store, &other, None).is_empty(), "said once, not at every tick");

        // The sign-in comes back (the user signed in again): the status is put back.
        write(tmp.path(), r#"{"claudeAiOauth":{"accessToken":"a","refreshToken":"r","expiresAt":4102444800000}}"#);
        assert!(check(&store, &other, None).is_empty());
        assert_eq!(store.identity_list(None).unwrap()[0].status, oauth_status::VALID);
    }

    #[test]
    fn one_unreadable_look_does_not_mark_a_working_account() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_str().unwrap().to_string();
        let good = r#"{"claudeAiOauth":{"accessToken":"a","refreshToken":"r","expiresAt":4102444800000}}"#;
        write(tmp.path(), good);
        let store = Store::open_in_memory().unwrap();
        store.identity_upsert(&account("watch-acct-3", &dir)).unwrap();
        assert!(check(&store, &store, None).is_empty());

        write(tmp.path(), "{ half written");
        assert!(check(&store, &store, None).is_empty());
        write(tmp.path(), good);
        assert!(check(&store, &store, None).is_empty());
        write(tmp.path(), "{ half written");
        assert!(check(&store, &store, None).is_empty(), "two bad reads that are not in a row are not a wipe");
        assert_eq!(store.identity_list(None).unwrap()[0].status, "ok");
    }

    #[test]
    fn an_account_that_was_never_signed_in_is_not_reported() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), r#"{}"#);
        let store = Store::open_in_memory().unwrap();
        let acct = account("watch-acct-2", tmp.path().to_str().unwrap());
        store.identity_upsert(&acct).unwrap();
        for _ in 0..3 {
            assert!(check(&store, &store, None).is_empty());
        }
    }
}
