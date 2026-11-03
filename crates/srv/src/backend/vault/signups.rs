// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Sign-ins made on a sign-up form (spec SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md
//! §6.2): the password srv generates for a new-password field is a `login`
//! entry of its own before the page sees it, tagged [`UNCONFIRMED`] until the
//! form is sent with it. It never joins another entry, even one for the same
//! user name: a sign-up the site refuses must not replace a password that works.

use std::collections::BTreeMap;

use super::signins::{self, Space};
use super::{insert, record, seal, EntryChange, NewEntry, Parts, PERSON};
use crate::backend::storage::store::Store;

/// On a sign-in made on a sign-up form that hasn't been sent with it yet.
pub const UNCONFIRMED: &str = "unconfirmed: made on sign-up, not yet sent";

/// Keeps `password`, made for a sign-up on `s`, as a new entry with the user
/// name typed so far. Returns its id and label.
pub fn create(store: &Store, s: &Space, username: &str, password: &str) -> Result<(String, String), String> {
    let mut fields = BTreeMap::from([(signins::SITE.to_string(), s.origin.clone()), (signins::PROFILE.to_string(), s.profile.clone())]);
    if !username.is_empty() {
        fields.insert(signins::USERNAME.to_string(), username.to_string());
    }
    let mut parts = Parts::default();
    parts.0.insert("password".to_string(), password.to_string());
    let label = signins::label_for(s);
    let entry = NewEntry { kind: "login".into(), label: label.clone(), fields, tags: vec![UNCONFIRMED.to_string()], parts };
    let id = insert(store, entry, PERSON, "made", "on a sign-up form")?;
    Ok((id, label))
}

/// The site refused the password made for entry `id` and the person asked for
/// another: the new one replaces it, the refused one kept as a previous
/// version. Only while the entry is still unconfirmed; `None` otherwise, and
/// the caller makes a new entry.
pub fn replace(store: &Store, id: &str, password: &str) -> Result<Option<String>, String> {
    let Some(row) = store.vault_get(id).map_err(|e| e.to_string())? else { return Ok(None) };
    let tags = signins::tags_of(&row);
    if !tags.iter().any(|t| t == UNCONFIRMED) {
        return Ok(None);
    }
    let mut parts = Parts::default();
    parts.0.insert("password".to_string(), password.to_string());
    super::update(store, EntryChange { id: row.id.clone(), label: row.label.clone(), fields: signins::fields_of(&row), tags, parts, clear: Vec::new() })?;
    Ok(Some(row.label))
}

/// A sign-up form was sent with `password` as `username`: if that is the
/// password made for entry `id`, which is still unconfirmed, the entry records
/// the user name sent and loses its tag. False for any other password (the
/// person typed their own), or an entry already confirmed or gone.
pub fn confirm(store: &Store, id: &str, username: &str, password: &str) -> Result<bool, String> {
    let Some(row) = store.vault_get(id).map_err(|e| e.to_string())? else { return Ok(false) };
    let mut tags = signins::tags_of(&row);
    if row.kind != "login" || !tags.iter().any(|t| t == UNCONFIRMED) {
        return Ok(false);
    }
    let parts = seal::open(&seal::key(false)?, &row.id, &row.sealed)?;
    if parts.get("password") != Some(password) {
        return Ok(false);
    }
    tags.retain(|t| t != UNCONFIRMED);
    let mut fields = signins::fields_of(&row);
    let site = fields.get(signins::SITE).cloned().unwrap_or_default();
    if !username.is_empty() {
        fields.insert(signins::USERNAME.to_string(), username.to_string());
    }
    super::update(store, EntryChange { id: row.id.clone(), label: row.label.clone(), fields, tags, parts: Parts::default(), clear: Vec::new() })?;
    record(store, &row.id, &row.label, PERSON, "sent on sign-up", &site, "done");
    Ok(true)
}

#[cfg(test)]
#[path = "signups_tests.rs"]
mod tests;
