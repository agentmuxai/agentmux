// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Shares: a login the person lets an agent's own browser profile use
//! (docs/specs/SPEC_VAULT_AGENTS_ENTER_CREDENTIALS_WHEREVER_ASKED_2026_11_16.md §5).
//!
//! A login's `shared` field lists the agents by permanent uid, comma separated, or
//! holds `*` for every agent. Only the person writes it (`inspect::person_only`
//! refuses an agent's update of it). It changes nothing else about the entry. A
//! share lets the agent ask: each first use is still approved as a `Browser`
//! `signin` is (browser_signin.rs).

use std::collections::BTreeMap;

use agentmux_common::time::now_ms;

use super::signins::{fields_of, REALM, SITE};
use super::{record, PERSON};
use crate::backend::storage::store::Store;
use crate::backend::storage::vault::VaultRow;

/// The login field that lists the agents (by uid) an entry is shared with.
pub const SHARED: &str = "shared";
/// In [`SHARED`]: every agent.
pub const EVERY_AGENT: &str = "*";
const MAX_SHARES: usize = 32;

/// Whether `name` is the share field of a `kind` entry. It is not among the kind's listed fields, which are what the
/// Vault pane's forms offer: a share is made with the Share button, never typed as a field.
pub fn is_field(kind: &str, name: &str) -> bool {
    kind == "login" && name == SHARED
}

/// Who `fields` are shared with: agent uids, or [`EVERY_AGENT`].
pub fn listed(fields: &BTreeMap<String, String>) -> Vec<String> {
    fields.get(SHARED).map(|v| v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()).unwrap_or_default()
}

/// Whether `fields` are shared with the agent whose permanent uid is `uid`.
pub fn names(fields: &BTreeMap<String, String>, uid: &str) -> bool {
    !uid.is_empty() && listed(fields).iter().any(|w| w == uid || w == EVERY_AGENT)
}

/// What an "Always" the person gave `uid` for a shared login starts with: `grant_key` in
/// browser_signin.rs names the agent by uid for a shared login, never by a name.
pub fn grant_prefix(uid: &str) -> String {
    format!("signin:{uid}|")
}

/// An agent's permanent uid, as a share can name it: never the "every agent" mark or a list.
pub(super) fn is_uid(v: &str) -> bool {
    !v.is_empty() && v != EVERY_AGENT && v.len() <= 64 && v.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn joined(list: &[String]) -> String {
    list.join(",")
}

/// Share entry `id` with `who` (an agent's uid, or [`EVERY_AGENT`]), as the person: a login saved for a site.
/// Returns false when it already was.
pub fn add(store: &Store, id: &str, who: &str) -> Result<bool, String> {
    if who != EVERY_AGENT && !is_uid(who) {
        return Err("name the agent by its permanent uid, or * for every agent".into());
    }
    // A new share starts by asking: an "Always" this agent holds for the entry from before, or one answered while an
    // earlier share was being removed, goes first.
    revoke_always(store, id, who, None)?;
    change(store, id, who, "shared", true, |list| {
        if list.iter().any(|w| w == who) {
            return Ok(false);
        }
        if list.len() >= MAX_SHARES {
            return Err(format!("an entry is shared with at most {MAX_SHARES} agents"));
        }
        list.push(who.to_string());
        Ok(true)
    })
}

/// Stop sharing entry `id` with `who`. Ends the "Always" the agent holds for it and returns whether a share was removed.
pub fn remove(store: &Store, id: &str, who: &str) -> Result<bool, String> {
    // A removal needs no eligibility: a login that has since lost its site, or gained a realm, can still be unshared.
    let removed = change(store, id, who, "unshared", false, |list| {
        let before = list.len();
        list.retain(|w| w != who);
        Ok(list.len() != before)
    })?;
    end_always(store, id, who)?;
    Ok(removed)
}

fn change(store: &Store, id: &str, who: &str, action: &str, eligible: bool, edit: impl Fn(&mut Vec<String>) -> Result<bool, String>) -> Result<bool, String> {
    for _ in 0..3 {
        let row = store.vault_get(id).map_err(|e| e.to_string())?.ok_or_else(|| format!("no entry {id}"))?;
        let mut fields = fields_of(&row);
        if eligible && (row.kind != "login" || !fields.contains_key(SITE)) {
            return Err("only a login saved for a site can be shared".into());
        }
        if eligible && (fields.contains_key(REALM) || fields.contains_key(super::signins::PROXY)) {
            return Err("a sign-in for a site's HTTP prompt or a proxy is typed by the browser itself and can't be shared".into());
        }
        let mut list = listed(&fields);
        if !edit(&mut list)? {
            return Ok(false);
        }
        if list.is_empty() {
            fields.remove(SHARED);
        } else {
            fields.insert(SHARED.to_string(), joined(&list));
        }
        let changed = VaultRow { fields: serde_json::to_string(&fields).map_err(|e| e.to_string())?, updated_at: now_ms(), ..row.clone() };
        if store.vault_update_kept(&changed, &row.sealed, None).map_err(|e| e.to_string())? {
            // The line names no agent: an agent may read an entry's history, and who a login is shared with is not
            // for it (A8). The entry's card lists each share for the person.
            record(store, &row.id, &row.label, PERSON, action, if who == EVERY_AGENT { "every agent" } else { "an agent" }, "done");
            return Ok(true);
        }
    }
    Err(format!("the entry {id} kept changing: try again"))
}

/// Ends every "Always" `who` holds for entry `id` through a share: an agent's uid, or for
/// [`EVERY_AGENT`] those of every agent the entry is no longer shared with.
pub fn end_always(store: &Store, id: &str, who: &str) -> Result<(), String> {
    let row = store.vault_get(id).map_err(|e| e.to_string())?;
    let still = row.as_ref().map(fields_of).unwrap_or_default();
    revoke_always(store, id, who, Some(&still))
}

/// [`end_always`] for the entry as `still` has it: an agent `still` names keeps its own. `None` ends them all.
fn revoke_always(store: &Store, id: &str, who: &str, still: Option<&BTreeMap<String, String>>) -> Result<(), String> {
    let keys = store.vault_grants().map_err(|e| e.to_string())?.remove(id).unwrap_or_default();
    for key in keys {
        let Some(rest) = key.strip_prefix("signin:") else { continue };
        let mut parts = rest.splitn(3, '|');
        let (agent, profile) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
        // A share is used in the agent's own profile, never Personal's or a named one's.
        if !profile.starts_with("profile:agent-") || still.is_some_and(|f| names(f, agent)) {
            continue;
        }
        if who == EVERY_AGENT || agent == who {
            store.vault_grant_revoke(id, &key).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Agent `uid` is gone: its shares go, and the "Always" it held with them (A9).
pub fn forget_agent(store: &Store, uid: &str) -> Result<usize, String> {
    if !is_uid(uid) {
        return Ok(0);
    }
    let mut removed = 0;
    for row in store.vault_list().map_err(|e| e.to_string())? {
        if listed(&fields_of(&row)).iter().any(|w| w == uid) {
            // One entry that can't be changed now does not leave the others shared: the rest still go.
            match remove(store, &row.id, uid) {
                Ok(true) => removed += 1,
                Ok(false) => {}
                Err(e) => tracing::warn!(entry = %row.id, error = %e, "vault: couldn't remove a deleted agent's share"),
            }
        }
    }
    // Always held for it by an entry it was no longer named on.
    let prefix = grant_prefix(uid);
    for (entry, keys) in store.vault_grants().map_err(|e| e.to_string())? {
        for key in keys.into_iter().filter(|k| k.starts_with(&prefix)) {
            if let Err(e) = store.vault_grant_revoke(&entry, &key) {
                tracing::warn!(entry = %entry, error = %e, "vault: couldn't end a deleted agent's Always");
            }
        }
    }
    Ok(removed)
}

#[cfg(test)]
#[path = "shares_tests.rs"]
mod tests;
