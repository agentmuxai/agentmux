// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Vault` `import`: an agent has the Vault copy a secret from AWS Secrets Manager, so the person never types it and
//! the agent never sees it (docs/specs/SPEC_VAULT_PARITY_ENTRY_HISTORY_REASONS_AND_IMPORT_2026_11_06.md §7).
//!
//! This is the part with no server in it: checking what the agent asked for (before anything is asked of the person),
//! reading the secret with the host's AWS CLI, picking the value out and keeping it. The value lives in `Zeroizing`
//! buffers from the CLI's stdout to the sealed row. The CLI's stdout is never logged, kept or put in an error; the
//! error sentences are fixed ones, with at most a cleaned first line of the CLI's stderr (AWS's own words, which hold
//! no value).

use std::collections::BTreeMap;
use std::sync::Arc;

use agentmux_common::time::now_ms;
use serde::Deserialize;
use zeroize::{Zeroize, Zeroizing};

use super::{history, kinds, record_row, seal, validate, vaults, versions, NewEntry, Parts};
use crate::backend::storage::store::Store;
use crate::backend::storage::vault::{VaultActivityRow, VaultRow};
use crate::identity::aws_broker::mint::{login_lapsed, Minted, RunError, RunSpec, Runner, RECIPE_TIMEOUT};
use crate::identity::aws_broker::policy::Policy;

/// The Activity action, and what a version replaced by an import is marked with.
pub const IMPORTED: &str = "imported";
/// The service the card names.
pub const SERVICE: &str = "AWS Secrets Manager";
const MAX_SECRET_ID: usize = 512;
const MAX_KEY: usize = 200;
const MAX_PROFILE: usize = 128;
/// How much of AWS's own error line an unknown failure shows.
const SAID_KEPT: usize = 200;

/// Where the secret is: its id (or ARN), the field of a JSON secret to take, and how to reach AWS.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct Source {
    pub aws_secret_id: String,
    /// A field of a JSON secret, as a dotted path (`db.password`); the whole secret when left out.
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub region: Option<String>,
    /// A container agent: the AWS grant to read under. A native agent: the host's AWS profile (its default when left out).
    #[serde(default)]
    pub profile: Option<String>,
}

/// What an agent asks `import` for.
#[derive(Debug, Clone, Deserialize)]
pub struct ImportRequest {
    pub source: Source,
    /// The kind of the new entry; not needed with `id`.
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub fields: BTreeMap<String, String>,
    /// The Vault for a new entry, by name or id; the agent's first when left out.
    #[serde(default)]
    pub vault: Option<String>,
    /// Replace this entry's main value instead of making a new entry.
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    /// Task numbers, PRs and the like, kept with the history row.
    #[serde(default)]
    pub refs: Vec<String>,
}

/// Where the value goes.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    New { kind: &'static str, part: &'static str, label: String, fields: BTreeMap<String, String>, vault_id: String },
    Replace { id: String, created_at: i64, kind: &'static str, part: &'static str, label: String, vault_id: String },
}

/// An import that has been checked: nothing in it needs the store again, except to put the value in.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub source: Source,
    pub target: Target,
    pub vault_name: String,
    pub reason: String,
    pub refs: Vec<String>,
}

impl Plan {
    /// The Activity line's target: `aws-sm:<id>` and `#<key>` when a field is picked.
    pub fn from_text(&self) -> String {
        match &self.source.key {
            Some(key) => format!("aws-sm:{}#{key}", self.source.aws_secret_id),
            None => format!("aws-sm:{}", self.source.aws_secret_id),
        }
    }

    pub fn label(&self) -> &str {
        match &self.target {
            Target::New { label, .. } | Target::Replace { label, .. } => label,
        }
    }

    /// The entry replaced, if the import replaces one.
    pub fn entry_id(&self) -> &str {
        match &self.target {
            Target::New { .. } => "",
            Target::Replace { id, .. } => id,
        }
    }

    pub fn kind(&self) -> &'static str {
        match &self.target {
            Target::New { kind, .. } | Target::Replace { kind, .. } => kind,
        }
    }
}

fn plain(s: &str, max: usize, allowed: impl Fn(char) -> bool) -> bool {
    !s.is_empty() && s.chars().count() <= max && !s.starts_with('-') && s.chars().all(allowed)
}

/// An AWS region's name: `us-east-1`, `eu-central-2`, `us-gov-west-1`.
fn is_region(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    s.len() <= 25
        && parts.len() >= 3
        && parts[0].len() == 2
        && parts.iter().take(parts.len() - 1).all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_lowercase()))
        && parts.last().is_some_and(|n| !n.is_empty() && n.len() <= 2 && n.bytes().all(|b| b.is_ascii_digit()))
}

/// `source` with its optional parts trimmed, or why it can't be used. Narrow character sets: the id and key go to the
/// CLI as arguments and to the person's card, so nothing else is let through.
pub fn check_source(source: &Source) -> Result<Source, String> {
    let clean = |s: &Option<String>| s.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(str::to_string);
    let id = source.aws_secret_id.trim();
    if !plain(id, MAX_SECRET_ID, |c| c.is_ascii_alphanumeric() || "/_+=.@:-".contains(c)) {
        return Err("aws_secret_id is the secret's name or ARN: letters, digits and /_+=.@:- only, at most 512 characters".into());
    }
    let key = clean(&source.key);
    if let Some(k) = &key {
        let dotted = k.split('.').all(|seg| plain(seg, MAX_KEY, |c| c.is_ascii_alphanumeric() || c == '_' || c == '-')) && k.chars().count() <= MAX_KEY;
        if !dotted {
            return Err("key is a field of a JSON secret, as a dotted path (db.password): letters, digits, _ and - only".into());
        }
    }
    let region = clean(&source.region);
    if region.as_deref().is_some_and(|r| !is_region(r)) {
        return Err("region is an AWS region such as us-east-1".into());
    }
    let profile = clean(&source.profile);
    if profile.as_deref().is_some_and(|p| !plain(p, MAX_PROFILE, |c| c.is_ascii_alphanumeric() || "_.@:/+=-".contains(c))) {
        return Err("profile is the name of an AWS grant (a container agent) or of a profile on this computer (a native one)".into());
    }
    Ok(Source { aws_secret_id: id.to_string(), key, region, profile })
}

/// Every check of an import, before the person is asked: the reason, the source, and the entry or Vault the value goes
/// in, for an agent that may use only the Vaults it is let into.
pub fn check(store: &Store, req: &ImportRequest, agent: &str) -> Result<Plan, String> {
    let reason = history::require_reason(req.reason.as_deref())?;
    let source = check_source(&req.source)?;
    let refs = req.refs.iter().map(|r| r.trim().to_string()).filter(|r| !r.is_empty()).collect();
    let id = req.id.as_deref().map(str::trim).filter(|i| !i.is_empty());
    let (target, vault_id) = match id {
        Some(id) => replace_target(store, id, &req.kind, agent)?,
        None => new_target(store, req, agent)?,
    };
    let vault_name = store.vault_info(&vault_id).map_err(|e| e.to_string())?.map(|v| v.name).unwrap_or_default();
    Ok(Plan { source, target, vault_name, reason, refs })
}

fn replace_target(store: &Store, id: &str, kind: &str, agent: &str) -> Result<(Target, String), String> {
    let no_entry = || format!("no entry {id}");
    let row = store.vault_get(id).map_err(|e| e.to_string())?.ok_or_else(no_entry)?;
    if !vaults::entry_row_usable(store, &row.vault_id, Some(agent))? {
        return Err(no_entry());
    }
    let info = kinds::kind(&row.kind).ok_or_else(|| format!("entry {id} has an unknown kind"))?;
    if !kind.trim().is_empty() && kind.trim() != row.kind {
        return Err(format!("{id} is a {} entry: leave kind out to replace its value", row.kind));
    }
    let vault_id = row.vault_id.clone();
    Ok((Target::Replace { id: row.id, created_at: row.created_at, kind: info.name, part: info.parts[0], label: row.label, vault_id: vault_id.clone() }, vault_id))
}

fn new_target(store: &Store, req: &ImportRequest, agent: &str) -> Result<(Target, String), String> {
    let kind = kinds::kind(&req.kind).ok_or_else(|| {
        let names: Vec<&str> = kinds::KINDS.iter().map(|k| k.name).collect();
        format!("import makes a new entry of a kind: one of {}, or give id to replace an entry's value", names.join(", "))
    })?;
    let label = req.label.trim().to_string();
    let fields: BTreeMap<String, String> = req.fields.iter().map(|(k, v)| (k.clone(), v.trim().to_string())).filter(|(_, v)| !v.is_empty()).collect();
    validate(kind, &label, &fields, &[], &Parts::default())?;
    let vault_id = match req.vault.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        Some(which) => vaults::resolve(store, which, Some(Some(agent)))?.id,
        None => vaults::default_for(store, Some(agent))?,
    };
    Ok((Target::New { kind: kind.name, part: kind.parts[0], label, fields, vault_id: vault_id.clone() }, vault_id))
}

/// Whether a container agent may read under the grant `profile`, before the person is asked (the broker checks again
/// when it mints): the grant exists, is on, and is open to the agent.
pub fn check_grant(policy: &Result<Option<Arc<Policy>>, String>, profile: Option<&str>, slug: &str) -> Result<(), String> {
    let Some(profile) = profile else {
        return Err("a container agent names the AWS grant to read under: source.profile (aws configure list-profiles lists yours)".into());
    };
    let policy = match policy {
        Ok(Some(p)) if !p.grants.is_empty() => p,
        Ok(_) => return Err("AWS isn't shared with container agents on this machine, so there is nothing to read with".into()),
        Err(_) => return Err("the AWS grants file on this machine doesn't parse, so AWS can't be shared until a person fixes it".into()),
    };
    let Some(grant) = policy.grants.get(profile) else {
        return Err(format!("No AWS grant named '{profile}'. This machine shares: {}.", policy.names().join(", ")));
    };
    if grant.disabled {
        return Err(format!("The grant '{profile}' is switched off on this machine."));
    }
    if !grant.open_to(slug) {
        return Err(format!("The grant '{profile}' isn't open to you. Ask the person to add you to it."));
    }
    Ok(())
}

/// How the CLI is given its credentials.
pub enum Via<'a> {
    /// The host's own AWS setup (a native agent), under `profile` if one is named.
    Host,
    /// A broker session minted for a container agent's grant.
    Session(&'a Minted),
}

fn argv(source: &Source, via: &Via<'_>) -> Vec<String> {
    let mut args: Vec<String> = ["secretsmanager", "get-secret-value", "--secret-id", source.aws_secret_id.as_str(), "--query", "SecretString", "--output", "json"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    if let Some(region) = &source.region {
        args.extend(["--region".to_string(), region.clone()]);
    }
    if let (Via::Host, Some(profile)) = (via, &source.profile) {
        args.extend(["--profile".to_string(), profile.clone()]);
    }
    args
}

/// Runs the AWS CLI's Secrets Manager read and returns the secret's text. The CLI's stdout is the secret: it is wiped
/// when this returns, and nothing of it goes into an error.
pub async fn read_secret<R: Runner>(runner: &R, source: &Source, via: Via<'_>) -> Result<Zeroizing<String>, String> {
    let Some(program) = runner.which("aws") else {
        return Err("this computer has no AWS CLI (v2.9 or newer), which an import reads with".into());
    };
    let mut env = vec![("AWS_PAGER".to_string(), String::new())];
    if let Via::Session(s) = &via {
        env.push(("AWS_ACCESS_KEY_ID".into(), s.access_key_id.to_string()));
        env.push(("AWS_SECRET_ACCESS_KEY".into(), s.secret_access_key.to_string()));
        env.push(("AWS_SESSION_TOKEN".into(), s.session_token.to_string()));
    }
    let spec = RunSpec { program, args: argv(source, &via), env, timeout: RECIPE_TIMEOUT };
    let out = match runner.run(spec).await {
        Ok(out) => out,
        Err(RunError::TimedOut) => return Err("AWS took too long to answer (25 s). Try again in a minute.".into()),
        Err(RunError::Spawn(_)) => return Err("couldn't run the AWS CLI on this computer".into()),
    };
    let stdout = Zeroizing::new(out.stdout);
    if !out.success {
        return Err(aws_said(&out.stderr, source));
    }
    // `null` is a secret kept as binary: SecretString is empty then.
    let text: Option<String> = serde_json::from_slice(stdout.as_slice()).map_err(|_| "AWS answered with something that isn't a secret's text".to_string())?;
    let text = Zeroizing::new(text.ok_or("that secret is binary, not text: import copies text secrets only")?);
    Ok(text)
}

/// 12-digit runs (account numbers) cut out of a line of AWS's.
fn without_accounts(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut digits = String::new();
    for c in line.chars().chain(std::iter::once(' ')) {
        if c.is_ascii_digit() {
            digits.push(c);
            continue;
        }
        out.push_str(if digits.len() >= 12 { "…" } else { &digits });
        digits.clear();
        out.push(c);
    }
    out.pop();
    out
}

/// What went wrong, from the CLI's stderr, in a sentence the agent is told.
fn aws_said(stderr: &str, source: &Source) -> String {
    let s = stderr.to_lowercase();
    if s.contains("accessdenied") || s.contains("not authorized") || s.contains("explicit deny") {
        "AWS refused: this identity can't read that secret".into()
    } else if s.contains("resourcenotfound") || s.contains("can't find the specified secret") {
        "AWS has no secret by that id, in that region".into()
    } else if s.contains("decryptionfailure") || (s.contains("kms") && s.contains("denied")) {
        "AWS refused: this identity can't decrypt that secret".into()
    } else if login_lapsed(stderr) {
        let profile = source.profile.as_deref().map(|p| format!(" --profile {p}")).unwrap_or_default();
        format!("The host's AWS login has lapsed. A person runs `aws sso login{profile}` on this computer.")
    } else if s.contains("unable to locate credentials") || (s.contains("could not be found") && s.contains("profile")) {
        "there is no AWS login on this computer for that profile".into()
    } else if s.contains("could not connect") || s.contains("could not resolve") || s.contains("timed out") {
        "couldn't reach AWS from this computer".into()
    } else {
        let first = stderr.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("it exited with an error and said nothing");
        let cut: String = without_accounts(first).chars().take(SAID_KEPT).collect();
        format!("AWS said: {cut}")
    }
}

/// Overwrites every string in `v`.
fn wipe(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::String(s) => s.zeroize(),
        serde_json::Value::Array(items) => items.iter_mut().for_each(wipe),
        serde_json::Value::Object(map) => map.iter_mut().for_each(|(_, item)| wipe(item)),
        _ => {}
    }
}

/// The string or number at the dotted `key` in `json`.
fn at_key(json: &serde_json::Value, key: &str) -> Result<Zeroizing<String>, String> {
    let mut at = json;
    for step in key.split('.') {
        at = at.get(step).ok_or_else(|| format!("that secret has no key {key}"))?;
    }
    match at {
        serde_json::Value::String(s) => Ok(Zeroizing::new(s.clone())),
        serde_json::Value::Number(n) => Ok(Zeroizing::new(n.to_string())),
        _ => Err(format!("{key} is not a string or a number, which is all a Vault entry holds")),
    }
}

/// The value to keep: the whole secret, or the string or number at `key` in a JSON one. What is parsed and not kept is
/// wiped before it is dropped.
pub fn pick(secret: Zeroizing<String>, key: Option<&str>) -> Result<Zeroizing<String>, String> {
    let value = match key {
        None => secret,
        Some(key) => {
            let mut json: serde_json::Value = serde_json::from_str(&secret).map_err(|_| "that secret isn't JSON, so a key can't be picked from it".to_string())?;
            let picked = at_key(&json, key);
            wipe(&mut json);
            picked?
        }
    };
    if value.trim().is_empty() {
        return Err("that secret is empty".into());
    }
    if value.len() > super::MAX_PART {
        return Err("that secret is longer than 1 MiB, which a Vault entry can't hold".into());
    }
    Ok(value)
}

/// What an import made or changed.
#[derive(Debug, PartialEq, serde::Serialize)]
pub struct Imported {
    pub id: String,
    pub label: String,
    pub vault: String,
}

/// Puts `value` in: a new entry, or the main value of the one named, whose old value is kept 7 days as a version. The
/// `imported` line goes in with the agent's reason and references; it holds no value.
pub fn store_value(store: &Store, plan: &Plan, value: &Zeroizing<String>, agent: &str) -> Result<Imported, String> {
    let (id, label) = match &plan.target {
        Target::New { kind, part, label, fields, vault_id } => {
            let mut parts = Parts::default();
            parts.0.insert(part.to_string(), value.to_string());
            let entry = NewEntry { kind: kind.to_string(), label: label.clone(), fields: fields.clone(), tags: Vec::new(), parts };
            super::insert_unrecorded(store, entry, vault_id)?
        }
        Target::Replace { id, created_at, part, label, .. } => {
            replace_main(store, id, *created_at, part, value, agent)?;
            (id.clone(), label.clone())
        }
    };
    note(store, plan, &id, &label, agent, "done");
    Ok(Imported { id, label, vault: plan.vault_name.clone() })
}

fn replace_main(store: &Store, id: &str, created_at: i64, part: &str, value: &Zeroizing<String>, agent: &str) -> Result<(), String> {
    let key = seal::key(false)?;
    let gone = || "the entry was deleted while the secret was read, so nothing was saved".to_string();
    let row = store.vault_get(id).map_err(|e| e.to_string())?.filter(|r| r.created_at == created_at).ok_or_else(gone)?;
    let mut parts = seal::open(&key, &row.id, &row.sealed)?;
    if let Some(mut old) = parts.0.insert(part.to_string(), value.to_string()) {
        old.zeroize();
    }
    let kept = versions::of(&row, agent, IMPORTED);
    versions::prune(store);
    let was = row.sealed.clone();
    let changed = VaultRow { sealed: seal::seal(&key, &row.id, &parts)?, updated_at: now_ms(), ..row };
    if !store.vault_update_kept(&changed, &was, Some(&kept)).map_err(|e| e.to_string())? {
        return Err(format!("{} changed while the secret was read, so nothing was saved", changed.label));
    }
    Ok(())
}

/// The record of an import's outcome (`done`, `denied`, `failed`): who, from where, why. No value.
pub fn note(store: &Store, plan: &Plan, id: &str, label: &str, agent: &str, outcome: &str) {
    record_row(
        store,
        VaultActivityRow {
            entry_id: id.into(),
            entry_label: label.into(),
            actor: agent.into(),
            action: IMPORTED.into(),
            target: plan.from_text(),
            outcome: outcome.into(),
            reason: plan.reason.clone(),
            refs: history::refs_text(&plan.refs),
            ..Default::default()
        },
    );
}

/// Reads the secret, picks the value and keeps it. The store is written on the blocking pool.
pub async fn perform<R: Runner>(store: &Arc<Store>, runner: &R, plan: &Plan, via: Via<'_>, agent: &str) -> Result<Imported, String> {
    let secret = read_secret(runner, &plan.source, via).await?;
    let value = pick(secret, plan.source.key.as_deref())?;
    let (store, plan, agent) = (Arc::clone(store), plan.clone(), agent.to_string());
    tokio::task::spawn_blocking(move || store_value(&store, &plan, &value, &agent)).await.map_err(|e| e.to_string())?
}

#[cfg(test)]
#[path = "aws_import_tests.rs"]
mod tests;
