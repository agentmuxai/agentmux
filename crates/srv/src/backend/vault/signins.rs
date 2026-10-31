// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Sign-ins a browser profile keeps (spec SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md
//! §9): `login` entries whose `profile` field names the profile whose tabs may
//! use them, `personal` or `profile:<id>`. A login without a site belongs to no
//! profile. HTTP Basic/Digest sign-ins are such entries with a `realm` (and
//! `proxy` for a proxy's), moved here from the keychain once (§5.4, §9.3).

use std::collections::BTreeMap;

use agentmux_common::time::now_ms;

use super::{record, seal, EntryChange, NewEntry, Parts, PERSON};
use crate::backend::storage::store::Store;
use crate::backend::storage::vault::VaultRow;

pub const PROFILE: &str = "profile";
pub const REALM: &str = "realm";
pub const PROXY: &str = "proxy";
pub const PERSONAL: &str = "personal";
const SITE: &str = "site";
const USERNAME: &str = "username";

/// On a sign-in moved from the keychain: no record says which profile used it.
pub const FROM_KEYCHAIN: &str = "moved from the old sign-in store: check the profile";
/// On a sign-in the site turned down right after it was filled: not offered
/// again until a new password is saved for it.
pub const REFUSED: &str = "the site refused it: check the password";

/// Whether `v` names a profile a sign-in can belong to.
pub fn is_profile(v: &str) -> bool {
    v == PERSONAL || v.strip_prefix("profile:").is_some_and(crate::backend::browser_profiles_store::is_profile_id)
}

/// The profile a tab browsing as `identity` (its `browser:identity`) keeps
/// sign-ins under; `None` for Incognito, which never saves one (§9.1).
pub fn profile_for_tab(identity: Option<&str>) -> Option<String> {
    match identity {
        None => Some(PERSONAL.to_string()),
        Some(i) if i.starts_with("profile:") && is_profile(i) => Some(i.to_string()),
        Some(_) => None,
    }
}

/// A login with a site belongs to Personal unless it names another profile; one
/// without a site belongs to none.
pub(super) fn settle_profile(kind: &str, fields: &mut BTreeMap<String, String>) {
    if kind != "login" {
        return;
    }
    if fields.contains_key(SITE) {
        fields.entry(PROFILE.to_string()).or_insert_with(|| PERSONAL.to_string());
    } else {
        fields.remove(PROFILE);
    }
}

/// Where an HTTP sign-in is used: the profile, the origin (a proxy's, for a
/// proxy), and the realm the site names.
#[derive(Debug, Clone)]
pub struct Space {
    pub profile: String,
    pub origin: String,
    pub realm: String,
    pub proxy: bool,
}

fn fields_of(row: &VaultRow) -> BTreeMap<String, String> {
    serde_json::from_str(&row.fields).unwrap_or_default()
}

fn tags_of(row: &VaultRow) -> Vec<String> {
    serde_json::from_str(&row.tags).unwrap_or_default()
}

fn holds(row: &VaultRow, s: &Space) -> bool {
    let f = fields_of(row);
    let get = |k: &str| f.get(k).map(String::as_str).unwrap_or("");
    row.kind == "login" && get(PROFILE) == s.profile && get(SITE) == s.origin && get(REALM) == s.realm && f.contains_key(PROXY) == s.proxy
}

/// The sign-ins kept for `s`, newest first.
fn held(store: &Store, s: &Space) -> Result<Vec<VaultRow>, String> {
    let mut rows: Vec<VaultRow> = store.vault_list().map_err(|e| e.to_string())?.into_iter().filter(|r| holds(r, s)).collect();
    rows.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(rows)
}

/// The sign-in a tab is offered for `s`: the newest the site hasn't refused.
/// Returns it and its user name.
pub fn find(store: &Store, s: &Space) -> Result<Option<(VaultRow, String)>, String> {
    Ok(held(store, s)?
        .into_iter()
        .find(|r| !tags_of(r).iter().any(|t| t == REFUSED))
        .map(|r| {
            let user = fields_of(&r).get(USERNAME).cloned().unwrap_or_default();
            (r, user)
        }))
}

/// The user name and password for `s`, once the person approved their use:
/// logged as a sign-in, the password never.
pub fn fill(store: &Store, s: &Space) -> Result<Option<(String, zeroize::Zeroizing<String>)>, String> {
    let Some((row, user)) = find(store, s)? else { return Ok(None) };
    let parts = seal::open(&seal::key(false)?, &row.id, &row.sealed)?;
    let password = zeroize::Zeroizing::new(parts.get("password").unwrap_or("").to_string());
    record(store, &row.id, &row.label, PERSON, SIGNED_IN, &s.origin, "done");
    if let Err(e) = store.vault_touch(&row.id, now_ms()) {
        tracing::warn!(error = %e, "vault: couldn't note a sign-in's use");
    }
    Ok(Some((user, password)))
}

/// The action an HTTP sign-in's use is logged as.
pub const SIGNED_IN: &str = "signed in";

/// Keeps the user name and password the person typed for `s`. The same user's
/// entry gets the new password (the old one kept as a version); another user
/// gets an entry of its own. Returns the entry's id.
pub fn save(store: &Store, s: &Space, username: &str, password: &str) -> Result<String, String> {
    save_as(store, s, username, password, None)
}

fn save_as(store: &Store, s: &Space, username: &str, password: &str, tag: Option<&str>) -> Result<String, String> {
    let same_user = held(store, s)?.into_iter().find(|r| fields_of(r).get(USERNAME).map(String::as_str).unwrap_or("") == username);
    if let Some(row) = same_user {
        let parts = seal::open(&seal::key(false)?, &row.id, &row.sealed)?;
        let mut tags = tags_of(&row);
        let refused = tags.iter().any(|t| t == REFUSED);
        if parts.get("password") == Some(password) && !refused {
            return Ok(row.id);
        }
        tags.retain(|t| t != REFUSED);
        let mut new = Parts::default();
        new.0.insert("password".to_string(), password.to_string());
        super::update(store, EntryChange { id: row.id.clone(), label: row.label.clone(), fields: fields_of(&row), tags, parts: new, clear: Vec::new() })?;
        return Ok(row.id);
    }
    let mut fields = BTreeMap::from([(SITE.to_string(), s.origin.clone()), (PROFILE.to_string(), s.profile.clone())]);
    if !username.is_empty() {
        fields.insert(USERNAME.to_string(), username.to_string());
    }
    if !s.realm.is_empty() {
        fields.insert(REALM.to_string(), s.realm.clone());
    }
    if s.proxy {
        fields.insert(PROXY.to_string(), "yes".to_string());
    }
    let mut parts = Parts::default();
    parts.0.insert("password".to_string(), password.to_string());
    let tags = tag.map(|t| vec![t.to_string()]).unwrap_or_default();
    let entry = NewEntry { kind: "login".into(), label: label_for(s), fields, tags, parts };
    let (action, target) = if tag.is_some() { ("moved in", "from the old sign-in store") } else { ("saved", "from a browser sign-in") };
    super::insert(store, entry, PERSON, action, target)
}

/// A sign-in's label: the site's host, and the realm it names.
fn label_for(s: &Space) -> String {
    let host = s.origin.split_once("://").map_or(s.origin.as_str(), |(_, h)| h);
    let label = if s.realm.is_empty() { host.to_string() } else { format!("{host} ({})", s.realm) };
    label.chars().take(super::MAX_LABEL).collect()
}

/// The site turned down the sign-in filled for `s`: it is kept, since it may be
/// the person's only copy, but not offered again until a new password is saved.
pub fn refused(store: &Store, s: &Space) -> Result<(), String> {
    let Some((row, _)) = find(store, s)? else { return Ok(()) };
    let mut tags = tags_of(&row);
    if tags.len() < super::MAX_TAGS {
        tags.push(REFUSED.to_string());
    }
    let changed = VaultRow { tags: serde_json::to_string(&tags).map_err(|e| e.to_string())?, updated_at: now_ms(), ..row.clone() };
    if store.vault_update_kept(&changed, &row.sealed, None).map_err(|e| e.to_string())? {
        record(store, &row.id, &row.label, PERSON, "refused by the site", &s.origin, "done");
    }
    Ok(())
}

/// How many sign-ins each named profile holds, by profile id.
pub fn counts(store: &Store) -> Result<BTreeMap<String, u32>, String> {
    let mut out = BTreeMap::new();
    for row in store.vault_list().map_err(|e| e.to_string())? {
        if let Some(id) = fields_of(&row).get(PROFILE).and_then(|p| p.strip_prefix("profile:")) {
            *out.entry(id.to_string()).or_insert(0) += 1;
        }
    }
    Ok(out)
}

/// Profile `id` (named `name`) is being deleted: its sign-ins move to Personal,
/// tagged with where they came from, or are deleted with it (§9.4). Returns how
/// many there were.
pub fn profile_deleted(store: &Store, id: &str, name: &str, keep: bool) -> Result<usize, String> {
    let profile = format!("profile:{id}");
    let rows: Vec<VaultRow> =
        store.vault_list().map_err(|e| e.to_string())?.into_iter().filter(|r| fields_of(r).get(PROFILE) == Some(&profile)).collect();
    for row in &rows {
        if !keep {
            super::delete(store, &row.id)?;
            continue;
        }
        let mut fields = fields_of(row);
        fields.insert(PROFILE.to_string(), PERSONAL.to_string());
        let mut tags = tags_of(row);
        if tags.len() < super::MAX_TAGS {
            tags.push(format!("from deleted profile {name}").chars().take(super::MAX_LABEL).collect());
        }
        let changed = VaultRow {
            fields: serde_json::to_string(&fields).map_err(|e| e.to_string())?,
            tags: serde_json::to_string(&tags).map_err(|e| e.to_string())?,
            updated_at: now_ms(),
            ..row.clone()
        };
        if store.vault_update_kept(&changed, &row.sealed, None).map_err(|e| e.to_string())? {
            record(store, &row.id, &row.label, PERSON, "moved to Personal", &format!("profile {name} was deleted"), "done");
        }
    }
    Ok(rows.len())
}

/// Moves the HTTP sign-ins the keychain holds into the Vault, once: each goes
/// to Personal, tagged for the person to check, and its keychain copy is then
/// deleted. Only the ones the old store noted can be found (the keychain can't
/// be listed). One the keychain can't read now stays for the next start.
/// Returns how many moved.
pub fn move_from_keychain(store: &Store) -> usize {
    use crate::identity::browser_credential_store as old;
    move_from(store, old::load, old::delete)
}

/// Reads or deletes a sign-in at its keychain address `(identity_id, origin, realm, is_proxy)`.
type OldLoad = fn(&str, &str, &str, bool) -> Result<Option<crate::identity::browser_credential_store::BrowserCredential>, String>;
type OldDelete = fn(&str, &str, &str, bool) -> Result<(), String>;

/// [`move_from_keychain`], reading and deleting with `load` and `delete`.
fn move_from(store: &Store, load: OldLoad, delete: OldDelete) -> usize {
    let rows = match store.browser_signins_saved() {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(error = %e, "vault: couldn't read the old browser sign-ins");
            return 0;
        }
    };
    let mut moved = 0;
    for (identity, origin, realm, proxy) in rows {
        let space = Space { profile: PERSONAL.to_string(), origin: origin.clone(), realm: realm.clone(), proxy };
        match load(&identity, &origin, &realm, proxy) {
            Ok(Some(cred)) => {
                if let Err(e) = save_as(store, &space, &cred.username, &cred.password, Some(FROM_KEYCHAIN)) {
                    tracing::warn!(error = %e, "vault: couldn't move a browser sign-in from the keychain");
                    continue;
                }
                if let Err(e) = delete(&identity, &origin, &realm, proxy) {
                    tracing::warn!(error = %e, "vault: moved a browser sign-in but couldn't delete its keychain copy");
                }
                moved += 1;
            }
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(error = %e, "vault: the keychain couldn't read an old browser sign-in; trying again next start");
                continue;
            }
        }
        if let Err(e) = store.browser_signin_forget(&identity, &origin, &realm, proxy) {
            tracing::warn!(error = %e, "vault: couldn't forget a moved browser sign-in");
        }
    }
    if moved > 0 {
        tracing::info!(moved, "vault: moved the browser's HTTP sign-ins from the keychain");
    }
    moved
}

#[cfg(test)]
#[path = "signins_tests.rs"]
mod tests;
