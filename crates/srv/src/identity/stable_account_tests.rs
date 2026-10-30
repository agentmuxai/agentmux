// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use serde_json::json;

use super::*;

fn signed_in_dir(account: &str, org: &str, email: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let claude_json = json!({ "oauthAccount": { "accountUuid": account, "organizationUuid": org, "emailAddress": email } });
    std::fs::write(dir.path().join(".claude.json"), claude_json.to_string()).unwrap();
    dir
}

fn account(id: &str, provider: &str, dir: &std::path::Path, email: &str) -> IdentityAccount {
    IdentityAccount {
        id: id.to_string(),
        name: id.to_string(),
        provider: provider.to_string(),
        kind: "oauth".to_string(),
        display_name: String::new(),
        secret_ref: SecretRef::OAuthConfigDir { dir: dir.display().to_string() },
        context: json!({ "email": email }),
        status: "valid".to_string(),
        created_at: 0,
        updated_at: 0,
    }
}

#[test]
fn one_provider_account_has_one_id_whatever_its_local_id_and_email_case() {
    let (a, b) = (signed_in_dir("acc-1", "org-1", "Me@Example.com"), signed_in_dir("acc-1", "org-1", "me@example.com"));
    let on_this_machine = stable_account_id(&account("local-uuid-1", "claude", a.path(), "Me@Example.com")).unwrap();
    let on_another = stable_account_id(&account("other-uuid-9", "claude-code", b.path(), "me@example.com")).unwrap();
    assert_eq!(on_this_machine, on_another);
    assert!(on_this_machine.starts_with("claude:"), "{on_this_machine}");
}

#[test]
fn one_email_in_two_organizations_is_two_accounts() {
    let (personal, team) = (signed_in_dir("acc-1", "org-personal", "me@example.com"), signed_in_dir("acc-1", "org-team", "me@example.com"));
    let id = |dir: &tempfile::TempDir| stable_account_id(&account("x", "claude", dir.path(), "me@example.com")).unwrap();
    assert_ne!(id(&personal), id(&team));
}

#[test]
fn the_id_holds_neither_the_provider_ids_nor_the_email() {
    let dir = signed_in_dir("acc-secret-uuid", "org-secret-uuid", "me@example.com");
    let id = stable_account_id(&account("x", "claude", dir.path(), "me@example.com")).unwrap();
    assert!(!id.contains("acc-secret") && !id.contains("org-secret") && !id.contains("example"), "{id}");
}

#[test]
fn without_provider_ids_the_email_names_the_account() {
    let empty = tempfile::tempdir().unwrap();
    let id = |email: &str| stable_account_id(&account("x", "codex", empty.path(), email));
    assert_eq!(id("A@b.com"), id(" a@B.com "));
    assert_ne!(id("a@b.com"), id("c@b.com"));
    assert!(id("a@b.com").unwrap().starts_with("codex:e"));
}

#[test]
fn an_account_with_no_provider_ids_and_no_email_has_no_stable_id() {
    let empty = tempfile::tempdir().unwrap();
    let mut a = account("x", "anthropic", empty.path(), "");
    a.context = json!({});
    assert_eq!(stable_account_id(&a), None);
}
