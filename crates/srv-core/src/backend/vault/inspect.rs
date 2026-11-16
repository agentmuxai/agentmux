// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What an agent can ask the Vault about an entry and about the Vault itself: `show`, `update` (details only),
//! `verify` and `health` (docs/specs/SPEC_VAULT_PARITY_ENTRY_HISTORY_REASONS_AND_IMPORT_2026_11_06.md §8). Every
//! answer is ids, names, states and counts: no sealed value, tail or hash of one. A problem line says what is wrong
//! in fixed words and never quotes a field's value.

use std::collections::BTreeMap;

use agentmux_common::time::now_ms;
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

use super::{expiry, history, kinds, record_line, seal, signins, signups, validate, vaults, Line, Parts};
use crate::backend::storage::store::Store;
use crate::backend::storage::vault::{VaultRow, VALUE_SETTING};

/// The Activity action of a change to an entry's label, plain fields or tags.
pub const CHANGED_DETAILS: &str = "changed details";
/// An entry expiring within this many days counts as expiring soon.
const SOON_DAYS: i64 = 14;
/// An entry nobody has used for this long counts as unused.
const UNUSED_DAYS: i64 = 90;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// The Vault key, or why it couldn't be read: what `verify` and `health` open the sealed rows with.
pub type KeyResult = Result<seal::Key, String>;

/// Every entry in a Vault `agent` may use (`None`: one that isn't proven, which sees the Vaults open to every agent).
fn usable(store: &Store, agent: Option<&str>) -> Result<Vec<VaultRow>, String> {
    let vaults = store.vaults().map_err(|e| e.to_string())?;
    let rows = store.vault_list().map_err(|e| e.to_string())?;
    Ok(rows.into_iter().filter(|r| vaults.iter().find(|v| v.id == r.vault_id).is_none_or(|v| vaults::may_use(v, agent))).collect())
}

/// `id`'s entry if `agent` may use it; anything else is "no entry", as for an id that isn't there.
fn entry_for(store: &Store, id: &str, agent: Option<&str>) -> Result<VaultRow, String> {
    let no_entry = || format!("no entry {id}");
    let row = store.vault_get(id.trim()).map_err(|e| e.to_string())?.ok_or_else(no_entry)?;
    if !vaults::entry_row_usable(store, &row.vault_id, agent)? {
        return Err(no_entry());
    }
    Ok(row)
}

fn fields_of(row: &VaultRow) -> BTreeMap<String, String> {
    serde_json::from_str(&row.fields).unwrap_or_default()
}

/// An entry as `show` gives it.
#[derive(Debug, Serialize, PartialEq)]
pub struct Shown {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub vault: String,
    pub fields: BTreeMap<String, String>,
    pub tags: Vec<String>,
    /// Which secret parts hold a value: names only. Empty when the Vault key can't open the entry (`readable` false).
    pub parts_set: Vec<String>,
    pub readable: bool,
    /// The date its value stops working, when it has one.
    pub expires: Option<String>,
    /// `none`, `ok`, `soon` (within 14 days) or `expired`.
    pub expiry: &'static str,
    pub days_left: Option<i64>,
    /// How many times its value was set (the version it is on now). 0 for an entry a file import made: its history
    /// has one summary line for the whole file, not one of its own.
    pub value_version: i64,
    /// Earlier values kept for 7 days.
    pub kept_versions: usize,
    pub created_at: i64,
    pub updated_at: i64,
    /// 0 when it was never used.
    pub last_used_at: i64,
}

fn expiry_state(row: &VaultRow, today: chrono::NaiveDate) -> (&'static str, Option<i64>) {
    let Some(date) = expiry::of(row) else { return ("none", None) };
    let left = (date - today).num_days();
    let state = if expiry::expired(row, today) { "expired" } else if left <= SOON_DAYS { "soon" } else { "ok" };
    (state, Some(left))
}

/// `show`: the entry's kind, label, plain fields, tags, which parts are set, expiry state, Vault, version counts and
/// last use, for an entry in a Vault `agent` may use.
pub fn show(store: &Store, id: &str, agent: Option<&str>, key: &KeyResult, today: chrono::NaiveDate) -> Result<Shown, String> {
    let row = entry_for(store, id, agent)?;
    let kind = kinds::kind(&row.kind).ok_or_else(|| format!("entry {} has an unknown kind", row.id))?;
    let (parts_set, readable) = match key.as_ref().ok().and_then(|k| seal::open(k, &row.id, &row.sealed).ok()) {
        Some(mut parts) => {
            let set = kind.parts.iter().filter(|p| parts.get(p).is_some_and(|v| !v.is_empty())).map(|p| p.to_string()).collect();
            parts.0.values_mut().for_each(|v| v.zeroize());
            (set, true)
        }
        None => (Vec::new(), false),
    };
    let (expiry, days_left) = expiry_state(&row, today);
    let value_version = store.vault_activity_for_entry(&row.id, &VALUE_SETTING, row.created_at, 1).map_err(|e| e.to_string())?.first().map_or(0, |r| r.version);
    let kept_versions = store.vault_versions(&row.id, row.created_at).map_err(|e| e.to_string())?.len();
    let vault = store.vault_info(&row.vault_id).map_err(|e| e.to_string())?.map(|v| v.name).unwrap_or_default();
    Ok(Shown {
        fields: fields_of(&row),
        tags: serde_json::from_str(&row.tags).unwrap_or_default(),
        expires: expiry::of(&row).map(|d| d.to_string()),
        id: row.id,
        kind: row.kind,
        label: row.label,
        vault,
        parts_set,
        readable,
        expiry,
        days_left,
        value_version,
        kept_versions,
        created_at: row.created_at,
        updated_at: row.updated_at,
        last_used_at: row.last_used_at,
    })
}

/// What an agent asks `update` for: a new label, plain fields (an empty value removes one) and a new set of tags.
/// Never a value: that is `rotate`, `generate`, `import` with `id` or `capture`.
#[derive(Debug, Deserialize)]
pub struct UpdateRequest {
    pub id: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub fields: BTreeMap<String, String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Updated {
    pub id: String,
    pub label: String,
    /// What changed, as the history line says it: `label`, `field notes`, `expires 2027-01-31`, `tags`.
    pub changed: Vec<String>,
}

/// Fields that say where a saved sign-in is offered (the site, the realm, which browser profile, a proxy): the
/// person's to set, like choosing a sign-in Vault. An agent that could change them could point a login's password at
/// a site of its own.
fn person_only(field: &str) -> bool {
    [signins::SITE, signins::REALM, signins::PROFILE, signins::PROXY].contains(&field)
}

/// Tags AgentMux itself sets to keep a sign-in from being offered (the site refused it, or it was made on a sign-up
/// and not yet sent): an agent can neither add nor clear them.
const RESERVED_TAGS: [&str; 2] = [signins::REFUSED, signups::UNCONFIRMED];

/// The tags after `asked` replaces `now`: the reserved ones the entry has stay, and asking for one it lacks is refused.
fn apply_tags(now: &[String], asked: &[String]) -> Result<Vec<String>, String> {
    if let Some(tag) = asked.iter().find(|t| RESERVED_TAGS.contains(&t.as_str()) && !now.contains(t)) {
        return Err(format!("the tag {tag:?} is set by AgentMux, not by an agent"));
    }
    let mut tags: Vec<String> = asked.iter().filter(|t| !RESERVED_TAGS.contains(&t.as_str())).cloned().collect();
    tags.extend(now.iter().filter(|t| RESERVED_TAGS.contains(&t.as_str())).cloned());
    Ok(tags)
}

/// The fields after `asked` is applied to `now`: a value is trimmed, an empty one removes the field.
fn apply_fields(kind: &kinds::Kind, now: &BTreeMap<String, String>, asked: &BTreeMap<String, String>) -> Result<BTreeMap<String, String>, String> {
    let mut fields = now.clone();
    for (name, value) in asked {
        if person_only(name) {
            return Err(format!("{name} decides where a saved sign-in is offered: the person sets it in the Vault pane"));
        }
        if !kind.fields.contains(&name.as_str()) {
            return Err(format!("a {} entry has no field {name:?}", kind.name));
        }
        match value.trim() {
            "" => fields.remove(name),
            v => fields.insert(name.clone(), v.to_string()),
        };
    }
    Ok(fields)
}

/// What differs between an entry's details before and after, named the way its history line names it. A field's value
/// is never in it, except an expiry date (no secret, and the person is told of it anyway).
fn what_changed(row: &VaultRow, label: &str, fields: &BTreeMap<String, String>, tags: &[String]) -> Vec<String> {
    let before = fields_of(row);
    let mut out = Vec::new();
    if label != row.label {
        out.push("label".to_string());
    }
    for name in before.keys().chain(fields.keys()).collect::<std::collections::BTreeSet<_>>() {
        if before.get(name) == fields.get(name) {
            continue;
        }
        out.push(match (name.as_str(), fields.get(name)) {
            (expiry::FIELD, Some(date)) => format!("expires {date}"),
            (expiry::FIELD, None) => "no expiry date".to_string(),
            _ => format!("field {name}"),
        });
    }
    let old_tags: Vec<String> = serde_json::from_str(&row.tags).unwrap_or_default();
    if old_tags != tags {
        out.push("tags".to_string());
    }
    out
}

/// `update`: changes an entry's label, plain fields and tags, with the agent's reason, for an entry in a Vault it may
/// use. The sealed value is not opened or touched. Records one `changed details` line.
pub fn update_details(store: &Store, req: &UpdateRequest, agent: &str) -> Result<Updated, String> {
    let reason = history::require_reason(req.reason.as_deref())?;
    let row = entry_for(store, &req.id, Some(agent))?;
    let kind = kinds::kind(&row.kind).ok_or_else(|| format!("entry {} has an unknown kind", row.id))?;
    let label = req.label.as_deref().map(str::trim).filter(|l| !l.is_empty()).unwrap_or(&row.label).to_string();
    let fields = apply_fields(kind, &fields_of(&row), &req.fields)?;
    let had: Vec<String> = serde_json::from_str(&row.tags).unwrap_or_default();
    let tags = match &req.tags {
        Some(tags) => apply_tags(&had, &tags.iter().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect::<Vec<_>>())?,
        None => had,
    };
    validate(kind, &label, &fields, &tags, &Parts::default())?;
    let changed = what_changed(&row, &label, &fields, &tags);
    if changed.is_empty() {
        return Err("nothing to change: give a new label, a field's new value or tags that differ from what the entry has".into());
    }
    let was = row.sealed.clone();
    let new = VaultRow {
        label: label.clone(),
        fields: serde_json::to_string(&fields).map_err(|e| e.to_string())?,
        tags: serde_json::to_string(&tags).map_err(|e| e.to_string())?,
        updated_at: now_ms(),
        ..row
    };
    if !store.vault_update_kept(&new, &was, None).map_err(|e| e.to_string())? {
        return Err(format!("{label} changed while this was being saved, so nothing was changed: look at it again"));
    }
    let target = changed.join(", ");
    record_line(store, Line { entry_id: &new.id, label: &label, actor: agent, action: CHANGED_DETAILS, target: &target, outcome: "done", reason: &reason });
    Ok(Updated { id: new.id, label, changed })
}

/// One thing `verify` found wrong. `id` is empty for the history as a whole.
#[derive(Debug, Serialize, PartialEq)]
pub struct Problem {
    pub id: String,
    pub problem: String,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Verified {
    /// How many entries were looked at.
    pub checked: usize,
    pub ok: bool,
    pub problems: Vec<Problem>,
}

/// What is wrong with an entry's structure and its sealed row, in fixed words: its kind, its plain fields, its main
/// part, and that the Vault key opens it. Never a value. A removed history line is found by the chain, not here.
fn entry_problems(row: &VaultRow, key: &KeyResult, today: chrono::NaiveDate) -> Vec<String> {
    let Some(kind) = kinds::kind(&row.kind) else { return vec![format!("its kind {:?} isn't one this AgentMux knows", row.kind)] };
    let mut out = Vec::new();
    if row.label.trim().is_empty() {
        out.push("it has no label".to_string());
    }
    for (name, value) in fields_of(row) {
        if !kind.fields.contains(&name.as_str()) {
            out.push(format!("it has a field a {} entry doesn't have: {name}", kind.name));
        } else if name == expiry::FIELD && expiry::parse(&value).is_err() {
            out.push("its expiry date isn't a date (YYYY-MM-DD)".to_string());
        } else if name == signins::PROFILE && !signins::is_profile(&value) {
            out.push("its profile field isn't a browser profile".to_string());
        }
    }
    if let Some(date) = expiry::of(row).filter(|_| expiry::expired(row, today)) {
        out.push(format!("its value expired on {date}: rotate or replace it"));
    }
    out.extend(sealed_problems(row, kind, key));
    out
}

fn sealed_problems(row: &VaultRow, kind: &kinds::Kind, key: &KeyResult) -> Option<String> {
    let key = match key {
        Ok(key) => key,
        Err(_) => return Some("its sealed value can't be checked: the Vault key can't be read".to_string()),
    };
    let Ok(mut parts) = seal::open(key, &row.id, &row.sealed) else {
        return Some("its sealed value doesn't open with this computer's Vault key (it was sealed by another key, or it is damaged)".to_string());
    };
    let main = kind.parts[0];
    let missing = !parts.get(main).is_some_and(|v| !v.is_empty());
    let extra: Vec<String> = parts.0.keys().filter(|p| !kind.parts.contains(&p.as_str())).cloned().collect();
    parts.0.values_mut().for_each(|v| v.zeroize());
    if missing {
        return Some(format!("its {main} isn't set"));
    }
    (!extra.is_empty()).then(|| format!("it has a part a {} entry doesn't have: {}", kind.name, extra.join(", ")))
}

/// `verify`: each entry (the one named, else every entry `agent` may use) and, without an id, the whole history's
/// chain. `key` is the Vault key as read (or why it couldn't be). An expired entry is a problem.
pub fn verify(store: &Store, id: Option<&str>, agent: Option<&str>, key: &KeyResult, today: chrono::NaiveDate) -> Result<Verified, String> {
    let rows = match id.map(str::trim).filter(|i| !i.is_empty()) {
        Some(id) => vec![entry_for(store, id, agent)?],
        None => usable(store, agent)?,
    };
    let mut problems = Vec::new();
    for row in &rows {
        for problem in entry_problems(row, key, today) {
            problems.push(Problem { id: row.id.clone(), problem });
        }
    }
    if id.is_none_or(|i| i.trim().is_empty()) {
        let chain = history::check(store)?;
        if !chain.intact {
            let at = chain.broken_at.map(|s| format!(" at line {s}")).unwrap_or_default();
            problems.push(Problem { id: String::new(), problem: format!("the history's chain doesn't hold{at}: {}", chain.problem) });
        }
    }
    Ok(Verified { checked: rows.len(), ok: problems.is_empty(), problems })
}

#[derive(Debug, Serialize, PartialEq)]
pub struct KeyStore {
    pub readable: bool,
    pub backend: String,
    pub problem: String,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct HistoryState {
    pub intact: bool,
    pub checked: usize,
    pub before_chain: usize,
    pub unchained_after: usize,
    pub head_checked: bool,
    pub problem: String,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Health {
    pub key_store: KeyStore,
    /// Over the Vaults `agent` may use.
    pub entries: usize,
    pub vaults: usize,
    pub expiring_within_14_days: usize,
    pub expired: usize,
    pub unused_for_90_days: usize,
    pub history: HistoryState,
    /// This agent's own, still waiting for the person.
    pub pending_requests: usize,
    pub pending_uses: usize,
    pub schema_revision: usize,
}

/// `health`: the key store, counts over the Vaults `agent` may use, the history chain, this agent's pending asks and
/// the schema revision. No value.
pub fn health(store: &Store, agent: Option<&str>, key: &KeyResult, backend: &str, today: chrono::NaiveDate) -> Result<Health, String> {
    let rows = usable(store, agent)?;
    let vaults = store.vaults().map_err(|e| e.to_string())?.iter().filter(|v| vaults::may_use(v, agent)).count();
    let (mut soon, mut expired, mut unused) = (0, 0, 0);
    let stale_before = now_ms() - UNUSED_DAYS * DAY_MS;
    for row in &rows {
        match expiry_state(row, today).0 {
            "expired" => expired += 1,
            "soon" => soon += 1,
            _ => {}
        }
        if row.last_used_at.max(row.created_at) < stale_before {
            unused += 1;
        }
    }
    let chain = history::check(store)?;
    let me = |a: &str| agent.is_some_and(|me| me == a);
    Ok(Health {
        key_store: KeyStore { readable: key.is_ok(), backend: backend.to_string(), problem: key.as_ref().err().cloned().unwrap_or_default() },
        entries: rows.len(),
        vaults,
        expiring_within_14_days: soon,
        expired,
        unused_for_90_days: unused,
        history: HistoryState {
            intact: chain.intact,
            checked: chain.checked,
            before_chain: chain.before_chain,
            unchained_after: chain.unchained_after,
            head_checked: chain.head_checked,
            problem: chain.problem,
        },
        pending_requests: super::requests::requests().list().iter().filter(|r| me(&r.agent)).count(),
        pending_uses: super::uses::uses().list().iter().filter(|u| me(&u.agent)).count(),
        schema_revision: crate::backend::storage::vault::schema_revision(),
    })
}

#[cfg(test)]
#[path = "inspect_tests.rs"]
mod tests;
