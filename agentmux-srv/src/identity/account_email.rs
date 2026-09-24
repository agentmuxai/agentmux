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
/// one today; other providers return `None` and keep showing their name.
pub fn email_from_oauth_dir(provider: &str, dir: &str) -> Option<String> {
    if provider != "claude" || dir.is_empty() {
        return None;
    }
    let raw = std::fs::read_to_string(std::path::Path::new(dir).join(".claude.json")).ok()?;
    let json: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let email = json.get("oauthAccount")?.get("emailAddress")?.as_str()?.trim();
    (!email.is_empty()).then(|| email.to_string())
}

/// Set `account.context.email` from its config dir when that differs from
/// what is stored. Returns whether the account changed. A different email
/// replaces the stored one — an account re-authenticated as another user
/// must not keep showing the old address (§4). An account whose dir reports
/// no email is left as it is.
pub fn refresh_account_email(account: &mut IdentityAccount) -> bool {
    if account.kind != "oauth" {
        return false;
    }
    let SecretRef::OAuthConfigDir { dir } = &account.secret_ref else {
        return false;
    };
    let Some(email) = email_from_oauth_dir(&account.provider, dir) else {
        return false;
    };
    if account.context.get("email").and_then(|v| v.as_str()) == Some(email.as_str()) {
        return false;
    }
    if !account.context.is_object() {
        account.context = serde_json::json!({});
    }
    account.context["email"] = serde_json::Value::String(email);
    true
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
        assert!(refresh_account_email(&mut blank), "backfilled");
        assert_eq!(blank.context["email"], "new@example.com");
        assert!(!refresh_account_email(&mut blank), "unchanged the second time");

        let mut stale = account(d, serde_json::json!({"email": "old@example.com", "k": 1}));
        assert!(refresh_account_email(&mut stale), "a re-login as someone else");
        assert_eq!(stale.context["email"], "new@example.com");
        assert_eq!(stale.context["k"], 1, "other context kept");

        let mut unknown = account("/no/such/dir", serde_json::json!({"email": "kept@example.com"}));
        assert!(!refresh_account_email(&mut unknown), "no email on disk: left alone");
        assert_eq!(unknown.context["email"], "kept@example.com");
    }
}
