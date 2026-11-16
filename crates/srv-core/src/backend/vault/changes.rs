// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A password an agent changes on a site that already has an account
//! (docs/specs/SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md §7.3).
//!
//! The new password is a **pending version** of the saved sign-in for that site and user
//! ([`super::versions`], as a rotation's is), not an entry of its own and not the entry's value: the
//! saved password stays the one that works until the site was sent the form with the new one. When the
//! page's listener reports a password-change form sent with exactly the pending value, the pending
//! value becomes the entry's and the old one is kept 7 days, replaced by the agent that made the change,
//! for the person to put back. A form never sent (or refused) leaves the saved password as it was; the
//! pending value stays for the person in the entry's earlier values, as an interrupted rotation's does.

use super::signins::{self, Space};
use super::versions::{self, Rotation};
use super::{record, seal, EntryChange, Parts};
use crate::backend::storage::store::Store;
use crate::backend::storage::vault::VaultRow;

/// What the version is called in the entry's history.
pub const HOW: &str = "password change";
/// What the Activity line says when the site was sent the new password.
pub const CHANGED: &str = "changed password";

/// The saved sign-in the agent means: the one `profile` keeps for `s`'s site, for `username` (the one
/// the agent or the form names) or, when none is named, the site's only one. It must be in a Vault the
/// agent may use, and not one made on a sign-up that was never sent.
pub fn saved_login(store: &Store, s: &Space, username: &str, agent: &str) -> Result<VaultRow, String> {
    let mut rows = Vec::new();
    for row in signins::held(store, s)? {
        let never_sent = signins::tags_of(&row).iter().any(|t| t == super::signups::UNCONFIRMED);
        if !never_sent && super::vaults::entry_usable(store, &row.id, Some(agent))? {
            rows.push(row);
        }
    }
    let named = |r: &VaultRow| signins::fields_of(r).get(signins::USERNAME).map(String::as_str).unwrap_or("") == username;
    let found: Vec<VaultRow> = if username.is_empty() { rows } else { rows.into_iter().filter(named).collect() };
    match found.len() {
        1 => found.into_iter().next().ok_or_else(|| "no sign-in".to_string()),
        0 => Err(format!(
            "the Vault has no saved sign-in for {} in this tab's profile that {agent} may use, so there is no password to change: sign in with it first, or ask the person",
            s.origin
        )),
        n => Err(format!("{n} sign-ins are saved for {} in this tab's profile: give username, the user name this account signs in with", s.origin)),
    }
}

/// Keeps `password` as the pending change of `row`'s password, made by `agent`. A change made earlier and
/// never sent is set aside for the person (the site may still hold it).
pub fn start(store: &Store, row: &VaultRow, password: &str, agent: &str) -> Result<Rotation, String> {
    versions::set_aside_pending(store, row, HOW)?;
    versions::start_pending(store, &row.id, password, agent, HOW)
}

/// The form of a change was never typed into: nothing saw the pending value, so none is kept.
pub fn abandon(store: &Store, change: &Rotation) -> Result<(), String> {
    versions::abandon_rotation(store, change)
}

/// A password-change form of `s` was sent with `password`. If that is the pending change of one of the
/// site's saved sign-ins, it becomes the sign-in's password and the old one is kept 7 days. Gives the
/// entry. `None` for any other password: the person's own, offered "Update password?" like any sign-in.
pub fn confirm_sent(store: &Store, s: &Space, password: &str) -> Result<Option<String>, String> {
    for row in signins::held(store, s)? {
        let Some(change) = versions::pending_is(store, &row, password, HOW)? else { continue };
        versions::finish_rotation(store, &row.id, &change, Ok(()), &change.by)?;
        // The site took it: a sign-in it had turned down is worth offering again.
        let tags: Vec<String> = signins::tags_of(&row).into_iter().filter(|t| t != signins::REFUSED).collect();
        if tags != signins::tags_of(&row) {
            super::update(store, EntryChange { id: row.id.clone(), label: row.label.clone(), fields: signins::fields_of(&row), tags, parts: Parts::default(), clear: Vec::new() })?;
        }
        record(store, &row.id, &row.label, &change.by, CHANGED, &s.origin, "done");
        return Ok(Some(row.id));
    }
    Ok(None)
}

/// Whether entry `id`'s current password is `password`: for tests and callers that must not hand it out.
#[cfg(test)]
pub(super) fn current_is(store: &Store, id: &str, password: &str) -> bool {
    let Ok(Some(row)) = store.vault_get(id) else { return false };
    seal::key(false).ok().and_then(|k| seal::open(&k, &row.id, &row.sealed).ok()).is_some_and(|p| p.get("password") == Some(password))
}

#[cfg(test)]
#[path = "changes_tests.rs"]
mod tests;
