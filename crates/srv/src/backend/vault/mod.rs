// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The Vault: the person's secrets, which agents use by name without seeing
//! them (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md).
//!
//! The person writes the Vault through the host (`server::vault_handlers`).
//! Agents list entries' names, ask for new ones, and create entries whose value
//! the Vault makes itself (`generate`), which no one has seen. Everything here
//! is blocking (the store, the keychain): call it off the async runtime.

pub mod expiry;
pub mod files;
pub mod generate;
pub mod import;
pub mod kdbx_export;
pub mod kdbx_names;
pub mod kinds;
pub mod plain_export;
pub mod requests;
pub mod scrub;
pub mod seal;
pub mod slots;
pub mod transfer;
pub mod uses;
pub mod versions;
mod wordlist;

use std::collections::BTreeMap;

use agentmux_common::time::now_ms;
use serde::{Deserialize, Serialize};

use crate::backend::storage::store::Store;
use crate::backend::storage::vault::{VaultActivityRow, VaultRow};
use kinds::VaultSection;
use seal::Parts;

/// The actor the activity log names for the person.
pub const PERSON: &str = "you";

const MAX_LABEL: usize = 200;
const MAX_FIELD: usize = 4096;
/// §8.7's cap on one value.
const MAX_PART: usize = 1024 * 1024;
const MAX_TAGS: usize = 50;
const MAX_ID: usize = 48;

/// An entry as the pane shows it: no value, only which parts are set and
/// the main value's last characters.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct VaultEntryInfo {
    pub id: String,
    pub kind: String,
    pub section: VaultSection,
    pub label: String,
    pub fields: BTreeMap<String, String>,
    pub tags: Vec<String>,
    /// The parts that hold a value.
    pub parts_set: Vec<String>,
    /// The sites an agent may fill this entry into without asking again.
    pub grants: Vec<String>,
    pub tail: String,
    /// Earlier values kept for 7 days, which the person can put back.
    pub versions: u32,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
    #[ts(type = "number")]
    pub last_used_at: i64,
}

/// An entry as an agent sees it (`Vault` `list`): what it is called, what
/// kind it is, and when it expires, a date being no secret. No value, tail,
/// other field or grant.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct VaultAgentEntry {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub section: VaultSection,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires: Option<String>,
}

#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct VaultActivity {
    #[ts(type = "number")]
    pub at: i64,
    pub entry_id: String,
    pub entry_label: String,
    pub actor: String,
    pub action: String,
    pub target: String,
    pub outcome: String,
}

/// A new entry from the pane.
#[derive(Deserialize)]
pub struct NewEntry {
    pub kind: String,
    pub label: String,
    #[serde(default)]
    pub fields: BTreeMap<String, String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub parts: Parts,
}

/// A change to an entry from the pane. A part left out keeps its value;
/// `clear` empties the parts it names.
#[derive(Deserialize)]
pub struct EntryChange {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub fields: BTreeMap<String, String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub parts: Parts,
    #[serde(default)]
    pub clear: Vec<String>,
}

fn validate(kind: &kinds::Kind, label: &str, fields: &BTreeMap<String, String>, tags: &[String], parts: &Parts) -> Result<(), String> {
    if label.trim().is_empty() {
        return Err("an entry needs a label".into());
    }
    if label.chars().count() > MAX_LABEL {
        return Err(format!("a label is at most {MAX_LABEL} characters"));
    }
    for (k, v) in fields {
        if !kind.fields.contains(&k.as_str()) {
            return Err(format!("a {} entry has no field {k:?}", kind.name));
        }
        if v.len() > MAX_FIELD {
            return Err(format!("{k} is longer than {MAX_FIELD} bytes"));
        }
        if k == expiry::FIELD {
            expiry::parse(v)?;
        }
    }
    if tags.len() > MAX_TAGS || tags.iter().any(|t| t.chars().count() > MAX_LABEL) {
        return Err(format!("at most {MAX_TAGS} tags, each at most {MAX_LABEL} characters"));
    }
    for (k, v) in &parts.0 {
        if !kind.parts.contains(&k.as_str()) {
            return Err(format!("a {} entry has no part {k:?}", kind.name));
        }
        if v.len() > MAX_PART {
            return Err(format!("{k} is longer than 1 MiB"));
        }
    }
    Ok(())
}

/// Fields and parts with nothing in them aren't kept.
fn trimmed(fields: BTreeMap<String, String>) -> BTreeMap<String, String> {
    fields.into_iter().map(|(k, v)| (k, v.trim().to_string())).filter(|(_, v)| !v.is_empty()).collect()
}

fn drop_empty(parts: &mut Parts) {
    parts.0.retain(|_, v| !v.is_empty());
}

/// A readable id that never changes: the kind and the label, e.g.
/// `ssn-alex`, unique with a number when taken. No `.`: P3 names a part as
/// `<id>.<part>`.
fn new_id(store: &Store, kind: &str, label: &str) -> Result<String, String> {
    let mut slug = String::new();
    for c in format!("{kind}-{label}").chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let mut base: String = slug.trim_matches('-').chars().take(MAX_ID).collect();
    base = base.trim_end_matches('-').to_string();
    for n in 1..1000 {
        let id = if n == 1 { base.clone() } else { format!("{base}-{n}") };
        if store.vault_get(&id).map_err(|e| e.to_string())?.is_none() {
            return Ok(id);
        }
    }
    Err("too many entries with that label".into())
}

/// The actions that use an entry's value: one of an expired entry says so.
const USES: &[&str] = &["filled a web form", "ran a command", "revealed", "copied"];

/// Adds a line to the activity log; a failure to is logged, not raised.
pub fn record(store: &Store, entry_id: &str, entry_label: &str, actor: &str, action: &str, target: &str, outcome: &str) {
    let mut target = target.to_string();
    if USES.contains(&action) && !entry_id.is_empty() {
        if let Ok(Some(row)) = store.vault_get(entry_id) {
            if expiry::expired(&row, expiry::today()) {
                target = if target.is_empty() { "(expired)".into() } else { format!("{target} (expired)") };
            }
        }
    }
    let row = VaultActivityRow {
        at: now_ms(),
        entry_id: entry_id.into(),
        entry_label: entry_label.into(),
        actor: actor.into(),
        action: action.into(),
        target,
        outcome: outcome.into(),
    };
    if let Err(e) = store.vault_activity_add(&row) {
        tracing::warn!(error = %e, action, "vault: couldn't record activity");
    }
}

/// Saves a new entry from the person; returns its id.
pub fn create(store: &Store, entry: NewEntry) -> Result<String, String> {
    insert(store, entry, PERSON, "created", "")
}

/// Saves a new entry and records who made it and how.
fn insert(store: &Store, mut entry: NewEntry, actor: &str, action: &str, target: &str) -> Result<String, String> {
    let kind = kinds::kind(&entry.kind).ok_or_else(|| format!("unknown kind {:?}", entry.kind))?;
    drop_empty(&mut entry.parts);
    let fields = trimmed(std::mem::take(&mut entry.fields));
    validate(kind, &entry.label, &fields, &entry.tags, &entry.parts)?;
    if entry.parts.get(kind.parts[0]).is_none() {
        return Err(format!("a {} entry needs its {}", kind.name, kind.parts[0]));
    }
    let key = seal::key(store.vault_count().map_err(|e| e.to_string())? == 0)?;
    let label = entry.label.trim().to_string();
    let id = new_id(store, kind.name, &label)?;
    let now = now_ms();
    let row = VaultRow {
        id: id.clone(),
        kind: kind.name.into(),
        label: label.clone(),
        fields: serde_json::to_string(&fields).map_err(|e| e.to_string())?,
        tags: serde_json::to_string(&entry.tags).map_err(|e| e.to_string())?,
        sealed: seal::seal(&key, &id, &entry.parts)?,
        created_at: now,
        updated_at: now,
        last_used_at: 0,
    };
    store.vault_insert(&row).map_err(|e| e.to_string())?;
    record(store, &id, &label, actor, action, target, "done");
    Ok(id)
}

/// The kinds `generate` makes, and the part it fills: a value no person needs to
/// choose. An ID number or a card is the person's, never made up.
pub const GENERATED_KINDS: &[(&str, &str)] = &[("api-key", "value"), ("login", "password"), ("note", "text")];

/// What an agent asks `generate` for.
#[derive(Debug, Deserialize)]
pub struct GenerateRequest {
    pub kind: String,
    pub label: String,
    #[serde(default)]
    pub why: String,
    /// Plain fields of the kind (a login's site and username, an API key's service).
    #[serde(default)]
    pub fields: BTreeMap<String, String>,
    #[serde(default)]
    pub rules: generate::RulesRequest,
    /// When the value stops working (`YYYY-MM-DD`), if the agent knows: the person
    /// is reminded before then (spec §15.2).
    #[serde(default)]
    pub expires: Option<String>,
}

/// What `generate` made: the entry, never its value.
#[derive(Debug, Serialize, PartialEq)]
pub struct Generated {
    pub id: String,
    pub label: String,
    pub kind: String,
    /// The rules it was made under, e.g. "20 characters from 66 choices, 123 bits".
    pub made: String,
}

/// Creates an entry whose main value the Vault makes, for `actor` (the agent that
/// asked). The value goes from the generator into the sealed row and nowhere else.
pub fn generate_entry(store: &Store, req: GenerateRequest, actor: &str) -> Result<Generated, String> {
    let Some((kind, part)) = GENERATED_KINDS.iter().find(|(k, _)| *k == req.kind) else {
        let names: Vec<&str> = GENERATED_KINDS.iter().map(|(k, _)| *k).collect();
        return Err(format!("generate makes {} entries, not {:?}: an ID number or a card is the person's, so ask for it with request", names.join(", "), req.kind));
    };
    let rules = generate::rules(&req.rules)?;
    let value = generate::generate(&rules)?;
    let mut parts = Parts::default();
    parts.0.insert(part.to_string(), value.to_string());
    let why: String = req.why.trim().chars().take(160).collect();
    let target = if why.is_empty() { generate::describe(&rules) } else { why };
    let mut fields = req.fields;
    if let Some(date) = req.expires.filter(|d| !d.trim().is_empty()) {
        fields.insert(expiry::FIELD.to_string(), date);
    }
    let entry = NewEntry { kind: kind.to_string(), label: req.label, fields, tags: Vec::new(), parts };
    let label = entry.label.trim().to_string();
    let id = insert(store, entry, actor, "generated", &target)?;
    Ok(Generated { id, label, kind: kind.to_string(), made: generate::describe(&rules) })
}

/// Changes an entry's label, fields, tags and the parts given.
pub fn update(store: &Store, mut change: EntryChange) -> Result<(), String> {
    let row = store.vault_get(&change.id).map_err(|e| e.to_string())?.ok_or_else(|| format!("no entry {}", change.id))?;
    let kind = kinds::kind(&row.kind).ok_or_else(|| format!("entry {} has an unknown kind", row.id))?;
    drop_empty(&mut change.parts);
    let fields = trimmed(std::mem::take(&mut change.fields));
    validate(kind, &change.label, &fields, &change.tags, &change.parts)?;
    if change.clear.iter().any(|p| p == kind.parts[0]) {
        return Err(format!("a {} entry needs its {}", kind.name, kind.parts[0]));
    }
    let key = seal::key(false)?;
    let mut parts = seal::open(&key, &row.id, &row.sealed)?;
    for p in &change.clear {
        if let Some(mut v) = parts.0.remove(p) {
            zeroize::Zeroize::zeroize(&mut v);
        }
    }
    // A new main value is the person's: the next rotation is theirs, not the agent's that made the old one.
    let replaced = change.parts.0.contains_key(kind.parts[0]);
    if replaced {
        versions::keep(store, &row, PERSON, "replaced")?;
    }
    for (k, v) in std::mem::take(&mut change.parts.0) {
        if let Some(mut old) = parts.0.insert(k, v) {
            zeroize::Zeroize::zeroize(&mut old);
        }
    }
    let label = change.label.trim().to_string();
    let old_fields: BTreeMap<String, String> = serde_json::from_str(&row.fields).unwrap_or_default();
    let target = match (old_fields.get(expiry::FIELD), fields.get(expiry::FIELD)) {
        (old, Some(new)) if old != Some(new) => format!("expires {new}"),
        (Some(_), None) => "no expiry date".to_string(),
        _ => String::new(),
    };
    let changed = VaultRow {
        label: label.clone(),
        fields: serde_json::to_string(&fields).map_err(|e| e.to_string())?,
        tags: serde_json::to_string(&change.tags).map_err(|e| e.to_string())?,
        sealed: seal::seal(&key, &row.id, &parts)?,
        updated_at: now_ms(),
        ..row
    };
    if !store.vault_update(&changed).map_err(|e| e.to_string())? {
        return Err(format!("no entry {}", changed.id));
    }
    record(store, &changed.id, &label, PERSON, if replaced { "replaced" } else { "changed" }, &target, "done");
    Ok(())
}

/// One part's value, for the person: shown in the pane (`revealed`) or put on
/// the clipboard by the host (`copied`). Only the host reaches this, after the
/// OS has confirmed the person. A reveal is logged here, the value never is; a
/// copy is logged by `record_copy` once the host has put it on the clipboard, so
/// the trail never says copied when nothing was.
pub fn reveal(store: &Store, id: &str, part: &str, action: &str) -> Result<zeroize::Zeroizing<String>, String> {
    if action != "revealed" && action != "copied" {
        return Err(format!("unknown use {action:?}"));
    }
    let row = store.vault_get(id).map_err(|e| e.to_string())?.ok_or_else(|| format!("no entry {id}"))?;
    let kind = kinds::kind(&row.kind).ok_or_else(|| format!("entry {id} has an unknown kind"))?;
    if !kind.parts.contains(&part) {
        return Err(format!("a {} entry has no part {part:?}", kind.name));
    }
    let key = seal::key(false)?;
    let parts = seal::open(&key, &row.id, &row.sealed)?;
    let value = parts.get(part).ok_or_else(|| format!("{} has no {part} saved", row.label))?;
    let value = zeroize::Zeroizing::new(value.to_string());
    if action == "revealed" {
        store.vault_touch(&row.id, now_ms()).map_err(|e| e.to_string())?;
        record(store, &row.id, &row.label, PERSON, action, part, "done");
    }
    Ok(value)
}

/// The host's report of what became of a copy: `done` notes the use, `failed`
/// only says it didn't happen.
pub fn record_copy(store: &Store, id: &str, part: &str, done: bool) -> Result<(), String> {
    let row = store.vault_get(id).map_err(|e| e.to_string())?.ok_or_else(|| format!("no entry {id}"))?;
    if !kinds::kind(&row.kind).is_some_and(|k| k.parts.contains(&part)) {
        return Err(format!("{} has no part {part:?}", row.label));
    }
    if done {
        store.vault_touch(&row.id, now_ms()).map_err(|e| e.to_string())?;
    }
    record(store, &row.id, &row.label, PERSON, "copied", part, if done { "done" } else { "failed" });
    Ok(())
}

/// What a web form fill needs to know about an entry, before anything is asked:
/// the entry, the part, and the kind of field that part belongs in.
pub struct FillTarget {
    /// Empty for a value the person will type, with no entry yet.
    pub id: String,
    /// The kind's name.
    pub kind: &'static str,
    /// There is no entry: the person types the value (spec section 6.5).
    pub typed: bool,
    /// The kind, for keeping what they typed.
    pub kind_info: &'static kinds::Kind,
    pub label: String,
    pub part: &'static str,
    pub slot: Option<&'static str>,
    pub what: String,
    /// Which incarnation of the id this is: a deleted entry's id is reused by a
    /// new one of the same kind and label, and neither a grant nor a value may
    /// cross from one to the other while the person decides.
    pub created_at: i64,
}

/// Resolves `"id"` or `"id.part"` for a fill, or says why it can't be filled.
pub fn fill_target(store: &Store, vault: &str) -> Result<FillTarget, String> {
    let (id, part) = slots::split(vault);
    let row = store.vault_get(id).map_err(|e| e.to_string())?.ok_or_else(|| format!("no Vault entry {id:?}: Vault's list action shows the entries"))?;
    let kind = kinds::kind(&row.kind).ok_or_else(|| format!("entry {id} has an unknown kind"))?;
    let part = slots::fillable_part(kind, part)?;
    let slot = slots::slot(kind, part);
    let what = slots::what(kind, part);
    Ok(FillTarget { id: row.id, kind: kind.name, typed: false, kind_info: kind, label: row.label, part, slot, what, created_at: row.created_at })
}

/// For a value no entry holds yet: the kind's main part, to be typed by the person.
pub fn request_target(kind: &str, label: &str) -> Result<FillTarget, String> {
    let kind = kinds::kind(kind).ok_or_else(|| format!("unknown kind {kind:?}"))?;
    let label = label.trim();
    if label.is_empty() || label.chars().count() > MAX_LABEL {
        return Err("request_value needs a label saying what the value is, e.g. \"Alex's SSN\"".into());
    }
    let part = slots::fillable_part(kind, None)?;
    Ok(FillTarget { id: String::new(), kind: kind.name, typed: true, kind_info: kind, label: label.to_string(), part, slot: slots::slot(kind, part), what: slots::what(kind, part), created_at: 0 })
}

/// What a command's `secrets` entry names (`"id"` or `"id.part"`): the entry and
/// the part handed to the process, the kind's main part unless another is named.
/// Unlike a web form, a command may be given any kind of entry; the person is
/// asked first (spec section 7.3).
pub fn command_target(store: &Store, vault: &str) -> Result<FillTarget, String> {
    let (id, part) = slots::split(vault);
    let row = store.vault_get(id).map_err(|e| e.to_string())?.ok_or_else(|| format!("no Vault entry {id:?}: Vault's list action shows the entries"))?;
    let kind = kinds::kind(&row.kind).ok_or_else(|| format!("entry {id} has an unknown kind"))?;
    let part = match part {
        None => kind.parts[0],
        Some(p) => *kind.parts.iter().find(|x| **x == p).ok_or_else(|| format!("{} has no part {p:?}; its parts are {}", row.label, kind.parts.join(", ")))?,
    };
    Ok(FillTarget { id: row.id, kind: kind.name, typed: false, kind_info: kind, label: row.label, part, slot: None, what: slots::what(kind, part), created_at: row.created_at })
}

/// A part's value for the host to type; never returned to an agent. The use is
/// logged by the caller, with the site and the outcome.
pub fn value_for_fill(store: &Store, id: &str, created_at: i64, part: &str) -> Result<zeroize::Zeroizing<String>, String> {
    let row = store.vault_get(id).map_err(|e| e.to_string())?.ok_or_else(|| format!("no entry {id}"))?;
    if row.created_at != created_at {
        return Err(format!("{} was replaced while the person was deciding: nothing was typed", row.label));
    }
    let key = seal::key(false)?;
    let parts = seal::open(&key, &row.id, &row.sealed)?;
    let value = parts.get(part).ok_or_else(|| format!("{} has no {part} saved", row.label))?;
    Ok(zeroize::Zeroizing::new(value.to_string()))
}

pub fn delete(store: &Store, id: &str) -> Result<(), String> {
    let row = store.vault_get(id).map_err(|e| e.to_string())?.ok_or_else(|| format!("no entry {id}"))?;
    store.vault_delete(id).map_err(|e| e.to_string())?;
    // An agent asking again for the entry it was given must not get this id.
    requests::requests().forget_entry(id);
    record(store, id, &row.label, PERSON, "deleted", "", "done");
    Ok(())
}

/// Every entry for the pane, and why the values couldn't be opened if they
/// couldn't (the tails are then empty).
pub fn list_for_pane(store: &Store) -> Result<(Vec<VaultEntryInfo>, Option<String>), String> {
    let rows = store.vault_list().map_err(|e| e.to_string())?;
    let mut grants = store.vault_grants().map_err(|e| e.to_string())?;
    versions::prune(store);
    let counts = store.vault_version_counts().map_err(|e| e.to_string())?;
    let key = if rows.is_empty() { Ok(None) } else { seal::key(false).map(Some) };
    let mut problem = key.as_ref().err().cloned();
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(kind) = kinds::kind(&row.kind) else { continue };
        let (parts_set, tail) = match &key {
            Ok(Some(key)) => match seal::open(key, &row.id, &row.sealed) {
                Ok(parts) => (parts.0.keys().cloned().collect(), parts.get(kind.parts[0]).map(|v| kinds::tail(kind, v)).unwrap_or_default()),
                Err(e) => {
                    problem.get_or_insert(e);
                    (Vec::new(), String::new())
                }
            },
            _ => (Vec::new(), String::new()),
        };
        out.push(VaultEntryInfo {
            fields: serde_json::from_str(&row.fields).unwrap_or_default(),
            tags: serde_json::from_str(&row.tags).unwrap_or_default(),
            section: kind.section,
            grants: grants.remove(&row.id).unwrap_or_default(),
            versions: counts.get(&row.id).filter(|(created, _)| *created == row.created_at).map(|(_, n)| *n).unwrap_or(0),
            id: row.id,
            kind: row.kind,
            label: row.label,
            parts_set,
            tail,
            created_at: row.created_at,
            updated_at: row.updated_at,
            last_used_at: row.last_used_at,
        });
    }
    Ok((out, problem))
}

/// Every entry as an agent may see it. Reads no value, so it never needs the
/// keychain.
pub fn list_for_agent(store: &Store) -> Result<Vec<VaultAgentEntry>, String> {
    Ok(store
        .vault_list()
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter_map(|r| {
            let section = kinds::kind(&r.kind)?.section;
            let expires = expiry::of(&r).map(|d| d.to_string());
            Some(VaultAgentEntry { id: r.id, label: r.label, kind: r.kind, section, expires })
        })
        .collect())
}

pub fn activity(store: &Store, limit: i64) -> Result<Vec<VaultActivity>, String> {
    Ok(store
        .vault_activity_list(limit.clamp(1, 1000))
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|r| VaultActivity {
            at: r.at,
            entry_id: r.entry_id,
            entry_label: r.entry_label,
            actor: r.actor,
            action: r.action,
            target: r.target,
            outcome: r.outcome,
        })
        .collect())
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod transfer_tests;
