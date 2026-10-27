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

fn high_of(a: &IdentityAccount) -> u32 {
    a.context.get("seqmax").and_then(|n| n.as_u64()).and_then(|n| u32::try_from(n).ok()).unwrap_or(0)
}

/// One account's numbering to write: its number (`None`: it keeps the one it has)
/// and the highest number its group has handed out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumberingUpdate {
    pub id: String,
    pub seq: Option<u32>,
    pub high: u32,
}

/// The numbering to write. Each account with an email and no number gets the
/// group's highest number ever handed out plus one, oldest first (creation time,
/// then id), so an existing group is numbered in the order its accounts were made.
/// "Ever": every member records the group's high-water mark (`seqmax`), so deleting
/// the account that held the highest number doesn't free it for the next sign-in.
pub fn numbering_updates(accounts: &[IdentityAccount]) -> Vec<NumberingUpdate> {
    let mut groups: HashMap<(String, String), Vec<&IdentityAccount>> = HashMap::new();
    for a in accounts {
        if let Some(k) = group_key(a) {
            groups.entry(k).or_default().push(a);
        }
    }
    let mut out = Vec::new();
    for members in groups.values_mut() {
        let base = members.iter().map(|a| seq_of(a).unwrap_or(0).max(high_of(a))).max().unwrap_or(0);
        members.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
        let mut next = base;
        let mut assigned = HashMap::new();
        for a in members.iter().filter(|a| seq_of(a).is_none()) {
            next += 1;
            assigned.insert(a.id.clone(), next);
        }
        for a in members.iter() {
            let seq = assigned.get(&a.id).copied();
            if seq.is_some() || high_of(a) != next {
                out.push(NumberingUpdate { id: a.id.clone(), seq, high: next });
            }
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
    let positive = |v: Option<&serde_json::Value>| v.and_then(|n| n.as_u64()).filter(|n| *n > 0);
    let (stored_seq, stored_high) = (positive(stored.get("seq")), positive(stored.get("seqmax")));
    if stored_seq.is_none() && stored_high.is_none() {
        return;
    }
    if !incoming.is_object() {
        *incoming = serde_json::json!({});
    }
    let stored_email = stored.get("email").and_then(|e| e.as_str()).unwrap_or("");
    match incoming.get("email").and_then(|e| e.as_str()).filter(|e| !e.trim().is_empty()) {
        // Another address is another group: the numbering the incoming context carries
        // (a reconnect clones the old context and replaces only the email) is the old group's.
        Some(email) if !same_email(email, stored_email) => {
            if let Some(ctx) = incoming.as_object_mut() {
                ctx.remove("seq");
                ctx.remove("seqmax");
            }
            return;
        }
        Some(_) => {}
        None => incoming["email"] = serde_json::json!(stored_email),
    }
    if positive(incoming.get("seq")).is_none() {
        if let Some(seq) = stored_seq {
            incoming["seq"] = serde_json::json!(seq);
        }
    }
    // The high-water mark only ever rises: a copy of the account from before another
    // sign-in was numbered and deleted must not lower it.
    let high = [stored_high, stored_seq, positive(incoming.get("seqmax"))].into_iter().flatten().max();
    if let Some(high) = high {
        incoming["seqmax"] = serde_json::json!(high);
    }
}

/// Whether `wanted` (an id, name, display name, email, or `email ·N`) names `a`.
/// A bare email that more than one account in `all` carries names none of them:
/// the caller reports the candidates instead of taking the first.
pub fn names(a: &IdentityAccount, wanted: &str, all: &[IdentityAccount]) -> bool {
    let wanted = wanted.trim();
    let email = a.context.get("email").and_then(|e| e.as_str()).unwrap_or("");
    let shared = group_size(a, all) >= 2;
    // The names a person sees for it: the display name, the email, the account name;
    // `label ·N` while it shares them with another account.
    let labels = [a.display_name.as_str(), email, a.name.as_str()];
    let spoken = normalize_dot(wanted);
    if labels.iter().any(|l| !l.is_empty() && numbered_label(a, l, all) != *l && numbered_label(a, l, all).eq_ignore_ascii_case(&spoken)) {
        return true;
    }
    // A bare email its group shares, or a display name two accounts carry, names none of
    // them, numbered or not yet: by the group, not by the number, which an account not
    // listed yet doesn't have. A display name only one carries still names it.
    let same_display = |o: &IdentityAccount| !o.display_name.is_empty() && o.display_name.eq_ignore_ascii_case(wanted);
    let bare_is_ambiguous = (shared && !email.is_empty() && email.eq_ignore_ascii_case(wanted))
        || (same_display(a) && all.iter().filter(|o| same_display(o)).count() >= 2);
    !bare_is_ambiguous && [a.id.as_str(), a.name.as_str(), a.display_name.as_str(), email].into_iter().any(|l| !l.is_empty() && l.eq_ignore_ascii_case(wanted))
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
        let mut got: Vec<(String, Option<u32>, u32)> = numbering_updates(&all).into_iter().map(|u| (u.id, u.seq, u.high)).collect();
        got.sort();
        assert_eq!(
            got,
            [("a".to_string(), Some(1), 2), ("b".to_string(), Some(2), 2), ("c".to_string(), Some(1), 1)],
            "case-folded group, oldest first; every member records the group's high-water mark"
        );
    }

    #[test]
    fn a_number_is_never_reused_and_shows_only_beside_a_sibling() {
        // `·1` was deleted: the survivor stays `·2`, the next sign-in is `·3`.
        let survivor = acct("b", "me@example.com", Some(2), 20);
        let all = [survivor.clone(), acct("d", "me@example.com", None, 30)];
        assert_eq!(numbering_updates(&all).into_iter().find(|u| u.id == "d").and_then(|u| u.seq), Some(3));
        // The highest number is deleted: the next sign-in doesn't get it again.
        let mut one = acct("a", "me@example.com", Some(1), 10);
        one.context["seqmax"] = serde_json::json!(2);
        let updates = numbering_updates(&[one, acct("e", "me@example.com", None, 40)]);
        assert_eq!(updates.iter().find(|u| u.id == "e").and_then(|u| u.seq), Some(3), "{updates:?}");
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
        // A reconnect that replaced only the email on a clone of the old context drops the old numbering.
        let mut cloned = serde_json::json!({ "email": "else@example.com", "seq": 2, "seqmax": 4 });
        carry_over(&stored, &mut cloned);
        assert!(cloned.get("seq").is_none() && cloned.get("seqmax").is_none(), "{cloned}");
        let mut given = serde_json::json!({ "email": "me@example.com", "seq": 5 });
        carry_over(&stored, &mut given);
        assert_eq!(given["seq"], 5, "an explicit number stands");
        // A stale copy of an account can't lower the group's high-water mark.
        let stored = serde_json::json!({ "email": "me@example.com", "seq": 1, "seqmax": 3 });
        let mut stale = serde_json::json!({ "email": "me@example.com", "seq": 1, "seqmax": 1 });
        carry_over(&stored, &mut stale);
        assert_eq!(stale["seqmax"], 3);
    }

    #[test]
    fn a_different_provider_or_email_is_another_group() {
        let mut codex = acct("x", "me@example.com", Some(1), 1);
        codex.provider = "codex".into();
        let all = [acct("a", "me@example.com", Some(1), 1), codex];
        assert_eq!(numbered_label(&all[0], "me@example.com", &all), "me@example.com");
        assert!(numbering_updates(&[acct("n", "", None, 1)]).is_empty(), "no email, nothing to number");
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
        // A display name is advertised as `Work ·2` when the pair shares it; that must be accepted.
        let mut work = [acct("a", "me@example.com", Some(1), 1), acct("b", "me@example.com", Some(2), 2)];
        for w in &mut work {
            w.display_name = "Work".into();
        }
        assert!(names(&work[1], "Work \u{b7}2", &work) && !names(&work[0], "Work \u{b7}2", &work));
        assert!(!names(&work[0], "Work", &work) && !names(&work[1], "Work", &work), "a bare shared display name names neither");
        work[0].display_name = "Personal".into();
        assert!(names(&work[0], "Personal", &work) && names(&work[1], "Work", &work), "a display name only one carries still names it");
        let lone = [acct("a", "me@example.com", Some(1), 1)];
        assert!(names(&lone[0], "ME@example.com", &lone), "a lone account answers to its email");
    }
}
