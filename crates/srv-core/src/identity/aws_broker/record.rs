// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What the grants file said when someone last allowed it: the record that makes a change to the file visible and
//! keeps a new or wider grant from working until it is acknowledged.
//!
//! The broker keeps one small file beside the grants file (`aws-grants.record.json`): the hash of the grants file as
//! last read and, for each grant, a summary (tier, agents, switched off, reason needed, limits and a fingerprint of the
//! recipe, never the recipe's text or any credential), what was last acknowledged, and which recorded change put the
//! grant there. Reading the grants file after a change compares it with that record:
//! - a grant that is new, or wider than what was acknowledged (a write tier where there was a read tier, more agents,
//!   a different recipe, switched back on, no reason needed, longer sessions, a higher hourly cap), is **held**: the
//!   broker refuses it until a native agent or a person allows it, with a reason that is recorded;
//! - a grant that is removed or narrower takes effect at once;
//! - every added, removed, widened or narrowed grant is an event the caller records in the Audit view;
//! - a file whose grants are unchanged records nothing.
//!
//! The first time the broker reads a file it has no record of, it records that file as the baseline: every grant in it
//! counts as acknowledged, and the Audit view says so. This is pure: reading and writing the record file are the
//! functions at the bottom.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::policy::{Grant, Hold, Mint, Policy, Tier};

const VERSION: u32 = 1;
/// How many changes the record keeps, newest last.
const KEEP_CHANGES: usize = 200;
/// Who a grant of the baseline is acknowledged by.
pub const BASELINE_BY: &str = "baseline";
/// The longest reason that is kept.
pub const MAX_REASON: usize = 300;

/// What a grant allows, as far as a change to it can widen it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Summary {
    pub tier: String,
    /// Lower-case agent slugs, sorted; `*` is everyone.
    pub agents: Vec<String>,
    pub disabled: bool,
    pub needs_reason: bool,
    pub max_seconds: u64,
    pub max_mints_per_hour: u32,
    /// A short fingerprint of how the credentials are made (the profile or the command), so a changed recipe is seen
    /// without the record holding the recipe.
    pub recipe: String,
}

impl Summary {
    pub fn of(g: &Grant) -> Summary {
        let mut agents = g.agents.clone();
        agents.sort();
        agents.dedup();
        let mut hash = Sha256::new();
        match &g.mint {
            Mint::HostProfile { profile } => {
                hash.update(b"profile\0");
                hash.update(profile.as_bytes());
            }
            Mint::Command { argv } => {
                hash.update(b"command\0");
                for arg in argv {
                    hash.update(arg.as_bytes());
                    hash.update([0u8]);
                }
            }
        }
        // A grant that takes a caller's tag is another recipe when its key or prefix changes. One that takes none hashes as it
        // always did, so a record made before this stays valid.
        if let Some(t) = &g.caller_tag {
            hash.update(b"caller_tag\0");
            hash.update(t.key.as_bytes());
            hash.update([0u8]);
            hash.update(t.prefix.as_bytes());
        }
        Summary {
            tier: g.tier.as_str().to_string(),
            agents,
            disabled: g.disabled,
            needs_reason: g.needs_reason,
            max_seconds: g.max_seconds,
            max_mints_per_hour: g.max_mints_per_hour,
            recipe: hex::encode(&hash.finalize()[..8]),
        }
    }

    fn is_write(&self) -> bool {
        self.tier == Tier::Write.as_str()
    }
}

/// How a grant now compares with what was acknowledged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delta {
    Same,
    Narrower,
    /// Wider, and in what.
    Wider(Vec<&'static str>),
}

pub fn compare(acknowledged: &Summary, now: &Summary) -> Delta {
    if acknowledged == now {
        return Delta::Same;
    }
    let mut wider = Vec::new();
    if now.is_write() && !acknowledged.is_write() {
        wider.push("tier");
    }
    let everyone = |s: &Summary| s.agents.iter().any(|a| a == "*");
    if !everyone(acknowledged) && (everyone(now) || now.agents.iter().any(|a| !acknowledged.agents.contains(a))) {
        wider.push("agents");
    }
    if now.recipe != acknowledged.recipe {
        wider.push("recipe");
    }
    if acknowledged.disabled && !now.disabled {
        wider.push("switched on");
    }
    if acknowledged.needs_reason && !now.needs_reason {
        wider.push("no reason needed");
    }
    if now.max_seconds > acknowledged.max_seconds {
        wider.push("session length");
    }
    if now.max_mints_per_hour > acknowledged.max_mints_per_hour {
        wider.push("hourly cap");
    }
    if wider.is_empty() {
        Delta::Narrower
    } else {
        Delta::Wider(wider)
    }
}

/// What the broker does with a grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Honoured,
    /// New or wider than what was acknowledged: refused until it is allowed.
    Held,
    /// Someone said it must not be used: refused, and not asked about again until the file changes again.
    Refused,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::Honoured => "honoured",
            State::Held => "held",
            State::Refused => "refused",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// What the grants file says now.
    pub current: Summary,
    /// What was last allowed (by the baseline, a narrowing, or `decide`); none for a grant never allowed.
    pub acknowledged: Option<Summary>,
    /// A `current` that was refused.
    pub refused: Option<Summary>,
    pub added_ms: i64,
    /// The change that added the grant, and the last change to it.
    pub added_change: String,
    pub change: String,
    pub allowed_by: Option<String>,
    pub allowed_reason: Option<String>,
    pub allowed_ms: Option<i64>,
}

impl Entry {
    pub fn state(&self) -> State {
        if self.acknowledged.as_ref() == Some(&self.current) {
            State::Honoured
        } else if self.refused.as_ref() == Some(&self.current) {
            State::Refused
        } else {
            State::Held
        }
    }
}

/// One line of what changed, as the Identity surface shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeRow {
    pub id: String,
    pub at_ms: i64,
    pub kind: String,
    pub grant: Option<String>,
    pub text: String,
    pub by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub version: u32,
    /// The hash of the grants file's text as last read.
    pub file_sha256: String,
    pub default_grant: Option<String>,
    pub grants: BTreeMap<String, Entry>,
    pub changes: Vec<ChangeRow>,
}

/// Something the Audit view should show. `held` marks the ones a person should look at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub id: String,
    pub kind: &'static str,
    pub grant: Option<String>,
    pub text: String,
    pub by: Option<String>,
    /// A new or wider grant is waiting: the row is recorded as a failure so the view marks it.
    pub held: bool,
}

pub fn sha256_hex(text: &str) -> String {
    hex::encode(Sha256::digest(text.as_bytes()))
}

fn agents_text(s: &Summary) -> String {
    match s.agents.as_slice() {
        [] => "no agent".to_string(),
        [one] if one == "*" => "every agent".to_string(),
        [one] => format!("agent {one}"),
        many => format!("{} agents ({})", many.len(), many.join(", ")),
    }
}

/// The events of one `sync`, with the ids they are given: `c<time in hex>-<n>`.
struct Log {
    now_ms: i64,
    seq: usize,
    events: Vec<Event>,
}

impl Log {
    /// Records an event and returns its id, which the grant it is about keeps as its last change.
    fn push(&mut self, kind: &'static str, grant: Option<&str>, text: String, held: bool) -> String {
        self.seq += 1;
        let id = format!("c{:x}-{}", self.now_ms, self.seq);
        self.events.push(Event { id: id.clone(), kind, grant: grant.map(str::to_string), text, by: None, held });
        id
    }
}

/// Compare the grants file `policy` (with the hash of its text) with what was recorded, and say what changed. `old`
/// is the record read from disk: none on the first run, which records the file as the baseline.
pub fn sync(old: Option<Record>, policy: &Policy, file_sha256: &str, now_ms: i64) -> (Record, Vec<Event>) {
    let mut log = Log { now_ms, seq: 0, events: Vec::new() };
    let Some(mut rec) = old else {
        let rec = baseline(policy, file_sha256, &mut log);
        return finish(rec, log.events, now_ms);
    };
    if rec.file_sha256 == file_sha256 {
        return (rec, log.events);
    }
    rec.file_sha256 = file_sha256.to_string();
    for (name, g) in &policy.grants {
        sync_grant(&mut rec, name, Summary::of(g), &mut log);
    }
    let gone: Vec<String> = rec.grants.keys().filter(|n| !policy.grants.contains_key(*n)).cloned().collect();
    for name in gone {
        rec.grants.remove(&name);
        log.push("removed", Some(&name), format!("grant '{name}' was removed: it stopped working at once"), false);
    }
    if rec.default_grant != policy.default_grant {
        rec.default_grant = policy.default_grant.clone();
        let text = match &rec.default_grant {
            Some(d) => format!("the default grant is now '{d}'"),
            None => "the grants file no longer names a default grant".to_string(),
        };
        log.push("default", None, text, false);
    }
    finish(rec, log.events, now_ms)
}

/// The record of a file there was none of: everything in it counts as allowed, and the Audit view says so.
fn baseline(policy: &Policy, file_sha256: &str, log: &mut Log) -> Record {
    let mut grants = BTreeMap::new();
    let mut writes = 0;
    for (name, g) in &policy.grants {
        let s = Summary::of(g);
        writes += usize::from(s.is_write());
        let entry = Entry {
            current: s.clone(),
            acknowledged: Some(s),
            refused: None,
            added_ms: log.now_ms,
            added_change: String::new(),
            change: String::new(),
            allowed_by: Some(BASELINE_BY.to_string()),
            allowed_reason: None,
            allowed_ms: Some(log.now_ms),
        };
        grants.insert(name.clone(), entry);
    }
    let text = format!("no earlier record of the grants file: this version is the baseline, {} grants ({writes} write) counted as allowed", grants.len());
    let id = log.push("baseline", None, text, false);
    for e in grants.values_mut() {
        e.added_change = id.clone();
        e.change = id.clone();
    }
    Record { version: VERSION, file_sha256: file_sha256.to_string(), default_grant: policy.default_grant.clone(), grants, changes: Vec::new() }
}

/// One grant of the file against the record: new, unchanged, or changed.
fn sync_grant(rec: &mut Record, name: &str, now: Summary, log: &mut Log) {
    let Some(entry) = rec.grants.get_mut(name) else {
        add_grant(rec, name, now, log);
        return;
    };
    if entry.current == now {
        return;
    }
    let delta = entry.acknowledged.as_ref().map(|a| compare(a, &now));
    let id = match delta {
        Some(Delta::Same) => {
            entry.refused = None;
            log.push("reverted", Some(name), format!("grant '{name}' is back to what was allowed"), false)
        }
        Some(Delta::Narrower) => {
            entry.acknowledged = Some(now.clone());
            entry.refused = None;
            entry.allowed_by = Some("narrowed".to_string());
            entry.allowed_reason = None;
            entry.allowed_ms = Some(log.now_ms);
            let text = format!("grant '{name}' is narrower now ({} tier, {}): in force at once", now.tier, agents_text(&now));
            log.push("narrowed", Some(name), text, false)
        }
        Some(Delta::Wider(dims)) => {
            let text = format!("grant '{name}' is wider than what was allowed ({}): held until a native agent or a person allows it", dims.join(", "));
            log.push("widened", Some(name), text, true)
        }
        // Never allowed (added, and edited again before anyone looked): still held.
        None => {
            let text = format!("grant '{name}' changed again before it was allowed ({} tier, {}): still held", now.tier, agents_text(&now));
            log.push("widened", Some(name), text, true)
        }
    };
    entry.current = now;
    entry.change = id;
}

/// A grant the record has not seen. One that is switched off can't be used, so it waits for nothing; switching it on
/// later is a widening.
fn add_grant(rec: &mut Record, name: &str, now: Summary, log: &mut Log) {
    let inert = now.disabled;
    let tail = if inert { " (switched off)" } else { "; held until a native agent or a person allows it" };
    let id = log.push("added", Some(name), format!("added {} grant '{name}' for {}{tail}", now.tier, agents_text(&now)), !inert);
    let entry = Entry {
        current: now.clone(),
        acknowledged: inert.then(|| now.clone()),
        refused: None,
        added_ms: log.now_ms,
        added_change: id.clone(),
        change: id,
        allowed_by: inert.then(|| "switched off".to_string()),
        allowed_reason: None,
        allowed_ms: inert.then_some(log.now_ms),
    };
    rec.grants.insert(name.to_string(), entry);
}

fn finish(mut rec: Record, events: Vec<Event>, now_ms: i64) -> (Record, Vec<Event>) {
    for e in &events {
        rec.changes.push(ChangeRow { id: e.id.clone(), at_ms: now_ms, kind: e.kind.to_string(), grant: e.grant.clone(), text: e.text.clone(), by: e.by.clone() });
    }
    let extra = rec.changes.len().saturating_sub(KEEP_CHANGES);
    rec.changes.drain(..extra);
    (rec, events)
}

/// The grants as the broker honours them: a held or refused grant is marked, so it is refused whatever else is true.
pub fn effective(policy: &Policy, rec: &Record) -> Policy {
    let mut out = policy.clone();
    for (name, g) in out.grants.iter_mut() {
        g.hold = match rec.grants.get(name).map(Entry::state) {
            Some(State::Honoured) => None,
            Some(State::Refused) => Some(Hold::Refused),
            // A grant the record does not know is as good as a new one.
            Some(State::Held) | None => Some(Hold::Awaiting),
        };
    }
    out
}

/// Every grant held: what the broker does when the record can't be read, so a damaged record never lets a grant through.
pub fn hold_all(policy: &Policy) -> Policy {
    let mut out = policy.clone();
    for g in out.grants.values_mut() {
        g.hold = Some(Hold::Awaiting);
    }
    out
}

/// An answer to a held grant.
pub struct Decision<'a> {
    pub grant: &'a str,
    /// The change the answerer saw (`change` in the list): an answer to a grant that has changed since is refused.
    pub change: &'a str,
    pub allow: bool,
    pub by: &'a str,
    pub reason: &'a str,
}

pub fn decide(rec: &mut Record, d: &Decision<'_>, now_ms: i64) -> Result<Event, String> {
    let reason: String = d.reason.chars().filter(|c| !c.is_control()).take(MAX_REASON).collect();
    let reason = reason.trim().to_string();
    if reason.is_empty() {
        return Err("a reason is needed: what the grant is for and why it may be used".to_string());
    }
    let entry = rec.grants.get_mut(d.grant).ok_or_else(|| format!("no grant named '{}' in the record", d.grant))?;
    if entry.change != d.change {
        return Err(format!("grant '{}' has changed since you read it (now change {}): read the grants again", d.grant, entry.change));
    }
    let id = format!("c{now_ms:x}-d");
    let state = entry.state();
    if d.allow {
        if state == State::Honoured {
            return Err(format!("nothing waits on grant '{}': it is already allowed", d.grant));
        }
        entry.acknowledged = Some(entry.current.clone());
        entry.refused = None;
        entry.allowed_by = Some(d.by.to_string());
        entry.allowed_reason = Some(reason.clone());
        entry.allowed_ms = Some(now_ms);
    } else {
        if state == State::Refused {
            return Err(format!("grant '{}' is already refused", d.grant));
        }
        if state == State::Honoured {
            return Err(format!("grant '{}' is in force, not waiting: remove it from the grants file to stop it", d.grant));
        }
        entry.refused = Some(entry.current.clone());
    }
    entry.change = id.clone();
    let (kind, verb) = if d.allow { ("allowed", "allowed") } else { ("refused", "refused") };
    let event = Event { id, kind, grant: Some(d.grant.to_string()), text: format!("grant '{}' {verb} by {}: {reason}", d.grant, d.by), by: Some(d.by.to_string()), held: false };
    rec.changes.push(ChangeRow { id: event.id.clone(), at_ms: now_ms, kind: kind.to_string(), grant: event.grant.clone(), text: event.text.clone(), by: event.by.clone() });
    let extra = rec.changes.len().saturating_sub(KEEP_CHANGES);
    rec.changes.drain(..extra);
    Ok(event)
}

/// One grant as the Identity surface lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GrantView {
    pub name: String,
    pub tier: String,
    pub agents: Vec<String>,
    pub disabled: bool,
    pub state: &'static str,
    pub added_ms: i64,
    pub added_change: String,
    pub change: String,
    pub allowed_by: Option<String>,
    pub allowed_reason: Option<String>,
    pub allowed_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct View {
    pub grants: Vec<GrantView>,
    /// The most recent changes, newest first.
    pub changes: Vec<ChangeRow>,
}

pub fn view(rec: &Record) -> View {
    let grants = rec
        .grants
        .iter()
        .map(|(name, e)| GrantView {
            name: name.clone(),
            tier: e.current.tier.clone(),
            agents: e.current.agents.clone(),
            disabled: e.current.disabled,
            state: e.state().as_str(),
            added_ms: e.added_ms,
            added_change: e.added_change.clone(),
            change: e.change.clone(),
            allowed_by: e.allowed_by.clone(),
            allowed_reason: e.allowed_reason.clone(),
            allowed_ms: e.allowed_ms,
        })
        .collect();
    View { grants, changes: rec.changes.iter().rev().take(50).cloned().collect() }
}

/// `aws-grants.record.json` beside the grants file (the same stem, so a test's own file has its own record).
pub fn path_beside(policy_file: &Path) -> PathBuf {
    let stem = policy_file.file_stem().and_then(|s| s.to_str()).unwrap_or("aws-grants");
    policy_file.with_file_name(format!("{stem}.record.json"))
}

/// The record on disk: none when there is no file (the first run), an error when it is there and can't be read.
pub fn read(path: &Path) -> Result<Option<Record>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("can't read {}: {e}", path.display())),
    };
    let rec: Record = serde_json::from_str(&text).map_err(|e| format!("{} doesn't parse: {e}", path.display()))?;
    if rec.version != VERSION {
        return Err(format!("{} is version {}: this AgentMux reads version {VERSION}", path.display(), rec.version));
    }
    Ok(Some(rec))
}

/// Writes the record in one step, so a reader never sees half of it.
pub fn write(path: &Path, rec: &Record) -> Result<(), String> {
    let text = serde_json::to_string_pretty(rec).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("can't write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("can't replace {}: {e}", path.display()))
}

#[cfg(test)]
#[path = "record_tests.rs"]
mod tests;
