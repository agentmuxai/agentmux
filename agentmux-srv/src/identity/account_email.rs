// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The login email of an OAuth provider account, read from the account's own
//! config dir — `SPEC_ACCOUNT_EMAIL_IN_ARMORY_2026_09_23.md` §4.
//!
//! #3541 recorded the email only when the login transcript printed one, and
//! Claude's login does not, so every Claude account showed its generic name
//! (`claude-oauth`). The CLI does record who is signed in: Claude writes
//! `oauthAccount.emailAddress` to `.claude.json` inside the config dir the
//! account's `SecretRef::OAuthConfigDir` points at. Reading it there covers
//! accounts that authenticated before #3541 (the backfill) and follows a
//! re-login as a different user.

use crate::backend::storage::identities::{IdentityAccount, SecretRef};

/// The email the provider CLI recorded in `dir`, if any. Only Claude writes
/// one today (`claude`, or its `claude-code` alias); other providers return
/// `None` and keep showing their name. The ambient `~/.claude` is skipped: a
/// legacy account bound there shares credentials with a terminal `claude`,
/// whose own config (`~/.claude.json`) is outside it, so the `.claude.json`
/// inside may name a user no longer signed in.
pub fn email_from_oauth_dir(provider: &str, dir: &str) -> Option<String> {
    if !matches!(provider, "claude" | "claude-code") || dir.is_empty() {
        return None;
    }
    let claude = crate::backend::providers::get_provider("claude")?;
    if crate::backend::providers::is_provider_ambient_home_dir(claude, dir) {
        return None;
    }
    let raw = std::fs::read_to_string(std::path::Path::new(dir).join(".claude.json")).ok()?;
    let json: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let email = json.get("oauthAccount")?.get("emailAddress")?.as_str()?.trim();
    (!email.is_empty()).then(|| email.to_string())
}

/// Set `account.context.email` from its config dir when that differs from
/// what is stored, and return the new email (the caller persists only that
/// field, `Store::identity_set_context_email`). A different email replaces
/// the stored one — an account re-authenticated as another user must not
/// keep showing the old address (§4). An account whose dir reports no email
/// is left as it is.
pub fn refresh_account_email(account: &mut IdentityAccount) -> Option<String> {
    if account.kind != "oauth" {
        return None;
    }
    let SecretRef::OAuthConfigDir { dir } = &account.secret_ref else {
        return None;
    };
    let email = email_from_oauth_dir(&account.provider, dir)?;
    if account.context.get("email").and_then(|v| v.as_str()) == Some(email.as_str()) {
        return None;
    }
    if !account.context.is_object() {
        account.context = serde_json::json!({});
    }
    account.context["email"] = serde_json::Value::String(email.clone());
    Some(email)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(dir: &str, context: serde_json::Value) -> IdentityAccount {
        IdentityAccount {
            id: "acc-1".into(),
            name: "claude-oauth".into(),
            provider: "claude".into(),
            kind: "oauth".into(),
            display_name: String::new(),
            secret_ref: SecretRef::OAuthConfigDir { dir: dir.into() },
            context,
            status: "valid".into(),
            created_at: 0,
            updated_at: 0,
        }
    }

    fn dir_with(claude_json: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".claude.json"), claude_json).unwrap();
        dir
    }

    #[test]
    fn reads_claudes_recorded_login_email() {
        let dir = dir_with(r#"{"oauthAccount":{"emailAddress":" me@example.com "}}"#);
        let d = dir.path().to_str().unwrap();
        assert_eq!(email_from_oauth_dir("claude", d).as_deref(), Some("me@example.com"));
        assert_eq!(email_from_oauth_dir("claude-code", d).as_deref(), Some("me@example.com"));
        assert_eq!(email_from_oauth_dir("codex", d), None, "only Claude records one here");
        let empty = dir_with(r#"{"oauthAccount":{"emailAddress":""}}"#);
        assert_eq!(email_from_oauth_dir("claude", empty.path().to_str().unwrap()), None);
        assert_eq!(email_from_oauth_dir("claude", "/no/such/dir"), None);
    }

    #[test]
    fn refresh_backfills_updates_and_leaves_alone() {
        let dir = dir_with(r#"{"oauthAccount":{"emailAddress":"new@example.com"}}"#);
        let d = dir.path().to_str().unwrap();

        let mut blank = account(d, serde_json::json!({}));
        assert_eq!(refresh_account_email(&mut blank).as_deref(), Some("new@example.com"), "backfilled");
        assert_eq!(blank.context["email"], "new@example.com");
        assert_eq!(refresh_account_email(&mut blank), None, "unchanged the second time");

        let mut stale = account(d, serde_json::json!({"email": "old@example.com", "k": 1}));
        assert!(refresh_account_email(&mut stale).is_some(), "a re-login as someone else");
        assert_eq!(stale.context["email"], "new@example.com");
        assert_eq!(stale.context["k"], 1, "other context kept");

        let mut unknown = account("/no/such/dir", serde_json::json!({"email": "kept@example.com"}));
        assert_eq!(refresh_account_email(&mut unknown), None, "no email on disk: left alone");
        assert_eq!(unknown.context["email"], "kept@example.com");
    }

    /// The backfill's write touches `context.email` only: other context keys,
    /// `status` and `updated_at` stand, and an account that is gone is not
    /// re-created.
    #[test]
    fn the_store_write_sets_the_email_and_nothing_else() {
        let store = crate::backend::storage::store::Store::open_in_memory().unwrap();
        let mut acct = account("/dir", serde_json::json!({"k": 1}));
        acct.status = "needs_reauth".into();
        acct.updated_at = 42;
        store.identity_upsert(&acct).unwrap();

        assert!(store.identity_set_context_email("acc-1", "me@example.com").unwrap());
        let got = store.identity_get("acc-1").unwrap().unwrap();
        assert_eq!(got.context["email"], "me@example.com");
        assert_eq!(got.context["k"], 1);
        assert_eq!(got.status, "needs_reauth");
        assert_eq!(got.updated_at, 42);

        assert!(!store.identity_set_context_email("acc-gone", "x@example.com").unwrap());
        assert!(store.identity_get("acc-gone").unwrap().is_none(), "never re-created");
    }
}
