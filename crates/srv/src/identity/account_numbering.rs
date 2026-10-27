// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//! Numbers for accounts that share a provider and an email
//! (`docs/specs/SPEC_ACCOUNT_SWITCH_FROM_THE_COMPOSER_NO_BIND_2026_10_26.md` §3.3).
//!
//! Two Claude sign-ins of one address both show `user@example.com`. Each gets a
//! number, kept in `context.seq` and never reused, so `·2` stays `·2` when
//! `·1` is deleted and across restarts. The number shows only while two or more
//! accounts share the key; a lone account shows its bare name.

use std::collections::HashMap;

use crate::backend::storage::identities::IdentityAccount;

/// The provider and (case-folded) email two accounts must share to be numbered
/// together. `None` without an email: nothing to tell apart.
fn group_key(a: &IdentityAccount) -> Option<(String, String)> {
    let email = a.context.get("email")?.as_str()?.trim().to_lowercase();
    if email.is_empty() {
        return None;
    }
    let provider = match crate::backend::providers::resolve_provider_alias(&a.provider) {
        "" => a.provider.to_lowercase(),
        c => c.to_string(),
    };
    Some((provider, email))
}

fn seq_of(a: &IdentityAccount) -> Option<u32> {
    a.context.get("seq")?.as_u64().and_then(|n| u32::try_from(n).ok()).filter(|n| *n > 0)
}

/// The numbers to write: each account with an email and no number gets the
/// group's highest number plus one, oldest first (creation time, then id), so
/// an existing group is numbered in the order its accounts were made.
pub fn missing_seqs(accounts: &[IdentityAccount]) -> Vec<(String, u32)> {
    let mut groups: HashMap<(String, String), Vec<&IdentityAccount>> = HashMap::new();
    for a in accounts {
        if let Some(k) = group_key(a) {
            groups.entry(k).or_default().push(a);
        }
    }
    let mut out = Vec::new();
    for members in groups.values_mut() {
        let mut next = members.iter().filter_map(|a| seq_of(a)).max().unwrap_or(0) + 1;
        members.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
        for a in members.iter().filter(|a| seq_of(a).is_none()) {
            out.push((a.id.clone(), next));
            next += 1;
        }
    }
    out
}

/// `a`'s name for a person: `label` with ` ·N` after it while another account
/// shares its provider and email. `all` is every account that could be shown
/// beside it.
pub fn numbered_label(a: &IdentityAccount, label: &str, all: &[IdentityAccount]) -> String {
    let (Some(_), Some(n)) = (group_key(a), seq_of(a)) else {
        return label.to_string();
    };
    if group_size(a, all) < 2 {
        return label.to_string();
    }
    format!("{label} \u{b7}{n}")
}

/// How many of `all` share `a`'s provider and email, `a` included; 1 without an email.
fn group_size(a: &IdentityAccount, all: &[IdentityAccount]) -> usize {
    match group_key(a) {
        Some(key) => all.iter().filter(|o| group_key(o).as_ref() == Some(&key)).count(),
        None => 1,
    }
}

/// Whether two emails are the same group's: equal ignoring case and padding.
pub fn same_email(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

/// An account is written again with a `context` that doesn't carry what the
/// stored one had: a re-login that rebuilds it as `{ "email": .. }`, or a client
/// holding a copy from before numbering. Keep the stored number when the new
/// context has none and names the same address (or none: the email backfill
/// settles that on the next list, and drops the number if it really changed).
pub fn carry_over(stored: &serde_json::Value, incoming: &mut serde_json::Value) {
    let Some(seq) = stored.get("seq").filter(|n| n.as_u64().is_some_and(|n| n > 0)) else {
        return;
    };
    if incoming.get("seq").is_some_and(|n| n.as_u64().is_some_and(|n| n > 0)) {
        return;
    }
    if !incoming.is_object() {
        *incoming = serde_json::json!({});
    }
    let stored_email = stored.get("email").and_then(|e| e.as_str()).unwrap_or("");
    match incoming.get("email").and_then(|e| e.as_str()).filter(|e| !e.trim().is_empty()) {
        Some(email) if !same_email(email, stored_email) => return,
        Some(_) => {}
        None => incoming["email"] = serde_json::json!(stored_email),
    }
    incoming["seq"] = seq.clone();
}

/// Whether `wanted` (an id, name, display name, email, or `email ·N`) names `a`.
/// A bare email that more than one account in `all` carries names none of them:
/// the caller reports the candidates instead of taking the first.
pub fn names(a: &IdentityAccount, wanted: &str, all: &[IdentityAccount]) -> bool {
    let wanted = wanted.trim();
    let email = a.context.get("email").and_then(|e| e.as_str()).unwrap_or("");
    let numbered = numbered_label(a, email, all);
    let is_numbered = !email.is_empty() && numbered != email;
    if is_numbered && numbered.eq_ignore_ascii_case(&normalize_dot(wanted)) {
        return true;
    }
    // By the group, not by the number: an account not listed yet has none.
    let bare_email_is_ambiguous = !email.is_empty() && email.eq_ignore_ascii_case(wanted) && group_size(a, all) >= 2;
    !bare_email_is_ambiguous
        && [a.id.as_str(), a.name.as_str(), a.display_name.as_str(), email]
            .into_iter()
            .any(|s| !s.is_empty() && s.eq_ignore_ascii_case(wanted))
}

/// `user@example.com 2` and `user@example.com·2` read as `user@example.com ·2`.
fn normalize_dot(wanted: &str) -> String {
    let spaced = wanted.replace('\u{b7}', " \u{b7}");
    let mut parts = spaced.split_whitespace();
    if let (Some(email), Some(rest), None) = (parts.next(), parts.next(), parts.next()) {
        let n = rest.trim_start_matches('\u{b7}');
        if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) {
            return format!("{email} \u{b7}{n}");
        }
    }
    wanted.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::identities::SecretRef;

    fn acct(id: &str, email: &str, seq: Option<u32>, created_at: i64) -> IdentityAccount {
        let mut context = serde_json::json!({ "email": email });
        if let Some(n) = seq {
            context["seq"] = serde_json::json!(n);
        }
        IdentityAccount {
            id: id.into(),
            name: "claude-oauth".into(),
            provider: "claude".into(),
            kind: "oauth".into(),
            display_name: String::new(),
            secret_ref: SecretRef::OAuthConfigDir { dir: format!("/d/{id}") },
            context,
            status: "valid".into(),
            created_at,
            updated_at: created_at,
        }
    }

    #[test]
    fn an_existing_group_is_numbered_oldest_first() {
        let all = [acct("b", "Me@Example.com", None, 20), acct("a", "me@example.com", None, 10), acct("c", "other@example.com", None, 5)];
        let mut got = missing_seqs(&all);
        got.sort();
        assert_eq!(got, [("a".to_string(), 1), ("b".to_string(), 2), ("c".to_string(), 1)], "case-folded group, oldest first");
    }

    #[test]
    fn a_number_is_never_reused_and_shows_only_beside_a_sibling() {
        // `·1` was deleted: the survivor stays `·2`, the next sign-in is `·3`.
        let survivor = acct("b", "me@example.com", Some(2), 20);
        let all = [survivor.clone(), acct("d", "me@example.com", None, 30)];
        assert_eq!(missing_seqs(&all), [("d".to_string(), 3)]);
        assert_eq!(numbered_label(&survivor, "me@example.com", &[survivor.clone()]), "me@example.com", "alone again: bare");
        assert_eq!(numbered_label(&survivor, "me@example.com", &all), "me@example.com \u{b7}2");
    }

    #[test]
    fn a_rewrite_of_the_context_keeps_the_stored_number_unless_the_address_changed() {
        let stored = serde_json::json!({ "email": "me@example.com", "seq": 2 });
        let mut rebuilt = serde_json::json!({ "email": "Me@Example.com" });
        carry_over(&stored, &mut rebuilt);
        assert_eq!(rebuilt["seq"], 2, "a re-login of the same address keeps it");
        let mut empty = serde_json::json!({});
        carry_over(&stored, &mut empty);
        assert_eq!((empty["seq"].clone(), empty["email"].clone()), (serde_json::json!(2), serde_json::json!("me@example.com")), "a context with no email keeps both");
        let mut other = serde_json::json!({ "email": "else@example.com" });
        carry_over(&stored, &mut other);
        assert!(other.get("seq").is_none(), "another address is another group");
        let mut given = serde_json::json!({ "email": "me@example.com", "seq": 5 });
        carry_over(&stored, &mut given);
        assert_eq!(given["seq"], 5, "an explicit number stands");
    }

    #[test]
    fn a_different_provider_or_email_is_another_group() {
        let mut codex = acct("x", "me@example.com", Some(1), 1);
        codex.provider = "codex".into();
        let all = [acct("a", "me@example.com", Some(1), 1), codex];
        assert_eq!(numbered_label(&all[0], "me@example.com", &all), "me@example.com");
        assert!(missing_seqs(&[acct("n", "", None, 1)]).is_empty(), "no email, nothing to number");
    }

    #[test]
    fn a_bare_email_shared_by_two_accounts_names_neither_but_the_numbered_form_names_one() {
        let all = [acct("a", "me@example.com", Some(1), 1), acct("b", "me@example.com", Some(2), 2)];
        assert!(!names(&all[0], "me@example.com", &all) && !names(&all[1], "me@example.com", &all));
        assert!(names(&all[1], "me@example.com \u{b7}2", &all) && !names(&all[0], "me@example.com \u{b7}2", &all));
        assert!(names(&all[1], "me@example.com 2", &all), "the dot is optional when typing");
        assert!(names(&all[0], "a", &all), "the id still names it");
        let unlisted = [acct("a", "me@example.com", None, 1), acct("b", "me@example.com", None, 2)];
        assert!(!names(&unlisted[0], "me@example.com", &unlisted), "ambiguous before either has a number");
        let lone = [acct("a", "me@example.com", Some(1), 1)];
        assert!(names(&lone[0], "ME@example.com", &lone), "a lone account answers to its email");
    }
}
