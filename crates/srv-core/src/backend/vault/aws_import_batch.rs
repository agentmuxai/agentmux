// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Vault` `import_batch`: an agent has the Vault copy a named list of secrets from AWS Secrets Manager under one
//! approval (docs/specs/SPEC_VAULT_LINKED_ENTRIES_FLEET_VAULT_AND_BATCH_IMPORT_2026_11_15.md §4).
//!
//! This is the part with no server in it: every check of the list before the person is asked, the rows the person
//! approved, and the batch's own line in the history. Each secret is read and kept as `import` keeps one
//! ([`super::aws_import`]), so a value's way from the CLI to the sealed row is that one, and no value is in an answer,
//! an error or a history row here either. An item marked `link` is not read at all: it becomes a linked entry
//! ([`super::linked`]), which records where the key lives.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::aws_import::{self, ImportRequest, Imported, Plan, Source, Target, Via, SERVICE};
use super::{history, kinds, linked, record_row, vaults, Parts};
use crate::backend::storage::store::Store;
use crate::backend::storage::vault::VaultActivityRow;
use crate::identity::aws_broker::mint::Runner;

/// The Activity action of the batch's own line.
pub const IMPORTED_BATCH: &str = "imported a batch";
/// The most rows one approval shows.
pub const MAX_ITEMS: usize = 40;
/// What a batch's reference starts with, in the refs of its own line and of every line of its items.
pub const BATCH_REF: &str = "batch:";
/// How long a batch's reads may take together; a row not reached by then says so and the rest of the answer is kept.
pub const BUDGET: Duration = Duration::from_secs(300);
const MAX_NAME: usize = 60;
/// How much of the line the labels the person left out may take.
const LEFT_OUT_KEPT: usize = 300;
const ONE_PLACE: &str = "one batch, one account and region: every secret is read under the batch's source (profile and region), so split the list";
const LINK_IS_NEW: &str = "link: true makes a new entry that records where the key lives: leave id out, or leave link out to replace that entry's value";
/// The kind of a linked entry whose item names none.
const LINK_KIND: &str = "api-key";
const OUT_OF_TIME: &str = "not reached: the batch ran out of time before this secret was read; import it again";

/// What the whole batch is read under: one account and region.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct BatchSource {
    #[serde(default)]
    pub region: Option<String>,
    /// As `import`'s: a container agent's AWS grant, or a profile on this computer.
    #[serde(default)]
    pub profile: Option<String>,
}

/// One secret of a batch, and the entry it becomes.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct BatchItem {
    #[serde(default)]
    pub aws_secret_id: String,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub fields: BTreeMap<String, String>,
    /// Replace this entry's main value instead of making a new entry.
    #[serde(default)]
    pub id: Option<String>,
    /// Make a linked entry: record where the key lives and who reads it (`fields`: readers, rotated, copies, expires),
    /// and read no value.
    #[serde(default)]
    pub link: bool,
    /// The batch's own, when given at all: an item can't name another region, profile or Vault.
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub vault: Option<String>,
}

/// What an agent asks `import_batch` for.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct BatchRequest {
    /// A short name for the list, as the person reads it.
    #[serde(default)]
    pub batch: String,
    /// The one Vault every item goes into, by name or id; the agent's first when left out.
    #[serde(default)]
    pub vault: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub source: BatchSource,
    #[serde(default)]
    pub items: Vec<BatchItem>,
    #[serde(default)]
    pub refs: Vec<String>,
}

/// A batch that has been checked: every item is a checked import, all under one account and region and into one Vault.
#[derive(Debug, Clone, PartialEq)]
pub struct BatchPlan {
    /// This batch's own id, which the history lines of its items carry.
    pub id: String,
    pub name: String,
    pub vault_id: String,
    pub vault_name: String,
    pub reason: String,
    pub region: Option<String>,
    pub profile: Option<String>,
    /// The batch's reference first, then the agent's own.
    pub refs: Vec<String>,
    pub items: Vec<Plan>,
}

/// The rows of a batch the person approved, in the list's order, and the labels of the ones they left out.
#[derive(Debug, Clone, PartialEq)]
pub struct Approved {
    pub items: Vec<Plan>,
    pub left_out: Vec<String>,
    /// The fingerprint of the approved list: what was read is that list and nothing else.
    pub list: String,
}

/// What became of one approved row: the entry's id, or why nothing was kept. Never a value.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ItemResult {
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn named(s: &Option<String>) -> Option<&str> {
    s.as_deref().map(str::trim).filter(|v| !v.is_empty())
}

/// "1 secret", "7 secrets".
pub fn secrets(n: usize) -> String {
    format!("{n} secret{}", if n == 1 { "" } else { "s" })
}

fn batch_name(raw: &str) -> Result<String, String> {
    let name = history::one_line(raw, usize::MAX);
    match name.chars().count() {
        0 => Err("import_batch needs batch: a short name for the list, as the person will read it (host passwords)".into()),
        n if n > MAX_NAME => Err(format!("batch is a short name of at most {MAX_NAME} characters: yours is {n}")),
        _ => Ok(name),
    }
}

/// What an item is called in a refusal: its label, else the entry it replaces, else its secret.
fn called(item: &BatchItem) -> String {
    let name = [item.label.as_str(), item.id.as_deref().unwrap_or(""), item.aws_secret_id.as_str()].into_iter().map(str::trim).find(|s| !s.is_empty()).unwrap_or("unnamed");
    history::one_line(name, MAX_NAME)
}

/// The region and account an ARN names (`arn:aws:secretsmanager:<region>:<account>:secret:<name>`); none for a name.
fn arn_place(id: &str) -> Option<(&str, &str)> {
    let mut parts = id.splitn(7, ':');
    if parts.next()? != "arn" {
        return None;
    }
    let (_partition, _service, region, account) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    Some((region, account))
}

/// Every check of a batch, before the person is asked: the name, the size, the one account and region, the one Vault,
/// and each item as `import` checks one. The first thing wrong refuses the whole batch, naming the item.
pub fn check(store: &Store, req: &BatchRequest, agent: &str) -> Result<BatchPlan, String> {
    let reason = history::require_reason(req.reason.as_deref())?;
    let name = batch_name(&req.batch)?;
    match req.items.len() {
        0 => return Err("import_batch needs items: the secrets to copy, each with aws_secret_id, kind and label (or id, to replace an entry's value)".into()),
        n if n > MAX_ITEMS => return Err(format!("a batch is at most {MAX_ITEMS} items, so the person can read every row: this one has {n}; split it")),
        _ => {}
    }
    let (region, profile) = aws_import::check_place(&req.source.region, &req.source.profile)?;
    let vault = match named(&req.vault) {
        Some(which) => vaults::resolve(store, which, Some(Some(agent)))?,
        None => vaults::resolve(store, &vaults::default_for(store, Some(agent))?, Some(Some(agent)))?,
    };
    let id: String = uuid::Uuid::new_v4().simple().to_string().chars().take(12).collect();
    let refs = std::iter::once(format!("{BATCH_REF}{id}")).chain(req.refs.iter().cloned()).collect();
    let mut plan = BatchPlan { id, name, vault_id: vault.id, vault_name: vault.name, reason, region, profile, refs, items: Vec::new() };
    for (i, item) in req.items.iter().enumerate() {
        let checked = plan.item(store, item, agent).map_err(|why| format!("item {} ({}): {why}", i + 1, called(item)))?;
        plan.items.push(checked);
    }
    plan.one_of_each()?;
    plan.one_account()?;
    Ok(plan)
}

impl BatchPlan {
    fn one_vault(&self) -> String {
        format!("one batch, one Vault: this batch goes into {}, and this item is for another", self.vault_name)
    }

    /// One item, checked as `import` checks one, under the batch's source and into the batch's Vault.
    fn item(&self, store: &Store, item: &BatchItem, agent: &str) -> Result<Plan, String> {
        let another = |own: &Option<String>, batch: &Option<String>| named(own).is_some_and(|v| Some(v) != batch.as_deref());
        if another(&item.region, &self.region) || another(&item.profile, &self.profile) {
            return Err(ONE_PLACE.into());
        }
        if let Some(which) = named(&item.vault) {
            if vaults::resolve(store, which, Some(Some(agent)))?.id != self.vault_id {
                return Err(self.one_vault());
            }
        }
        if item.link {
            return self.link_item(item);
        }
        let request = ImportRequest {
            source: Source { aws_secret_id: item.aws_secret_id.clone(), key: item.key.clone(), region: self.region.clone(), profile: self.profile.clone() },
            kind: item.kind.clone(),
            label: item.label.clone(),
            fields: item.fields.clone(),
            vault: Some(self.vault_id.clone()),
            id: item.id.clone(),
            reason: Some(self.reason.clone()),
            refs: self.refs.clone(),
        };
        let plan = aws_import::check(store, &request, agent)?;
        match &plan.target {
            Target::Replace { vault_id, .. } if *vault_id != self.vault_id => Err(self.one_vault()),
            _ => Ok(plan),
        }
    }

    /// A link item, checked: a new entry of a kind (an API key when the item names none) that holds no value, with
    /// the secret as its place under the batch's region and profile.
    fn link_item(&self, item: &BatchItem) -> Result<Plan, String> {
        if named(&item.id).is_some() {
            return Err(LINK_IS_NEW.into());
        }
        let source = aws_import::check_source(&Source { aws_secret_id: item.aws_secret_id.clone(), key: item.key.clone(), region: self.region.clone(), profile: self.profile.clone() })?;
        let kind = kinds::kind(named(&Some(item.kind.clone())).unwrap_or(LINK_KIND)).ok_or_else(|| {
            let names: Vec<&str> = kinds::KINDS.iter().map(|k| k.name).collect();
            format!("a link is an entry of a kind, {LINK_KIND} when left out: one of {}", names.join(", "))
        })?;
        let label = item.label.trim().to_string();
        let fields = linked::fields_for(&source, &item.fields);
        super::validate(kind, &label, &fields, &[], &Parts::default())?;
        let target = Target::Link { kind: kind.name, label, fields, vault_id: self.vault_id.clone() };
        Ok(Plan { source, target, vault_name: self.vault_name.clone(), reason: self.reason.clone(), refs: self.refs.clone() })
    }

    /// No entry twice and no secret (with its key) twice: a row the person ticks is one copy into one entry.
    fn one_of_each(&self) -> Result<(), String> {
        let (mut labels, mut copied) = (HashSet::new(), HashSet::new());
        for item in &self.items {
            if !labels.insert(item.label().trim().to_lowercase()) {
                return Err(format!("two items are for {}: every row needs its own entry", item.label()));
            }
            if !copied.insert(item.from_text()) {
                return Err(format!("two items copy {}: name a secret, and a key of it, once", item.from_text()));
            }
        }
        Ok(())
    }

    /// Secrets named by ARN are all in one account and region, the batch's own region when it names one.
    fn one_account(&self) -> Result<(), String> {
        let places: HashSet<(&str, &str)> = self.items.iter().filter_map(|p| arn_place(&p.source.aws_secret_id)).collect();
        let another_region = self.region.as_deref().is_some_and(|r| places.iter().any(|(region, _)| *region != r));
        if places.len() > 1 || another_region {
            return Err(ONE_PLACE.into());
        }
        Ok(())
    }

    /// The rows the person left ticked (`rows`, by their place in the list), as what runs. A place the list doesn't
    /// have names nothing.
    pub fn approved(&self, rows: &[usize]) -> Approved {
        let (mut items, mut left_out) = (Vec::new(), Vec::new());
        for (i, item) in self.items.iter().enumerate() {
            if rows.contains(&i) {
                items.push(item.clone());
            } else {
                left_out.push(item.label().to_string());
            }
        }
        let list = self.fingerprint(&items);
        Approved { items, left_out, list }
    }

    /// SHA-256 over the batch and the approved rows (each field with its length in front), cut to 16 hex characters.
    fn fingerprint(&self, items: &[Plan]) -> String {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        let mut put = |text: &str| {
            h.update((text.len() as u64).to_le_bytes());
            h.update(text.as_bytes());
        };
        for text in [&self.id, &self.name, &self.vault_id, &self.reason] {
            put(text);
        }
        put(self.region.as_deref().unwrap_or(""));
        put(self.profile.as_deref().unwrap_or(""));
        for item in items {
            put(&item.from_text());
            put(item.kind());
            put(item.label());
            put(item.entry_id());
            put(if item.is_link() { "link" } else { "copy" });
        }
        hex::encode(h.finalize()).chars().take(16).collect()
    }

    /// What the batch asked for, for the line of one that ran nothing: "7 secrets from AWS Secrets Manager".
    pub fn asked_text(&self) -> String {
        format!("{} from {SERVICE}", secrets(self.items.len()))
    }

    /// The outcome and the text of the line of a batch that ran: how many rows were approved, stored and failed, the
    /// approved list's fingerprint, and the rows the person left out.
    pub fn ran_text(&self, approved: &Approved, results: &[ItemResult]) -> (&'static str, String) {
        let stored = results.iter().filter(|r| r.id.is_some()).count();
        let failed = results.len() - stored;
        let outcome = match (stored, failed) {
            (_, 0) => "done",
            (0, _) => "failed",
            _ => "partly done",
        };
        let mut text = format!("{} of {} approved, {stored} stored, {failed} failed; list {}", approved.items.len(), self.items.len(), approved.list);
        if !approved.left_out.is_empty() {
            text.push_str(&format!("; not approved: {}", listed(&approved.left_out)));
        }
        (outcome, text)
    }
}

/// `labels` in a line: as many as fit, then how many more.
fn listed(labels: &[String]) -> String {
    let mut out = String::new();
    for (i, label) in labels.iter().enumerate() {
        let label = history::one_line(label, 40);
        if out.chars().count() + label.chars().count() > LEFT_OUT_KEPT {
            return format!("{out} and {} more", labels.len() - i);
        }
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(&label);
    }
    out
}

/// The batch's own line (`done`, `partly done`, `failed`, `denied`, `expired`): who, the list's name, what became of
/// it, why. It names the batch in its refs, as every line of its items does. No value.
pub fn note(store: &Store, plan: &BatchPlan, agent: &str, outcome: &str, what: &str) {
    record_row(
        store,
        VaultActivityRow {
            entry_label: plan.name.clone(),
            actor: agent.into(),
            action: IMPORTED_BATCH.into(),
            target: what.into(),
            outcome: outcome.into(),
            reason: plan.reason.clone(),
            refs: history::refs_text(&plan.refs),
            ..Default::default()
        },
    );
}

/// One approved row: a link is made without a read, and any other row is read and kept as `import` does one.
async fn perform_one<R: Runner>(store: &Arc<Store>, runner: &R, plan: &Plan, via: Via<'_>, agent: &str) -> Result<Imported, String> {
    if !plan.is_link() {
        return aws_import::perform(store, runner, plan, via, agent).await;
    }
    let (store, plan, agent) = (Arc::clone(store), plan.clone(), agent.to_string());
    tokio::task::spawn_blocking(move || linked::make(&store, &plan, &agent)).await.map_err(|e| e.to_string())?
}

/// Reads and keeps each approved row in turn, as `import` does one. A row that fails is recorded as failed and the
/// rest go on: nothing is undone, since each entry is its own row with its own history line.
pub async fn perform<R: Runner>(store: &Arc<Store>, runner: &R, items: &[Plan], via: Via<'_>, agent: &str, budget: Duration) -> Vec<ItemResult> {
    let started = Instant::now();
    let mut results = Vec::with_capacity(items.len());
    for plan in items {
        let done = if started.elapsed() >= budget { Err(OUT_OF_TIME.to_string()) } else { perform_one(store, runner, plan, via, agent).await };
        results.push(match done {
            Ok(done) => ItemResult { label: done.label, id: Some(done.id), error: None },
            Err(why) => {
                let note = if plan.is_link() { linked::note } else { aws_import::note };
                note(store, plan, plan.entry_id(), plan.label(), agent, "failed");
                ItemResult { label: plan.label().to_string(), id: None, error: Some(why) }
            }
        });
    }
    results
}

#[cfg(test)]
#[path = "aws_import_batch_tests.rs"]
mod tests;
