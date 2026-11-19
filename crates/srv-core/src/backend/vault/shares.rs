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
//!
//! A sign-in for a site's HTTP prompt or a proxy (A11) is shared the same way. The
//! browser answers that prompt itself, in the agent's own profile
//! ([`signins::find_shared`]); the person is asked at each use unless they chose
//! "Always" for it, which is kept under [`prompt_grant_key`].

use std::collections::BTreeMap;

use agentmux_common::time::now_ms;

use super::signins::{fields_of, REALM, SITE, USERNAME};
use super::{history, inspect, record, record_line, signins, Line, PERSON};
use crate::backend::storage::store::Store;

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

/// The key the person's "Always" for a shared HTTP-prompt sign-in is kept under: [`grant_prefix`], so it is listed and
/// ended as any sign-in's "Always" is, then the agent's own browser profile and the origin, kind of prompt and realm
/// the person was shown. A login whose site or realm is edited afterwards has another key, so its next use asks again
/// (A6); no form sign-in's key ends this way, so neither kind of "Always" stands for the other.
pub fn prompt_grant_key(uid: &str, profile: &str, origin: &str, realm: &str, proxy: bool) -> String {
    format!("{}{profile}|{origin}|prompt:{}|{realm}", grant_prefix(uid), if proxy { "proxy" } else { "site" })
}

/// Whether a login with these `fields` is a sign-in for a site's HTTP prompt (it names a realm) or for a proxy: the
/// browser answers that prompt itself, and such a login is never typed into a form.
pub fn answers_prompt(fields: &BTreeMap<String, String>) -> bool {
    fields.contains_key(REALM) || fields.contains_key(super::signins::PROXY)
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
    add_shown(store, id, who, None)
}

/// [`add`], for a share the person allowed when shown `ask`: made only if the login is still the one they were shown.
fn add_shown(store: &Store, id: &str, who: &str, shown: Option<&Ask>) -> Result<bool, String> {
    if who != EVERY_AGENT && !is_uid(who) {
        return Err("name the agent by its permanent uid, or * for every agent".into());
    }
    // A new share starts by asking: an "Always" this agent holds for the entry from before, or one answered while an
    // earlier share was being removed, goes first.
    revoke_always(store, id, who, None)?;
    change(store, id, who, "shared", true, shown, |list| {
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
    let removed = change(store, id, who, "unshared", false, None, |list| {
        let before = list.len();
        list.retain(|w| w != who);
        Ok(list.len() != before)
    })?;
    end_always(store, id, who)?;
    Ok(removed)
}

/// Whether an entry of `kind` with these `fields` can be shared: a login saved for a site. One for a site's HTTP prompt
/// or a proxy is shared too ([`answers_prompt`]): the browser answers the prompt with it, and it is typed into no form.
pub fn check_shareable(kind: &str, fields: &BTreeMap<String, String>) -> Result<(), String> {
    if kind != "login" || !fields.contains_key(SITE) {
        return Err("only a login saved for a site can be shared".into());
    }
    Ok(())
}

fn change(store: &Store, id: &str, who: &str, action: &str, eligible: bool, shown: Option<&Ask>, edit: impl Fn(&mut Vec<String>) -> Result<bool, String>) -> Result<bool, String> {
    for _ in 0..3 {
        let row = store.vault_get(id).map_err(|e| e.to_string())?.ok_or_else(|| format!("no entry {id}"))?;
        let mut fields = fields_of(&row);
        if eligible {
            check_shareable(&row.kind, &fields)?;
        }
        // Checked on the row this write is made over, so no edit of the login can land between the check and the write.
        if let Some(ask) = shown {
            if &asked_of(&row, &fields, &ask.reason) != ask {
                return Err(format!("{} changed while the person was asked, so nothing was shared: ask again", ask.label));
            }
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
        // Written only if the fields are still as read, so two overlapping share changes can't each succeed on what the
        // other has undone (one removing an agent while another adds one must not bring the removed share back).
        let new_fields = serde_json::to_string(&fields).map_err(|e| e.to_string())?;
        if store.vault_set_fields_if(&row.id, row.created_at, &row.sealed, &row.fields, &new_fields, now_ms()).map_err(|e| e.to_string())? {
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

/// What an agent's `Vault share` asks the person, as the card shows it: the login, its site and its user name masked.
/// `created_at` binds it to this login: one deleted and made again is another.
#[derive(Debug, Clone, PartialEq)]
pub struct Ask {
    pub entry_id: String,
    pub label: String,
    pub site: String,
    pub user: String,
    pub created_at: i64,
    pub reason: String,
    /// The login answers a site's HTTP prompt or a proxy's ([`answers_prompt`]): `realm` is the realm it names and
    /// `proxy` whether it is a proxy's. A login that became one, or stopped being one, while the person was asked is
    /// not the login they were shown.
    pub prompt: bool,
    pub realm: String,
    pub proxy: bool,
}

/// What `plan_ask` found.
#[derive(Debug, PartialEq)]
pub enum Asked {
    /// The login is shared with the agent already: nobody is asked. Its label.
    Already(String),
    Ask(Ask),
}

fn asked_of(row: &super::VaultRow, fields: &BTreeMap<String, String>, reason: &str) -> Ask {
    Ask {
        entry_id: row.id.clone(),
        label: row.label.clone(),
        site: fields.get(SITE).cloned().unwrap_or_default(),
        user: signins::mask(fields.get(USERNAME).map(String::as_str).unwrap_or("")),
        created_at: row.created_at,
        reason: reason.to_string(),
        prompt: answers_prompt(fields),
        realm: fields.get(REALM).cloned().unwrap_or_default(),
        proxy: fields.contains_key(super::signins::PROXY),
    }
}

/// Every check of an agent's `Vault share`, with nothing written: the reason, that the agent may use the entry (anything
/// else is "no entry"), that it is a login that can be shared, and that it is not shared with the agent already. `uid`
/// is the agent's permanent uid, proven by the caller: an agent asks for a share for itself only.
pub fn plan_ask(store: &Store, id: &str, agent: &str, uid: &str, reason: Option<&str>) -> Result<Asked, String> {
    let reason = history::require_reason(reason)?;
    if !is_uid(uid) {
        return Err("this agent has no permanent identity a share could name".into());
    }
    let row = inspect::entry_for(store, id, Some(agent))?;
    let fields = fields_of(&row);
    check_shareable(&row.kind, &fields)?;
    if names(&fields, uid) {
        return Ok(Asked::Already(row.label));
    }
    Ok(Asked::Ask(asked_of(&row, &fields, &reason)))
}

/// The person allowed `ask`: shares the login with the agent `uid`, if it is still the login they were shown.
pub fn grant_asked(store: &Store, ask: &Ask, uid: &str) -> Result<(), String> {
    add_shown(store, &ask.entry_id, uid, Some(ask)).map(|_| ())
}

/// The person answered "Always" to `ask` for an HTTP-prompt sign-in: [`grant_asked`], and the browser then answers
/// the prompt in the agent's own `profile` without asking again. Only a login that answers a prompt has this: a form
/// login's "Always" is given at its first `signin`, never with the share (A6).
pub fn grant_asked_always(store: &Store, ask: &Ask, uid: &str, profile: &str) -> Result<(), String> {
    if !ask.prompt {
        return Err("only a sign-in for a site's HTTP prompt or a proxy is shared with an Always".into());
    }
    grant_asked(store, ask, uid)?;
    let key = prompt_grant_key(uid, profile, &ask.site, &ask.realm, ask.proxy);
    store.vault_grant_add(&ask.entry_id, &key, now_ms(), ask.created_at).map_err(|e| e.to_string())
}

/// Whether the person gave agent `uid` an "Always" for the shared HTTP-prompt sign-in `row`, used in `profile`: the
/// login is still shared with it and still names the site and realm the "Always" was given for.
pub fn prompt_always(store: &Store, row: &super::VaultRow, uid: &str, profile: &str) -> bool {
    let fields = fields_of(row);
    if !answers_prompt(&fields) || !names(&fields, uid) {
        return false;
    }
    let get = |k: &str| fields.get(k).map(String::as_str).unwrap_or("");
    let key = prompt_grant_key(uid, profile, get(SITE), get(REALM), fields.contains_key(super::signins::PROXY));
    store.vault_grant_check(&row.id, &key).unwrap_or(false)
}

/// The history line of an ask that did not end in a share: `outcome` is `denied`, `expired` or the like.
pub fn note_ask(store: &Store, ask: &Ask, agent: &str, outcome: &str) {
    record_line(store, Line { entry_id: &ask.entry_id, label: &ask.label, actor: agent, action: ASKED_TO_BE_SHARED, target: &ask.site, outcome, reason: &ask.reason });
}

/// The history action of an agent's request to be given a login.
pub const ASKED_TO_BE_SHARED: &str = "asked to be given it";

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

#[cfg(test)]
#[path = "shares_prompt_tests.rs"]
mod prompt_tests;
