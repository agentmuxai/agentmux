// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! An entry's machine copy: where a copy of its value lives outside the Vault (a secret store a service reads, a CI
//! secret), and a check that says whether that copy still equals the entry's value. A first step toward an entry's
//! "backing" (docs/specs/SPEC_VAULT_SHARED_ACROSS_MACHINES_LAN_AND_CLOUD_2026_11_01.md §11.1).
//!
//! The link is plain fields (`source_*`), allowed on every kind and never a value. The check reads the other place
//! with the access the asker already has, compares inside srv, and answers `same`, `different` or `could not read`
//! with why: neither value, nor a length or a hash of one, is in an answer, an error or a history line. A refusal to
//! read is the answer, not something to get around.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use agentmux_common::time::now_ms;
use serde::Serialize;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use super::aws_import::{self, Source, Via};
use super::{history, kinds, linked, record_line, seal, vaults, Line, PERSON};
use crate::backend::storage::store::Store;
use crate::backend::storage::vault::VaultRow;
use crate::identity::aws_broker::mint::Runner;

/// The kind of place the copy is in.
pub const KIND: &str = "source_kind";
/// Where in it: an AWS secret's name or ARN, a repository (`owner/repo`, or `owner/repo@environment`), or free text.
pub const LOCATION: &str = "source_location";
/// The key there: a JSON secret's dotted key, a CI secret's name.
pub const KEY: &str = "source_key";
/// An AWS secret's region and the profile (or a container agent's grant) it is read under.
pub const REGION: &str = "source_region";
pub const PROFILE: &str = "source_profile";
pub const FIELDS: [&str; 5] = [KIND, LOCATION, KEY, REGION, PROFILE];

/// A secret in AWS Secrets Manager: the one kind a check can read.
pub const AWS_SECRET: &str = "aws-secret";
/// A repository's or an environment's Actions secret, which nothing can read back.
pub const GITHUB_SECRET: &str = "github-secret";
/// Anywhere else, as the person describes it.
pub const OTHER: &str = "other";

/// The fields as a KeePass export names them, so a database written from the Vault brings the link back.
pub const TITLES: [(&str, &str); 5] = [
    (KIND, "Machine copy kind"),
    (LOCATION, "Machine copy location"),
    (KEY, "Machine copy key"),
    (REGION, "Machine copy region"),
    (PROFILE, "Machine copy profile"),
];

/// The Activity action of a check.
pub const CHECKED: &str = "checked";
pub const SAME: &str = "same";
pub const DIFFERENT: &str = "different";
pub const COULD_NOT_READ: &str = "could not read";
/// What a check of a linked entry's place finds, from the place's description alone (`linked`).
pub const PRESENT: &str = "present";
pub const CHANGED: &str = "changed since recorded";
pub const MISSING: &str = "missing";

/// How many checks agents get of one entry in an hour, all agents together: a check answers one bit about the sealed
/// value, so it is not something to ask in a loop.
pub const AGENT_CHECKS_PER_HOUR: usize = 6;
const HOUR_MS: i64 = 60 * 60 * 1000;
const MAX_OTHER: usize = 300;

/// Whether `name` is one of the link's fields, which every kind of entry may have.
pub fn is_field(name: &str) -> bool {
    FIELDS.contains(&name)
}

/// Where an entry's machine copy is. Empty parts are not set.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Link {
    pub kind: String,
    pub location: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub key: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub region: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub profile: String,
}

impl Link {
    /// The link in `fields`, if any of its fields is set.
    pub fn of(fields: &BTreeMap<String, String>) -> Option<Link> {
        if !fields.keys().any(|k| is_field(k)) {
            return None;
        }
        let get = |k: &str| fields.get(k).map(|v| v.trim().to_string()).unwrap_or_default();
        Some(Link { kind: get(KIND), location: get(LOCATION), key: get(KEY), region: get(REGION), profile: get(PROFILE) })
    }

    /// The link of an entry's row.
    pub fn of_row(row: &VaultRow) -> Option<Link> {
        Link::of(&serde_json::from_str(&row.fields).unwrap_or_default())
    }

    /// One line for a history row: `aws-secret:staging/app#db.password`.
    pub fn text(&self) -> String {
        if self.key.is_empty() {
            format!("{}:{}", self.kind, self.location)
        } else {
            format!("{}:{}#{}", self.kind, self.location, self.key)
        }
    }

    /// The place as a person reads it, for the question asked before an agent changes it.
    pub fn describe(&self) -> String {
        let mut out = match self.kind.as_str() {
            AWS_SECRET => format!("the AWS secret {}", self.location),
            GITHUB_SECRET => return format!("the GitHub secret {} of {}", self.key, self.location),
            _ => self.location.clone(),
        };
        for (name, value) in [("key", &self.key), ("region", &self.region), ("profile", &self.profile)] {
            if !value.is_empty() {
                out.push_str(&format!(", {name} {value}"));
            }
        }
        out
    }

    /// What an import-style read of it takes, for an AWS secret.
    pub(super) fn aws(&self) -> Option<Source> {
        let some = |s: &String| (!s.is_empty()).then(|| s.clone());
        (self.kind == AWS_SECRET).then(|| Source { aws_secret_id: self.location.clone(), key: some(&self.key), region: some(&self.region), profile: some(&self.profile) })
    }
}

fn one_line(s: &str, max: usize) -> bool {
    !s.is_empty() && s.chars().count() <= max && !s.chars().any(char::is_control)
}

fn name_like(s: &str, extra: &str) -> bool {
    !s.is_empty() && s.len() <= 100 && !s.starts_with('-') && s.chars().all(|c| c.is_ascii_alphanumeric() || extra.contains(c))
}

/// `owner/repo`, or `owner/repo@environment`.
fn is_repository(s: &str) -> bool {
    let (repo, env) = s.split_once('@').map_or((s, None), |(r, e)| (r, Some(e)));
    let mut parts = repo.split('/');
    let named = |p: Option<&str>| p.is_some_and(|p| name_like(p, "-_."));
    named(parts.next()) && named(parts.next()) && parts.next().is_none() && env.is_none_or(|e| name_like(e, "-_."))
}

/// An AWS secret's parts, each held to what the import takes (they go to the AWS CLI as arguments).
fn aws_ok(link: &Link) -> Result<(), String> {
    let ok = |s: Source| aws_import::check_source(&s).is_ok();
    let id = Source { aws_secret_id: link.location.clone(), ..Default::default() };
    if !ok(id.clone()) {
        return Err(format!("{LOCATION} is the AWS secret's name or ARN: letters, digits and /_+=.@:- only"));
    }
    if !link.key.is_empty() && !ok(Source { key: Some(link.key.clone()), ..id.clone() }) {
        return Err(format!("{KEY} is a field of a JSON secret, as a dotted path (db.password): letters, digits, _ and - only"));
    }
    if !link.region.is_empty() && !ok(Source { region: Some(link.region.clone()), ..id.clone() }) {
        return Err(format!("{REGION} is an AWS region such as us-east-1"));
    }
    if !link.profile.is_empty() && !ok(Source { profile: Some(link.profile.clone()), ..id }) {
        return Err(format!("{PROFILE} is the name of an AWS profile on this computer, or of a container agent's AWS grant"));
    }
    Ok(())
}

/// Whether the link in `fields` is one the Vault keeps: a known kind, a place written the way that kind is, and no
/// part the kind has no use for. No link at all is fine.
pub fn validate(fields: &BTreeMap<String, String>) -> Result<(), String> {
    let Some(link) = Link::of(fields) else { return Ok(()) };
    if link.location.is_empty() {
        return Err(format!("a machine copy needs {LOCATION}: where it is"));
    }
    if link.kind != AWS_SECRET && (!link.region.is_empty() || !link.profile.is_empty()) {
        return Err(format!("{REGION} and {PROFILE} are for an {AWS_SECRET} only"));
    }
    match link.kind.as_str() {
        AWS_SECRET => aws_ok(&link),
        GITHUB_SECRET if !is_repository(&link.location) => Err(format!("{LOCATION} is the repository, owner/repo, or owner/repo@environment for an environment's secret")),
        GITHUB_SECRET if !name_like(&link.key, "_") => Err(format!("{KEY} is the secret's name: letters, digits and _")),
        GITHUB_SECRET => Ok(()),
        OTHER if !one_line(&link.location, MAX_OTHER) || (!link.key.is_empty() && !one_line(&link.key, MAX_OTHER)) => {
            Err(format!("{LOCATION} and {KEY} are one line each, at most {MAX_OTHER} characters"))
        }
        OTHER => Ok(()),
        "" => Err(format!("a machine copy needs {KIND}: {AWS_SECRET}, {GITHUB_SECRET} or {OTHER}")),
        other => Err(format!("{other:?} isn't a kind of machine copy: {AWS_SECRET}, {GITHUB_SECRET} or {OTHER}")),
    }
}

/// An update that names a new kind of place takes the parts the new kind has no use for with it, unless the update
/// names them itself: an AWS secret's region and profile when the place becomes any other kind, and every part when
/// the kind is emptied, which takes the link off. Without it an agent that moved a link off AWS was refused over a
/// region the old place had left behind, and had to know to clear it (found in the Linux look check, 2026-11-14).
pub fn settle_kind(fields: &mut BTreeMap<String, String>, asked: &BTreeMap<String, String>) {
    if !asked.contains_key(KIND) {
        return;
    }
    let left_over: &[&str] = match fields.get(KIND).map(String::as_str) {
        None => &[LOCATION, KEY, REGION, PROFILE],
        Some(AWS_SECRET) => &[],
        Some(_) => &[REGION, PROFILE],
    };
    for name in left_over {
        if !asked.contains_key(*name) {
            fields.remove(*name);
        }
    }
}

/// An import that replaces an entry and names no machine copy leaves the entry's own link as it is.
pub fn carry_over(fields: &mut BTreeMap<String, String>, current: &VaultRow) {
    if fields.keys().any(|k| is_field(k)) {
        return;
    }
    let had: BTreeMap<String, String> = serde_json::from_str(&current.fields).unwrap_or_default();
    fields.extend(had.into_iter().filter(|(k, _)| is_field(k)));
}

/// The newest check of the entry's link as it is now: when, and what it found.
#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct VaultSourceCheck {
    #[ts(type = "number")]
    pub at: i64,
    /// `same`, `different`, or `could not read: <why>`.
    pub outcome: String,
}

/// The newest check of `row`'s link, when it is of the entry as it is now. A check of a place the link no longer
/// names says nothing about this one, and neither does a check made before the entry last changed: the change may
/// be the link's region or profile, which the history line does not name (an agent reads the history, and is never
/// told the profile), or the entry's own value, which an older "same" was not compared with.
pub fn last_check(store: &Store, row: &VaultRow, link: &Link) -> Option<VaultSourceCheck> {
    let newest = store.vault_activity_for_entry(&row.id, &[CHECKED], row.created_at, 1).ok()?.into_iter().next()?;
    (newest.target == link.text() && newest.at >= row.updated_at).then_some(VaultSourceCheck { at: newest.at, outcome: newest.outcome })
}

/// The link as `list` gives it to an agent: the place, and the newest check. Which login reads it is left out.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct AgentLink {
    pub kind: String,
    pub location: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked: Option<VaultSourceCheck>,
    /// The entry is a link (`linked`): it holds no value, and the key is only at the place.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub link_only: bool,
    /// Who or what reads the key, as recorded.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub readers: Vec<String>,
    /// When the key was last changed, as recorded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotated: Option<String>,
    /// Other places the same key is kept.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub copies: Vec<String>,
}

/// An entry's plain fields as an agent is given them: all of them but the login its machine copy is read under.
pub fn fields_for_agent(mut fields: BTreeMap<String, String>) -> BTreeMap<String, String> {
    fields.remove(PROFILE);
    fields
}

pub fn for_agent(store: &Store, row: &VaultRow) -> Option<AgentLink> {
    let link = Link::of_row(row)?;
    let checked = last_check(store, row, &link);
    let fields: BTreeMap<String, String> = serde_json::from_str(&row.fields).unwrap_or_default();
    let lines = |name: &str| linked::lines(fields.get(name).map_or("", String::as_str)).into_iter().map(str::to_string).collect();
    Some(AgentLink {
        kind: link.kind,
        location: link.location,
        key: link.key,
        checked,
        link_only: linked::is_linked(&fields),
        readers: lines(linked::READERS),
        rotated: fields.get(linked::ROTATED).cloned(),
        copies: lines(linked::COPIES),
    })
}

/// Agents' checks that are planned and have not written their history line yet, by store and entry. They count
/// toward the hour's limit with the lines already written: counted by the lines alone, checks started together each
/// saw fewer than the limit, and all of them ran.
static RUNNING: Mutex<BTreeMap<(usize, String), usize>> = Mutex::new(BTreeMap::new());

/// An agent's place among an entry's checks of the hour: held by the plan (and its clones) from the moment the check
/// is allowed until the plan is dropped, which is after the check's line is written.
#[derive(Debug, PartialEq)]
pub struct Slot {
    store: usize,
    entry_id: String,
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut running = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
        let key = (self.store, std::mem::take(&mut self.entry_id));
        match running.get_mut(&key) {
            Some(n) if *n > 1 => *n -= 1,
            _ => {
                running.remove(&key);
            }
        }
    }
}

/// A check that has been allowed to run: which entry, which place, for whom.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckPlan {
    pub entry_id: String,
    pub created_at: i64,
    pub label: String,
    /// The entry's main part, which the copy is compared with.
    pub part: &'static str,
    pub link: Link,
    /// The entry is a link: it holds no value, so the check reads the place's description and compares nothing.
    pub only: bool,
    /// The other places the key is kept, which a linked entry's check describes too.
    pub copies: Vec<Link>,
    /// The agent's id, or the person.
    pub actor: String,
    pub reason: String,
    /// An agent's check holds its place in the hour's limit; the person's has none.
    pub slot: Option<Arc<Slot>>,
}

fn plan_of(row: VaultRow, actor: &str, reason: String) -> Result<CheckPlan, String> {
    let kind = kinds::kind(&row.kind).ok_or_else(|| format!("entry {} has an unknown kind", row.id))?;
    let Some(link) = Link::of_row(&row) else {
        return Err(format!("{} has no machine copy recorded: set {KIND}, {LOCATION} and {KEY} on it first", row.label));
    };
    let fields: BTreeMap<String, String> = serde_json::from_str(&row.fields).unwrap_or_default();
    let (only, copies) = (linked::is_linked(&fields), linked::copies(&fields));
    Ok(CheckPlan { entry_id: row.id, created_at: row.created_at, label: row.label, part: kind.parts[0], link, only, copies, actor: actor.to_string(), reason, slot: None })
}

/// The person's check, from the Vault pane.
pub fn plan_for_person(store: &Store, id: &str) -> Result<CheckPlan, String> {
    let row = store.vault_get(id.trim()).map_err(|e| e.to_string())?.ok_or_else(|| format!("no entry {id}"))?;
    plan_of(row, PERSON, String::new())
}

/// An agent's check of an entry in a Vault it may use: its reason is required, and agents together get
/// [`AGENT_CHECKS_PER_HOUR`] checks of one entry in an hour, counting the ones still running. The count and the new
/// check's place are taken under one lock, so checks asked for at the same moment are counted one after the other.
pub fn plan_for_agent(store: &Store, id: &str, agent: &str, reason: Option<&str>) -> Result<CheckPlan, String> {
    let reason = history::require_reason(reason)?;
    let no_entry = || format!("no entry {id}");
    let row = store.vault_get(id.trim()).map_err(|e| e.to_string())?.ok_or_else(no_entry)?;
    if !vaults::entry_row_usable(store, &row.vault_id, Some(agent))? {
        return Err(no_entry());
    }
    let key = (store as *const Store as usize, row.id.clone());
    let slot = {
        let mut running = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
        let recent = store.vault_activity_for_entry(&row.id, &[CHECKED], now_ms() - HOUR_MS, 100).map_err(|e| e.to_string())?;
        let done = recent.iter().filter(|r| r.actor != PERSON).count();
        let now = running.get(&key).copied().unwrap_or(0);
        if done >= AGENT_CHECKS_PER_HOUR {
            return Err(format!("{} was checked {AGENT_CHECKS_PER_HOUR} times by agents in the last hour, which is the limit: its history has what they found", row.label));
        }
        if done + now >= AGENT_CHECKS_PER_HOUR {
            return Err(format!(
                "{} has {done} check(s) by agents in the last hour and {now} more running, which is the limit of {AGENT_CHECKS_PER_HOUR}: its history has what they find",
                row.label
            ));
        }
        *running.entry(key.clone()).or_insert(0) += 1;
        Arc::new(Slot { store: key.0, entry_id: key.1 })
    };
    plan_of(row, agent, reason).map(|plan| CheckPlan { slot: Some(slot), ..plan })
}

/// What a check found.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Same,
    Different,
    /// Why neither could be said: the other place refused, has no such key, or is a kind nothing reads back.
    CouldNotRead(String),
    /// A linked entry's place is there, and did not change after the date the entry records.
    Present,
    /// The place changed after the date the entry records: what it says about that.
    Changed(String),
    /// The place is not there, or is being deleted: which.
    Missing(String),
}

impl Outcome {
    pub fn word(&self) -> &'static str {
        match self {
            Outcome::Same => SAME,
            Outcome::Different => DIFFERENT,
            Outcome::CouldNotRead(_) => COULD_NOT_READ,
            Outcome::Present => PRESENT,
            Outcome::Changed(_) => CHANGED,
            Outcome::Missing(_) => MISSING,
        }
    }

    pub fn why(&self) -> &str {
        match self {
            Outcome::CouldNotRead(why) | Outcome::Changed(why) | Outcome::Missing(why) => why,
            _ => "",
        }
    }

    /// The history line's outcome: the word, and why when there is one to give.
    pub fn line(&self) -> String {
        match self.why() {
            "" => self.word().to_string(),
            why => format!("{}: {why}", self.word()),
        }
    }
}

/// A check's answer: ids, names, the outcome and when. No value.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Checked {
    pub id: String,
    pub label: String,
    /// `same`, `different` or `could not read`; for a linked entry `present`, `changed since recorded`, `missing`
    /// or `could not read`.
    pub outcome: &'static str,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub why: String,
    /// The place that was read, as the history names it.
    pub source: String,
    pub checked_at: i64,
    /// A linked entry: the day the place last changed, as the place says.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_changed: Option<String>,
    /// A linked entry: whether the place rotates the key itself, when the place says.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation: Option<bool>,
    /// A linked entry: what was found at each other place the key is kept.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub copies: Vec<linked::CopyChecked>,
}

impl Checked {
    /// The answer for `plan`'s entry and place, as of now.
    pub fn of(plan: &CheckPlan, outcome: &Outcome) -> Checked {
        Checked {
            id: plan.entry_id.clone(),
            label: plan.label.clone(),
            outcome: outcome.word(),
            why: outcome.why().to_string(),
            source: plan.link.text(),
            checked_at: now_ms(),
            last_changed: None,
            rotation: None,
            copies: Vec::new(),
        }
    }
}

/// Whether two values are equal, in time that does not depend on where they differ: their digests are compared, whole.
fn same(a: &[u8], b: &[u8]) -> bool {
    let (a, b) = (Sha256::digest(a), Sha256::digest(b));
    a.iter().zip(b.iter()).fold(0u8, |diff, (x, y)| diff | (x ^ y)) == 0
}

/// The copy at `link`, read with the asker's access. An AWS secret is the one place read today.
async fn read<R: Runner>(runner: &R, link: &Link, via: Via<'_>) -> Result<Zeroizing<String>, String> {
    let Some(source) = link.aws() else {
        return Err("this kind of place can't be read back from here".into());
    };
    let secret = aws_import::read_secret(runner, &source, via).await?;
    aws_import::pick(secret, source.key.as_deref())
}

/// The entry as it is once the read has come back, when it is still the entry the check was planned for and its
/// link is still the one that was read. A read takes seconds, and the link may change under it: a line written for
/// the old region or profile would count as the newest check of the new one, which the history names the same way.
pub(super) fn as_planned(store: &Store, plan: &CheckPlan) -> Result<VaultRow, String> {
    let Some(row) = store.vault_get(&plan.entry_id).ok().flatten().filter(|r| r.created_at == plan.created_at) else {
        return Err("the entry was deleted while its machine copy was read".into());
    };
    if Link::of_row(&row).as_ref() != Some(&plan.link) {
        return Err("where the entry's machine copy is was changed while it was read, so what was read says nothing about it: check again".into());
    }
    Ok(row)
}

/// The entry's own value against `other`. The entry is opened here and wiped when this returns.
fn compare(row: &VaultRow, plan: &CheckPlan, other: &Zeroizing<String>) -> Outcome {
    let cant = |why: &str| Outcome::CouldNotRead(why.to_string());
    let Ok(key) = seal::key(false) else { return cant("this computer's Vault key can't be read, so the entry's own value can't be opened") };
    let Ok(parts) = seal::open(&key, &row.id, &row.sealed) else { return cant("the entry's own value doesn't open with this computer's Vault key") };
    match parts.get(plan.part).filter(|v| !v.is_empty()) {
        Some(ours) if same(ours.as_bytes(), other.as_bytes()) => Outcome::Same,
        Some(_) => Outcome::Different,
        None => cant("the entry has no value of its own to compare"),
    }
}

/// Records what a check found (or why it did not run) in the entry's history, naming the place, and answers it.
pub fn conclude(store: &Store, plan: &CheckPlan, outcome: &Outcome) -> Checked {
    let source = plan.link.text();
    record_line(store, Line { entry_id: &plan.entry_id, label: &plan.label, actor: &plan.actor, action: CHECKED, target: &source, outcome: &outcome.line(), reason: &plan.reason });
    Checked::of(plan, outcome)
}

/// What a read of the copy came to, compared with the entry's value and recorded.
fn finish(store: &Store, plan: &CheckPlan, other: Result<Zeroizing<String>, String>) -> Checked {
    let outcome = match (as_planned(store, plan), &other) {
        (Err(why), _) => Outcome::CouldNotRead(why),
        (Ok(row), Ok(value)) => compare(&row, plan, value),
        (Ok(_), Err(why)) => Outcome::CouldNotRead(why.clone()),
    };
    conclude(store, plan, &outcome)
}

/// Reads the machine copy, compares it with the entry's value and records the outcome. Never fails: a place that
/// can't be read is an outcome. The store is read and written on the blocking pool.
pub async fn check<R: Runner>(store: &Arc<Store>, runner: &R, plan: &CheckPlan, via: Via<'_>) -> Checked {
    let stopped = Checked::of(plan, &Outcome::CouldNotRead("the check stopped before it finished".into()));
    let (store, plan) = (Arc::clone(store), plan.clone());
    // A linked entry holds no value to compare: its place is described, and nothing of the key is read.
    if plan.only {
        let found = linked::describe(runner, &plan, via).await;
        return tokio::task::spawn_blocking(move || linked::finish(&store, &plan, found)).await.unwrap_or(stopped);
    }
    let other = read(runner, &plan.link, via).await;
    tokio::task::spawn_blocking(move || finish(&store, &plan, other)).await.unwrap_or(stopped)
}

/// [`check`] from a thread of the blocking pool, where the pane's operations run: the read is waited for here.
pub fn check_blocking<R: Runner>(store: &Store, runner: &R, plan: &CheckPlan, via: Via<'_>) -> Result<Checked, String> {
    let runtime = tokio::runtime::Handle::try_current().map_err(|_| "the check can't run here".to_string())?;
    if plan.only {
        return Ok(linked::finish(store, plan, runtime.block_on(linked::describe(runner, plan, via))));
    }
    Ok(finish(store, plan, runtime.block_on(read(runner, &plan.link, via))))
}

#[cfg(test)]
#[path = "source_link_tests.rs"]
mod tests;
